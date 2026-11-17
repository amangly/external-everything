//! Provenance: pointblank-cheat/internal/offsets_verified.h (pb_id_20261115),
//! rust_external/src/game.rs, and internal/frame_builder.h. See README.md.
//! Module constants are RVAs; object-field constants are byte offsets.

pub const PROCESS_NAMES: &[&str] = &["PointBlank.exe"];
pub const IMAGE_SIZE: u32 = 0x01f6_f000;
pub const EXPECTED_SHA256: &str =
    "edeeeef3e76caa41b13c0a66762da95b90e989f09b8dff7d8480a9bd302ee084";
pub const ADDRESS_LIMIT: u64 = 0x1_0000_0000;
pub const MIN_POINTER: u64 = 0x1_0000;

pub const FRAMEWORK_RVA: u32 = 0x0123_9088;
pub const FRAMEWORK_CAMERA: u32 = 0xec;
pub const CAMERA_WIDTH: u32 = 0xb8;
pub const CAMERA_HEIGHT: u32 = 0xbc;
pub const CAMERA_PROJECTION: u32 = 0xd0;
pub const CAMERA_VIEW: u32 = 0xd4;
pub const ATTRIBUTE_MATRIX: u32 = 0x30;

pub const FRAMEWORK_VTABLE_RVA: u32 = 0x00e5_e064;
pub const CAMERA_VTABLE_RVA: u32 = 0x00e1_f10c;
pub const VIEW_VTABLE_RVA: u32 = 0x00e2_53ec;
pub const PROJECTION_VTABLE_RVA: u32 = 0x00e2_535c;
pub const CHARACTER_MANAGER_VTABLE_RVA: u32 = 0x00e6_8ef4;
pub const AVATAR_VTABLE_RVA: u32 = 0x00e9_9c78;
/// frame_builder.h: RTTI evidence transforms-29456.json.
pub const MATRIX_VTABLE_RVA: u32 = 0x00e1_9d5c;

pub const CHARACTER_MANAGER_IMPL: u32 = 0x14;
pub const IMPL_AVATAR_BEGIN: u32 = 0x08;
pub const IMPL_AVATAR_END: u32 = 0x0c;
pub const MAX_COLLECTION: usize = 128;

/// Callback dispatcher's observed current-Avatar slot, never a collection root.
pub const CURRENT_AVATAR_RVA: u32 = 0x0125_e100;
pub const AVATAR_MATRIX: u32 = 0xb88;
pub const AVATAR_HEALTH: u32 = 0xc38;
pub const HEALTH_ID: u32 = 0x30;
pub const HEALTH_VALUE: u32 = 0x34;
pub const MATRIX_TRANSLATION: u32 = 0x50;

/// Presentation estimates, rather than verified skeleton/body dimensions.
pub const BOX_HEIGHT_ESTIMATE: f32 = 1.8;
pub const BOX_WIDTH_HEIGHT_RATIO: f32 = 0.36;

/// Explicit image identity and module RVAs; object-field offsets remain shared.
/// The caller verifies `sha256` before using the profile for remote reads.
#[derive(Clone, Copy, Debug)]
pub struct BuildProfile {
    pub id: &'static str,
    pub sha256: &'static str,
    pub image_size: u32,
    pub framework_rva: u32,
    pub current_avatar_rva: u32,
    /// Observed split-pointer singleton wrappers; absent for unproven builds.
    pub manager_wrappers: Option<[u32; 2]>,
    pub framework_vtable: u32,
    pub camera_vtable: u32,
    pub view_vtable: u32,
    pub projection_vtable: u32,
    pub avatar_vtable: u32,
    pub character_manager_vtable: u32,
    pub matrix_vtable: u32,
}

/// Source-backed November 15 profile retained by every legacy entry point.
pub const LEGACY_PROFILE: BuildProfile = BuildProfile {
    id: "pb_id_20261115",
    sha256: EXPECTED_SHA256,
    image_size: IMAGE_SIZE,
    framework_rva: FRAMEWORK_RVA,
    current_avatar_rva: CURRENT_AVATAR_RVA,
    manager_wrappers: None,
    framework_vtable: FRAMEWORK_VTABLE_RVA,
    camera_vtable: CAMERA_VTABLE_RVA,
    view_vtable: VIEW_VTABLE_RVA,
    projection_vtable: PROJECTION_VTABLE_RVA,
    avatar_vtable: AVATAR_VTABLE_RVA,
    character_manager_vtable: CHARACTER_MANAGER_VTABLE_RVA,
    matrix_vtable: MATRIX_VTABLE_RVA,
};

/// November 16 image: root's captured RTTI and direct-read field-chain proof.
/// This profile supplies metadata; the host selects it by full hash and size.
pub const CURRENT_PROFILE: BuildProfile = BuildProfile {
    id: "pb_id_20261116",
    sha256: "d7de2e99c257164ce8022ecf81dd1ea5fe1fa7e9c6ae8bba58bb78c4cb459b46",
    image_size: 0x01f7_c000,
    framework_rva: 0x0124_5088,
    current_avatar_rva: 0x0126_a100,
    // Native singleton accessor + two-lifetime hyper-reV payload-sum proof.
    // Each initialized zero-flags wrapper stores a payload pointer at +0x0c.
    manager_wrappers: Some([0x0126_70b8, 0x0126_70cc]),
    framework_vtable: 0x00e6_8644,
    camera_vtable: 0x00e2_948c,
    view_vtable: 0x00e2_f76c,
    projection_vtable: 0x00e2_f6dc,
    avatar_vtable: 0x00ea_4410,
    character_manager_vtable: 0x00e7_34d4,
    matrix_vtable: 0x00e2_40dc,
};
