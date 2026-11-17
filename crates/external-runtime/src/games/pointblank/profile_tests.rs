//! Synthetic profile plumbing only: these values identify zero game builds.

use super::super::{capture_profile, find_manager_profile, validate_image_profile};
use super::*;

// Every module RVA is deliberately beyond the legacy image, while object fields
// retain the verified legacy layout. This is unrelated to installed-image data.
const SYNTHETIC_PROFILE: BuildProfile = BuildProfile {
    id: "synthetic-profile-only",
    sha256: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    image_size: 0x0280_0000,
    framework_rva: 0x0200_0100,
    current_avatar_rva: 0x0200_0200,
    manager_wrappers: None,
    framework_vtable: 0x0210_0100,
    camera_vtable: 0x0210_0200,
    view_vtable: 0x0210_0300,
    projection_vtable: 0x0210_0400,
    avatar_vtable: 0x0210_0500,
    character_manager_vtable: 0x0210_0600,
    matrix_vtable: 0x0210_0700,
};

fn profile_fixture(base: u32) -> Fixture {
    let memory = fixture(base);
    memory.u32(at(base, 0xd0), SYNTHETIC_PROFILE.image_size);
    for rva in [FRAMEWORK_RVA, CURRENT_AVATAR_RVA] {
        for offset in 0..4 {
            memory.gap(at(base, rva + offset));
        }
    }
    memory.u32(at(base, SYNTHETIC_PROFILE.framework_rva), FRAMEWORK);
    memory.u32(at(base, SYNTHETIC_PROFILE.current_avatar_rva), AVATAR);
    for (object, rva) in [
        (FRAMEWORK, SYNTHETIC_PROFILE.framework_vtable),
        (CAMERA, SYNTHETIC_PROFILE.camera_vtable),
        (VIEW, SYNTHETIC_PROFILE.view_vtable),
        (PROJECTION, SYNTHETIC_PROFILE.projection_vtable),
        (AVATAR, SYNTHETIC_PROFILE.avatar_vtable),
        (MANAGER, SYNTHETIC_PROFILE.character_manager_vtable),
        (MATRIX, SYNTHETIC_PROFILE.matrix_vtable),
    ] {
        memory.typed(base, object, rva);
    }
    memory
}

#[test]
fn legacy_profile_retains_exact_provenance_and_copy_metadata() {
    let copied = LEGACY_PROFILE;
    assert_eq!(LEGACY_PROFILE.id, "pb_id_20261115");
    assert_eq!(copied.sha256, EXPECTED_SHA256);
    assert_eq!(copied.image_size, IMAGE_SIZE);
    assert_eq!(copied.framework_rva, FRAMEWORK_RVA);
    assert_eq!(copied.current_avatar_rva, CURRENT_AVATAR_RVA);
    assert_eq!(copied.framework_vtable, FRAMEWORK_VTABLE_RVA);
    assert_eq!(copied.camera_vtable, CAMERA_VTABLE_RVA);
    assert_eq!(copied.view_vtable, VIEW_VTABLE_RVA);
    assert_eq!(copied.projection_vtable, PROJECTION_VTABLE_RVA);
    assert_eq!(copied.avatar_vtable, AVATAR_VTABLE_RVA);
    assert_eq!(
        copied.character_manager_vtable,
        CHARACTER_MANAGER_VTABLE_RVA
    );
    assert_eq!(copied.matrix_vtable, MATRIX_VTABLE_RVA);
    assert!(format!("{copied:?}").contains("pb_id_20261115"));
}

#[test]
fn synthetic_profile_relocates_all_globals_and_types_beyond_legacy_image() {
    let base = 0x0240_0000;
    let memory = profile_fixture(base);
    validate_image_profile(&memory, base, &SYNTHETIC_PROFILE).unwrap();
    assert!(validate_image(&memory, base).is_err());
    let frame = capture_profile(&memory, base, Some(MANAGER), &SYNTHETIC_PROFILE).unwrap();
    assert!(
        frame.status.starts_with("FullCollection:"),
        "{}",
        frame.status
    );
    assert_eq!(frame.entities.len(), 1);
    assert_eq!(frame.entities[0].address, AVATAR);
    assert_eq!(frame.entities[0].health, 132.25);
    assert_eq!(frame.entities[0].position, [0.0, 0.0, 10.0]);
    assert_eq!((frame.width, frame.height), (1920.0, 1080.0));
    let frame = capture_profile(&memory, base, None, &SYNTHETIC_PROFILE).unwrap();
    assert!(
        frame.status.starts_with("SingleAvatar:"),
        "{}",
        frame.status
    );
    assert_eq!(frame.entities.len(), 1);
    let reads = memory.reads.borrow();
    assert!(reads.contains(&(at(base, SYNTHETIC_PROFILE.framework_rva), 4)));
    assert!(reads.contains(&(at(base, SYNTHETIC_PROFILE.current_avatar_rva), 4)));
    assert!(!reads.iter().any(|&(address, _)| {
        address == at(base, FRAMEWORK_RVA) || address == at(base, CURRENT_AVATAR_RVA)
    }));
}

#[test]
fn synthetic_profile_scan_uses_its_vtable_and_preserves_zero_budget() {
    let memory = profile_fixture(BASE);
    let region = scan_region(&memory);
    assert_eq!(
        find_manager_profile(&memory, BASE, &[region], 0, &SYNTHETIC_PROFILE).unwrap(),
        None
    );
    assert_eq!(memory.reads.borrow().len(), 2);
    memory.reads.borrow_mut().clear();
    assert_eq!(
        find_manager_profile(&memory, BASE, &[region], 4096, &SYNTHETIC_PROFILE).unwrap(),
        Some(MANAGER)
    );
    assert!(
        memory
            .reads
            .borrow()
            .iter()
            .skip(2)
            .map(|&(_, size)| size)
            .sum::<usize>()
            <= 4096
    );
}

#[test]
fn profile_image_size_and_module_rva_bounds_are_exact() {
    let memory = profile_fixture(BASE);
    let wrong_size = BuildProfile {
        image_size: SYNTHETIC_PROFILE.image_size - 0x1000,
        ..SYNTHETIC_PROFILE
    };
    assert!(validate_image_profile(&memory, BASE, &wrong_size).is_err());
    for profile in [
        BuildProfile {
            framework_rva: SYNTHETIC_PROFILE.image_size,
            ..SYNTHETIC_PROFILE
        },
        BuildProfile {
            framework_vtable: SYNTHETIC_PROFILE.image_size,
            ..SYNTHETIC_PROFILE
        },
    ] {
        let frame = capture_profile(&memory, BASE, None, &profile).unwrap();
        assert!(
            frame.status.starts_with("WaitingCamera:"),
            "{}",
            frame.status
        );
        assert!(frame.entities.is_empty());
    }
    let profile = BuildProfile {
        current_avatar_rva: SYNTHETIC_PROFILE.image_size,
        ..SYNTHETIC_PROFILE
    };
    assert!(capture_profile(&memory, BASE, None, &profile).is_err());
    let profile = BuildProfile {
        character_manager_vtable: SYNTHETIC_PROFILE.image_size,
        ..SYNTHETIC_PROFILE
    };
    assert!(find_manager_profile(&memory, BASE, &[], 0, &profile).is_err());
}

#[test]
fn profile_hash_is_metadata_and_never_replaces_typed_proof() {
    let memory = profile_fixture(BASE);
    let metadata = BuildProfile {
        id: "different-synthetic-metadata",
        sha256: LEGACY_PROFILE.sha256,
        ..SYNTHETIC_PROFILE
    };
    assert!(
        capture_profile(&memory, BASE, Some(MANAGER), &metadata)
            .unwrap()
            .status
            .starts_with("FullCollection:")
    );
    for (object, legacy_rva, status) in [
        (FRAMEWORK, FRAMEWORK_VTABLE_RVA, "WaitingCamera:"),
        (CAMERA, CAMERA_VTABLE_RVA, "WaitingCamera:"),
        (VIEW, VIEW_VTABLE_RVA, "WaitingCamera:"),
        (PROJECTION, PROJECTION_VTABLE_RVA, "WaitingCamera:"),
        (MANAGER, CHARACTER_MANAGER_VTABLE_RVA, "CollectionInvalid:"),
        (AVATAR, AVATAR_VTABLE_RVA, "CollectionInvalid:"),
        (MATRIX, MATRIX_VTABLE_RVA, "FullCollection:"),
    ] {
        let memory = profile_fixture(BASE);
        memory.typed(BASE, object, legacy_rva);
        let frame = capture_profile(&memory, BASE, Some(MANAGER), &SYNTHETIC_PROFILE).unwrap();
        assert!(frame.status.starts_with(status), "{}", frame.status);
        assert!(frame.entities.is_empty());
    }
}
