//! Windows module metadata and scoped privilege handling. This module performs zero
//! OpenProcess/ReadProcessMemory/WriteProcessMemory calls or kernel writes.

use std::ffi::{OsString, c_void};
use std::fs::File;
use std::io::Read;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::Mutex;

use anyhow::{Context, Result, bail, ensure};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_SUCCESS, GetLastError, HANDLE, SetLastError,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_DEBUG_NAME,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::ProcessStatus::{
    K32EnumDeviceDrivers, K32GetDeviceDriverBaseNameW,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcessToken,
};

use crate::Connection;
use crate::pe::Pe;
use crate::process::{CurrentIdentity, KernelLayout, verify_kernel_image};

const MAX_MODULE_QUERY: usize = 64 * 1024 * 1024;
const MAX_KERNEL_FILE: u64 = 64 * 1024 * 1024;
const MAX_MODULES: usize = 65_536;
// Serialize our own scoped token adjustments, including clones on other threads.
static MODULE_QUERY_LOCK: Mutex<()> = Mutex::new(());

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(
        class: u32,
        output: *mut c_void,
        size: u32,
        returned: *mut u32,
    ) -> i32;
}

pub(crate) struct KernelContext {
    pub layout: KernelLayout,
    pub identity: CurrentIdentity,
}

pub(crate) fn kernel_context(connection: &Connection) -> Result<KernelContext> {
    let module = kernel_module()?;
    let path = system_directory()?.join(&module.name);
    let file =
        File::open(&path).with_context(|| format!("open kernel image {}", path.display()))?;
    let size = file.metadata()?.len();
    ensure!(
        (4096..=MAX_KERNEL_FILE).contains(&size),
        "kernel file size exceeds the 64 MiB bound"
    );
    let mut image = Vec::with_capacity(size as usize);
    file.take(MAX_KERNEL_FILE + 1).read_to_end(&mut image)?;
    ensure!(
        image.len() as u64 <= MAX_KERNEL_FILE,
        "kernel file grew beyond its read bound"
    );
    let pe = Pe::parse(&image).context("parse actual SystemDirectory kernel PE exports")?;
    verify_kernel_image(connection, connection.current_cr3(), module.base, &pe)?;
    let layout = KernelLayout::from_kernel(&pe, module.base)?;
    let peb: u64;
    // SAFETY: GS:[0x60] is the native Windows x64 TEB.ProcessEnvironmentBlock field.
    unsafe {
        core::arch::asm!(
            "mov {}, qword ptr gs:[0x60]",
            out(reg) peb,
            options(nostack, readonly, preserves_flags),
        );
    }
    Ok(KernelContext {
        layout,
        identity: CurrentIdentity {
            // SAFETY: GetCurrentProcessId requires zero arguments or owned resources.
            pid: unsafe { GetCurrentProcessId() },
            cr3: connection.current_cr3(),
            peb,
        },
    })
}

fn system_directory() -> Result<PathBuf> {
    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: buffer holds the indicated number of writable WCHARs.
    let count = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    ensure!(
        count > 0 && count < buffer.len(),
        "GetSystemDirectoryW failed or exceeded its bound"
    );
    Ok(PathBuf::from(OsString::from_wide(&buffer[..count])))
}

struct DebugPrivilege {
    token: HANDLE,
    previous: TOKEN_PRIVILEGES,
    active: bool,
}

impl DebugPrivilege {
    fn enable() -> Result<Self> {
        let mut token = null_mut();
        // SAFETY: current pseudo-handle is valid; token is a writable owned-handle output.
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES,
                &mut token,
            )
        };
        ensure!(
            opened != 0,
            "OpenProcessToken for scoped SeDebugPrivilege: {}",
            std::io::Error::last_os_error()
        );
        let mut guard = Self {
            token,
            previous: TOKEN_PRIVILEGES::default(),
            active: false,
        };
        let mut luid = Default::default();
        // SAFETY: the API receives a constant terminated privilege name and an output LUID.
        ensure!(
            unsafe { LookupPrivilegeValueW(null(), SE_DEBUG_NAME, &mut luid) } != 0,
            "LookupPrivilegeValueW(SeDebugPrivilege): {}",
            std::io::Error::last_os_error()
        );
        let requested = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let mut returned = 0;
        // SAFETY: both structures describe one privilege; previous state is retained for
        // exact restoration, including an already-enabled privilege's zero-change result.
        let (ok, error) = unsafe {
            SetLastError(ERROR_SUCCESS);
            let ok = AdjustTokenPrivileges(
                token,
                0,
                &requested,
                core::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                &mut guard.previous,
                &mut returned,
            );
            (ok, GetLastError())
        };
        guard.active = ok != 0 && guard.previous.PrivilegeCount != 0;
        ensure!(
            ok != 0 && error == ERROR_SUCCESS,
            "scoped SeDebugPrivilege adjustment failed with Win32 {error}"
        );
        Ok(guard)
    }

    fn restore(&mut self) -> Result<()> {
        if self.active {
            // SAFETY: restore the exact one-privilege state returned by Windows.
            let (ok, error) = unsafe {
                SetLastError(ERROR_SUCCESS);
                let ok =
                    AdjustTokenPrivileges(self.token, 0, &self.previous, 0, null_mut(), null_mut());
                (ok, GetLastError())
            };
            ensure!(
                ok != 0 && error == ERROR_SUCCESS,
                "scoped SeDebugPrivilege restoration failed with Win32 {error}"
            );
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for DebugPrivilege {
    fn drop(&mut self) {
        // The normal path checks restoration errors. This also restores on early return.
        let _ = self.restore();
        // SAFETY: successful OpenProcessToken transferred this real handle to the guard.
        unsafe {
            CloseHandle(self.token);
        }
    }
}

struct KernelModule {
    base: u64,
    name: String,
}

fn kernel_name(name: &str) -> bool {
    [
        "ntoskrnl.exe",
        "ntkrnlmp.exe",
        "ntkrnlpa.exe",
        "ntkrpamp.exe",
    ]
    .iter()
    .any(|wanted| name.eq_ignore_ascii_case(wanted))
}

fn kernel_module() -> Result<KernelModule> {
    let _lock = MODULE_QUERY_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("module-query privilege lock was poisoned"))?;
    let mut privilege = DebugPrivilege::enable();
    // A pre-enabled token or a platform permitting the NT query may still work when
    // token adjustment is restricted, so preserve both query diagnostics on failure.
    let result = k32_kernel_module().or_else(|k32| {
        nt_kernel_module()
            .with_context(|| format!("K32EnumDeviceDrivers path also failed: {k32:#}"))
    });
    if let Ok(guard) = &mut privilege {
        guard.restore()?;
    }
    result.with_context(|| match privilege {
        Ok(_) => "kernel-module enumeration with scoped SeDebugPrivilege".to_owned(),
        Err(error) => format!("kernel-module enumeration; privilege detail: {error:#}"),
    })
}

fn k32_kernel_module() -> Result<KernelModule> {
    let mut needed = 0_u32;
    // SAFETY: zero-capacity sizing call; only needed is writable.
    let ok = unsafe { K32EnumDeviceDrivers(null_mut(), 0, &mut needed) };
    ensure!(
        ok != 0 && needed > 0,
        "K32EnumDeviceDrivers sizing failed: {}",
        std::io::Error::last_os_error()
    );
    for _ in 0..6 {
        let count = needed as usize / core::mem::size_of::<usize>();
        ensure!(
            (needed as usize).is_multiple_of(core::mem::size_of::<usize>())
                && (1..=MAX_MODULES).contains(&count),
            "K32EnumDeviceDrivers count exceeds bounds"
        );
        let mut bases = vec![null_mut::<c_void>(); count];
        let capacity = u32::try_from(bases.len() * core::mem::size_of::<usize>())?;
        // SAFETY: bases has the indicated byte capacity and needed is a valid output.
        let ok = unsafe { K32EnumDeviceDrivers(bases.as_mut_ptr(), capacity, &mut needed) };
        ensure!(
            ok != 0,
            "K32EnumDeviceDrivers fill failed: {}",
            std::io::Error::last_os_error()
        );
        if needed > capacity {
            continue;
        }
        ensure!(
            (needed as usize).is_multiple_of(core::mem::size_of::<usize>()),
            "invalid module-list byte count"
        );
        for &base in bases
            .iter()
            .take(needed as usize / core::mem::size_of::<usize>())
        {
            if base.is_null() {
                continue; // Windows 11 can redact all bases when SeDebugPrivilege is disabled.
            }
            let mut name = [0_u16; 260];
            // SAFETY: enumerated base is an opaque input, name is a bounded output buffer.
            let count =
                unsafe { K32GetDeviceDriverBaseNameW(base, name.as_mut_ptr(), name.len() as u32) }
                    as usize;
            if count == 0 || count >= name.len() {
                continue;
            }
            let name =
                String::from_utf16(&name[..count]).context("kernel module basename UTF-16")?;
            if kernel_name(&name) {
                return Ok(KernelModule {
                    base: base as usize as u64,
                    name,
                });
            }
        }
        bail!("kernel module base was absent/redacted in K32EnumDeviceDrivers output")
    }
    bail!("K32EnumDeviceDrivers list changed across six attempts")
}

fn nt_kernel_module() -> Result<KernelModule> {
    // Matches vendor/usermode/src/nt/query.cpp's class-11 query and bounded growth rules.
    let mut needed = 0_u32;
    // SAFETY: class 11 supports a zero-capacity sizing query.
    let initial = unsafe { NtQuerySystemInformation(11, null_mut(), 0, &mut needed) };
    ensure!(
        needed > 0 && needed as usize <= MAX_MODULE_QUERY,
        "NtQuerySystemInformation(11) sizing NTSTATUS={:#x} bytes={needed}",
        initial as u32
    );
    for _ in 0..6 {
        let mut buffer = vec![0_u8; needed as usize];
        let mut returned = 0_u32;
        // SAFETY: buffer is valid for needed bytes, returned is a writable ULONG.
        let status = unsafe {
            NtQuerySystemInformation(11, buffer.as_mut_ptr().cast(), needed, &mut returned)
        };
        if status >= 0 {
            ensure!(
                returned as usize <= buffer.len(),
                "NT module query returned an invalid length"
            );
            if returned > 0 {
                buffer.truncate(returned as usize);
            }
            return parse_nt_module_list(&buffer);
        }
        ensure!(
            [0xc0000004, 0xc0000023].contains(&(status as u32)),
            "NtQuerySystemInformation(11) NTSTATUS={:#x} bytes={returned}",
            status as u32
        );
        let next = (returned as usize).max(
            (needed as usize)
                .checked_mul(2)
                .context("module query size overflow")?,
        );
        ensure!(
            next <= MAX_MODULE_QUERY,
            "NT module query exceeds its 64 MiB bound"
        );
        needed = next as u32;
    }
    bail!("NtQuerySystemInformation(11) list changed across six attempts")
}

fn parse_nt_module_list(buffer: &[u8]) -> Result<KernelModule> {
    use crate::pe::{bytes, u16_at, u32_at};
    // Native x64 RTL_PROCESS_MODULES: ULONG count + padding, then 296-byte entries.
    // Entry: Section/Mapping/Image pointers (24), image size/flags (8), four USHORTs
    // (8), and FullPathName[256]. OffsetToFileName is the last USHORT at +38.
    const ENTRY_SIZE: usize = 296;
    let count = u32_at(buffer, 0)? as usize;
    ensure!(
        (1..=MAX_MODULES).contains(&count),
        "NT module count exceeds bounds"
    );
    bytes(
        buffer,
        8,
        count
            .checked_mul(ENTRY_SIZE)
            .context("module list size overflow")?,
    )?;
    for index in 0..count {
        let entry = bytes(buffer, 8 + index * ENTRY_SIZE, ENTRY_SIZE)?;
        let base = u64::from_le_bytes(bytes(entry, 16, 8)?.try_into()?);
        let filename_offset = u16_at(entry, 38)? as usize;
        ensure!(
            filename_offset < 256,
            "NT module filename offset exceeds path buffer"
        );
        let path = bytes(entry, 40, 256)?;
        let end = path
            .iter()
            .position(|&b| b == 0)
            .context("unterminated NT module path")?;
        ensure!(
            filename_offset <= end,
            "NT module filename starts beyond path terminator"
        );
        let name = std::str::from_utf8(&path[filename_offset..end])
            .context("NT module basename encoding")?;
        if base != 0 && kernel_name(name) {
            return Ok(KernelModule {
                base,
                name: name.to_owned(),
            });
        }
    }
    bail!("native kernel module base was absent/redacted in NT class-11 output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_nt_module_list_is_bounded_and_maps_x64_fields() {
        let mut bytes = vec![0_u8; 8 + 296];
        bytes[..4].copy_from_slice(&1_u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&0xffff_f800_1234_0000_u64.to_le_bytes());
        let path = b"\\SystemRoot\\system32\\ntoskrnl.exe\0";
        let filename = path.iter().rposition(|&b| b == b'\\').unwrap() + 1;
        bytes[46..48].copy_from_slice(&(filename as u16).to_le_bytes());
        bytes[48..48 + path.len()].copy_from_slice(path);
        let module = parse_nt_module_list(&bytes).unwrap();
        assert_eq!(module.name, "ntoskrnl.exe");
        assert_eq!(module.base, 0xffff_f800_1234_0000);
        assert!(parse_nt_module_list(&bytes[..bytes.len() - 1]).is_err());
        bytes[46..48].copy_from_slice(&256_u16.to_le_bytes());
        assert!(parse_nt_module_list(&bytes).is_err());
    }
}
