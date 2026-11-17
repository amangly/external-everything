//! Live capture shares strict guards, but projects fresh positions with the final
//! valid camera instead of treating normal matrix movement as collection churn.

use super::super::{Frame, capture_live_profile, capture_profile};
use super::*;

fn live(memory: &impl Memory, manager: Option<u32>) -> Frame {
    capture_live_profile(memory, BASE, manager, &LEGACY_PROFILE).unwrap()
}

#[test]
fn live_camera_motion_reprojects_every_fresh_entity_and_strict_still_discards() {
    for manager in [Some(MANAGER), None] {
        let memory = fixture(BASE);
        memory.avatar(BASE, AVATAR2);
        memory.u32(at(VECTOR, 4), AVATAR2);
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 8);
        memory.zeros(u64::from(MATRIX + 0x2000), 0x5c);
        memory.typed(BASE, MATRIX + 0x2000, MATRIX_VTABLE_RVA);
        memory.f32(at(MATRIX + 0x2000, MATRIX_TRANSLATION), -2.0);
        memory.f32(at(MATRIX + 0x2000, MATRIX_TRANSLATION + 4), 0.0);
        memory.f32(at(MATRIX + 0x2000, MATRIX_TRANSLATION + 8), 20.0);
        memory.u32(at(AVATAR2, AVATAR_MATRIX), MATRIX + 0x2000);
        // Dead entities keep their source data; presentation decides visibility.
        memory.f32(at(HEALTH, HEALTH_VALUE), 0.0);

        let initial = live(&memory, manager);
        assert_eq!(initial.entities[0].position, [0.0, 0.0, 10.0]);
        let changed = ChangeOnRead {
            inner: &memory,
            trigger: at(AVATAR, AVATAR_MATRIX),
            nth: 1,
            hits: Cell::new(0),
            writes: vec![
                (
                    at(VIEW, ATTRIBUTE_MATRIX + 12 * 4),
                    (-1.0_f32).to_le_bytes().to_vec(),
                ),
                (
                    at(PROJECTION, ATTRIBUTE_MATRIX),
                    0.703125_f32.to_le_bytes().to_vec(),
                ),
                (
                    at(PROJECTION, ATTRIBUTE_MATRIX + 5 * 4),
                    1.25_f32.to_le_bytes().to_vec(),
                ),
                (
                    at(MATRIX, MATRIX_TRANSLATION),
                    3.0_f32.to_le_bytes().to_vec(),
                ),
            ],
        };
        let frame = live(&changed, manager);
        let expected_count = if manager.is_some() { 2 } else { 1 };
        assert_eq!(frame.entities.len(), expected_count, "{}", frame.status);
        assert_eq!(frame.entities[0].position, [3.0, 0.0, 10.0]);
        assert_eq!(frame.entities[0].health, 0.0);
        assert_eq!((frame.width, frame.height), (1920.0, 1080.0));
        let mut view = identity();
        view[12] = -1.0;
        let mut projection = perspective();
        projection[0] = 0.703125;
        projection[5] = 1.25;
        let final_product = geometry::multiply(&view, &projection).unwrap();
        let initial_product = geometry::multiply(&identity(), &perspective()).unwrap();
        for entity in &frame.entities {
            let expected =
                geometry::rectangle(&final_product, entity.position, 1920.0, 1080.0).unwrap();
            let old =
                geometry::rectangle(&initial_product, entity.position, 1920.0, 1080.0).unwrap();
            let actual = entity.screen.unwrap();
            for index in 0..4 {
                near(actual[index], expected[index]);
            }
            assert!((actual[0] - old[0]).abs() > 1.0);
        }
        // A second valid motion remains rejected through the strict diagnostic
        // API, even though the same change is accepted by the live API above.
        let strict_change = ChangeOnRead::u32(
            &memory,
            at(AVATAR, AVATAR_MATRIX),
            1,
            at(VIEW, ATTRIBUTE_MATRIX + 12 * 4),
            (-2.0_f32).to_bits(),
        );
        let strict = capture_profile(&strict_change, BASE, manager, &LEGACY_PROFILE).unwrap();
        assert!(strict.entities.is_empty());
        assert!(strict.status.starts_with(if manager.is_some() {
            "CollectionChanged:"
        } else {
            "SingleAvatarChanged:"
        }));
    }
}

#[test]
fn live_rejects_valid_but_retargeted_camera_chain_objects() {
    for manager in [Some(MANAGER), None] {
        for object in [FRAMEWORK, CAMERA, VIEW, PROJECTION] {
            let memory = fixture(BASE);
            let replacement = object + 0x1_0000;
            let (slot, length) = match object {
                FRAMEWORK => (at(BASE, FRAMEWORK_RVA), 0xf0),
                CAMERA => (at(FRAMEWORK, FRAMEWORK_CAMERA), 0xd8),
                VIEW => (at(CAMERA, CAMERA_VIEW), 0x70),
                PROJECTION => (at(CAMERA, CAMERA_PROJECTION), 0x70),
                _ => unreachable!(),
            };
            let mut bytes = vec![0; length];
            memory.read(u64::from(object), &mut bytes).unwrap();
            memory.put(u64::from(replacement), &bytes);
            let changed =
                ChangeOnRead::u32(&memory, at(AVATAR, AVATAR_MATRIX), 1, slot, replacement);
            let frame = live(&changed, manager);
            assert!(frame.entities.is_empty(), "accepted retargeted {object:#x}");
            assert!(frame.status.contains("Changed:"), "{}", frame.status);
        }
    }
}

#[test]
fn live_rejects_valid_camera_resize_and_every_final_camera_retype() {
    for manager in [Some(MANAGER), None] {
        let memory = fixture(BASE);
        // Scaling both dimensions preserves the projection's aspect proof, so
        // identity/dimension stability alone must reject this valid new viewport.
        let changed = ChangeOnRead {
            inner: &memory,
            trigger: at(AVATAR, AVATAR_MATRIX),
            nth: 1,
            hits: Cell::new(0),
            writes: vec![
                (at(CAMERA, CAMERA_WIDTH), 1280.0_f32.to_le_bytes().to_vec()),
                (at(CAMERA, CAMERA_HEIGHT), 720.0_f32.to_le_bytes().to_vec()),
            ],
        };
        let frame = live(&changed, manager);
        assert!(frame.entities.is_empty(), "{}", frame.status);
        assert!(frame.status.contains("Changed:"));

        for object in [FRAMEWORK, CAMERA, VIEW, PROJECTION] {
            let memory = fixture(BASE);
            let changed =
                ChangeOnRead::u32(&memory, at(AVATAR, AVATAR_MATRIX), 1, u64::from(object), 0);
            let frame = live(&changed, manager);
            assert!(frame.entities.is_empty(), "accepted retyped {object:#x}");
            assert!(frame.status.contains("Changed:"), "{}", frame.status);
        }
    }
}

#[test]
fn live_rejects_final_nonfinite_nonorthonormal_and_singular_matrices() {
    for manager in [Some(MANAGER), None] {
        for (address, value) in [
            (at(VIEW, ATTRIBUTE_MATRIX + 12 * 4), f32::NAN),
            (at(VIEW, ATTRIBUTE_MATRIX), 2.0),
            (at(PROJECTION, ATTRIBUTE_MATRIX + 14 * 4), 0.0),
            (at(PROJECTION, ATTRIBUTE_MATRIX + 5 * 4), f32::INFINITY),
        ] {
            let memory = fixture(BASE);
            let changed = ChangeOnRead::u32(
                &memory,
                at(AVATAR, AVATAR_MATRIX),
                1,
                address,
                value.to_bits(),
            );
            let frame = live(&changed, manager);
            assert!(
                frame.entities.is_empty(),
                "accepted final matrix {address:#x}"
            );
            assert!(frame.status.contains("Changed:"), "{}", frame.status);
        }
    }
}

#[test]
fn live_retains_whole_collection_bounds_entries_backlink_and_type_guards() {
    for (address, value) in [
        (u64::from(VECTOR), AVATAR2),
        (at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 8),
        (at(MANAGER, CHARACTER_MANAGER_IMPL), IMPLEMENTATION + 4),
        (u64::from(IMPLEMENTATION), MANAGER + 4),
        (u64::from(MANAGER), 0),
        (u64::from(AVATAR), 0),
    ] {
        let memory = fixture(BASE);
        memory.avatar(BASE, AVATAR2);
        memory.u32(at(VECTOR, 4), AVATAR2);
        let changed = ChangeOnRead::u32(&memory, at(AVATAR, AVATAR_MATRIX), 1, address, value);
        let frame = live(&changed, Some(MANAGER));
        assert!(
            frame.entities.is_empty(),
            "accepted collection mutation {address:#x}"
        );
        assert!(
            frame.status.starts_with("CollectionChanged:"),
            "{}",
            frame.status
        );
    }
}

#[test]
fn live_retains_avatar_field_matrix_type_and_current_slot_guards() {
    for manager in [Some(MANAGER), None] {
        for address in [at(AVATAR, AVATAR_MATRIX), at(AVATAR, AVATAR_HEALTH)] {
            let memory = fixture(BASE);
            let changed =
                ChangeOnRead::u32(&memory, at(AVATAR, AVATAR_MATRIX), 2, address, 0xffff_fff0);
            let frame = live(&changed, manager);
            assert!(
                frame.entities.is_empty(),
                "accepted field swap {address:#x}"
            );
        }
        let memory = fixture(BASE);
        let changed =
            ChangeOnRead::u32(&memory, at(AVATAR, AVATAR_MATRIX), 2, u64::from(MATRIX), 0);
        assert!(live(&changed, manager).entities.is_empty());
    }
    let memory = fixture(BASE);
    memory.avatar(BASE, AVATAR2);
    let slot = at(BASE, CURRENT_AVATAR_RVA);
    let changed = ChangeOnRead::u32(&memory, slot, 2, slot, AVATAR2);
    let frame = live(&changed, None);
    assert!(frame.entities.is_empty());
    assert!(frame.status.starts_with("SingleAvatarChanged:"));
}
