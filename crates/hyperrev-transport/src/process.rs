//! Read-only native-kernel process metadata, adapted from the pinned Palepale kproc.rs.
//!
//! Its hypercall ABI is deliberately *not* reused. EPROCESS.DirectoryTableBase=0x28 and
//! ActiveProcessLinks=UniqueProcessId+8 are reference-derived hypotheses, checked against
//! System/current-process identity, reciprocal list links, and the selected image's MZ/PE.
//! Other EPROCESS fields come from the actual kernel file's exported leaf instructions.

use std::collections::HashSet;

use anyhow::{Context, Result, ensure};

use crate::pe::{ImageHeader, LeafKind, MAX_PE_HEADER_OFFSET, Pe, u16_at, u32_at};
use crate::{CR3_PHYSICAL_FRAME_MASK, Connection, Memory, canonical, valid_cr3};

/// Resolved native process identity. Image addresses and CR3 retain their full width.
#[derive(Clone, Debug)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub image_base: u64,
    pub image_size: u32,
    pub cr3: u64,
    pub eprocess: u64,
}

/// Find a process using case-insensitive executable basenames, never a built-in game alias.
///
/// Only the supplied names are considered. In particular, Main.exe is never implicitly
/// substituted for PointBlank.exe. Multiple matches are an error rather than an arbitrary
/// selection. Long names may match the kernel's exact 15-byte truncation.
pub fn find_process(connection: &Connection, names: &[&str]) -> Result<Option<Process>> {
    ensure!(
        !names.is_empty() && names.iter().all(|name| !normalize(name).is_empty()),
        "process names must contain a nonempty executable basename"
    );
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        let context = crate::windows::kernel_context(connection)?;
        let records = walk(
            connection,
            connection.current_cr3(),
            &context.layout,
            context.identity,
        )?;
        let candidates: Vec<_> = records
            .into_iter()
            .filter(|record| names.iter().any(|name| name_matches(&record.name, name)))
            .collect();
        ensure!(
            candidates.len() <= 1,
            "multiple processes match {}; select an explicit PID",
            names.join(", ")
        );
        candidates
            .first()
            .map(|record| resolve(connection, record))
            .transpose()
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        let _ = connection;
        anyhow::bail!("native process discovery requires Windows x86-64")
    }
}

/// Resolve an explicit PID using the same checked native-kernel metadata path.
pub fn process_by_pid(connection: &Connection, pid: u32) -> Result<Option<Process>> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        let context = crate::windows::kernel_context(connection)?;
        let records = walk(
            connection,
            connection.current_cr3(),
            &context.layout,
            context.identity,
        )?;
        records
            .iter()
            .find(|record| record.pid == pid)
            .map(|record| resolve(connection, record))
            .transpose()
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        let _ = (connection, pid);
        anyhow::bail!("native process discovery requires Windows x86-64")
    }
}

const MAX_PROCESSES: usize = 65_536;
// Provenance: build/reference/palepale/crates/loader/src/kproc.rs:39-49 in hyper-reV.
// This value is never used to fabricate or decrypt another page-table root.
const REFERENCE_DIRECTORY_TABLE_BASE_OFFSET: u64 = 0x28;

#[derive(Clone, Debug)]
pub(crate) struct KernelLayout {
    pub system_pointer: u64,
    pub pid: u64,
    pub links: u64,
    pub name: u64,
    pub peb: u64,
    pub section_base: Option<u64>,
}

impl KernelLayout {
    pub fn from_kernel(pe: &Pe<'_>, base: u64) -> Result<Self> {
        kernel_pointer(base)?;
        let pid = pe.leaf_displacement("PsGetProcessId", LeafKind::LoadPointer)?;
        let links = pid
            .checked_add(8)
            .context("ActiveProcessLinks field overflow")?;
        let name = pe.leaf_displacement("PsGetProcessImageFileName", LeafKind::Address)?;
        let peb = pe.leaf_displacement("PsGetProcessPeb", LeafKind::LoadPointer)?;
        // If the optional accessor has a different implementation, the exported PEB
        // accessor still supplies the reference-supported native PEB+0x10 image field.
        let section_base = pe
            .leaf_displacement("PsGetProcessSectionBaseAddress", LeafKind::LoadPointer)
            .ok();
        let system_pointer = base
            .checked_add(pe.export_rva("PsInitialSystemProcess")? as u64)
            .context("PsInitialSystemProcess address overflow")?;
        kernel_pointer(system_pointer)?;
        Ok(Self {
            system_pointer,
            pid,
            links,
            name,
            peb,
            section_base,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CurrentIdentity {
    pub pid: u32,
    pub cr3: u64,
    pub peb: u64,
}

#[derive(Clone, Debug)]
struct Record {
    pid: u32,
    name: String,
    cr3: u64,
    eprocess: u64,
    peb: u64,
    section_base: u64,
}

fn kernel_pointer(value: u64) -> Result<()> {
    ensure!(
        canonical(value) && value >> 63 == 1 && value & 7 == 0,
        "invalid aligned kernel pointer {value:#x}"
    );
    Ok(())
}

fn field(base: u64, offset: u64) -> Result<u64> {
    base.checked_add(offset)
        .context("kernel field address overflow")
}

fn read64(memory: &impl Memory, cr3: u64, address: u64) -> Result<u64> {
    let mut value = [0; 8];
    memory
        .read(cr3, address, &mut value)
        .with_context(|| format!("kernel metadata read at {address:#x}"))?;
    Ok(u64::from_le_bytes(value))
}

fn links(memory: &impl Memory, cr3: u64, address: u64) -> Result<(u64, u64)> {
    kernel_pointer(address)?;
    let mut value = [0; 16];
    memory
        .read(cr3, address, &mut value)
        .context("ActiveProcessLinks read")?;
    let flink = u64::from_le_bytes(value[..8].try_into()?);
    let blink = u64::from_le_bytes(value[8..].try_into()?);
    kernel_pointer(flink)?;
    kernel_pointer(blink)?;
    Ok((flink, blink))
}

fn walk(
    memory: &impl Memory,
    kernel_cr3: u64,
    layout: &KernelLayout,
    identity: CurrentIdentity,
) -> Result<Vec<Record>> {
    valid_cr3(kernel_cr3)?;
    let start = read64(memory, kernel_cr3, layout.system_pointer)?;
    kernel_pointer(start)?;
    ensure!(
        read64(memory, kernel_cr3, field(start, layout.pid)?)? == 4,
        "derived UniqueProcessId field failed the PID 4 System check"
    );
    let mut current = start;
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    let mut self_seen = false;
    loop {
        ensure!(
            seen.len() < MAX_PROCESSES,
            "ActiveProcessLinks exceeded {MAX_PROCESSES} entries"
        );
        ensure!(
            seen.insert(current),
            "ActiveProcessLinks contains a premature cycle"
        );
        kernel_pointer(current)?;
        let link_address = field(current, layout.links)?;
        let (flink, blink) = links(memory, kernel_cr3, link_address)?;
        // Read both neighboring LIST_ENTRYs. A guessed pid+8 field or a changing list
        // fails explicitly rather than silently producing a fabricated process record.
        ensure!(
            links(memory, kernel_cr3, flink)?.1 == link_address,
            "ActiveProcessLinks forward/backward reciprocity failed"
        );
        ensure!(
            links(memory, kernel_cr3, blink)?.0 == link_address,
            "ActiveProcessLinks backward/forward reciprocity failed"
        );
        let pid = u32::try_from(read64(memory, kernel_cr3, field(current, layout.pid)?)?)
            .context("native process ID exceeds u32")?;
        let mut raw_name = [0; 15];
        memory.read(kernel_cr3, field(current, layout.name)?, &mut raw_name)?;
        let end = raw_name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(raw_name.len());
        let name = String::from_utf8_lossy(&raw_name[..end]).into_owned();
        let record = Record {
            pid,
            name,
            cr3: read64(
                memory,
                kernel_cr3,
                field(current, REFERENCE_DIRECTORY_TABLE_BASE_OFFSET)?,
            )?,
            eprocess: current,
            peb: read64(memory, kernel_cr3, field(current, layout.peb)?)?,
            section_base: match layout.section_base {
                Some(offset) => read64(memory, kernel_cr3, field(current, offset)?)?,
                None => 0,
            },
        };
        if current == start {
            ensure!(
                record.name.eq_ignore_ascii_case("System"),
                "PID 4 image name validation failed"
            );
            valid_cr3(record.cr3).context("PID 4 DirectoryTableBase validation")?;
        }
        if record.pid == identity.pid {
            ensure!(!self_seen, "duplicate current PID in ActiveProcessLinks");
            valid_cr3(record.cr3)?;
            ensure!(
                record.cr3 & CR3_PHYSICAL_FRAME_MASK == identity.cr3 & CR3_PHYSICAL_FRAME_MASK,
                "reference DirectoryTableBase field disagrees with the current process CR3"
            );
            ensure!(
                identity.peb != 0 && record.peb == identity.peb,
                "derived native PEB field disagrees with the current process PEB"
            );
            self_seen = true;
        }
        ensure!(
            links(memory, kernel_cr3, link_address)? == (flink, blink),
            "ActiveProcessLinks changed during validation; retry discovery"
        );
        result.push(record);
        let next = flink
            .checked_sub(layout.links)
            .context("ActiveProcessLinks subtraction underflow")?;
        if next == start {
            break;
        }
        ensure!(
            !seen.contains(&next),
            "ActiveProcessLinks loops outside its System head"
        );
        current = next;
    }
    ensure!(
        self_seen,
        "current PID was absent from the validated System process list"
    );
    Ok(result)
}

fn normalize(value: &str) -> String {
    value
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase()
}

fn name_matches(observed: &str, requested: &str) -> bool {
    let have = normalize(observed);
    let want = normalize(requested);
    if have.is_empty() || want.is_empty() {
        return false;
    }
    if have == want {
        return true;
    }
    // Exact EPROCESS-width truncation only; shorter accidental prefixes never match.
    have.len() == 15 && want.len() > 15 && want.starts_with(&have)
}

fn remote_header(memory: &impl Memory, cr3: u64, base: u64) -> Result<ImageHeader> {
    ensure!(
        base != 0 && canonical(base) && base & 0xfff == 0,
        "invalid image base {base:#x}"
    );
    let mut dos = [0; 64];
    memory
        .read(cr3, base, &mut dos)
        .context("selected CR3 image MZ read")?;
    ensure!(
        &dos[..2] == b"MZ",
        "selected CR3 image MZ validation failed; DirectoryTableBase may be invalid"
    );
    let nt = u32_at(&dos, 0x3c)? as usize;
    ensure!(
        (0x40..=MAX_PE_HEADER_OFFSET).contains(&nt),
        "selected image PE header offset exceeds bounds"
    );
    let nt_address = base
        .checked_add(nt as u64)
        .context("image PE address overflow")?;
    let mut fixed = [0; 24];
    memory
        .read(cr3, nt_address, &mut fixed)
        .context("selected CR3 image PE read")?;
    ensure!(
        &fixed[..4] == b"PE\0\0",
        "selected CR3 image PE signature validation failed"
    );
    let optional = u16_at(&fixed, 20)? as usize;
    ensure!(
        (96..=4096).contains(&optional),
        "selected image optional header exceeds bounds"
    );
    // Assemble only the checked header pieces; an enormous DOS stub is never allocated/read.
    // ImageHeader::parse requires indexed DOS/NT locations, so cap allocation by the transfer limit.
    let header_len = nt
        .checked_add(24 + optional)
        .context("image header size overflow")?;
    ensure!(
        header_len <= crate::MAX_TRANSFER_SIZE,
        "selected image headers exceed the 1 MiB bound"
    );
    let mut header = vec![0; header_len];
    header[..dos.len()].copy_from_slice(&dos);
    header[nt..nt + 24].copy_from_slice(&fixed);
    memory
        .read(
            cr3,
            nt_address
                .checked_add(24)
                .context("optional-header address overflow")?,
            &mut header[nt + 24..],
        )
        .context("selected image optional-header read")?;
    let result = ImageHeader::parse(&header)?;
    ensure!(
        base.checked_add(result.image_size as u64 - 1)
            .is_some_and(canonical),
        "selected image extent is noncanonical or overflowing"
    );
    ensure!(
        (nt as u64) < result.image_size as u64,
        "selected PE headers exceed image size"
    );
    Ok(result)
}

pub(crate) fn verify_kernel_image(
    memory: &impl Memory,
    cr3: u64,
    base: u64,
    pe: &Pe<'_>,
) -> Result<()> {
    let live = remote_header(memory, cr3, base).context("running kernel PE identity read")?;
    ensure!(
        live.machine == pe.header.machine
            && live.magic == pe.header.magic
            && live.timestamp == pe.header.timestamp
            && live.image_size == pe.header.image_size,
        "running kernel PE identity differs from the SystemDirectory kernel file"
    );
    Ok(())
}

fn resolve(memory: &impl Memory, record: &Record) -> Result<Process> {
    valid_cr3(record.cr3)
        .with_context(|| format!("PID {} DirectoryTableBase validation", record.pid))?;
    let image_base = if record.section_base != 0 {
        record.section_base
    } else {
        ensure!(
            record.peb != 0 && canonical(record.peb) && record.peb >> 63 == 0,
            "PID {} has an invalid native PEB and an absent section base",
            record.pid
        );
        // Provenance: Palepale kproc.rs:441-449, native (64-bit) PEB.ImageBaseAddress.
        read64(memory, record.cr3, field(record.peb, 0x10)?)?
    };
    let header = remote_header(memory, record.cr3, image_base).with_context(|| {
        format!(
            "PID {} CR3 {:#x} image validation failed; page-table root remains unmodified",
            record.pid, record.cr3
        )
    })?;
    Ok(Process {
        pid: record.pid,
        name: record.name.clone(),
        image_base,
        image_size: header.image_size,
        cr3: record.cr3,
        eprocess: record.eprocess,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    struct MockMemory {
        bytes: BTreeMap<(u64, u64), u8>,
        reads: RefCell<Vec<(u64, u64, usize)>>,
    }
    impl MockMemory {
        fn put(&mut self, cr3: u64, address: u64, value: &[u8]) {
            for (index, &byte) in value.iter().enumerate() {
                self.bytes.insert((cr3, address + index as u64), byte);
            }
        }
        fn put64(&mut self, cr3: u64, address: u64, value: u64) {
            self.put(cr3, address, &value.to_le_bytes());
        }
    }
    impl Memory for MockMemory {
        fn read(&self, cr3: u64, address: u64, out: &mut [u8]) -> Result<()> {
            self.reads.borrow_mut().push((cr3, address, out.len()));
            for (index, byte) in out.iter_mut().enumerate() {
                *byte = *self
                    .bytes
                    .get(&(cr3, address + index as u64))
                    .context("unmapped mock byte")?;
            }
            Ok(())
        }
    }

    const LOCAL: u64 = 0x1234_5000;
    const TARGET: u64 = 0x9876_5000;
    const SYSTEM: u64 = 0xffff_8000_0000_1000;
    const CURRENT: u64 = SYSTEM + 0x3000;
    const GAME: u64 = SYSTEM + 0x6000;

    fn setup() -> (MockMemory, KernelLayout, CurrentIdentity) {
        let layout = KernelLayout {
            system_pointer: 0xffff_8000_1000_0000,
            pid: 0x440,
            links: 0x448,
            name: 0x5a8,
            peb: 0x550,
            section_base: Some(0x520),
        };
        let mut memory = MockMemory::default();
        memory.put64(LOCAL, layout.system_pointer, SYSTEM);
        let records = [
            (SYSTEM, 4, "System", 0x7777_0000, 0),
            (CURRENT, 123, "bridge.exe", LOCAL, 0x7ff6_0000_0000),
            (GAME, 456, "PointBlank.exe", TARGET, 0x7ff6_1111_0000),
        ];
        for (index, &(base, pid, name, cr3, peb)) in records.iter().enumerate() {
            memory.put64(LOCAL, base + layout.pid, pid);
            memory.put64(LOCAL, base + 0x28, cr3);
            memory.put64(LOCAL, base + layout.peb, peb);
            memory.put64(
                LOCAL,
                base + layout.section_base.unwrap(),
                if pid == 456 { 0x400000 } else { 0 },
            );
            let mut raw = [0; 15];
            raw[..name.len()].copy_from_slice(name.as_bytes());
            memory.put(LOCAL, base + layout.name, &raw);
            let next = records[(index + 1) % 3].0;
            let previous = records[(index + 2) % 3].0;
            memory.put64(LOCAL, base + layout.links, next + layout.links);
            memory.put64(LOCAL, base + layout.links + 8, previous + layout.links);
        }
        (
            memory,
            layout,
            CurrentIdentity {
                pid: 123,
                cr3: LOCAL,
                peb: 0x7ff6_0000_0000,
            },
        )
    }

    fn image32(memory: &mut MockMemory, cr3: u64, base: u64) {
        let mut header = vec![0; 0x178];
        header[..2].copy_from_slice(b"MZ");
        header[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        header[0x80..0x84].copy_from_slice(b"PE\0\0");
        header[0x84..0x86].copy_from_slice(&0x14cu16.to_le_bytes());
        header[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        header[0x94..0x96].copy_from_slice(&0xe0u16.to_le_bytes());
        header[0x98..0x9a].copy_from_slice(&0x10bu16.to_le_bytes());
        header[0xd0..0xd4].copy_from_slice(&0x01f6f000u32.to_le_bytes());
        header[0xd4..0xd8].copy_from_slice(&0x200u32.to_le_bytes());
        memory.put(cr3, base, &header);
    }

    #[test]
    fn mock_walk_checks_system_current_and_reciprocal_links() {
        let (memory, layout, identity) = setup();
        let records = walk(&memory, LOCAL, &layout, identity).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[2].cr3, TARGET);
        assert_eq!(records[2].name, "PointBlank.exe");
    }

    #[test]
    fn mock_target_image_reads_use_target_cr3_instead_of_bridge_cr3() {
        let (mut memory, layout, identity) = setup();
        image32(&mut memory, TARGET, 0x400000);
        let records = walk(&memory, LOCAL, &layout, identity).unwrap();
        memory.reads.borrow_mut().clear();
        let process = resolve(&memory, &records[2]).unwrap();
        assert_eq!(process.image_base, 0x400000);
        assert_eq!(process.image_size, 0x01f6f000);
        assert_eq!(process.cr3, TARGET);
        assert!(
            memory
                .reads
                .borrow()
                .iter()
                .all(|&(cr3, _, _)| cr3 == TARGET)
        );
    }

    #[test]
    fn mock_peb_fallback_uses_native_peb_and_target_cr3() {
        let (mut memory, mut layout, identity) = setup();
        layout.section_base = None;
        memory.put64(TARGET, 0x7ff6_1111_0010, 0x400000);
        image32(&mut memory, TARGET, 0x400000);
        let records = walk(&memory, LOCAL, &layout, identity).unwrap();
        assert_eq!(resolve(&memory, &records[2]).unwrap().image_base, 0x400000);
    }

    #[test]
    fn mock_rejects_corrupted_links_and_guessed_identity_fields() {
        let (mut memory, layout, identity) = setup();
        memory.put64(LOCAL, GAME + layout.links + 8, SYSTEM + layout.links);
        assert!(walk(&memory, LOCAL, &layout, identity).is_err());
        let (memory, layout, mut identity) = setup();
        identity.cr3 = TARGET;
        assert!(walk(&memory, LOCAL, &layout, identity).is_err());
        let (memory, layout, mut identity) = setup();
        identity.peb += 0x1000;
        assert!(walk(&memory, LOCAL, &layout, identity).is_err());
        let (mut memory, layout, identity) = setup();
        memory.put64(LOCAL, SYSTEM + layout.pid, 5);
        assert!(walk(&memory, LOCAL, &layout, identity).is_err());
    }

    #[test]
    fn mock_invalid_target_mz_is_an_error_without_cr3_substitution() {
        let (mut memory, layout, identity) = setup();
        image32(&mut memory, LOCAL, 0x400000);
        let records = walk(&memory, LOCAL, &layout, identity).unwrap();
        assert!(resolve(&memory, &records[2]).is_err());
    }

    #[test]
    fn names_are_case_insensitive_and_never_alias_main() {
        assert!(name_matches("PointBlank.exe", "pointblank.EXE"));
        assert!(name_matches("PointBlank.exe", r"C:\Games\PointBlank.exe"));
        assert!(!name_matches("Main.exe", "PointBlank.exe"));
        assert!(!name_matches("Point", "PointBlank.exe"));
        assert!(!name_matches("", "PointBlank.exe"));
        assert!(name_matches("ExampleLongGame", "ExampleLongGame.exe"));
    }
}
