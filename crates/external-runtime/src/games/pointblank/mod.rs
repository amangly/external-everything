//! Read-only, typed PointBlank frame capture with explicit x86 build profiles.
//!
//! Legacy entry points retain [`LEGACY_PROFILE`] and [`EXPECTED_SHA256`]. Profile-aware
//! callers verify the on-disk image against their [`BuildProfile::sha256`] before
//! capture, which also verifies remote PE identity before following game offsets.

mod geometry;
pub mod offsets;
mod reader;

pub use offsets::{
    BuildProfile, CURRENT_PROFILE, EXPECTED_SHA256, IMAGE_SIZE, LEGACY_PROFILE, PROCESS_NAMES,
};
pub use reader::{
    capture, capture_profile, find_manager, find_manager_profile, validate_image,
    validate_image_profile,
};

/// A complete direct virtual-memory read in the already selected target process.
///
/// Implementations return an error for a short read. The reader neither owns a
/// process handle nor adapts the old command bridge into eight-byte transactions.
pub trait Memory {
    fn read(&self, address: u64, out: &mut [u8]) -> anyhow::Result<()>;
}

#[derive(Clone, Debug, Default)]
pub struct Frame {
    /// `FullCollection`, `SingleAvatar`, or an explicit waiting/rejected state.
    pub status: String,
    pub width: f32,
    pub height: f32,
    pub entities: Vec<Entity>,
    pub manager: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Entity {
    pub address: u32,
    /// The source float is preserved, including observed values above 100.
    pub health: f32,
    /// Verified body-transform translation; semantic foot anchoring is unresolved.
    pub position: [f32; 3],
    /// Actual `[left, top, width, height]` pixels in the source camera viewport.
    /// A standing-height presentation estimate supplies the box, rather than bones.
    pub screen: Option<[f32; 4]>,
}

#[cfg(test)]
mod tests;
