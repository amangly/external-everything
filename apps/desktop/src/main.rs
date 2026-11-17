#![cfg_attr(windows, windows_subsystem = "windows")]

mod backend;
mod catalog;
mod probe;
mod ui;

const APP_TITLE: &str = "External Everything";

fn run() -> anyhow::Result<()> {
    if std::env::args().any(|argument| argument == "--internal-attachment-probe") {
        return probe::internal();
    }
    if std::env::args().any(|argument| argument == "--inspect-flow") {
        let snapshot = backend::inspect()?;
        let (stage, cr3) = match snapshot.attachment {
            hyperrev_transport::Readiness::Ready { cr3 } => ("games", Some(cr3)),
            hyperrev_transport::Readiness::Absent { .. } if snapshot.installed => {
                ("restart_required", None)
            }
            hyperrev_transport::Readiness::Absent { .. } => ("setup_required", None),
        };
        println!(
            "{}",
            serde_json::json!({"stage": stage, "cr3": cr3, "installed": snapshot.installed, "games": snapshot.games.len()})
        );
        return Ok(());
    }
    if hyperrev_loader::platform::elevate_gui_if_needed()? {
        return Ok(());
    }
    ui::run()
}

fn main() {
    if let Err(error) = run() {
        let message = format!("{APP_TITLE}: {error:#}");
        eprintln!("{message}");
        #[cfg(windows)]
        if !std::env::args().any(|argument| argument.starts_with("--")) {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            let text: Vec<_> = message.encode_utf16().chain([0]).collect();
            let title: Vec<_> = APP_TITLE.encode_utf16().chain([0]).collect();
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    text.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        std::process::exit(1);
    }
}
