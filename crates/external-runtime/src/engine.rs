use crate::games::pointblank::{self, Frame, Memory};
use anyhow::{Context, Result, ensure};
use hyperrev_transport::{Connection, Process};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub status: String,
    pub detail: String,
    pub pid: Option<u32>,
    pub base: Option<u64>,
    pub cr3: Option<u64>,
    pub frame: Option<Frame>,
}

impl Snapshot {
    pub fn waiting(message: impl Into<String>) -> Self {
        Self {
            status: message.into(),
            detail: String::new(),
            pid: None,
            base: None,
            cr3: None,
            frame: None,
        }
    }
}

struct TargetMemory<'a> {
    connection: &'a Connection,
    cr3: u64,
}
impl Memory for TargetMemory<'_> {
    fn read(&self, address: u64, out: &mut [u8]) -> Result<()> {
        self.connection.read(self.cr3, address, out)
    }
}

pub struct Engine {
    connection: Connection,
    process: Option<Process>,
    profile: Option<pointblank::BuildProfile>,
    observed: Option<Process>,
    manager: Option<u32>,
    regions: Vec<(u64, usize)>,
    region_index: usize,
    region_offset: usize,
    ticks: u64,
}

impl Engine {
    pub fn new(connection: Connection) -> Self {
        Self {
            connection,
            process: None,
            profile: None,
            observed: None,
            manager: None,
            regions: Vec::new(),
            region_index: 0,
            region_offset: 0,
            ticks: 0,
        }
    }

    pub fn memory_check(&self) -> Result<()> {
        self.connection.self_test()
    }

    pub fn poll(&mut self) -> Snapshot {
        match self.capture() {
            Ok(snapshot) => snapshot,
            Err(error) => Snapshot {
                status: "PointBlank check failed".into(),
                detail: format!("{error:#}"),
                pid: self
                    .process
                    .as_ref()
                    .or(self.observed.as_ref())
                    .map(|p| p.pid),
                base: self
                    .process
                    .as_ref()
                    .or(self.observed.as_ref())
                    .map(|p| p.image_base),
                cr3: self
                    .process
                    .as_ref()
                    .or(self.observed.as_ref())
                    .map(|p| p.cr3),
                frame: None,
            },
        }
    }

    fn capture(&mut self) -> Result<Snapshot> {
        self.ticks = self.ticks.wrapping_add(1);
        if let Some(process) = &self.process
            && self.ticks.is_multiple_of(10)
        {
            let same = hyperrev_transport::process_by_pid(&self.connection, process.pid)?;
            if same.as_ref().is_none_or(|current| {
                current.eprocess != process.eprocess
                    || current.image_base != process.image_base
                    || current.cr3 != process.cr3
            }) {
                self.process = None;
                self.profile = None;
                self.observed = None;
                self.manager = None;
                self.regions.clear();
            }
        }
        if self.process.is_none() {
            if self.ticks > 1 && !self.ticks.is_multiple_of(10) {
                return Ok(Snapshot::waiting("Waiting for PointBlank"));
            }
            let Some(pid) = native::game_pid(pointblank::PROCESS_NAMES)? else {
                return Ok(Snapshot::waiting("Waiting for PointBlank"));
            };
            let Some(process) = hyperrev_transport::process_by_pid(&self.connection, pid)? else {
                return Ok(Snapshot::waiting("Waiting for PointBlank"));
            };
            ensure!(
                process.image_base <= u32::MAX as u64,
                "PointBlank offsets require a PE32 process"
            );
            self.observed = Some(process.clone());
            let path = native::image_path(process.pid)?;
            let profile = verify_image(&path)?;
            ensure!(
                process.image_size == profile.image_size,
                "PointBlank image size differs from the selected verified build"
            );
            self.profile = Some(profile);
            self.regions = native::regions(process.pid).unwrap_or_default();
            self.region_index = 0;
            self.region_offset = 0;
            self.manager = None;
            self.process = Some(process);
        }
        let process = self
            .process
            .as_ref()
            .context("PointBlank target disappeared")?;
        let profile = self
            .profile
            .as_ref()
            .context("PointBlank build profile is absent")?;
        let memory = TargetMemory {
            connection: &self.connection,
            cr3: process.cr3,
        };
        // Bound discovery to eight windows (2 MiB) per background poll so the
        // first launch finds the typed collection promptly without blocking GPUI.
        for _ in 0..8 {
            if self.manager.is_some() || self.regions.is_empty() {
                break;
            }
            let (address, length) = self.regions[self.region_index];
            let remaining = length.saturating_sub(self.region_offset);
            let length = remaining.min(256 * 1024);
            if length > 0 {
                self.manager = pointblank::find_manager_profile(
                    &memory,
                    process.image_base as u32,
                    &[(address + self.region_offset as u64, length)],
                    2 * length + 4096,
                    profile,
                )?;
            }
            self.region_offset = self.region_offset.saturating_add(length);
            if self.region_offset >= self.regions[self.region_index].1 {
                self.region_offset = 0;
                self.region_index = (self.region_index + 1) % self.regions.len();
            }
        }
        let mut frame = match pointblank::capture_profile(
            &memory,
            process.image_base as u32,
            self.manager,
            profile,
        ) {
            Ok(frame) => frame,
            Err(_) if self.manager.is_some() => {
                self.manager = None;
                pointblank::capture_profile(&memory, process.image_base as u32, None, profile)?
            }
            Err(error) => return Err(error),
        };
        if frame.status.starts_with("CollectionInvalid:") {
            self.manager = None;
            frame = pointblank::capture_profile(&memory, process.image_base as u32, None, profile)?;
        }
        Ok(Snapshot {
            status: frame.status.clone(),
            detail: format!(
                "Capture: {}\nOffset build: {}\nExecutable SHA-256: {}\nManager: {:?}\nEntities: {}\nReads: hyper-reV CPUID virtual-copy",
                frame.status,
                profile.id,
                profile.sha256,
                frame.manager,
                frame.entities.len()
            ),
            pid: Some(process.pid),
            base: Some(process.image_base),
            cr3: Some(process.cr3),
            frame: Some(frame),
        })
    }
}

pub fn target_image_path(pid: u32) -> Result<std::path::PathBuf> {
    native::image_path(pid)
}

pub fn verify_image(path: &Path) -> Result<pointblank::BuildProfile> {
    let file = fs::File::open(path)
        .with_context(|| format!("Open PointBlank build {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= 128 * 1024 * 1024,
        "PointBlank executable is outside its size limit"
    );
    let mut bytes = Vec::new();
    file.take(128 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 128 * 1024 * 1024,
        "PointBlank executable exceeds its size limit"
    );
    profile_for_hash(&format!("{:x}", Sha256::digest(&bytes)))
}

fn profile_for_hash(hash: &str) -> Result<pointblank::BuildProfile> {
    [pointblank::LEGACY_PROFILE, pointblank::CURRENT_PROFILE]
        .into_iter()
        .find(|profile| profile.sha256 == hash)
        .context("PointBlank executable SHA-256 is outside the verified build profiles")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_known_pointblank_hashes_select_profiles() {
        assert_eq!(
            profile_for_hash(pointblank::LEGACY_PROFILE.sha256)
                .unwrap()
                .id,
            pointblank::LEGACY_PROFILE.id
        );
        assert_eq!(
            profile_for_hash(pointblank::CURRENT_PROFILE.sha256)
                .unwrap()
                .id,
            pointblank::CURRENT_PROFILE.id
        );
        assert!(profile_for_hash(&"0".repeat(64)).is_err());
        assert!(profile_for_hash("").is_err());
    }
}

#[cfg(windows)]
mod native {
    use anyhow::{Context, Result, ensure};
    use std::{ffi::OsString, mem::size_of, os::windows::ffi::OsStringExt, path::PathBuf};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Memory::{
                MEM_COMMIT, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS,
                VirtualQueryEx,
            },
            Threading::{
                OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
        },
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub fn game_pid(names: &[&str]) -> Result<Option<u32>> {
        use std::mem::size_of;
        use windows_sys::Win32::{
            Foundation::{HWND, INVALID_HANDLE_VALUE, LPARAM},
            System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            UI::WindowsAndMessaging::{
                EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible,
            },
        };
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        ensure!(
            raw != INVALID_HANDLE_VALUE,
            "List game processes: {}",
            std::io::Error::last_os_error()
        );
        let snapshot = Handle(raw);
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut candidates = Vec::new();
        let mut available = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while available {
            let end = entry
                .szExeFile
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if names.iter().any(|wanted| name.eq_ignore_ascii_case(wanted)) {
                candidates.push(entry.th32ProcessID);
            }
            available = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        struct Find<'a> {
            candidates: &'a [u32],
            pid: Option<u32>,
        }
        unsafe extern "system" fn visible(window: HWND, param: LPARAM) -> i32 {
            let find = unsafe { &mut *(param as *mut Find<'_>) };
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(window, &mut pid);
            }
            if find.candidates.contains(&pid)
                && unsafe { IsWindowVisible(window) } != 0
                && unsafe { GetWindow(window, GW_OWNER) }.is_null()
            {
                find.pid = Some(pid);
                return 0;
            }
            1
        }
        let mut find = Find {
            candidates: &candidates,
            pid: None,
        };
        unsafe {
            EnumWindows(Some(visible), (&mut find as *mut Find<'_>) as LPARAM);
        }
        Ok(find.pid.or_else(|| candidates.first().copied()))
    }
    pub fn image_path(pid: u32) -> Result<PathBuf> {
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        ensure!(
            !raw.is_null(),
            "Query PointBlank process path: {}",
            std::io::Error::last_os_error()
        );
        let handle = Handle(raw);
        let mut data = vec![0u16; 32768];
        let mut size = data.len() as u32;
        ensure!(
            unsafe { QueryFullProcessImageNameW(handle.0, 0, data.as_mut_ptr(), &mut size) } != 0,
            "Read PointBlank image path: {}",
            std::io::Error::last_os_error()
        );
        Ok(PathBuf::from(OsString::from_wide(&data[..size as usize])))
    }
    pub fn regions(pid: u32) -> Result<Vec<(u64, usize)>> {
        let raw = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid) };
        ensure!(
            !raw.is_null(),
            "Query PointBlank memory regions: {}",
            std::io::Error::last_os_error()
        );
        let handle = Handle(raw);
        let mut address = 0x10000u64;
        let mut regions = Vec::new();
        for _ in 0..65536 {
            if address >= 0x1_0000_0000 {
                break;
            }
            let mut info = MEMORY_BASIC_INFORMATION::default();
            let count = unsafe {
                VirtualQueryEx(
                    handle.0,
                    address as *const _,
                    &mut info,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if count == 0 {
                break;
            }
            ensure!(
                count >= size_of::<MEMORY_BASIC_INFORMATION>(),
                "Truncated PointBlank region metadata"
            );
            let base = info.BaseAddress as u64;
            let length = info.RegionSize;
            let next = base
                .checked_add(length as u64)
                .context("PointBlank region overflow")?;
            ensure!(
                next > address,
                "PointBlank memory-region walk stopped advancing"
            );
            if info.State == MEM_COMMIT
                && info.Type == MEM_PRIVATE
                && info.Protect & (PAGE_GUARD | PAGE_NOACCESS) == 0
                && matches!(info.Protect & 0xff, 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80)
            {
                let length = (length as u64).min(0x1_0000_0000 - base) as usize;
                if length > 0 {
                    regions.push((base, length));
                }
            }
            address = next;
        }
        Ok(regions)
    }
}

#[cfg(not(windows))]
mod native {
    use anyhow::Result;
    use std::path::PathBuf;
    pub fn game_pid(_names: &[&str]) -> Result<Option<u32>> {
        anyhow::bail!("PointBlank discovery requires Windows")
    }
    pub fn image_path(_pid: u32) -> Result<PathBuf> {
        anyhow::bail!("PointBlank process discovery requires Windows")
    }
    pub fn regions(_pid: u32) -> Result<Vec<(u64, usize)>> {
        anyhow::bail!("PointBlank region discovery requires Windows")
    }
}
