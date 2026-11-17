//! PointBlank presentation using the copied Palepale native renderers.
//! Source: amangly/palepale@6490070271354e2a13a30e107065338c530a7322,
//! crates/df/src/pubg/{mod,menu,settings,esp}.rs and common renderer APIs.
mod esp;
#[cfg(windows)]
mod macros;
#[cfg(windows)]
mod menu;
#[cfg(windows)]
mod native;
mod settings;
mod timing;

#[derive(Debug, Default, serde::Serialize)]
pub struct RunReport {
    /// Successful Direct2D BeginDraw/EndDraw passes, including transparent clears.
    pub frames: u64,
    /// Successful visible D3D11/ImGui menu presentations.
    pub menu_frames: u64,
    /// Entity count in the most recently received real game snapshot.
    pub entities: usize,
    /// Successful frames that drew at least one projected live entity.
    pub esp_frames: u64,
    /// Largest live projected-entity count painted during this session.
    pub projected_entities: usize,
    pub game_pid: Option<u32>,
    pub d3d11_ready: bool,
    pub d2d_ready: bool,
    pub elapsed_ms: f64,
    pub render_hz: f64,
    pub reader_polls: u64,
    pub reader_hz: f64,
    pub accepted_samples: u64,
    pub transient_samples: u64,
    pub capture_mean_ms: f64,
    pub capture_max_ms: f64,
    pub data_age_mean_ms: f64,
    pub data_age_p95_ms: f64,
    pub data_age_max_ms: f64,
    pub time_to_collection_ms: Option<f64>,
    pub time_to_first_esp_ms: Option<f64>,
}

#[cfg(windows)]
pub use native::run;

#[cfg(not(windows))]
pub fn run(
    _engine: crate::engine::Engine,
    _limit: Option<std::time::Duration>,
) -> anyhow::Result<RunReport> {
    anyhow::bail!("Native presentation requires Windows")
}
