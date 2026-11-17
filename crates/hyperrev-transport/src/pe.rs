//! Bounded PE parsing. RVAs are mapped through raw section data, never used as file offsets.

use anyhow::{Context, Result, bail, ensure};

const MAX_SECTIONS: usize = 96;
const MAX_EXPORTS: usize = 65_536;
const MAX_EXPORT_NAME: usize = 512;
pub(crate) const MAX_PE_HEADER_OFFSET: usize = 1024 * 1024;

pub(crate) fn bytes(data: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    let end = offset.checked_add(len).context("PE byte range overflow")?;
    data.get(offset..end).context("truncated PE byte range")
}

pub(crate) fn u16_at(data: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(bytes(data, offset, 2)?.try_into()?))
}

pub(crate) fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes(data, offset, 4)?.try_into()?))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ImageHeader {
    pub machine: u16,
    pub magic: u16,
    pub timestamp: u32,
    pub image_size: u32,
    pub headers_size: u32,
    pub nt_offset: usize,
    pub optional_size: usize,
    pub section_count: usize,
}

impl ImageHeader {
    pub fn parse(data: &[u8]) -> Result<Self> {
        ensure!(
            bytes(data, 0, 2)? == b"MZ",
            "image has an invalid MZ signature"
        );
        let nt_offset = u32_at(data, 0x3c)? as usize;
        ensure!(
            (0x40..=MAX_PE_HEADER_OFFSET).contains(&nt_offset),
            "PE header offset exceeds bounds"
        );
        ensure!(
            bytes(data, nt_offset, 4)? == b"PE\0\0",
            "image has an invalid PE signature"
        );
        let machine = u16_at(data, nt_offset + 4)?;
        let section_count = u16_at(data, nt_offset + 6)? as usize;
        ensure!(
            (1..=MAX_SECTIONS).contains(&section_count),
            "PE section count exceeds bounds"
        );
        let optional_size = u16_at(data, nt_offset + 20)? as usize;
        let optional = nt_offset + 24;
        let magic = u16_at(data, optional)?;
        let minimum_optional = match (machine, magic) {
            (0x14c, 0x10b) => 96,
            (0x8664, 0x20b) => 112,
            _ => bail!("unsupported PE machine/magic {machine:#x}/{magic:#x}"),
        };
        ensure!(
            (minimum_optional..=4096).contains(&optional_size),
            "PE optional header length exceeds bounds"
        );
        bytes(data, optional, optional_size)?;
        let image_size = u32_at(data, optional + 56)?;
        let headers_size = u32_at(data, optional + 60)?;
        ensure!(
            (0x1000..=0x8000_0000).contains(&image_size),
            "PE image size exceeds bounds"
        );
        let section_end = optional
            .checked_add(optional_size)
            .and_then(|v| v.checked_add(section_count * 40))
            .context("PE section table overflow")?;
        ensure!(
            headers_size as usize >= section_end && headers_size <= image_size,
            "PE SizeOfHeaders does not cover its section table"
        );
        Ok(Self {
            machine,
            magic,
            timestamp: u32_at(data, nt_offset + 8)?,
            image_size,
            headers_size,
            nt_offset,
            optional_size,
            section_count,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

pub(crate) struct Pe<'a> {
    data: &'a [u8],
    pub header: ImageHeader,
    sections: Vec<Section>,
    export_rva: u32,
    export_size: u32,
}

#[derive(Clone, Copy)]
pub(crate) enum LeafKind {
    LoadPointer,
    Address,
}

impl<'a> Pe<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let header = ImageHeader::parse(data)?;
        ensure!(
            header.machine == 0x8664 && header.magic == 0x20b,
            "kernel file must be PE32+ AMD64"
        );
        bytes(data, 0, header.headers_size as usize)?;
        let table = header.nt_offset + 24 + header.optional_size;
        let mut sections = Vec::with_capacity(header.section_count);
        for index in 0..header.section_count {
            let offset = table + index * 40;
            bytes(data, offset, 40)?;
            let section = Section {
                virtual_size: u32_at(data, offset + 8)?,
                virtual_address: u32_at(data, offset + 12)?,
                raw_size: u32_at(data, offset + 16)?,
                raw_offset: u32_at(data, offset + 20)?,
            };
            let virtual_end = section
                .virtual_address
                .checked_add(section.virtual_size.max(section.raw_size))
                .context("PE section virtual range overflow")?;
            ensure!(
                virtual_end <= header.image_size,
                "PE section exceeds SizeOfImage"
            );
            if section.raw_size != 0 {
                bytes(data, section.raw_offset as usize, section.raw_size as usize)?;
            }
            sections.push(section);
        }
        let optional = header.nt_offset + 24;
        ensure!(
            u32_at(data, optional + 108)? >= 1 && header.optional_size >= 120,
            "kernel PE has an absent export directory"
        );
        let export_rva = u32_at(data, optional + 112)?;
        let export_size = u32_at(data, optional + 116)?;
        ensure!(
            export_rva != 0 && export_size >= 40,
            "kernel PE has an empty export directory"
        );
        let export_end = export_rva
            .checked_add(export_size)
            .context("PE export range overflow")?;
        ensure!(
            export_end <= header.image_size,
            "PE exports exceed SizeOfImage"
        );
        let pe = Self {
            data,
            header,
            sections,
            export_rva,
            export_size,
        };
        pe.rva_bytes(export_rva, 40)?;
        Ok(pe)
    }

    fn rva_bytes(&self, rva: u32, len: usize) -> Result<&'a [u8]> {
        let len32 = u32::try_from(len).context("PE RVA request length overflow")?;
        let end = rva.checked_add(len32).context("PE RVA request overflow")?;
        ensure!(end <= self.header.image_size, "PE RVA exceeds SizeOfImage");
        if rva < self.header.headers_size {
            ensure!(
                end <= self.header.headers_size,
                "PE RVA crosses header boundary"
            );
            return bytes(self.data, rva as usize, len);
        }
        let mut found = None;
        for section in &self.sections {
            let section_end = section
                .virtual_address
                .checked_add(section.virtual_size.max(section.raw_size))
                .context("PE section range overflow")?;
            if rva >= section.virtual_address && rva < section_end {
                ensure!(found.is_none(), "overlapping PE section RVA mappings");
                let delta = rva - section.virtual_address;
                ensure!(
                    delta
                        .checked_add(len32)
                        .is_some_and(|v| v <= section.raw_size),
                    "PE RVA is virtual-only or crosses raw section bounds"
                );
                let offset = section
                    .raw_offset
                    .checked_add(delta)
                    .context("PE raw offset overflow")?;
                found = Some(bytes(self.data, offset as usize, len)?);
            }
        }
        found.context("PE RVA has an absent raw mapping")
    }

    fn export_name(&self, rva: u32) -> Result<&'a [u8]> {
        // Check each byte through the mapper: even a missing terminator stays bounded.
        for length in 0..MAX_EXPORT_NAME {
            let next = rva
                .checked_add(length as u32)
                .context("PE export name overflow")?;
            if self.rva_bytes(next, 1)?[0] == 0 {
                return self.rva_bytes(rva, length);
            }
        }
        bail!("PE export name exceeds {MAX_EXPORT_NAME} bytes")
    }

    pub fn export_rva(&self, want: &str) -> Result<u32> {
        let directory = self.rva_bytes(self.export_rva, 40)?;
        let functions_count = u32_at(directory, 20)? as usize;
        let names_count = u32_at(directory, 24)? as usize;
        ensure!(
            (1..=MAX_EXPORTS).contains(&functions_count) && names_count <= MAX_EXPORTS,
            "PE export counts exceed bounds"
        );
        let functions = self.rva_bytes(u32_at(directory, 28)?, functions_count * 4)?;
        let names = self.rva_bytes(u32_at(directory, 32)?, names_count * 4)?;
        let ordinals = self.rva_bytes(u32_at(directory, 36)?, names_count * 2)?;
        for index in 0..names_count {
            if self.export_name(u32_at(names, index * 4)?)? != want.as_bytes() {
                continue;
            }
            let ordinal = u16_at(ordinals, index * 2)? as usize;
            ensure!(
                ordinal < functions_count,
                "PE export ordinal exceeds function table"
            );
            let rva = u32_at(functions, ordinal * 4)?;
            ensure!(
                rva != 0 && rva < self.header.image_size,
                "invalid PE export RVA for {want}"
            );
            ensure!(
                rva < self.export_rva || rva >= self.export_rva + self.export_size,
                "forwarded PE export {want} is unsupported"
            );
            return Ok(rva);
        }
        bail!("kernel export {want} is absent")
    }

    pub fn leaf_displacement(&self, name: &str, kind: LeafKind) -> Result<u64> {
        let rva = self.export_rva(name)?;
        let mut code = self.rva_bytes(rva, 16)?;
        if code.starts_with(&[0xf3, 0x0f, 0x1e, 0xfa]) {
            code = &code[4..]; // CET ENDBR64 is a prefix, never an indirect branch to follow.
        }
        let opcode = match kind {
            LeafKind::LoadPointer => 0x8b,
            LeafKind::Address => 0x8d,
        };
        let displacement =
            if code.get(..3) == Some(&[0x48, opcode, 0x81]) && code.get(7) == Some(&0xc3) {
                u32_at(code, 3)? as u64
            } else if code.get(..3) == Some(&[0x48, opcode, 0x41]) && code.get(4) == Some(&0xc3) {
                let disp = code[3] as i8;
                ensure!(disp > 0, "negative kernel leaf displacement for {name}");
                disp as u64
            } else {
                bail!("unrecognized kernel leaf body for {name}; version offsets stay unguessed")
            };
        ensure!(
            (8..=0x2000).contains(&displacement),
            "kernel field displacement exceeds bounds for {name}"
        );
        if matches!(kind, LeafKind::LoadPointer) {
            ensure!(
                displacement % 8 == 0,
                "unaligned pointer displacement for {name}"
            );
        }
        Ok(displacement)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put16(data: &mut [u8], at: usize, value: u16) {
        data[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn put32(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn fixture() -> Vec<u8> {
        let mut data = vec![0u8; 0x800];
        data[..2].copy_from_slice(b"MZ");
        put32(&mut data, 0x3c, 0x80);
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        put16(&mut data, 0x84, 0x8664);
        put16(&mut data, 0x86, 1);
        put32(&mut data, 0x88, 0x12345678);
        put16(&mut data, 0x94, 0xf0);
        put16(&mut data, 0x98, 0x20b);
        put32(&mut data, 0x98 + 56, 0x2000);
        put32(&mut data, 0x98 + 60, 0x200);
        put32(&mut data, 0x98 + 108, 16);
        put32(&mut data, 0x98 + 112, 0x1000);
        put32(&mut data, 0x98 + 116, 0x100);
        let section = 0x188;
        put32(&mut data, section + 8, 0x600);
        put32(&mut data, section + 12, 0x1000);
        put32(&mut data, section + 16, 0x600);
        put32(&mut data, section + 20, 0x200);
        // Export directory at RVA 0x1000, file offset 0x200.
        put32(&mut data, 0x200 + 20, 1);
        put32(&mut data, 0x200 + 24, 1);
        put32(&mut data, 0x200 + 28, 0x1040);
        put32(&mut data, 0x200 + 32, 0x1044);
        put32(&mut data, 0x200 + 36, 0x1048);
        put32(&mut data, 0x240, 0x1200);
        put32(&mut data, 0x244, 0x1060);
        put16(&mut data, 0x248, 0);
        data[0x260..0x260 + 15].copy_from_slice(b"PsGetProcessId\0");
        data[0x400..0x408].copy_from_slice(&[0x48, 0x8b, 0x81, 0x40, 0x04, 0, 0, 0xc3]);
        data
    }

    #[test]
    fn synthetic_export_maps_rva_through_section() {
        let data = fixture();
        let pe = Pe::parse(&data).unwrap();
        assert_eq!(pe.export_rva("PsGetProcessId").unwrap(), 0x1200);
        assert_eq!(
            pe.leaf_displacement("PsGetProcessId", LeafKind::LoadPointer)
                .unwrap(),
            0x440
        );
        assert!(
            pe.leaf_displacement("PsGetProcessId", LeafKind::Address)
                .is_err()
        );
    }

    #[test]
    fn synthetic_image32_is_accepted_for_remote_images() {
        let mut data = fixture();
        put16(&mut data, 0x84, 0x14c);
        put16(&mut data, 0x98, 0x10b);
        put32(&mut data, 0x98 + 56, 0x01f6f000);
        let image = ImageHeader::parse(&data).unwrap();
        assert_eq!(image.image_size, 0x01f6f000);
        assert!(Pe::parse(&data).is_err());
    }

    #[test]
    fn synthetic_export_rejects_bad_ordinals_forwarders_and_counts() {
        let mut data = fixture();
        put16(&mut data, 0x248, 1);
        assert!(
            Pe::parse(&data)
                .unwrap()
                .export_rva("PsGetProcessId")
                .is_err()
        );
        data = fixture();
        put32(&mut data, 0x240, 0x1060);
        assert!(
            Pe::parse(&data)
                .unwrap()
                .export_rva("PsGetProcessId")
                .is_err()
        );
        data = fixture();
        put32(&mut data, 0x218, u32::MAX);
        assert!(
            Pe::parse(&data)
                .unwrap()
                .export_rva("PsGetProcessId")
                .is_err()
        );
    }

    #[test]
    fn synthetic_parser_rejects_truncation_and_virtual_only_ranges() {
        for length in [0, 2, 0x3f, 0x80, 0x98, 0x200, 0x700] {
            assert!(Pe::parse(&fixture()[..length]).is_err());
        }
        let mut data = fixture();
        put32(&mut data, 0x188 + 16, 0x100);
        assert!(
            Pe::parse(&data)
                .unwrap()
                .leaf_displacement("PsGetProcessId", LeafKind::LoadPointer)
                .is_err()
        );
        data = fixture();
        put32(&mut data, 0x3c, u32::MAX);
        assert!(Pe::parse(&data).is_err());
    }

    #[test]
    fn kernel_leaf_body_is_required_instead_of_guessed_offsets() {
        let mut data = fixture();
        data[0x400] = 0xe9;
        assert!(
            Pe::parse(&data)
                .unwrap()
                .leaf_displacement("PsGetProcessId", LeafKind::LoadPointer)
                .is_err()
        );
        data = fixture();
        data[0x400..0x40c].copy_from_slice(&[
            0xf3, 0x0f, 0x1e, 0xfa, 0x48, 0x8b, 0x81, 0x40, 4, 0, 0, 0xc3,
        ]);
        assert_eq!(
            Pe::parse(&data)
                .unwrap()
                .leaf_displacement("PsGetProcessId", LeafKind::LoadPointer)
                .unwrap(),
            0x440
        );
    }
}
