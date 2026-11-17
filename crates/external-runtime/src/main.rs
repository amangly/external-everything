#![cfg_attr(windows, windows_subsystem = "windows")]

mod diagnostic;
use anyhow::{Result, ensure};
use external_runtime::{contract, engine::Engine};
use hyperrev_transport::Connection;

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.as_slice() == ["contract", "--json"] {
        println!("{}", serde_json::to_string(&contract::current())?);
        return Ok(());
    }
    if args.as_slice() == ["--self-test"] {
        let connection = Connection::open()?;
        connection.self_test()?;
        let process = hyperrev_transport::process_by_pid(&connection, std::process::id())?
            .ok_or_else(|| anyhow::anyhow!("Native current-process discovery failed"))?;
        let mut header = [0u8; 2];
        connection.read(process.cr3, process.image_base, &mut header)?;
        ensure!(
            &header == b"MZ",
            "Discovered current process does not expose its PE header through hypercalls"
        );
        println!(
            "{}",
            serde_json::json!({"attachment":true,"memory_read_write":"verified","process_discovery":"verified","pid":process.pid,"cr3":process.cr3,"base":process.image_base})
        );
        return Ok(());
    }
    if args.len() == 4 && args[0] == "--read-pointblank" {
        let result = diagnostic::read_ranges(
            args[1].parse::<u32>()?,
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
        )?;
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }
    if args.len() == 3 && args[0] == "--diagnose-pointblank" {
        let pid = args[1].parse::<u32>()?;
        let evidence = diagnostic::capture(pid, std::path::Path::new(&args[2]))?;
        println!(
            "{}",
            serde_json::json!({"pid":pid,"evidence":std::path::Path::new(&args[2]).join("evidence.json"),"image_size":evidence["image_size"],"disk_sha256":evidence["disk_sha256"]})
        );
        return Ok(());
    }
    if args.as_slice() == ["--snapshot"] || (args.len() == 2 && args[0] == "--snapshot-scan") {
        let attempts = if args.len() == 2 {
            args[1].parse::<usize>()?
        } else {
            1
        };
        ensure!(
            (1..=2000).contains(&attempts),
            "Snapshot scan requires 1..2000 bounded polling attempts"
        );
        let mut engine = Engine::new(Connection::open()?);
        let mut snapshot = engine.poll();
        for _ in 1..attempts {
            if snapshot
                .frame
                .as_ref()
                .is_some_and(|frame| frame.status.starts_with("FullCollection"))
            {
                break;
            }
            // Discovery is time-gated independently from live capture. Allow
            // bounded diagnostic scans to advance those gates instead of spinning.
            std::thread::sleep(std::time::Duration::from_millis(8));
            snapshot = engine.poll();
        }
        println!(
            "{}",
            serde_json::json!({"game":"pointblank","status":snapshot.status,"detail":snapshot.detail,"pid":snapshot.pid,"base":snapshot.base,"cr3":snapshot.cr3,"entities":snapshot.frame.as_ref().map(|frame|frame.entities.len()).unwrap_or(0),"capture":snapshot.frame.as_ref().map(|frame|serde_json::json!({"manager":frame.manager,"screen":[frame.width,frame.height],"entities":frame.entities.iter().map(|entity|serde_json::json!({"address":entity.address,"health":entity.health,"position":entity.position,"screen":entity.screen})).collect::<Vec<_>>()}))})
        );
        return Ok(());
    }
    let smoke = args.len() == 2 && args[0] == "--render-smoke";
    let limit = if smoke {
        let seconds = args[1].parse::<u64>()?;
        ensure!(
            (1..=10).contains(&seconds),
            "Render smoke duration requires 1..10 seconds"
        );
        Some(std::time::Duration::from_secs(seconds))
    } else {
        None
    };
    ensure!(
        smoke || args.as_slice() == ["--game", "pointblank"],
        "Use --game pointblank, contract --json, --snapshot, --self-test, or --render-smoke <seconds>"
    );
    contract::signal(contract::STARTED_EVENT)?;
    let engine = Engine::new(Connection::open()?);
    #[cfg(windows)]
    {
        let report = external_runtime::presentation::run(engine, limit)?;
        if smoke {
            println!("{}", serde_json::to_string(&report)?);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (engine, limit);
        anyhow::bail!("The external renderer requires Windows")
    }
}
fn main() {
    // Match Palepale's native entry point: graphics/window coordinates are
    // physical pixels, independently of the GPUI launcher's DPI initialization.
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    if let Err(error) = run() {
        let text = format!("PointBlank runtime: {error:#}");
        eprintln!("{text}");
        #[cfg(windows)]
        if std::env::var_os(contract::STARTED_EVENT).is_none()
            && std::env::args().any(|arg| arg == "--game")
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            let message: Vec<_> = text.encode_utf16().chain([0]).collect();
            let title: Vec<_> = "External Everything".encode_utf16().chain([0]).collect();
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    message.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        std::process::exit(1);
    }
}
