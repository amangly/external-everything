//! Synthetic fixtures for the observed split-pointer singleton, without game calls.
use super::*;
use crate::games::pointblank::direct_manager_profile;

const PAYLOAD_A: u32 = 0x3300_0000;
const PAYLOAD_B: u32 = 0x3300_0010;

fn direct_fixture(base: u32, profile: &BuildProfile, values: [u32; 2]) -> Fixture {
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
    for (rva, payload) in profile
        .manager_wrappers
        .unwrap()
        .into_iter()
        .zip([PAYLOAD_A, PAYLOAD_B])
    {
        memory.zeros(at(base, rva), 0x14);
        memory.u32(at(base, rva + 8), 1);
        memory.u32(at(base, rva + 0x0c), payload);
    }
    memory.u32(u64::from(PAYLOAD_A), values[0]);
    memory.u32(u64::from(PAYLOAD_B), values[1]);
    memory
}

fn current_fixture() -> Fixture {
    direct_fixture(BASE, &CURRENT_PROFILE, [0x6625, MANAGER - 0x6625])
}

#[test]
fn direct_wrapper_offsets_are_admitted_only_for_the_observed_current_build() {
    assert_eq!(LEGACY_PROFILE.manager_wrappers, None);
    assert_eq!(
        CURRENT_PROFILE.manager_wrappers,
        Some([0x0126_70b8, 0x0126_70cc])
    );
    let memory = current_fixture();
    let candidate = direct_manager_profile(&memory, BASE, &CURRENT_PROFILE)
        .unwrap()
        .unwrap();
    assert_eq!((candidate.address, candidate.count), (MANAGER, 1));
}

#[test]
fn direct_manager_proves_empty_owner_then_population_without_any_heap_scan() {
    for begin in [0, VECTOR, 0xfcf7_d080] {
        let memory = current_fixture();
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_BEGIN), begin);
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), begin);
        let candidate = direct_manager_profile(&memory, BASE, &CURRENT_PROFILE)
            .unwrap()
            .unwrap();
        assert_eq!((candidate.address, candidate.count), (MANAGER, 0));
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_BEGIN), VECTOR);
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 4);
        let candidate = direct_manager_profile(&memory, BASE, &CURRENT_PROFILE)
            .unwrap()
            .unwrap();
        assert_eq!((candidate.address, candidate.count), (MANAGER, 1));
    }
}

#[test]
fn direct_payload_sum_wraps_in_the_x86_address_space() {
    let seed = 0xf000_0001u32;
    let memory = direct_fixture(BASE, &CURRENT_PROFILE, [seed, MANAGER.wrapping_sub(seed)]);
    assert_eq!(
        direct_manager_profile(&memory, BASE, &CURRENT_PROFILE)
            .unwrap()
            .unwrap()
            .address,
        MANAGER
    );
}

#[test]
fn direct_resolver_relocates_with_an_explicit_synthetic_profile() {
    let base = 0x0240_0000;
    let profile = BuildProfile {
        manager_wrappers: Some([0x1000, 0x1020]),
        ..LEGACY_PROFILE
    };
    let memory = direct_fixture(base, &profile, [0x6af, MANAGER - 0x6af]);
    assert_eq!(
        direct_manager_profile(&memory, base, &profile)
            .unwrap()
            .unwrap()
            .address,
        MANAGER
    );
    let reads = memory.reads.borrow();
    assert!(reads.contains(&(at(base, 0x1000), 0x14)));
    assert!(reads.contains(&(at(base, 0x1020), 0x14)));
    assert!(!reads.iter().any(|&(address, _)| {
        CURRENT_PROFILE
            .manager_wrappers
            .unwrap()
            .iter()
            .any(|&rva| address == at(base, rva))
    }));
}

#[test]
fn direct_resolver_without_proven_wrapper_rvas_performs_zero_reads() {
    let memory = Fixture::default();
    assert!(
        direct_manager_profile(&memory, BASE, &LEGACY_PROFILE)
            .unwrap()
            .is_none()
    );
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn direct_resolver_preserves_mandatory_pe_identity_errors() {
    for (address, bytes) in [
        (u64::from(BASE), b"ZZ".to_vec()),
        (at(BASE, 0x80), b"XX\0\0".to_vec()),
        (at(BASE, 0x84), 0x8664u16.to_le_bytes().to_vec()),
        (at(BASE, 0xd0), IMAGE_SIZE.to_le_bytes().to_vec()),
    ] {
        let memory = current_fixture();
        memory.put(address, &bytes);
        assert!(direct_manager_profile(&memory, BASE, &CURRENT_PROFILE).is_err());
        assert!(memory.reads.borrow().len() <= 2);
    }
}

#[test]
fn direct_resolver_rejects_uninitialized_flags_and_bad_payload_pointers() {
    for rva in CURRENT_PROFILE.manager_wrappers.unwrap() {
        for (offset, value) in [
            (0, 1),
            (4, 1),
            (8, 0),
            (8, 2),
            (0x10, 1),
            (0x0c, 0),
            (0x0c, 0xffff),
            (0x0c, PAYLOAD_A + 1),
        ] {
            let memory = current_fixture();
            memory.u32(at(BASE, rva + offset), value);
            assert!(
                direct_manager_profile(&memory, BASE, &CURRENT_PROFILE)
                    .unwrap()
                    .is_none(),
                "wrapper {rva:#x}, field {offset:#x}={value:#x}"
            );
        }
    }
    for manager in [0, 0xffff, MANAGER + 1] {
        let memory = direct_fixture(BASE, &CURRENT_PROFILE, [0, manager]);
        assert!(
            direct_manager_profile(&memory, BASE, &CURRENT_PROFILE)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn direct_wrapper_spans_stay_inside_the_selected_module() {
    let memory = current_fixture();
    for rvas in [
        [CURRENT_PROFILE.image_size, 0x1000],
        [CURRENT_PROFILE.image_size - 0x10, 0x1000],
        [u32::MAX - 3, 0x1000],
        [0x1001, 0x1020],
    ] {
        let profile = BuildProfile {
            manager_wrappers: Some(rvas),
            ..CURRENT_PROFILE
        };
        assert!(
            direct_manager_profile(&memory, BASE, &profile)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn direct_proof_read_failures_return_retryable_none() {
    let rvas = CURRENT_PROFILE.manager_wrappers.unwrap();
    for address in [
        at(BASE, rvas[0]),
        at(BASE, rvas[1]),
        u64::from(PAYLOAD_A),
        u64::from(PAYLOAD_B),
        u64::from(MANAGER),
        u64::from(IMPLEMENTATION),
        u64::from(VECTOR),
        u64::from(AVATAR),
    ] {
        let memory = current_fixture();
        memory.gap(address);
        assert!(
            direct_manager_profile(&memory, BASE, &CURRENT_PROFILE)
                .unwrap()
                .is_none(),
            "read hole at {address:#x}"
        );
    }
}

#[test]
fn direct_proof_retains_manager_type_backlink_and_vector_guards() {
    for (address, value) in [
        (u64::from(MANAGER), BASE + CHARACTER_MANAGER_VTABLE_RVA),
        (at(MANAGER, CHARACTER_MANAGER_IMPL), 0),
        (u64::from(IMPLEMENTATION), MANAGER + 4),
        (at(IMPLEMENTATION, IMPL_AVATAR_BEGIN), VECTOR + 1),
        (at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR - 4),
        (at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 3),
        (at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 129 * 4),
        (u64::from(AVATAR), 0),
    ] {
        let memory = current_fixture();
        memory.u32(address, value);
        assert!(
            direct_manager_profile(&memory, BASE, &CURRENT_PROFILE)
                .unwrap()
                .is_none()
        );
    }
    let memory = current_fixture();
    memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 8);
    memory.u32(at(VECTOR, 4), AVATAR);
    assert!(
        direct_manager_profile(&memory, BASE, &CURRENT_PROFILE)
            .unwrap()
            .is_none()
    );
}

#[test]
fn direct_proof_rejects_wrapper_and_payload_swaps_even_when_the_sum_is_unchanged() {
    let rvas = CURRENT_PROFILE.manager_wrappers.unwrap();
    for swap_wrappers in [true, false] {
        let memory = current_fixture();
        let (trigger, writes) = if swap_wrappers {
            (
                at(BASE, rvas[0]),
                vec![
                    (at(BASE, rvas[0] + 0x0c), PAYLOAD_B.to_le_bytes().to_vec()),
                    (at(BASE, rvas[1] + 0x0c), PAYLOAD_A.to_le_bytes().to_vec()),
                ],
            )
        } else {
            (
                u64::from(PAYLOAD_A),
                vec![
                    (
                        u64::from(PAYLOAD_A),
                        (MANAGER - 0x6625).to_le_bytes().to_vec(),
                    ),
                    (u64::from(PAYLOAD_B), 0x6625u32.to_le_bytes().to_vec()),
                ],
            )
        };
        let changed = ChangeOnRead {
            inner: &memory,
            trigger,
            nth: 2,
            hits: Cell::new(0),
            writes,
        };
        assert!(
            direct_manager_profile(&changed, BASE, &CURRENT_PROFILE)
                .unwrap()
                .is_none()
        );
        assert_eq!(changed.hits.get(), 2);
    }
}

#[test]
fn direct_proof_rereads_both_wrappers_and_payloads_after_typed_validation() {
    let rvas = CURRENT_PROFILE.manager_wrappers.unwrap();
    for address in [
        at(BASE, rvas[0]),
        at(BASE, rvas[1]),
        u64::from(PAYLOAD_A),
        u64::from(PAYLOAD_B),
    ] {
        let memory = current_fixture();
        let changed = ChangeOnRead::u32(&memory, address, 2, address, 1);
        assert!(
            direct_manager_profile(&changed, BASE, &CURRENT_PROFILE)
                .unwrap()
                .is_none()
        );
        assert_eq!(changed.hits.get(), 2);
    }
}

#[test]
fn direct_proof_requires_two_unchanged_typed_collections() {
    for (trigger, address, value) in [
        (u64::from(MANAGER), u64::from(MANAGER), 0),
        (
            u64::from(IMPLEMENTATION),
            u64::from(IMPLEMENTATION),
            MANAGER + 4,
        ),
        (u64::from(VECTOR), u64::from(VECTOR), AVATAR2),
        (u64::from(AVATAR), u64::from(AVATAR), 0),
    ] {
        let memory = current_fixture();
        memory.typed(BASE, AVATAR2, CURRENT_PROFILE.avatar_vtable);
        let changed = ChangeOnRead::u32(&memory, trigger, 2, address, value);
        assert!(
            direct_manager_profile(&changed, BASE, &CURRENT_PROFILE)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn direct_proof_has_bounded_reads_even_for_the_largest_typed_collection() {
    let memory = current_fixture();
    memory.u32(
        at(IMPLEMENTATION, IMPL_AVATAR_END),
        VECTOR + MAX_COLLECTION as u32 * 4,
    );
    for index in 0..MAX_COLLECTION as u32 {
        let avatar = AVATAR + index * 0x10;
        memory.typed(BASE, avatar, CURRENT_PROFILE.avatar_vtable);
        memory.u32(at(VECTOR, index * 4), avatar);
    }
    memory.reads.borrow_mut().clear();
    let candidate = direct_manager_profile(&memory, BASE, &CURRENT_PROFILE)
        .unwrap()
        .unwrap();
    assert_eq!(candidate.count, MAX_COLLECTION);
    let reads = memory.reads.borrow();
    assert_eq!(reads.len(), 16 + MAX_COLLECTION * 2);
    assert!(
        reads
            .iter()
            .all(|&(_, length)| length <= MAX_COLLECTION * 4)
    );
    assert!(reads.iter().map(|&(_, length)| length).sum::<usize>() < 4096);
    assert!(reads.iter().all(|&(address, _)| {
        address == u64::from(BASE)
            || address == at(BASE, 0x80)
            || CURRENT_PROFILE
                .manager_wrappers
                .unwrap()
                .iter()
                .any(|&rva| address == at(BASE, rva))
            || [PAYLOAD_A, PAYLOAD_B, MANAGER, IMPLEMENTATION, VECTOR]
                .into_iter()
                .any(|known| address == u64::from(known))
            || (0..MAX_COLLECTION as u32).any(|index| address == at(AVATAR, index * 0x10))
    }));
}
