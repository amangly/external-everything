use super::*;
use crate::games::pointblank::{
    capture_live_profile, find_manager_profile, manager_candidates_profile, probe_manager_profile,
    scan_manager_candidates_profile,
};

fn for_profile(base: u32, profile: &BuildProfile) -> Fixture {
    let memory = fixture(base);
    memory.u32(at(base, 0xd0), profile.image_size);
    memory.u32(at(base, profile.framework_rva), FRAMEWORK);
    memory.u32(at(base, profile.current_avatar_rva), AVATAR);
    for (object, rva) in [
        (FRAMEWORK, profile.framework_vtable),
        (CAMERA, profile.camera_vtable),
        (VIEW, profile.view_vtable),
        (PROJECTION, profile.projection_vtable),
        (AVATAR, profile.avatar_vtable),
        (MANAGER, profile.character_manager_vtable),
        (MATRIX, profile.matrix_vtable),
    ] {
        memory.typed(base, object, rva);
    }
    memory
}

#[test]
fn both_profiles_cache_empty_manager_then_probe_population_without_rescanning() {
    for profile in [LEGACY_PROFILE, CURRENT_PROFILE] {
        for (begin, end) in [(0, 0), (VECTOR, VECTOR)] {
            let memory = for_profile(BASE, &profile);
            let region = scan_region(&memory);
            memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_BEGIN), begin);
            memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), end);
            let candidates =
                manager_candidates_profile(&memory, BASE, &[region], 4096, &profile).unwrap();
            assert_eq!(candidates.len(), 1);
            assert_eq!((candidates[0].address, candidates[0].count), (MANAGER, 0));
            assert_eq!(
                find_manager_profile(&memory, BASE, &[region], 4096, &profile).unwrap(),
                None
            );
            assert_eq!(
                probe_manager_profile(&memory, BASE, MANAGER, &profile)
                    .unwrap()
                    .count,
                0
            );

            memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_BEGIN), VECTOR);
            memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 4);
            memory.reads.borrow_mut().clear();
            let populated = probe_manager_profile(&memory, BASE, MANAGER, &profile).unwrap();
            assert_eq!((populated.address, populated.count), (MANAGER, 1));
            assert_eq!(
                memory
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(a, n)| a == u64::from(VECTOR) && n == 4)
                    .count(),
                2
            );
            assert!(memory.reads.borrow().iter().all(|&(_, n)| n <= 0x54));
            let frame =
                capture_live_profile(&memory, BASE, Some(populated.address), &profile).unwrap();
            assert!(frame.status.starts_with("FullCollection: 1 typed entries"));
            assert_eq!(frame.entities.len(), 1);
            assert!(frame.entities[0].screen.is_some());
        }
    }
}

#[test]
fn candidate_probes_keep_all_type_backlink_pointer_and_vector_guards() {
    for profile in [LEGACY_PROFILE, CURRENT_PROFILE] {
        for (address, value) in [
            (u64::from(MANAGER), 0),
            (at(MANAGER, CHARACTER_MANAGER_IMPL), 0xffff_fff0),
            (u64::from(IMPLEMENTATION), MANAGER + 4),
            (at(IMPLEMENTATION, IMPL_AVATAR_BEGIN), VECTOR + 1),
            (at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR - 4),
            (at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 3),
            (at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 129 * 4),
            (at(IMPLEMENTATION, IMPL_AVATAR_BEGIN), 0),
            (u64::from(AVATAR), 0),
        ] {
            let memory = for_profile(BASE, &profile);
            let region = scan_region(&memory);
            memory.u32(address, value);
            assert!(probe_manager_profile(&memory, BASE, MANAGER, &profile).is_err());
            assert!(
                manager_candidates_profile(&memory, BASE, &[region], 4096, &profile)
                    .unwrap()
                    .is_empty()
            );
        }
        let memory = for_profile(BASE, &profile);
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 8);
        memory.u32(at(VECTOR, 4), AVATAR);
        assert!(probe_manager_profile(&memory, BASE, MANAGER, &profile).is_err());
        memory.put(u64::from(BASE), b"ZZ");
        memory.reads.borrow_mut().clear();
        assert!(probe_manager_profile(&memory, BASE, MANAGER, &profile).is_err());
        assert_eq!(memory.reads.borrow().len(), 1);
    }
}

#[test]
fn empty_and_populated_probes_require_two_identical_collections() {
    for profile in [LEGACY_PROFILE, CURRENT_PROFILE] {
        let memory = for_profile(BASE, &profile);
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR);
        let changed = ChangeOnRead::u32(
            &memory,
            u64::from(IMPLEMENTATION),
            2,
            at(IMPLEMENTATION, IMPL_AVATAR_END),
            VECTOR + 4,
        );
        assert!(probe_manager_profile(&changed, BASE, MANAGER, &profile).is_err());

        let memory = for_profile(BASE, &profile);
        let region = scan_region(&memory);
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR);
        let changed = ChangeOnRead::u32(
            &memory,
            u64::from(IMPLEMENTATION),
            2,
            at(IMPLEMENTATION, IMPL_AVATAR_END),
            VECTOR + 4,
        );
        assert!(
            manager_candidates_profile(&changed, BASE, &[region], 4096, &profile)
                .unwrap()
                .is_empty()
        );

        let memory = for_profile(BASE, &profile);
        memory.typed(BASE, AVATAR2, profile.avatar_vtable);
        let changed = ChangeOnRead::u32(&memory, u64::from(VECTOR), 2, u64::from(VECTOR), AVATAR2);
        assert!(probe_manager_profile(&changed, BASE, MANAGER, &profile).is_err());
        let memory = for_profile(BASE, &profile);
        let changed = ChangeOnRead::u32(&memory, u64::from(AVATAR), 2, u64::from(AVATAR), 0);
        assert!(probe_manager_profile(&changed, BASE, MANAGER, &profile).is_err());
    }
}

#[test]
fn candidate_scan_bounds_results_and_resumes_after_the_last_admitted_slot() {
    let memory = fixture(BASE);
    memory.zeros(u64::from(MANAGER), 65 * 0x20);
    for index in 0..65u32 {
        let manager = MANAGER + index * 0x20;
        let implementation = IMPLEMENTATION + index * 0x20;
        memory.typed(BASE, manager, LEGACY_PROFILE.character_manager_vtable);
        memory.u32(at(manager, CHARACTER_MANAGER_IMPL), implementation);
        memory.zeros(u64::from(implementation), 0x10);
        memory.u32(u64::from(implementation), manager);
    }
    let region = (u64::from(MANAGER), 65 * 0x20);
    let (candidates, consumed) =
        scan_manager_candidates_profile(&memory, BASE, &[region], 64 * 1024, &LEGACY_PROFILE)
            .unwrap();
    assert_eq!(candidates.len(), 64);
    for (index, candidate) in candidates.iter().enumerate() {
        assert_eq!(candidate.address, MANAGER + index as u32 * 0x20);
        assert_eq!(candidate.count, 0);
    }
    assert_eq!(consumed, 63 * 0x20 + 4);
    let rest = manager_candidates_profile(
        &memory,
        BASE,
        &[(region.0 + consumed as u64, region.1 - consumed)],
        4096,
        &LEGACY_PROFILE,
    )
    .unwrap();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].address, MANAGER + 64 * 0x20);
    let repeated = manager_candidates_profile(
        &memory,
        BASE,
        &[(region.0, 3 * 0x20), (region.0, 3 * 0x20)],
        4096,
        &LEGACY_PROFILE,
    )
    .unwrap();
    assert_eq!(repeated.len(), 3);
}

#[test]
fn budget_stop_preserves_unproven_final_cell_and_reports_actual_progress() {
    let memory = fixture(BASE);
    memory.zeros(u64::from(MANAGER) - 4, 4);
    memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR);
    memory.reads.borrow_mut().clear();
    let region = (u64::from(MANAGER) - 4, 8);
    let (candidates, consumed) =
        scan_manager_candidates_profile(&memory, BASE, &[region], 16, &LEGACY_PROFILE).unwrap();
    assert!(candidates.is_empty());
    assert_eq!(consumed, 4);
    assert!(
        memory
            .reads
            .borrow()
            .iter()
            .skip(2)
            .map(|&(_, n)| n)
            .sum::<usize>()
            <= 16
    );
    let (candidates, consumed) = scan_manager_candidates_profile(
        &memory,
        BASE,
        &[(u64::from(MANAGER), 4)],
        4096,
        &LEGACY_PROFILE,
    )
    .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(consumed, 4);
    assert_eq!(candidates[0].address, MANAGER);

    memory.reads.borrow_mut().clear();
    let (candidates, consumed) =
        scan_manager_candidates_profile(&memory, BASE, &[region], 0, &LEGACY_PROFILE).unwrap();
    assert!(candidates.is_empty());
    assert_eq!(consumed, 0);
    assert_eq!(memory.reads.borrow().len(), 2);
}

#[test]
fn candidate_scan_skips_invalid_regions_with_bounded_x86_reads() {
    let memory = fixture(BASE);
    let regions = [(ADDRESS_LIMIT, 0x1000), (u64::MAX - 2, 64)];
    let (candidates, consumed) =
        scan_manager_candidates_profile(&memory, BASE, &regions, 4096, &LEGACY_PROFILE).unwrap();
    assert!(candidates.is_empty());
    assert_eq!(consumed, 0x1000 + 64);
    assert_eq!(memory.reads.borrow().len(), 2);
}
