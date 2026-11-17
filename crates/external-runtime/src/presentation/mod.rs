//! PointBlank presentation using the copied Palepale native renderers.
//! Source: amangly/palepale@6490070271354e2a13a30e107065338c530a7322,
//! crates/df/src/pubg/{mod,menu,settings,esp}.rs and common renderer APIs.
mod esp;
#[cfg(windows)]
mod menu;
#[cfg(windows)]
mod native;
mod settings;

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
