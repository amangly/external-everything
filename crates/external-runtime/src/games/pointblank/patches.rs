//! Exact-build, reversible single-byte feature patches over the selected CR3.
//!
//! The host owns process/CR3 lifetime and verifies the complete on-disk SHA-256
//! before calling this module. This controller owns only bytes it wrote. Pending
//! ownership survives failed reads, writes, rollback, and restoration; the host
//! must keep the original memory target alive until restoration succeeds.
//! Evidence: build/pbzptid-offsets/offset-ledger.json and the read-only three-
//! lifetime comparison live-verification-048166b90acf46b7aa83f91865bc815e.json.

use anyhow::{Context, Result, bail, ensure};

use super::{BuildProfile, CURRENT_PROFILE};

/// Complete direct virtual-memory operations in one already selected process.
/// A short or indeterminate operation returns an error, including a write that
/// may already have changed memory. The controller retains ownership accordingly.
pub trait Memory {
    fn read(&self, address: u64, out: &mut [u8]) -> Result<()>;
    fn write(&self, address: u64, bytes: &[u8]) -> Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Requested {
    pub names: bool,
    pub anti_kick: bool,
}

const MAX_GUARD: usize = 64;
const X86_END: u64 = 0x1_0000_0000;
const MIN_BASE: u64 = 0x1_0000;
const PROFILE_ID: &str = "pb_id_20261116";
const PROFILE_SHA256: &str = "d7de2e99c257164ce8022ecf81dd1ea5fe1fa7e9c6ae8bba58bb78c4cb459b46";
const PROFILE_IMAGE_SIZE: u32 = 0x01f7_c000;

#[derive(Clone, Copy, Debug)]
struct Descriptor {
    name: &'static str,
    guard_rva: u32,
    guard: &'static [u8],
    patch_rva: u32,
    offset: usize,
    original: u8,
    replacement: u8,
}

// CGameCharaCollisionContext::UpdateNickOnTarget: reset 18 nickname flags,
// stride eight, beginning at this+0x63d. Change the immediate, never the opcode.
const NAMES_GUARD: [u8; 21] = [
    0x6a, 0x12, 0x81, 0xc1, 0x3d, 0x06, 0x00, 0x00, 0x58, 0xc6, 0x01, 0x00, 0x8d, 0x49, 0x08, 0x83,
    0xe8, 0x01, 0x75, 0xf5, 0xc3,
];
// Ready-room callback event 801: JS -> JMP, retaining its +0x15 displacement.
// This is the recovered specific callback path, rather than every kick source.
const ANTI_KICK_GUARD: [u8; 39] = [
    0x55, 0x8b, 0xec, 0x8b, 0x45, 0x08, 0x56, 0x8b, 0xf1, 0x85, 0xc0, 0x78, 0x15, 0x0f, 0x9f, 0xc0,
    0x0f, 0xb6, 0xc0, 0x50, 0xe8, 0x3a, 0xd9, 0xff, 0xff, 0x6a, 0x08, 0x8b, 0xce, 0xe8, 0x16, 0xdf,
    0xda, 0xff, 0x5e, 0x5d, 0xc2, 0x08, 0x00,
];
const DESCRIPTORS: [Descriptor; 2] = [
    Descriptor {
        name: "NameTag",
        guard_rva: 0x0067_922c,
        guard: &NAMES_GUARD,
        patch_rva: 0x0067_9237,
        offset: 11,
        original: 0x00,
        replacement: 0x01,
    },
    Descriptor {
        name: "AntiKick",
        guard_rva: 0x0073_d4d6,
        guard: &ANTI_KICK_GUARD,
        patch_rva: 0x0073_d4e1,
        offset: 11,
        original: 0x78,
        replacement: 0xeb,
    },
];

#[derive(Clone, Copy, Debug)]
struct Owned {
    original: u8,
    /// True only after the complete guard was observed with our replacement.
    active: bool,
}

#[derive(Clone, Copy, Debug)]
struct Target {
    base: u32,
    profile: BuildProfile,
}

#[derive(Clone, Copy)]
struct Undo {
    index: usize,
    owner: Option<Owned>,
    byte: u8,
}

/// A controller belongs to one host-held process/CR3 lifetime at a time.
/// It deliberately has no automatic Drop writes: its transport is host-owned.
#[derive(Default)]
pub struct Controller {
    target: Option<Target>,
    owned: [Option<Owned>; 2],
}

impl Controller {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a bounded transaction. Failure undoes changes made by this call,
    /// preserving earlier ownership and retaining any incomplete undo for retry.
    pub fn sync(
        &mut self,
        memory: &impl Memory,
        base: u32,
        profile: &BuildProfile,
        requested: Requested,
    ) -> Result<()> {
        admit_profile(profile)?;
        if let Some(target) = self.target {
            ensure!(
                target.base == base,
                "restore owned PointBlank patches before changing the target base"
            );
            admit_profile(&target.profile)?;
        }
        let target = Target {
            base,
            profile: *profile,
        };
        for index in 0..DESCRIPTORS.len() {
            self.mark_uncertain(index);
        }
        validate_target(memory, target)?;
        let desired = [requested.names, requested.anti_kick];
        // Preflight both features before the first mutation. Unrequested,
        // unowned bytes are untouched, including externally enabled patches.
        let mut observed = [None; 2];
        for index in 0..DESCRIPTORS.len() {
            if desired[index] || self.owned[index].is_some() {
                self.mark_uncertain(index);
                let byte = read_guard(memory, target, DESCRIPTORS[index], self.owned[index])?;
                if let Some(owner) = &mut self.owned[index] {
                    owner.active = byte == DESCRIPTORS[index].replacement;
                }
                observed[index] = Some(byte);
            }
        }

        let mut undo = Vec::with_capacity(2);
        for index in 0..DESCRIPTORS.len() {
            let Some(byte) = observed[index] else {
                continue;
            };
            let descriptor = DESCRIPTORS[index];
            let wanted = if desired[index] {
                descriptor.replacement
            } else {
                descriptor.original
            };
            if byte == wanted {
                if !desired[index] {
                    // An owned byte was already restored to its exact preimage.
                    undo.push(Undo {
                        index,
                        owner: self.owned[index],
                        byte,
                    });
                    self.owned[index] = None;
                }
                continue;
            }
            let result = (|| -> Result<()> {
                // Recheck immediately before claiming ownership. A newly foreign
                // enabled byte between preflight and this read stays unowned.
                self.mark_uncertain(index);
                let current = read_guard(memory, target, descriptor, self.owned[index])?;
                ensure!(
                    current == byte,
                    "{} changed during the transaction",
                    descriptor.name
                );
                undo.push(Undo {
                    index,
                    owner: self.owned[index],
                    byte,
                });
                self.target = Some(target);
                if self.owned[index].is_none() {
                    self.owned[index] = Some(Owned {
                        original: byte,
                        active: false,
                    });
                }
                // Ownership precedes write, including an indeterminate failure.
                write_checked(memory, target, descriptor, wanted)?;
                if desired[index] {
                    self.owned[index]
                        .as_mut()
                        .expect("owner precedes write")
                        .active = true;
                } else {
                    self.owned[index] = None;
                }
                Ok(())
            })();
            if let Err(error) = result {
                let mut errors = vec![format!("{}: {error:#}", descriptor.name)];
                for change in undo.into_iter().rev() {
                    if let Err(error) = self.undo(memory, target, change) {
                        errors.push(format!(
                            "rollback {}: {error:#}",
                            DESCRIPTORS[change.index].name
                        ));
                    }
                }
                self.clear_empty_target();
                bail!("{}", errors.join("; "));
            }
        }
        self.clear_empty_target();
        Ok(())
    }

    /// Restores only owned bytes, preserving foreign bytes and changed guards.
    /// Errors retain outstanding ownership so the same target can be retried.
    pub fn restore(&mut self, memory: &impl Memory) -> Result<()> {
        if !self.owns_patches() {
            self.target = None;
            return Ok(());
        }
        let target = self
            .target
            .context("owned patches have lost their target identity")?;
        for index in 0..DESCRIPTORS.len() {
            self.mark_uncertain(index);
        }
        validate_target(memory, target)?;
        let mut errors = Vec::new();
        for index in (0..DESCRIPTORS.len()).rev() {
            if self.owned[index].is_some()
                && let Err(error) = self.restore_one(memory, target, index)
            {
                errors.push(format!("restore {}: {error:#}", DESCRIPTORS[index].name));
            }
        }
        self.clear_empty_target();
        ensure!(errors.is_empty(), "{}", errors.join("; "));
        Ok(())
    }

    /// Last fully verified applied state. An indeterminate operation clears its
    /// active flag while retaining ownership, exposed by `owns_patches`.
    pub fn active(&self) -> Requested {
        Requested {
            names: self.owned[0].is_some_and(|owner| owner.active),
            anti_kick: self.owned[1].is_some_and(|owner| owner.active),
        }
    }

    pub fn owns_patches(&self) -> bool {
        self.owned.iter().any(Option::is_some)
    }

    fn mark_uncertain(&mut self, index: usize) {
        if let Some(owner) = &mut self.owned[index] {
            owner.active = false;
        }
    }

    fn clear_empty_target(&mut self) {
        if !self.owns_patches() {
            self.target = None;
        }
    }

    fn restore_one(&mut self, memory: &impl Memory, target: Target, index: usize) -> Result<()> {
        let descriptor = DESCRIPTORS[index];
        let owner = self.owned[index].context("restore requires ownership")?;
        let byte = read_guard(memory, target, descriptor, Some(owner))?;
        if byte == owner.original {
            self.owned[index] = None;
            return Ok(());
        }
        let result = write_checked(memory, target, descriptor, owner.original);
        if result.is_ok() {
            self.owned[index] = None;
        } else {
            // A failed write may still have completed. Only an exact complete
            // preimage readback permits releasing ownership.
            if matches!(read_guard(memory, target, descriptor, Some(owner)), Ok(value) if value == owner.original)
            {
                self.owned[index] = None;
            }
        }
        result
    }

    fn undo(&mut self, memory: &impl Memory, target: Target, change: Undo) -> Result<()> {
        let index = change.index;
        let descriptor = DESCRIPTORS[index];
        // Reinstall prior ownership before even attempting to undo a successful
        // disable; a failing rollback must retain that outstanding obligation.
        if self.owned[index].is_none() {
            self.owned[index] = change.owner.or(Some(Owned {
                original: descriptor.original,
                active: false,
            }));
        }
        self.target = Some(target);
        self.mark_uncertain(index);
        let current = read_guard(memory, target, descriptor, self.owned[index])?;
        if current != change.byte {
            let result = write_checked(memory, target, descriptor, change.byte);
            if let Err(error) = result {
                if matches!(read_guard(memory, target, descriptor, self.owned[index]), Ok(value) if value == change.byte)
                {
                    self.owned[index] = change.owner.map(|mut owner| {
                        owner.active = change.byte == descriptor.replacement;
                        owner
                    });
                }
                return Err(error);
            }
        }
        self.owned[index] = change.owner.map(|mut owner| {
            owner.active = change.byte == descriptor.replacement;
            owner
        });
        Ok(())
    }
}

fn admit_profile(profile: &BuildProfile) -> Result<()> {
    let expected = CURRENT_PROFILE;
    ensure!(
        expected.id == PROFILE_ID
            && expected.sha256 == PROFILE_SHA256
            && expected.image_size == PROFILE_IMAGE_SIZE,
        "PointBlank patch descriptors require their pinned current profile"
    );
    ensure!(
        profile.id == expected.id
            && profile.sha256 == expected.sha256
            && profile.image_size == expected.image_size
            && profile.framework_rva == expected.framework_rva
            && profile.current_avatar_rva == expected.current_avatar_rva
            && profile.manager_wrappers == expected.manager_wrappers
            && profile.framework_vtable == expected.framework_vtable
            && profile.camera_vtable == expected.camera_vtable
            && profile.view_vtable == expected.view_vtable
            && profile.projection_vtable == expected.projection_vtable
            && profile.avatar_vtable == expected.avatar_vtable
            && profile.character_manager_vtable == expected.character_manager_vtable
            && profile.matrix_vtable == expected.matrix_vtable,
        "PointBlank patch profile is unsupported or altered"
    );
    Ok(())
}

fn image_end(target: Target) -> Result<u64> {
    let base = u64::from(target.base);
    let end = base
        .checked_add(u64::from(target.profile.image_size))
        .context("image overflow")?;
    ensure!(
        base >= MIN_BASE && end <= X86_END,
        "PointBlank patch image exceeds x86 bounds"
    );
    Ok(end)
}

fn addresses(target: Target, descriptor: Descriptor) -> Result<(u64, u64)> {
    let end = image_end(target)?;
    ensure!(
        !descriptor.guard.is_empty() && descriptor.guard.len() <= MAX_GUARD,
        "invalid patch guard length"
    );
    ensure!(
        descriptor.offset < descriptor.guard.len(),
        "patch outside its guard"
    );
    ensure!(
        descriptor.guard[descriptor.offset] == descriptor.original,
        "invalid descriptor preimage"
    );
    let offset = u32::try_from(descriptor.offset).context("patch offset overflow")?;
    ensure!(
        descriptor.guard_rva.checked_add(offset) == Some(descriptor.patch_rva),
        "inconsistent patch RVA"
    );
    let guard_length = u32::try_from(descriptor.guard.len()).context("guard length overflow")?;
    let guard_end = descriptor
        .guard_rva
        .checked_add(guard_length)
        .context("guard RVA overflow")?;
    ensure!(
        guard_end <= target.profile.image_size,
        "patch guard exceeds image"
    );
    let guard = u64::from(target.base)
        .checked_add(u64::from(descriptor.guard_rva))
        .context("guard address overflow")?;
    let patch = u64::from(target.base)
        .checked_add(u64::from(descriptor.patch_rva))
        .context("patch address overflow")?;
    ensure!(
        guard
            .checked_add(u64::from(guard_length))
            .is_some_and(|value| value <= end)
            && patch < end,
        "patch exceeds module bounds"
    );
    Ok((guard, patch))
}

// Distinct from both Memory traits: fully-qualified delegation avoids accidental
// recursive reads. Split the reader's 0x54-byte PE header into <=64-byte reads.
struct ReadAdapter<'a, M: Memory> {
    memory: &'a M,
    target: Target,
}

impl<M: Memory> super::Memory for ReadAdapter<'_, M> {
    fn read(&self, address: u64, out: &mut [u8]) -> Result<()> {
        let end = address
            .checked_add(u64::try_from(out.len())?)
            .context("PE read overflow")?;
        ensure!(
            address >= u64::from(self.target.base) && end <= image_end(self.target)?,
            "PE read outside image"
        );
        let mut next = address;
        for chunk in out.chunks_mut(MAX_GUARD) {
            Memory::read(self.memory, next, chunk)?;
            next = next
                .checked_add(u64::try_from(chunk.len())?)
                .context("PE chunk overflow")?;
        }
        Ok(())
    }
}

fn validate_target(memory: &impl Memory, target: Target) -> Result<()> {
    admit_profile(&target.profile)?;
    image_end(target)?;
    for descriptor in DESCRIPTORS {
        addresses(target, descriptor)?;
    }
    super::validate_image_profile(
        &ReadAdapter { memory, target },
        target.base,
        &target.profile,
    )
}

fn read_guard(
    memory: &impl Memory,
    target: Target,
    descriptor: Descriptor,
    owner: Option<Owned>,
) -> Result<u8> {
    let (address, _) = addresses(target, descriptor)?;
    let mut bytes = [0; MAX_GUARD];
    let bytes = &mut bytes[..descriptor.guard.len()];
    Memory::read(memory, address, bytes)
        .with_context(|| format!("{} guard read", descriptor.name))?;
    for (index, (&actual, &expected)) in bytes.iter().zip(descriptor.guard).enumerate() {
        if index == descriptor.offset {
            if let Some(owner) = owner {
                ensure!(
                    owner.original == descriptor.original,
                    "{} ownership preimage mismatch",
                    descriptor.name
                );
                ensure!(
                    actual == owner.original || actual == descriptor.replacement,
                    "{} owned byte has a foreign value",
                    descriptor.name
                );
            } else {
                ensure!(
                    actual == expected,
                    "{} preimage conflict; externally enabled bytes remain unowned",
                    descriptor.name
                );
            }
        } else {
            ensure!(
                actual == expected,
                "{} surrounding guard changed at +{index:#x}",
                descriptor.name
            );
        }
    }
    Ok(bytes[descriptor.offset])
}

fn write_checked(
    memory: &impl Memory,
    target: Target,
    descriptor: Descriptor,
    value: u8,
) -> Result<()> {
    let (_, address) = addresses(target, descriptor)?;
    let write = Memory::write(memory, address, &[value]);
    // Always read back, even when the write returned an indeterminate error.
    let read = read_guard(
        memory,
        target,
        descriptor,
        Some(Owned {
            original: descriptor.original,
            active: false,
        }),
    );
    match (write, read) {
        (Ok(()), Ok(actual)) if actual == value => Ok(()),
        (Err(error), _) => Err(error).with_context(|| format!("{} byte write", descriptor.name)),
        (Ok(()), Err(error)) => Err(error).with_context(|| format!("{} readback", descriptor.name)),
        (Ok(()), Ok(actual)) => bail!(
            "{} readback expected {value:#04x}, received {actual:#04x}",
            descriptor.name
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::pointblank::LEGACY_PROFILE;
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
    };

    const BASE: u32 = 0x0040_0000;

    #[derive(Clone, Copy)]
    enum WriteFault {
        Before,
        After,
        Ignore,
        Foreign,
    }

    #[derive(Default)]
    struct Fixture {
        bytes: RefCell<BTreeMap<u64, u8>>,
        reads: RefCell<Vec<(u64, usize)>>,
        writes: RefCell<Vec<(u64, Vec<u8>)>>,
        write_faults: RefCell<BTreeMap<usize, WriteFault>>,
        read_fail_at: Cell<Option<u64>>,
        fail_read_after_write: Cell<Option<usize>>,
    }

    impl Fixture {
        fn put(&self, address: u64, bytes: &[u8]) {
            for (index, byte) in bytes.iter().enumerate() {
                self.bytes
                    .borrow_mut()
                    .insert(address + index as u64, *byte);
            }
        }
        fn get(&self, address: u64) -> u8 {
            self.bytes.borrow()[&address]
        }
        fn patch_address(index: usize) -> u64 {
            u64::from(BASE) + u64::from(DESCRIPTORS[index].patch_rva)
        }
        fn guard_address(index: usize) -> u64 {
            u64::from(BASE) + u64::from(DESCRIPTORS[index].guard_rva)
        }
        fn fault(&self, call: usize, fault: WriteFault) {
            self.write_faults.borrow_mut().insert(call, fault);
        }
    }

    impl Memory for Fixture {
        fn read(&self, address: u64, out: &mut [u8]) -> Result<()> {
            self.reads.borrow_mut().push((address, out.len()));
            if self.read_fail_at.get() == Some(address) {
                bail!("injected read error");
            }
            let bytes = self.bytes.borrow();
            for (index, byte) in out.iter_mut().enumerate() {
                *byte = *bytes
                    .get(&(address + index as u64))
                    .context("fixture hole")?;
            }
            Ok(())
        }
        fn write(&self, address: u64, bytes: &[u8]) -> Result<()> {
            self.writes.borrow_mut().push((address, bytes.to_vec()));
            let call = self.writes.borrow().len();
            let fault = self.write_faults.borrow_mut().remove(&call);
            if matches!(fault, Some(WriteFault::Before)) {
                bail!("injected pre-write error");
            }
            if !matches!(fault, Some(WriteFault::Ignore)) {
                self.put(
                    address,
                    if matches!(fault, Some(WriteFault::Foreign)) {
                        &[0xcc]
                    } else {
                        bytes
                    },
                );
            }
            if self.fail_read_after_write.get() == Some(call) {
                let index = DESCRIPTORS
                    .iter()
                    .position(|item| u64::from(BASE) + u64::from(item.patch_rva) == address)
                    .unwrap();
                self.read_fail_at.set(Some(Self::guard_address(index)));
            }
            if matches!(fault, Some(WriteFault::After)) {
                bail!("injected indeterminate write");
            }
            Ok(())
        }
    }

    fn fixture() -> Fixture {
        let memory = Fixture::default();
        memory.put(u64::from(BASE), &[0; 0x40]);
        memory.put(u64::from(BASE), b"MZ");
        memory.put(u64::from(BASE) + 0x3c, &0x80_u32.to_le_bytes());
        memory.put(u64::from(BASE) + 0x80, &[0; 0x54]);
        memory.put(u64::from(BASE) + 0x80, b"PE\0\0");
        memory.put(u64::from(BASE) + 0x84, &0x14c_u16.to_le_bytes());
        memory.put(u64::from(BASE) + 0x94, &0xe0_u16.to_le_bytes());
        memory.put(u64::from(BASE) + 0x98, &0x10b_u16.to_le_bytes());
        memory.put(u64::from(BASE) + 0xd0, &PROFILE_IMAGE_SIZE.to_le_bytes());
        for (index, descriptor) in DESCRIPTORS.iter().enumerate() {
            memory.put(Fixture::guard_address(index), descriptor.guard);
        }
        memory
    }

    fn request(names: bool, anti_kick: bool) -> Requested {
        Requested { names, anti_kick }
    }
    fn sync(
        controller: &mut Controller,
        memory: &Fixture,
        names: bool,
        anti_kick: bool,
    ) -> Result<()> {
        controller.sync(memory, BASE, &CURRENT_PROFILE, request(names, anti_kick))
    }
    fn active(controller: &Controller) -> (bool, bool) {
        let value = controller.active();
        (value.names, value.anti_kick)
    }
    fn originals(memory: &Fixture) {
        for (index, descriptor) in DESCRIPTORS.iter().enumerate() {
            for (offset, byte) in descriptor.guard.iter().enumerate() {
                assert_eq!(
                    memory.get(Fixture::guard_address(index) + offset as u64),
                    *byte
                );
            }
        }
    }

    #[test]
    fn descriptors_pin_exact_full_guards_and_single_byte_fields() {
        assert_eq!(NAMES_GUARD.len(), 21);
        assert_eq!(ANTI_KICK_GUARD.len(), 39);
        assert_eq!(
            &NAMES_GUARD[9..20],
            &[
                0xc6, 0x01, 0x00, 0x8d, 0x49, 0x08, 0x83, 0xe8, 0x01, 0x75, 0xf5
            ]
        );
        assert_eq!(&ANTI_KICK_GUARD[11..13], &[0x78, 0x15]);
        assert_eq!(&ANTI_KICK_GUARD[34..], &[0x5e, 0x5d, 0xc2, 0x08, 0x00]);
        assert_eq!(DESCRIPTORS[0].patch_rva, 0x679237);
        assert_eq!(DESCRIPTORS[1].patch_rva, 0x73d4e1);
        for descriptor in DESCRIPTORS {
            assert_eq!(descriptor.offset, 11);
            assert_eq!(descriptor.guard[descriptor.offset], descriptor.original);
            addresses(
                Target {
                    base: BASE,
                    profile: CURRENT_PROFILE,
                },
                descriptor,
            )
            .unwrap();
        }
    }

    #[test]
    fn exact_one_byte_enable_and_restore_for_both_features() {
        let memory = fixture();
        let before = memory.bytes.borrow().clone();
        let mut controller = Controller::default();
        sync(&mut controller, &memory, true, true).unwrap();
        assert_eq!(active(&controller), (true, true));
        assert!(controller.owns_patches());
        let changed: Vec<_> = memory
            .bytes
            .borrow()
            .iter()
            .filter_map(|(&address, &byte)| {
                (before.get(&address).copied() != Some(byte)).then_some(address)
            })
            .collect();
        assert_eq!(
            changed,
            vec![Fixture::patch_address(0), Fixture::patch_address(1)]
        );
        assert_eq!(memory.get(Fixture::patch_address(0)), 1);
        assert_eq!(memory.get(Fixture::patch_address(1)), 0xeb);
        assert_eq!(memory.get(Fixture::patch_address(1) + 1), 0x15);
        controller.restore(&memory).unwrap();
        assert_eq!(*memory.bytes.borrow(), before);
        assert_eq!(active(&controller), (false, false));
        assert!(!controller.owns_patches());
        assert!(
            memory
                .writes
                .borrow()
                .iter()
                .all(|(_, bytes)| bytes.len() == 1)
        );
        assert!(
            memory
                .reads
                .borrow()
                .iter()
                .all(|(_, length)| *length <= 64)
        );
    }

    #[test]
    fn unchanged_mode_verifies_without_repeated_writes_and_selective_disable() {
        let memory = fixture();
        let mut controller = Controller::new();
        sync(&mut controller, &memory, true, true).unwrap();
        let count = memory.writes.borrow().len();
        for _ in 0..3 {
            sync(&mut controller, &memory, true, true).unwrap();
        }
        assert_eq!(memory.writes.borrow().len(), count);
        sync(&mut controller, &memory, false, true).unwrap();
        assert_eq!(active(&controller), (false, true));
        assert_eq!(memory.get(Fixture::patch_address(0)), 0);
        assert_eq!(memory.get(Fixture::patch_address(1)), 0xeb);
        sync(&mut controller, &memory, false, false).unwrap();
        assert!(!controller.owns_patches());
        originals(&memory);
    }

    #[test]
    fn corrupted_nametag_guard_and_pre_enabled_features_are_conflicts() {
        for (index, offset, byte) in [(0, 0, 0xcc), (0, 11, 1), (1, 11, 0xeb)] {
            let memory = fixture();
            memory.put(Fixture::guard_address(index) + offset, &[byte]);
            let mut controller = Controller::default();
            assert!(sync(&mut controller, &memory, true, true).is_err());
            assert!(!controller.owns_patches());
            assert!(memory.writes.borrow().is_empty());
            controller.restore(&memory).unwrap();
            assert_eq!(memory.get(Fixture::guard_address(index) + offset), byte);
        }
    }

    #[test]
    fn unrequested_external_patch_is_neither_claimed_nor_restored() {
        let memory = fixture();
        memory.put(Fixture::patch_address(1), &[0xeb]);
        let mut controller = Controller::default();
        sync(&mut controller, &memory, true, false).unwrap();
        controller.restore(&memory).unwrap();
        assert_eq!(memory.get(Fixture::patch_address(1)), 0xeb);
        assert!(!controller.owns_patches());
    }

    #[test]
    fn foreign_patch_or_surrounding_mutation_is_preserved_with_ownership() {
        for offset in [0, 11] {
            let memory = fixture();
            let mut controller = Controller::default();
            sync(&mut controller, &memory, true, false).unwrap();
            let address = Fixture::guard_address(0) + offset;
            memory.put(address, &[0xcc]);
            let writes = memory.writes.borrow().len();
            assert!(controller.restore(&memory).is_err());
            assert!(controller.owns_patches());
            assert!(!controller.active().names);
            assert_eq!(memory.get(address), 0xcc);
            assert_eq!(memory.writes.borrow().len(), writes);
        }
    }

    #[test]
    fn legacy_or_altered_profiles_are_rejected_without_remote_operations() {
        let profiles = [
            LEGACY_PROFILE,
            BuildProfile {
                sha256: "0000000000000000000000000000000000000000000000000000000000000000",
                ..CURRENT_PROFILE
            },
            BuildProfile {
                id: "other",
                ..CURRENT_PROFILE
            },
            BuildProfile {
                image_size: PROFILE_IMAGE_SIZE - 1,
                ..CURRENT_PROFILE
            },
            BuildProfile {
                framework_rva: 0,
                ..CURRENT_PROFILE
            },
            BuildProfile {
                manager_wrappers: None,
                ..CURRENT_PROFILE
            },
        ];
        for profile in profiles {
            let memory = fixture();
            let mut controller = Controller::default();
            assert!(
                controller
                    .sync(&memory, BASE, &profile, request(true, true))
                    .is_err()
            );
            assert!(memory.reads.borrow().is_empty());
            assert!(memory.writes.borrow().is_empty());
            assert!(!controller.owns_patches());
        }
    }

    #[test]
    fn pe_corruption_and_x86_overflow_precede_all_writes() {
        let cases: &[(u64, &[u8])] = &[
            (0, b"ZZ"),
            (0x80, b"PX\0\0"),
            (0x84, &[0x64, 0x86]),
            (0x94, &[0x20, 0]),
            (0x98, &[0x0b, 2]),
            (0xd0, &[0, 0, 0, 0]),
            (0x3c, &[0xff, 0xff, 0xff, 0xff]),
        ];
        for &(offset, bytes) in cases {
            let memory = fixture();
            memory.put(u64::from(BASE) + offset, bytes);
            let mut controller = Controller::default();
            assert!(sync(&mut controller, &memory, true, true).is_err());
            assert!(memory.writes.borrow().is_empty());
        }
        for base in [0, 0xffff_0000] {
            let memory = fixture();
            assert!(
                Controller::default()
                    .sync(&memory, base, &CURRENT_PROFILE, request(true, true))
                    .is_err()
            );
            assert!(memory.reads.borrow().is_empty());
        }
        let target = Target {
            base: BASE,
            profile: CURRENT_PROFILE,
        };
        assert!(
            addresses(
                target,
                Descriptor {
                    guard_rva: PROFILE_IMAGE_SIZE - 2,
                    patch_rva: PROFILE_IMAGE_SIZE + 9,
                    ..DESCRIPTORS[0]
                }
            )
            .is_err()
        );
    }

    #[test]
    fn second_write_failure_rolls_back_new_first_feature() {
        for fault in [WriteFault::Before, WriteFault::After, WriteFault::Ignore] {
            let memory = fixture();
            memory.fault(2, fault);
            let mut controller = Controller::default();
            assert!(sync(&mut controller, &memory, true, true).is_err());
            assert!(!controller.owns_patches());
            originals(&memory);
        }
    }

    #[test]
    fn failure_preserves_a_feature_owned_before_this_transaction() {
        let memory = fixture();
        let mut controller = Controller::default();
        sync(&mut controller, &memory, true, false).unwrap();
        memory.fault(2, WriteFault::After);
        assert!(sync(&mut controller, &memory, true, true).is_err());
        assert_eq!(active(&controller), (true, false));
        assert!(controller.owns_patches());
        assert_eq!(memory.get(Fixture::patch_address(0)), 1);
        assert_eq!(memory.get(Fixture::patch_address(1)), 0x78);
        controller.restore(&memory).unwrap();
    }

    #[test]
    fn failed_transaction_undoes_a_successful_disable_too() {
        let memory = fixture();
        let mut controller = Controller::default();
        sync(&mut controller, &memory, true, false).unwrap();
        memory.fault(3, WriteFault::Before);
        assert!(sync(&mut controller, &memory, false, true).is_err());
        assert_eq!(active(&controller), (true, false));
        assert_eq!(memory.get(Fixture::patch_address(0)), 1);
        controller.restore(&memory).unwrap();
    }

    #[test]
    fn indeterminate_first_write_is_owned_and_rolled_back() {
        let memory = fixture();
        memory.fault(1, WriteFault::After);
        let mut controller = Controller::default();
        assert!(sync(&mut controller, &memory, true, false).is_err());
        assert_eq!(memory.writes.borrow().len(), 2);
        assert!(!controller.owns_patches());
        originals(&memory);
    }

    #[test]
    fn rollback_read_failure_keeps_pending_ownership_until_retry() {
        let memory = fixture();
        memory.fail_read_after_write.set(Some(1));
        let mut controller = Controller::default();
        assert!(sync(&mut controller, &memory, true, false).is_err());
        assert!(controller.owns_patches());
        assert_eq!(active(&controller), (false, false));
        assert_eq!(memory.get(Fixture::patch_address(0)), 1);
        memory.read_fail_at.set(None);
        memory.fail_read_after_write.set(None);
        controller.restore(&memory).unwrap();
        assert!(!controller.owns_patches());
        originals(&memory);
    }

    #[test]
    fn restoration_read_failure_retains_owner_until_same_target_retry() {
        let memory = fixture();
        let mut controller = Controller::default();
        sync(&mut controller, &memory, true, false).unwrap();
        memory.read_fail_at.set(Some(Fixture::guard_address(0)));
        assert!(controller.restore(&memory).is_err());
        assert!(controller.owns_patches());
        assert_eq!(memory.get(Fixture::patch_address(0)), 1);
        memory.read_fail_at.set(None);
        controller.restore(&memory).unwrap();
        assert!(!controller.owns_patches());
        originals(&memory);
    }

    #[test]
    fn failed_restoration_write_and_readback_can_be_retried() {
        let memory = fixture();
        let mut controller = Controller::default();
        sync(&mut controller, &memory, true, false).unwrap();
        memory.fail_read_after_write.set(Some(2));
        assert!(controller.restore(&memory).is_err());
        assert!(controller.owns_patches());
        assert_eq!(memory.get(Fixture::patch_address(0)), 0);
        memory.read_fail_at.set(None);
        memory.fail_read_after_write.set(None);
        controller.restore(&memory).unwrap();
        assert!(!controller.owns_patches());
    }

    #[test]
    fn rollback_failure_and_foreign_values_are_retained_without_clobbering() {
        let memory = fixture();
        memory.fault(1, WriteFault::After);
        memory.fault(2, WriteFault::Before);
        let mut controller = Controller::default();
        assert!(sync(&mut controller, &memory, true, false).is_err());
        assert!(controller.owns_patches());
        controller.restore(&memory).unwrap();
        originals(&memory);

        let memory = fixture();
        memory.fault(1, WriteFault::Foreign);
        let mut controller = Controller::default();
        assert!(sync(&mut controller, &memory, true, false).is_err());
        assert!(controller.owns_patches());
        assert_eq!(memory.get(Fixture::patch_address(0)), 0xcc);
        assert_eq!(memory.writes.borrow().len(), 1);
    }

    #[test]
    fn owned_target_base_swap_is_rejected_before_remote_access() {
        let memory = fixture();
        let mut controller = Controller::default();
        sync(&mut controller, &memory, true, false).unwrap();
        let reads = memory.reads.borrow().len();
        let writes = memory.writes.borrow().len();
        assert!(
            controller
                .sync(
                    &memory,
                    BASE + 0x1000,
                    &CURRENT_PROFILE,
                    request(true, true)
                )
                .is_err()
        );
        assert_eq!(memory.reads.borrow().len(), reads);
        assert_eq!(memory.writes.borrow().len(), writes);
        assert!(controller.owns_patches());
        controller.restore(&memory).unwrap();
    }
}
