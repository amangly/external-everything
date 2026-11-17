use crate::contract;
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

pub fn inspect_contract(executable: &Path, game: &str) -> Result<contract::Contract> {
    let mut command = Command::new(executable);
    command
        .args(["contract", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let mut child = OwnedRuntime {
        child: command.spawn().context("Start runtime contract check")?,
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success(),
                "Runtime contract check exited with {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            bail!("Runtime contract check timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut output = Vec::new();
    child
        .child
        .stdout
        .take()
        .context("Runtime contract output is absent")?
        .take(65537)
        .read_to_end(&mut output)?;
    contract::validate(&output, game)
}

pub struct OwnedRuntime {
    child: Child,
}
impl OwnedRuntime {
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.child
            .try_wait()
            .context("Check external runtime process")
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}
impl Drop for OwnedRuntime {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn start(executable: &Path, game: &str) -> Result<OwnedRuntime> {
    inspect_contract(executable, game)?;
    #[cfg(windows)]
    {
        let events = events::StartupEvents::create()?;
        let directory = executable
            .parent()
            .context("External runtime directory is absent")?;
        let mut runtime = OwnedRuntime {
            child: Command::new(executable)
                .current_dir(directory)
                .args(["--game", game])
                .env(contract::STARTED_EVENT, &events.started_name)
                .env(contract::READY_EVENT, &events.ready_name)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("Open external game window")?,
        };
        events.wait_started(&mut runtime, Duration::from_secs(8))?;
        events.wait_ready(&mut runtime, Duration::from_secs(20))?;
        Ok(runtime)
    }
    #[cfg(not(windows))]
    {
        let _ = (executable, game);
        bail!("External runtime startup requires Windows")
    }
}

#[cfg(windows)]
mod events {
    use super::*;
    use std::{
        ffi::OsStr,
        os::windows::ffi::OsStrExt,
        ptr::null,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{CreateEventW, WaitForSingleObject},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Event(HANDLE);
    impl Drop for Event {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    impl Event {
        fn create(name: &str) -> Result<Self> {
            let name: Vec<_> = OsStr::new(name).encode_wide().chain([0]).collect();
            let handle = unsafe { CreateEventW(null(), 1, 0, name.as_ptr()) };
            ensure!(
                !handle.is_null(),
                "Create runtime startup event failed: {}",
                unsafe { GetLastError() }
            );
            Ok(Self(handle))
        }
        fn signaled(&self) -> Result<bool> {
            let result = unsafe { WaitForSingleObject(self.0, 0) };
            match result {
                WAIT_OBJECT_0 => Ok(true),
                WAIT_TIMEOUT => Ok(false),
                _ => bail!("Wait for runtime startup event failed: {}", unsafe {
                    GetLastError()
                }),
            }
        }
    }
    pub struct StartupEvents {
        pub started_name: String,
        pub ready_name: String,
        started: Event,
        ready: Event,
    }
    impl StartupEvents {
        pub fn create() -> Result<Self> {
            let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let prefix = format!(
                r"Local\ExternalEverything-runtime-{}-{nonce:x}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let started_name = format!("{prefix}-started");
            let ready_name = format!("{prefix}-ready");
            let started = Event::create(&started_name)?;
            let ready = Event::create(&ready_name)?;
            Ok(Self {
                started_name,
                ready_name,
                started,
                ready,
            })
        }
        fn wait(
            event: &Event,
            runtime: &mut OwnedRuntime,
            deadline: Duration,
            label: &str,
        ) -> Result<()> {
            let end = Instant::now() + deadline;
            loop {
                if let Some(status) = runtime.try_wait()? {
                    bail!("External runtime exited before {label}: {status}");
                }
                if event.signaled()? {
                    return Ok(());
                }
                if Instant::now() >= end {
                    bail!("External runtime timed out before {label}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        pub fn wait_started(&self, runtime: &mut OwnedRuntime, deadline: Duration) -> Result<()> {
            Self::wait(&self.started, runtime, deadline, "started")
        }
        pub fn wait_ready(&self, runtime: &mut OwnedRuntime, deadline: Duration) -> Result<()> {
            Self::wait(&self.ready, runtime, deadline, "ready")
        }
    }
}
