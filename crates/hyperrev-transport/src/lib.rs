//! Readiness queries for the pinned hyper-reV attachment, rather than boot configuration.
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
//! These operations request register-only answers; this crate exposes neither guest-memory
//! copying nor boot changes. The attachment's first-VM-exit initialization can still initialize
//! its internal state: see `vmexit/vmexit.cpp`:44-45 and `vmexit/first/first.cpp`:31-60.

use anyhow::{Result, ensure};

/// Existing vendor ABI key, placed in RCX bits 16..63.
#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
const ALLOWED_KEY: u64 = 0x8A3B55C7927;
#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
const TEST_OPERATION: u16 = 0;
#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
const CURRENT_CR3_OPERATION: u16 = 1;
const EXPECTED_RESPONSE: u64 = 0x1337;

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
    let response: u64;
    let _saved_rbx: u64;

    // SAFETY: CPUID is available to CPL3 on Windows x86-64. This follows the vendor ABI,
    // zeroing its five unused argument registers. LLVM reserves RBX for internal use on
    // some targets, so save/restore it through a distinct early-output scratch register
    // instead of declaring an RBX operand. Full-width RAX retains 64-bit CR3 responses;
    // __cpuid_count would truncate both the RCX request and returned RAX to 32 bits.
    // The asm uses neither the stack nor pointers; all changed allocatable registers
    // are outputs. Deliberately omit pure/nomem: readiness depends on external state,
    // and an attachment's initial VM exit can initialize its internal state.
    unsafe {
        core::arch::asm!(
            "mov {saved_rbx:r}, rbx",
            "cpuid",
            "mov rbx, {saved_rbx:r}",
            saved_rbx = out(reg) _saved_rbx,
            inout("rax") 0_u64 => response,
            inout("rcx") packed_request(operation) => _,
            inout("rdx") 0_u64 => _,
            inout("r8") 0_u64 => _,
            inout("r9") 0_u64 => _,
            inout("r10") 0_u64 => _,
            inout("r11") 0_u64 => _,
            options(nostack, preserves_flags),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_preserve_the_full_key_and_operation() {
        assert_eq!(packed_request(TEST_OPERATION), 0x08A3_B55C_7927_0000);
        assert_eq!(packed_request(CURRENT_CR3_OPERATION), 0x08A3_B55C_7927_0001);
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

    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    #[test]
    fn unsupported_platform_returns_an_error() {
        assert!(probe().is_err());
    }
}
