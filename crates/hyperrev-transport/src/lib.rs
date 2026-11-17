//! Direct readiness and bounded virtual-memory access for the pinned hyper-reV attachment.
//!
//! ABI source: vendor/hyper-reV at `73e865da4e9d67eb773cc2c334ce5c8aca8a4aa0`:
//! - [`shared/hypercall.hpp`](../../../vendor/hyper-reV/shared/hypercall.hpp):6-38
//!   defines the 48-bit key, 16-bit operation, and five argument registers.
//! - [`hc_stub.asm`](../../../vendor/hyper-reV/usermode/src/hypercall/hc_stub.asm):2-18
//!   uses CPUID with EAX=0, the full packed RCX, and preserved RBX.
//! - [`hypercall.cpp`](../../../vendor/hyper-reV/hyperv-attachment/src/vmexit/hypercall/hypercall.cpp):11-28
//!   returns `0x1337` for operation 0 and the current guest CR3 for operation 1.
//! - [`main.cpp`](../../../vendor/hyper-reV/usermode/src/main.cpp):48-70
//!   implements the vendor's matching `--check` sequence.
//!
//! Operation 3 copies virtual memory and returns a boolean, rather than a byte count. Reads
//! place the *target* CR3 in RDX and this process's CR3 in R9; writes reverse those roles.
//! Each transfer is bounded and split at both virtual-page boundaries. Boot configuration,
//! hooks, concealment, and kernel writes are outside this crate's process-discovery logic.
//! The attachment's first VM exit can initialize its internal state: see
//! `vmexit/vmexit.cpp`:44-45 and `vmexit/first/first.cpp`:31-60.

use anyhow::{Context, Result, ensure};

mod pe;
mod process;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod windows;

pub use process::{Process, find_process, process_by_pid};

/// Existing vendor ABI key, placed in RCX bits 16..63.
#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
const ALLOWED_KEY: u64 = 0x8A3B55C7927;
#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
const TEST_OPERATION: u16 = 0;
#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
const CURRENT_CR3_OPERATION: u16 = 1;
const EXPECTED_RESPONSE: u64 = 0x1337;
const VIRTUAL_COPY_OPERATION: u16 = 3;

/// Maximum total length of one read or write request (1 MiB).
pub const MAX_TRANSFER_SIZE: usize = 1024 * 1024;
const MAX_HYPERCALL_SIZE: usize = 64 * 1024;
const PAGE_SIZE: u64 = 4096;

// Broad x86-64 physical-frame field. Preserve low CR3 PCID/cache flags in returned data,
// and avoid making CPU-specific MAXPHYADDR or upper-control-bit assumptions here.
const CR3_PHYSICAL_FRAME_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// Observed attachment readiness, based on actual responses rather than staged boot files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// Ordinary CPUID, another hypervisor, or an absent attachment returned a different value.
    Absent { response: u64 },
    /// The vendor test succeeded and its current-guest-CR3 query returned a usable frame.
    Ready { cr3: u64 },
}

/// Classify responses without executing native instructions.
///
/// A test mismatch means absence regardless of `cr3`. A matching test with an unusable CR3
/// is an error rather than readiness. CR3 is returned intact, including its low PCID bits.
/// This is protocol readiness, rather than cryptographic authentication of an attachment.
pub fn classify(response: u64, cr3: u64) -> Result<Readiness> {
    if response != EXPECTED_RESPONSE {
        return Ok(Readiness::Absent { response });
    }

    ensure!(
        cr3 != 0,
        "hyper-reV probe succeeded but the CR3 query returned zero"
    );
    ensure!(
        cr3 & CR3_PHYSICAL_FRAME_MASK != 0,
        "hyper-reV probe succeeded but CR3 {cr3:#x} has an empty physical-frame field"
    );
    Ok(Readiness::Ready { cr3 })
}

/// Query the running attachment with the vendor's test and current-guest-CR3 operations.
///
/// On Windows x86-64 this executes CPUID, a CPL3 instruction, rather than VMCALL or a direct
/// privileged CR3 read. EAX=0 makes ordinary absence return the CPU's highest supported
/// basic leaf, which can be nonzero; only the exact vendor test response advances to the CR3
/// query. Other platforms return an error. This function performs zero firmware operations.
pub fn probe() -> Result<Readiness> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        let response = query(TEST_OPERATION);
        if response != EXPECTED_RESPONSE {
            return classify(response, 0);
        }
        classify(response, query(CURRENT_CR3_OPERATION))
    }

    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        anyhow::bail!("hyper-reV attachment probing requires Windows x86-64")
    }
}

#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
const fn packed_request(operation: u16) -> u64 {
    (ALLOWED_KEY << 16) | operation as u64
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn query(operation: u16) -> u64 {
    invoke(operation, [0; 5])
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn invoke(operation: u16, args: [u64; 5]) -> u64 {
    let response: u64;
    let _saved_rbx: u64;

    // SAFETY: CPUID is available to CPL3 on Windows x86-64. This follows the vendor ABI,
    // including all five argument registers. LLVM reserves RBX for internal use on
    // some targets, so save/restore it through a distinct early-output scratch register
    // instead of declaring an RBX operand. Full-width RAX retains 64-bit CR3 responses;
    // __cpuid_count would truncate both the RCX request and returned RAX to 32 bits.
    // All changed allocatable registers are in/out operands. The early-output scratch
    // cannot alias any input, including RDX/R8-R11. Deliberately omit pure/nomem/readonly:
    // virtual-copy hypercalls read/write Rust-owned buffers and external guest memory.
    unsafe {
        core::arch::asm!(
            "mov {saved_rbx:r}, rbx",
            "cpuid",
            "mov rbx, {saved_rbx:r}",
            saved_rbx = out(reg) _saved_rbx,
            inout("rax") 0_u64 => response,
            inout("rcx") packed_request(operation) => _,
            inout("rdx") args[0] => _,
            inout("r8") args[1] => _,
            inout("r9") args[2] => _,
            inout("r10") args[3] => _,
            inout("r11") args[4] => _,
            options(nostack, preserves_flags),
        );
    }
    response
}

/// A connection to the attachment in the current address space.
///
/// The captured CR3 belongs to this process, rather than any selected remote process.
/// Clones may be used on its other threads; target CR3 values always remain explicit.
#[derive(Clone, Debug)]
pub struct Connection {
    cr3: u64,
}

impl Connection {
    /// Probe the actual attachment and capture this process's full-width CR3.
    pub fn open() -> Result<Self> {
        match probe()? {
            Readiness::Ready { cr3 } => Ok(Self { cr3 }),
            Readiness::Absent { response } => {
                anyhow::bail!("hyper-reV attachment is absent (probe RAX={response:#x})")
            }
        }
    }

    pub fn current_cr3(&self) -> u64 {
        self.cr3
    }

    /// Read from `target_cr3` into a caller-owned local buffer.
    ///
    /// Errors can leave the output partly modified. The ABI returns only a full-copy
    /// boolean; this API deliberately reports neither a partial byte count nor atomicity.
    /// Local destination pages are CPU-touched for demand-zero/COW materialization before
    /// copying. This touches rather than pins pages; later paging can still fail a copy.
    pub fn read(&self, target_cr3: u64, address: u64, out: &mut [u8]) -> Result<()> {
        self.read_using(&NativeHypercall, target_cr3, address, out)
    }

    fn read_using(
        &self,
        hypercall: &impl Hypercall,
        target_cr3: u64,
        address: u64,
        out: &mut [u8],
    ) -> Result<()> {
        let local = out.as_mut_ptr() as usize as u64;
        let transfer = (|| -> Result<()> {
            // Validate remote/local ranges and the request bound before touching any page.
            validate_copy(target_cr3, address, self.cr3, local, out.len())?;
            prefault_destination(out);
            copy_with(hypercall, target_cr3, address, self.cr3, local, out.len())
        })();
        transfer.with_context(|| {
            format!(
                "read CR3={target_cr3:#x} VA={address:#x} len={:#x}",
                out.len()
            )
        })
    }

    /// Write the caller's local bytes into `target_cr3`.
    ///
    /// Writes are explicit and nontransactional. Process discovery and normal connection
    /// startup only read metadata; they never invoke this operation on a target process.
    pub fn write(&self, target_cr3: u64, address: u64, input: &[u8]) -> Result<()> {
        self.write_using(&NativeHypercall, target_cr3, address, input)
    }

    fn write_using(
        &self,
        hypercall: &impl Hypercall,
        target_cr3: u64,
        address: u64,
        input: &[u8],
    ) -> Result<()> {
        let local = input.as_ptr() as usize as u64;
        let transfer = (|| -> Result<()> {
            validate_copy(self.cr3, local, target_cr3, address, input.len())?;
            prefault_source(input);
            copy_with(hypercall, self.cr3, local, target_cr3, address, input.len())
        })();
        transfer.with_context(|| {
            format!(
                "write CR3={target_cr3:#x} VA={address:#x} len={:#x}",
                input.len()
            )
        })
    }

    /// Exercise both copy directions using only newly allocated, owned buffers.
    ///
    /// Tests 32 bytes and a 65 KiB multi-page, initially zeroed local destination.
    /// The changed buffers are restored through the hypercall and restoration is verified.
    /// This function is explicit; `open` and process selection never run a write test.
    pub fn self_test(&self) -> Result<()> {
        let original = std::array::from_fn::<_, 32, _>(|i| (i as u8).wrapping_mul(7) ^ 0x5a);
        let changed = std::array::from_fn::<_, 32, _>(|i| (i as u8).wrapping_mul(11) ^ 0xa5);
        let mut owned = Box::new(original);
        let address = owned.as_mut_ptr() as usize as u64;
        let mut observed = [0_u8; 32];
        self.read(self.cr3, address, &mut observed)
            .context("self-test initial hypercall read")?;
        ensure!(
            observed == original,
            "self-test initial read differs from owned bytes"
        );

        // Always attempt restoration, even if a write or verification reports failure.
        let change_result = (|| -> Result<()> {
            self.write(self.cr3, address, &changed)
                .context("self-test hypercall write")?;
            self.read(self.cr3, address, &mut observed)
                .context("self-test changed-buffer read")?;
            ensure!(
                observed == changed,
                "self-test hypercall write verification failed"
            );
            let local = std::array::from_fn::<_, 32, _>(|i| {
                // SAFETY: the allocation is alive, in bounds, and exclusively owned here.
                unsafe { core::ptr::read_volatile(owned.as_ptr().add(i)) }
            });
            ensure!(
                local == changed,
                "self-test local allocation stayed unchanged"
            );
            Ok(())
        })();
        let restore_result = (|| -> Result<()> {
            self.write(self.cr3, address, &original)
                .context("self-test restoring owned bytes")?;
            self.read(self.cr3, address, &mut observed)
                .context("self-test restored-buffer read")?;
            ensure!(
                observed == original,
                "self-test restoration verification failed"
            );
            let local = std::array::from_fn::<_, 32, _>(|i| {
                // SAFETY: same owned allocation; volatile also independently checks writes.
                unsafe { core::ptr::read_volatile(owned.as_ptr().add(i)) }
            });
            ensure!(local == original, "self-test local restoration differs");
            Ok(())
        })();
        restore_result?;
        change_result?;
        self.multi_page_self_test()
    }

    fn multi_page_self_test(&self) -> Result<()> {
        const LENGTH: usize = 65 * 1024;
        let original: Vec<u8> = (0..LENGTH).map(|i| (i % 251 + 1) as u8).collect();
        let changed: Vec<u8> = original.iter().rev().copied().collect();
        let mut owned = original.clone();
        // Keep this destination demand-zero until read() materializes every page.
        let mut observed = vec![0_u8; LENGTH];
        let address = owned.as_mut_ptr() as usize as u64;
        self.read(self.cr3, address, &mut observed)
            .context("multi-page self-test read into zeroed local buffer")?;
        ensure!(
            observed == original,
            "multi-page self-test initial read differs"
        );
        let change_result = (|| -> Result<()> {
            self.write(self.cr3, address, &changed)
                .context("multi-page self-test owned-buffer write")?;
            self.read(self.cr3, address, &mut observed)
                .context("multi-page self-test changed-buffer read")?;
            ensure!(
                observed == changed,
                "multi-page self-test write verification failed"
            );
            ensure!(
                owned.iter().enumerate().all(|(i, byte)| {
                    // SAFETY: each byte is within the live, exclusively owned allocation.
                    unsafe { core::ptr::read_volatile(byte) == changed[i] }
                }),
                "multi-page self-test local write verification failed"
            );
            Ok(())
        })();
        // Restoration remains unconditional after any attempted hypercall write.
        let restore_result = (|| -> Result<()> {
            self.write(self.cr3, address, &original)
                .context("multi-page self-test restoring owned bytes")?;
            self.read(self.cr3, address, &mut observed)
                .context("multi-page self-test restored-buffer read")?;
            ensure!(
                observed == original,
                "multi-page self-test restoration read differs"
            );
            ensure!(
                owned.iter().enumerate().all(|(i, byte)| {
                    // SAFETY: the same allocation remains alive until after restoration.
                    unsafe { core::ptr::read_volatile(byte) == original[i] }
                }),
                "multi-page self-test local restoration differs"
            );
            Ok(())
        })();
        restore_result?;
        change_result
    }
}

pub(crate) trait Memory {
    fn read(&self, cr3: u64, address: u64, out: &mut [u8]) -> Result<()>;
}

impl Memory for Connection {
    fn read(&self, cr3: u64, address: u64, out: &mut [u8]) -> Result<()> {
        Connection::read(self, cr3, address, out)
    }
}

trait Hypercall {
    fn call(&self, operation: u16, args: [u64; 5]) -> Result<u64>;
}

struct NativeHypercall;

impl Hypercall for NativeHypercall {
    fn call(&self, operation: u16, args: [u64; 5]) -> Result<u64> {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            Ok(invoke(operation, args))
        }
        #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
        {
            let _ = (operation, args);
            anyhow::bail!("hyper-reV memory operations require Windows x86-64")
        }
    }
}

pub(crate) fn valid_cr3(cr3: u64) -> Result<()> {
    ensure!(
        cr3 & CR3_PHYSICAL_FRAME_MASK != 0,
        "CR3 {cr3:#x} has an empty physical frame"
    );
    Ok(())
}

pub(crate) fn canonical(address: u64) -> bool {
    (((address as i64) << 16) >> 16) == address as i64
}

fn checked_range(address: u64, len: usize) -> Result<()> {
    // The native ABI uses uintptr_t. Keep the conversion checked even in pure 32-bit tests.
    let _ = usize::try_from(address).context("virtual address exceeds uintptr_t")?;
    if len == 0 {
        return Ok(());
    }
    let last = address
        .checked_add(u64::try_from(len - 1)?)
        .context("virtual-address range overflow")?;
    let _ = usize::try_from(last).context("virtual range exceeds uintptr_t")?;
    ensure!(
        canonical(address) && canonical(last),
        "noncanonical virtual-address range"
    );
    // Endpoints alone could straddle the entire noncanonical hole.
    ensure!(
        (address >> 47) == (last >> 47),
        "virtual range crosses canonical regions"
    );
    Ok(())
}

// Visit the first byte, each actual virtual-page boundary, and the last byte.
// Offsets are always inside the slice, including short or unaligned slices.
fn local_page_bytes(start: usize, len: usize, mut visit: impl FnMut(usize)) {
    if len == 0 {
        return;
    }
    let page = PAGE_SIZE as usize;
    let mut offset = 0;
    loop {
        visit(offset);
        let page_offset = ((start & (page - 1)) + (offset & (page - 1))) & (page - 1);
        let Some(next) = offset.checked_add(page - page_offset) else {
            break;
        };
        if next >= len {
            break;
        }
        offset = next;
    }
    if offset != len - 1 {
        visit(len - 1);
    }
}

fn prefault_destination(out: &mut [u8]) {
    let pointer = out.as_mut_ptr();
    local_page_bytes(pointer as usize, out.len(), |offset| {
        // SAFETY: the visitor only yields in-bounds offsets in this exclusive slice.
        // A volatile same-byte store materializes demand-zero and copy-on-write pages;
        // the guest-memory copier does not itself execute a CPU fault in this process.
        unsafe {
            let byte = pointer.add(offset);
            let value = core::ptr::read_volatile(byte);
            core::ptr::write_volatile(byte, value);
        }
    });
}

fn prefault_source(input: &[u8]) {
    let pointer = input.as_ptr();
    local_page_bytes(pointer as usize, input.len(), |offset| {
        // SAFETY: the visitor only yields in-bounds offsets. Input stays immutable;
        // a CPU read suffices for source residency, including shared zero pages.
        unsafe {
            core::ptr::read_volatile(pointer.add(offset));
        }
    });
}

fn validate_copy(
    source_cr3: u64,
    source: u64,
    destination_cr3: u64,
    destination: u64,
    len: usize,
) -> Result<()> {
    ensure!(
        len <= MAX_TRANSFER_SIZE,
        "transfer exceeds the 1 MiB request bound"
    );
    valid_cr3(source_cr3)?;
    valid_cr3(destination_cr3)?;
    checked_range(source, len)?;
    checked_range(destination, len)?;
    Ok(())
}

fn copy_with(
    hypercall: &impl Hypercall,
    source_cr3: u64,
    source: u64,
    destination_cr3: u64,
    destination: u64,
    len: usize,
) -> Result<()> {
    validate_copy(source_cr3, source, destination_cr3, destination, len)?;
    let mut done = 0_usize;
    while done < len {
        let src = source
            .checked_add(done as u64)
            .context("source offset overflow")?;
        let dst = destination
            .checked_add(done as u64)
            .context("destination offset overflow")?;
        let chunk = (len - done)
            .min(MAX_HYPERCALL_SIZE)
            .min((PAGE_SIZE - (src & (PAGE_SIZE - 1))) as usize)
            .min((PAGE_SIZE - (dst & (PAGE_SIZE - 1))) as usize);
        let result = hypercall.call(
            VIRTUAL_COPY_OPERATION,
            [source_cr3, src, destination_cr3, dst, chunk as u64],
        )?;
        ensure!(
            result == 1,
            "virtual-copy boolean failure RAX={result:#x} at offset {done:#x}; completed chunks cover {done:#x} bytes; failed chunk has indeterminate effects"
        );
        done += chunk;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_preserve_the_full_key_and_operation() {
        assert_eq!(packed_request(TEST_OPERATION), 0x08A3_B55C_7927_0000);
        assert_eq!(packed_request(CURRENT_CR3_OPERATION), 0x08A3_B55C_7927_0001);
        assert_eq!(
            packed_request(VIRTUAL_COPY_OPERATION),
            0x08A3_B55C_7927_0003
        );
        assert_eq!(packed_request(TEST_OPERATION) >> 16, ALLOWED_KEY);
    }

    #[test]
    fn ordinary_nonzero_cpuid_response_is_absent() {
        assert_eq!(
            classify(0x20, 0).unwrap(),
            Readiness::Absent { response: 0x20 }
        );
    }

    #[test]
    fn zero_response_is_absent() {
        assert_eq!(classify(0, 0).unwrap(), Readiness::Absent { response: 0 });
    }

    #[test]
    fn mismatching_test_never_accepts_a_cr3() {
        assert_eq!(
            classify(0x1338, 0x1234_5000).unwrap(),
            Readiness::Absent { response: 0x1338 }
        );
    }

    #[test]
    fn matching_test_with_zero_cr3_is_an_error() {
        assert!(classify(EXPECTED_RESPONSE, 0).is_err());
    }

    #[test]
    fn low_flags_alone_are_an_error() {
        assert!(classify(EXPECTED_RESPONSE, 0xfff).is_err());
    }

    #[test]
    fn high_control_bits_alone_are_an_error() {
        assert!(classify(EXPECTED_RESPONSE, 0x6000_0000_0000_0000).is_err());
    }

    #[test]
    fn full_width_frame_and_pcid_are_preserved() {
        let cr3 = 0x000A_BCDE_1234_5ABC;
        assert_eq!(
            classify(EXPECTED_RESPONSE, cr3).unwrap(),
            Readiness::Ready { cr3 }
        );
    }

    #[test]
    fn lowest_nonempty_frame_is_accepted() {
        assert_eq!(
            classify(EXPECTED_RESPONSE, 0x1000).unwrap(),
            Readiness::Ready { cr3: 0x1000 }
        );
    }

    #[derive(Default)]
    struct MockHypercall {
        calls: std::cell::RefCell<Vec<(u16, [u64; 5])>>,
        returns: std::cell::RefCell<std::collections::VecDeque<u64>>,
    }

    impl Hypercall for MockHypercall {
        fn call(&self, operation: u16, args: [u64; 5]) -> Result<u64> {
            self.calls.borrow_mut().push((operation, args));
            Ok(self.returns.borrow_mut().pop_front().unwrap_or(1))
        }
    }

    #[test]
    fn read_packs_target_cr3_as_source_and_local_buffer_as_destination() {
        let connection = Connection {
            cr3: 0x000a_1234_5007,
        };
        let target = 0x000b_9876_5009;
        let mock = MockHypercall::default();
        let mut output = [0; 32];
        let local_address = output.as_mut_ptr() as usize as u64;
        connection
            .read_using(&mock, target, 0x0040_0000, &mut output)
            .unwrap();
        let calls = mock.calls.borrow();
        assert_eq!(calls[0].0, VIRTUAL_COPY_OPERATION);
        assert_eq!(
            calls[0].1[..4],
            [target, 0x0040_0000, connection.cr3, local_address]
        );
        assert_eq!(calls.iter().map(|(_, args)| args[4]).sum::<u64>(), 32);
        assert!(
            calls
                .iter()
                .all(|(_, args)| args[0] == target && args[2] == connection.cr3)
        );
    }

    #[test]
    fn write_packs_local_buffer_as_source_and_target_cr3_as_destination() {
        let connection = Connection {
            cr3: 0x000a_1234_5007,
        };
        let target = 0x000b_9876_5009;
        let mock = MockHypercall::default();
        let input = [7; 32];
        let local_address = input.as_ptr() as usize as u64;
        connection
            .write_using(&mock, target, 0x0040_0000, &input)
            .unwrap();
        let calls = mock.calls.borrow();
        assert_eq!(calls[0].0, VIRTUAL_COPY_OPERATION);
        assert_eq!(
            calls[0].1[..4],
            [connection.cr3, local_address, target, 0x0040_0000]
        );
        assert_eq!(calls.iter().map(|(_, args)| args[4]).sum::<u64>(), 32);
    }

    #[test]
    fn chunks_obey_both_page_boundaries_and_cover_address_math_exactly() {
        let mock = MockHypercall::default();
        let len = 0x23456;
        copy_with(&mock, 0x1000, 0x1fff, 0x2000, 0x23fd, len).unwrap();
        let calls = mock.calls.borrow();
        assert_eq!(calls[0].1, [0x1000, 0x1fff, 0x2000, 0x23fd, 1]);
        let mut covered = 0;
        for &(operation, args) in calls.iter() {
            assert_eq!(operation, 3);
            assert_eq!(args[1], 0x1fff + covered);
            assert_eq!(args[3], 0x23fd + covered);
            assert!(args[4] > 0 && args[4] <= MAX_HYPERCALL_SIZE as u64);
            assert!((args[1] & 0xfff) + args[4] <= 4096);
            assert!((args[3] & 0xfff) + args[4] <= 4096);
            covered += args[4];
        }
        assert_eq!(covered, len as u64);
    }

    #[test]
    fn full_request_bound_is_accepted_and_larger_requests_are_rejected_before_calling() {
        let mock = MockHypercall::default();
        copy_with(&mock, 0x1000, 0x10000, 0x2000, 0x20000, MAX_TRANSFER_SIZE).unwrap();
        assert_eq!(mock.calls.borrow().len(), MAX_TRANSFER_SIZE / 4096);
        mock.calls.borrow_mut().clear();
        assert!(
            copy_with(
                &mock,
                0x1000,
                0x10000,
                0x2000,
                0x20000,
                MAX_TRANSFER_SIZE + 1
            )
            .is_err()
        );
        assert!(mock.calls.borrow().is_empty());
    }

    #[test]
    fn boolean_zero_or_count_like_response_is_failure_and_stops_next_chunks() {
        for response in [0, 2, 4096] {
            let mock = MockHypercall::default();
            mock.returns.borrow_mut().extend([1, response]);
            let error = copy_with(&mock, 0x1000, 0x10000, 0x2000, 0x20000, 0x3000).unwrap_err();
            assert_eq!(mock.calls.borrow().len(), 2);
            assert!(error.to_string().contains("at offset 0x1000"));
        }
    }

    #[test]
    fn rejects_overflow_noncanonical_ranges_and_empty_cr3_frames() {
        let mock = MockHypercall::default();
        for (src_cr3, source, destination, len) in [
            (0, 0x10000, 0x20000, 1),
            (0xfff, 0x10000, 0x20000, 1),
            (0x1000, u64::MAX - 1, 0x20000, 4),
            (0x1000, 0x0000_7fff_ffff_ffff, 0x20000, 2),
            (0x1000, 0x10000, 0x0000_8000_0000_0000, 1),
        ] {
            assert!(copy_with(&mock, src_cr3, source, 0x2000, destination, len).is_err());
        }
        assert!(mock.calls.borrow().is_empty());
        copy_with(&mock, 0x1000, 0x10000, 0x2000, 0x20000, 0).unwrap();
        assert!(mock.calls.borrow().is_empty());
    }

    #[test]
    fn uintptr_conversion_has_an_explicit_width_check() {
        if usize::BITS == 32 {
            assert!(checked_range(u32::MAX as u64 + 1, 1).is_err());
        } else {
            assert!(checked_range(0xffff_8000_0000_1000, 4096).is_ok());
        }
    }

    #[test]
    fn local_page_touch_offsets_cover_unaligned_boundaries_and_last_byte() {
        let offsets = |start, len| {
            let mut result = Vec::new();
            local_page_bytes(start, len, |offset| result.push(offset));
            result
        };
        assert_eq!(offsets(0x1040, 4096), vec![0, 0xfc0, 4095]);
        assert_eq!(offsets(0x1040, 0xfc1), vec![0, 0xfc0]);
        assert_eq!(offsets(0x1fff, 2), vec![0, 1]);
        assert_eq!(offsets(0x1000, 8192), vec![0, 4096, 8191]);
        assert_eq!(offsets(usize::MAX, 1), vec![0]);
        assert!(offsets(0x1040, 0).is_empty());
        for start in [0, 1, 0x40, 0xfc0, 0xfff] {
            for len in [1, 2, 63, 4095, 4096, 4097, 65 * 1024, MAX_TRANSFER_SIZE] {
                let touched = offsets(start, len);
                assert_eq!(touched.first(), Some(&0));
                assert_eq!(touched.last(), Some(&(len - 1)));
                assert!(touched.iter().all(|&offset| offset < len));
                assert!(touched.windows(2).all(|pair| pair[0] < pair[1]));
                let pages: std::collections::HashSet<_> = touched
                    .iter()
                    .map(|&offset| (start + offset) / PAGE_SIZE as usize)
                    .collect();
                let expected = ((start & 0xfff) + len).div_ceil(PAGE_SIZE as usize);
                assert_eq!(pages.len(), expected);
            }
        }
    }

    #[test]
    fn local_prefault_preserves_destination_and_source_bytes() {
        let mut buffer: Vec<u8> = (0..65 * 1024).map(|i| (i % 251) as u8).collect();
        let expected = buffer.clone();
        prefault_destination(&mut buffer[3..]);
        assert_eq!(buffer, expected);
        prefault_source(&buffer[7..]);
        assert_eq!(buffer, expected);
        prefault_destination(&mut []);
        prefault_source(&[]);
    }

    #[test]
    fn connection_rejects_oversized_local_buffers_before_hypercalls() {
        let connection = Connection { cr3: 0x1000 };
        let mock = MockHypercall::default();
        let mut buffer = vec![7_u8; MAX_TRANSFER_SIZE + 1];
        assert!(
            connection
                .read_using(&mock, 0x2000, 0x400000, &mut buffer)
                .is_err()
        );
        assert!(
            connection
                .write_using(&mock, 0x2000, 0x400000, &buffer)
                .is_err()
        );
        assert!(mock.calls.borrow().is_empty());
        assert!(buffer.iter().all(|&byte| byte == 7));
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    #[test]
    #[ignore = "requires an active attachment; writes only newly allocated self-owned buffers"]
    fn native_owned_memory_self_test() {
        Connection::open().unwrap().self_test().unwrap();
    }

    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    #[test]
    fn unsupported_platform_returns_an_error() {
        assert!(probe().is_err());
        assert!(Connection::open().is_err());
    }
}
