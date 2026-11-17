#![cfg_attr(windows, windows_subsystem = "windows")]

mod backend;
mod bundle;
mod catalog;
mod diagnostics;
mod probe;
mod recovery;
mod ui;

const APP_TITLE: &str = "External Everything";

fn run() -> anyhow::Result<()> {
    if std::env::args().any(|argument| argument == "--internal-attachment-probe") {
        return probe::internal();
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if diagnostics::run(&args)? {
        return Ok(());
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
            serde_json::json!({"stage": stage, "cr3": cr3, "installed": snapshot.installed, "games": snapshot.games.len(), "single_file":bundle::available()})
        );
        return Ok(());
    }
    if std::env::args().any(|argument| argument == "--verify-runtime-launch") {
        let session = backend::launch("pointblank")?;
        let pid = session.pid();
        let staging = session.staging_root().map(std::path::Path::to_path_buf);
        drop(session);
        let cleaned = staging
            .as_ref()
            .map(|path| path.try_exists().map(|exists| !exists))
            .transpose()?;
        anyhow::ensure!(
            cleaned != Some(false),
            "Launcher session left its temporary runtime behind"
        );
        println!(
            "{}",
            serde_json::json!({"game":"pointblank","runtime_pid":pid,"started":true,"window_ready":true,"staging_cleaned":cleaned})
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
