// Adapted from Palepale crates/df/src/common/keys.rs at
// 6490070271354e2a13a30e107065338c530a7322 (build/reference/palepale).
// Original Insert/End polling and public state API retained. The polling worker
// is owned and joinable; keyboard hooks and transport/stealth calls are omitted.
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_END, VK_INSERT};

static QUIT: AtomicBool = AtomicBool::new(false);
static MENU: AtomicBool = AtomicBool::new(false);
static LAST_INS: AtomicU64 = AtomicU64::new(0);
static POLLER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

pub fn quit() -> bool {
    QUIT.load(Ordering::Relaxed)
}

pub fn menu_open() -> bool {
    MENU.load(Ordering::Relaxed)
}

pub fn set_menu(open: bool) {
    MENU.store(open, Ordering::Relaxed);
}

pub fn request_quit() {
    if !QUIT.swap(true, Ordering::Relaxed) {
        super::olog::line("quit requested");
    }
}

fn toggle_menu() {
    let now = unsafe { GetTickCount64() };
    loop {
        let prev = LAST_INS.load(Ordering::Relaxed);
        if now.saturating_sub(prev) < 200 {
            return;
        }
        if LAST_INS
            .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            break;
        }
    }
    let open = !MENU.fetch_xor(true, Ordering::Relaxed);
    super::olog::line(if open { "insert open" } else { "insert hide" });
}

fn vk_down(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
}

fn poll_loop() {
    let mut prev_end = false;
    let mut prev_ins = false;
    super::olog::line("keys poll live");
    while !quit() {
        let end = vk_down(VK_END);
        let ins = vk_down(VK_INSERT);
        if end && !prev_end {
            request_quit();
        }
        if ins && !prev_ins {
            toggle_menu();
        }
        prev_end = end;
        prev_ins = ins;
        thread::sleep(Duration::from_millis(8));
    }
}

/// Start the original Insert/End polling worker once for this runtime session.
pub fn spawn() {
    let mut worker = POLLER.lock().unwrap_or_else(|error| error.into_inner());
    if worker.as_ref().is_some_and(|handle| !handle.is_finished()) {
        return;
    }
    if let Some(handle) = worker.take() {
        let _ = handle.join();
    }
    match thread::Builder::new()
        .name("keys-poll".into())
        .spawn(poll_loop)
    {
        Ok(handle) => *worker = Some(handle),
        Err(error) => {
            super::olog::line(&format!("keys poll failed: {error}"));
            request_quit();
        }
    }
}

/// Stop and join the polling worker during app-owned renderer teardown.
pub fn shutdown() {
    request_quit();
    let handle = POLLER
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

/// Prepare a subsequent renderer session after its previous worker has stopped.
pub fn reset() {
    shutdown();
    MENU.store(false, Ordering::Relaxed);
    LAST_INS.store(0, Ordering::Relaxed);
    QUIT.store(false, Ordering::Relaxed);
}
