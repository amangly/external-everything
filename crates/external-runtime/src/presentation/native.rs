// Render-loop plumbing adapted from amangly/palepale@6490070271354e2a13a30e107065338c530a7322
// crates/df/src/pubg/mod.rs: Overlay/Gui synchronization, visibility, click-through,
// menu polling and native frame pacing. Game capture remains the frozen Engine.
use super::{
    RunReport, esp,
    menu::Menu,
    settings::Settings,
    timing::{self, ReaderStats},
};
use crate::{
    common::{
        imgui_ui::Gui,
        keys,
        overlay::{self, Overlay},
    },
    contract,
    engine::{Engine, Snapshot},
};
use anyhow::{Context, Result, ensure};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_SMOKE_DURATION: Duration = Duration::from_secs(10);

struct Published {
    snapshot: Snapshot,
    accepted_at: Instant,
    stats: ReaderStats,
}

/// Owns the only Engine. Its mutex protects published snapshots, never I/O.
struct PollWorker {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Published>>,
    handle: Option<JoinHandle<()>>,
}

impl PollWorker {
    fn start(mut engine: Engine) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(Published {
            snapshot: Snapshot::waiting("Waiting for PointBlank"),
            accepted_at: Instant::now(),
            stats: ReaderStats::default(),
        }));
        let worker_stop = stop.clone();
        let worker_snapshot = snapshot.clone();
        let handle = thread::Builder::new()
            .name("pointblank-frame-reader".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    let started = Instant::now();
                    let next = engine.poll();
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(mut published) = worker_snapshot.lock() else {
                        break;
                    };
                    let status = next
                        .frame
                        .as_ref()
                        .map(|frame| frame.status.as_str())
                        .unwrap_or(&next.status);
                    let accepted = status.starts_with("FullCollection:")
                        || status.starts_with("SingleAvatar:");
                    let transient = status.starts_with("CollectionChanged:")
                        || status.starts_with("SingleAvatarChanged:");
                    let retain = timing::retain_transient(
                        status,
                        next.pid == published.snapshot.pid,
                        published.snapshot.frame.is_some(),
                        published.accepted_at.elapsed(),
                    );
                    published
                        .stats
                        .record(started.elapsed(), accepted, transient);
                    if !retain {
                        published.snapshot = next;
                        published.accepted_at = Instant::now();
                    }
                    drop(published);
                    // Use the reference's high-resolution waitable timer on this
                    // worker too; a 100-ms sleep previously capped reads at 10 Hz.
                    overlay::cap_hz(timing::READER_HZ);
                }
            })
            .context("Start owned PointBlank frame-reader worker")?;
        Ok(Self {
            stop,
            snapshot,
            handle: Some(handle),
        })
    }

    fn snapshot(&self) -> Result<(Snapshot, Option<f64>)> {
        ensure!(
            self.handle
                .as_ref()
                .is_some_and(|worker| !worker.is_finished()),
            "PointBlank frame-reader worker exited unexpectedly"
        );
        let published = self
            .snapshot
            .lock()
            .map_err(|_| anyhow::anyhow!("PointBlank snapshot lock was poisoned"))?;
        let age = published.accepted_at.elapsed();
        let mut snapshot = published.snapshot.clone();
        if snapshot.frame.is_some() && age > timing::MAX_DATA_AGE {
            snapshot.frame = None;
            snapshot.status = "Waiting for fresh frame".into();
        }
        let data_age = snapshot
            .frame
            .as_ref()
            .filter(|frame| !frame.entities.is_empty())
            .map(|_| age.as_secs_f64() * 1000.);
        Ok((snapshot, data_age))
    }

    fn statistics(&self) -> Result<ReaderStats> {
        Ok(self
            .snapshot
            .lock()
            .map_err(|_| anyhow::anyhow!("PointBlank snapshot lock was poisoned"))?
            .stats
            .clone())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("PointBlank frame-reader worker panicked"))?;
        }
        Ok(())
    }
}

impl Drop for PollWorker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct KeyLifetime;
impl KeyLifetime {
    fn start(menu_open: bool) -> Self {
        keys::reset();
        keys::set_menu(menu_open);
        keys::spawn();
        Self
    }
}
impl Drop for KeyLifetime {
    fn drop(&mut self) {
        keys::shutdown();
    }
}

fn pump_thread_quit() {
    use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_QUIT};
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, None, WM_QUIT, WM_QUIT, PM_REMOVE) }.as_bool() {
        keys::request_quit();
    }
}

/// Production passes `None`; render smoke passes a duration capped at ten seconds.
/// READY_EVENT is signaled only after both native windows and actual successful
/// Direct2D plus D3D11/ImGui menu paint passes, independently of game availability.
pub fn run(engine: Engine, limit: Option<Duration>) -> Result<RunReport> {
    let started = Instant::now();
    let mut settings = Settings::load();
    settings.menu = true;
    let mut overlay =
        Overlay::new_visible().context("Create native Direct2D/DirectWrite overlay")?;
    let mut gui = Gui::new_visible().context("Create native D3D11/ImGui menu")?;
    let _keys = KeyLifetime::start(settings.menu);
    let mut worker = PollWorker::start(engine)?;
    let mut menu = Menu::default();
    let limit = limit.map(|duration| duration.min(MAX_SMOKE_DURATION));
    let mut last_top = Instant::now();
    let mut report = RunReport::default();
    let mut ages = Vec::new();
    let mut ready = false;

    loop {
        overlay.pump();
        gui.pump();
        pump_thread_quit();
        if keys::quit() {
            break;
        }
        // Always execute the initial native passes, including a zero-duration
        // diagnostic request, before considering the smoke deadline.
        if ready && limit.is_some_and(|duration| started.elapsed() >= duration) {
            break;
        }

        let (snapshot, data_age) = worker.snapshot()?;
        if let Some(age) = data_age {
            if ages.len() == 4096 {
                ages.remove(0);
            }
            ages.push(age);
        }
        if report.time_to_collection_ms.is_none()
            && snapshot
                .frame
                .as_ref()
                .is_some_and(|frame| frame.manager.is_some() && !frame.entities.is_empty())
        {
            report.time_to_collection_ms = Some(started.elapsed().as_secs_f64() * 1000.);
        }
        report.game_pid = snapshot.pid;
        report.entities = snapshot
            .frame
            .as_ref()
            .map_or(0, |frame| frame.entities.len());
        overlay.sync(snapshot.pid.unwrap_or(0));
        gui.sync(&overlay);
        menu.poll(&mut settings);
        if !ready {
            settings.menu = true;
            keys::set_menu(true);
        }
        overlay.set_click_through(true);
        let draw_overlay = snapshot.frame.is_some() && settings.draw_any();
        if draw_overlay {
            overlay.show();
        } else {
            overlay.hide();
        }
        if settings.menu {
            gui.show();
            gui.set_click_through(false);
        } else {
            gui.set_click_through(true);
            gui.hide();
        }

        // Even an idle/game-closed start gets a real render target and a clear
        // pass. Clears also remove the previous snapshot when game data expires.
        overlay.begin().context("Begin Direct2D overlay frame")?;
        let projected = snapshot
            .frame
            .as_ref()
            .map_or(0, |frame| esp::draw(&overlay, frame, &settings));
        overlay.end().context("Present Direct2D overlay frame")?;
        if projected > 0 {
            report
                .time_to_first_esp_ms
                .get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.);
            report.esp_frames = report.esp_frames.saturating_add(1);
        }
        report.projected_entities = report.projected_entities.max(projected);
        report.frames = report.frames.saturating_add(1);
        report.d2d_ready = true;

        let menu_before_frame = settings.menu;
        if settings.menu {
            let _ = gui.frame(&overlay, |ui| menu.draw(ui, &mut settings, &snapshot));
            ensure!(!gui.paint_failed(), "D3D11/ImGui menu paint failed");
            ensure!(
                gui.surface_ready(),
                "D3D11/ImGui menu render target is absent after painting"
            );
            report.menu_frames = report.menu_frames.saturating_add(1);
            report.d3d11_ready = true;
        }
        // Synchronize local configuration changes only. Preserve Insert
        // transitions polled concurrently while the menu is painting.
        if settings.menu != menu_before_frame {
            keys::set_menu(settings.menu);
        }
        if !ready && report.d2d_ready && report.d3d11_ready {
            contract::signal(contract::READY_EVENT)
                .context("Signal native presentation readiness")?;
            ready = true;
        }
        if last_top.elapsed() >= Duration::from_millis(400) {
            if draw_overlay {
                overlay.pulse_top();
            }
            if settings.menu {
                gui.pulse_top();
            }
            last_top = Instant::now();
        }
        overlay::cap_hz(overlay.render_hz());
    }
    worker.shutdown()?;
    let seconds = started.elapsed().as_secs_f64().max(0.001);
    let stats = worker.statistics()?;
    report.elapsed_ms = seconds * 1000.;
    report.render_hz = report.frames as f64 / seconds;
    report.reader_polls = stats.polls;
    report.reader_hz = stats.polls as f64 / seconds;
    report.accepted_samples = stats.accepted;
    report.transient_samples = stats.transient;
    report.capture_mean_ms = stats.mean_ms();
    report.capture_max_ms = stats.max_ms;
    if !ages.is_empty() {
        report.data_age_mean_ms = ages.iter().sum::<f64>() / ages.len() as f64;
        ages.sort_by(f64::total_cmp);
        report.data_age_max_ms = ages[ages.len() - 1];
        report.data_age_p95_ms = ages[((ages.len() - 1) as f64 * 0.95).ceil() as usize];
    }
    ensure!(
        ready,
        "Native presentation exited before its first successful menu frame"
    );
    Ok(report)
}
