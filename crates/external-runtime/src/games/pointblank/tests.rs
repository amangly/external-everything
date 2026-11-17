use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

use anyhow::{Context, Result, bail};

use super::{Memory, capture, find_manager, geometry, offsets::*, validate_image};

const BASE: u32 = 0x0040_0000;
const FRAMEWORK: u32 = 0x1000_0000;
const CAMERA: u32 = 0x1000_1000;
const VIEW: u32 = 0x1000_2000;
const PROJECTION: u32 = 0x1000_3000;
const MANAGER: u32 = 0x2000_0000;
const IMPLEMENTATION: u32 = 0x2000_1000;
const VECTOR: u32 = 0x2100_0000;
const AVATAR: u32 = 0x3000_0000;
const AVATAR2: u32 = 0x3000_2000;
const MATRIX: u32 = 0x3100_0000;
const HEALTH: u32 = 0x3200_0000;

fn at(base: u32, offset: u32) -> u64 {
    u64::from(base).checked_add(u64::from(offset)).unwrap()
}

fn identity() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn perspective() -> [f32; 16] {
    [
        0.5625, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.001, 1.0, 0.0, 0.0, -0.1001, 0.0,
    ]
}

#[derive(Default)]
struct Fixture {
    bytes: RefCell<BTreeMap<u64, u8>>,
    reads: RefCell<Vec<(u64, usize)>>,
}

impl Fixture {
    fn put(&self, address: u64, bytes: &[u8]) {
        let mut memory = self.bytes.borrow_mut();
        for (i, value) in bytes.iter().enumerate() {
            memory.insert(address.checked_add(i as u64).unwrap(), *value);
        }
    }

    fn zeros(&self, address: u64, length: usize) {
        self.put(address, &vec![0; length]);
    }

    fn u32(&self, address: u64, value: u32) {
        self.put(address, &value.to_le_bytes());
    }

    fn f32(&self, address: u64, value: f32) {
        self.put(address, &value.to_le_bytes());
    }

    fn matrix(&self, address: u64, matrix: &[f32; 16]) {
        for (index, value) in matrix.iter().enumerate() {
            self.f32(address.checked_add(index as u64 * 4).unwrap(), *value);
        }
    }

    fn typed(&self, base: u32, object: u32, rva: u32) {
        self.u32(u64::from(object), base.checked_add(rva).unwrap());
    }

    fn avatar(&self, base: u32, address: u32) {
        self.typed(base, address, AVATAR_VTABLE_RVA);
        self.zeros(at(address, AVATAR_MATRIX), 0xb4);
        self.u32(at(address, AVATAR_MATRIX), MATRIX);
        self.u32(at(address, AVATAR_HEALTH), HEALTH);
    }

    fn gap(&self, address: u64) {
        self.bytes.borrow_mut().remove(&address);
    }
}

impl Memory for Fixture {
    fn read(&self, address: u64, out: &mut [u8]) -> Result<()> {
        self.reads.borrow_mut().push((address, out.len()));
        let bytes = self.bytes.borrow();
        for (offset, value) in out.iter_mut().enumerate() {
            let address = address
                .checked_add(offset as u64)
                .context("fixture address overflow")?;
            let Some(byte) = bytes.get(&address) else {
                bail!("fixture hole at {address:#x}")
            };
            *value = *byte;
        }
        Ok(())
    }
}

fn fixture(base: u32) -> Fixture {
    let memory = Fixture::default();
    memory.zeros(u64::from(base), 0x40);
    memory.put(u64::from(base), b"MZ");
    memory.u32(at(base, 0x3c), 0x80);
    memory.zeros(at(base, 0x80), 0xf8);
    memory.put(at(base, 0x80), b"PE\0\0");
    memory.put(at(base, 0x84), &0x014c_u16.to_le_bytes());
    memory.put(at(base, 0x94), &0x00e0_u16.to_le_bytes());
    memory.put(at(base, 0x98), &0x010b_u16.to_le_bytes());
    memory.u32(at(base, 0xd0), IMAGE_SIZE);
    memory.u32(at(base, FRAMEWORK_RVA), FRAMEWORK);
    memory.u32(at(base, CURRENT_AVATAR_RVA), AVATAR);

    memory.zeros(u64::from(FRAMEWORK), 0xf0);
    memory.typed(base, FRAMEWORK, FRAMEWORK_VTABLE_RVA);
    memory.u32(at(FRAMEWORK, FRAMEWORK_CAMERA), CAMERA);
    memory.zeros(u64::from(CAMERA), 0xd8);
    memory.typed(base, CAMERA, CAMERA_VTABLE_RVA);
    memory.f32(at(CAMERA, CAMERA_WIDTH), 1920.0);
    memory.f32(at(CAMERA, CAMERA_HEIGHT), 1080.0);
    memory.u32(at(CAMERA, CAMERA_VIEW), VIEW);
    memory.u32(at(CAMERA, CAMERA_PROJECTION), PROJECTION);
    memory.zeros(u64::from(VIEW), 0x70);
    memory.typed(base, VIEW, VIEW_VTABLE_RVA);
    memory.matrix(at(VIEW, ATTRIBUTE_MATRIX), &identity());
    memory.zeros(u64::from(PROJECTION), 0x70);
    memory.typed(base, PROJECTION, PROJECTION_VTABLE_RVA);
    memory.matrix(at(PROJECTION, ATTRIBUTE_MATRIX), &perspective());

    memory.zeros(u64::from(MANAGER), 0x18);
    memory.typed(base, MANAGER, CHARACTER_MANAGER_VTABLE_RVA);
    memory.u32(at(MANAGER, CHARACTER_MANAGER_IMPL), IMPLEMENTATION);
    memory.zeros(u64::from(IMPLEMENTATION), 0x10);
    memory.u32(u64::from(IMPLEMENTATION), MANAGER);
    memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_BEGIN), VECTOR);
    memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 4);
    memory.u32(u64::from(VECTOR), AVATAR);
    memory.avatar(base, AVATAR);
    memory.zeros(u64::from(MATRIX), 0x5c);
    memory.typed(base, MATRIX, MATRIX_VTABLE_RVA);
    memory.f32(at(MATRIX, MATRIX_TRANSLATION), 0.0);
    memory.f32(at(MATRIX, MATRIX_TRANSLATION + 4), 0.0);
    memory.f32(at(MATRIX, MATRIX_TRANSLATION + 8), 10.0);
    memory.zeros(at(HEALTH, HEALTH_ID), 8);
    memory.u32(at(HEALTH, HEALTH_ID), 0);
    memory.f32(at(HEALTH, HEALTH_VALUE), 132.25);
    memory
}

fn near(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.002,
        "{actual} differs from {expected}"
    );
}

#[test]
fn pinned_identity_and_remote_pe_pass() {
    assert_eq!(PROCESS_NAMES, &["PointBlank.exe"]);
    assert_eq!(IMAGE_SIZE, 0x01f6_f000);
    assert_eq!(
        EXPECTED_SHA256,
        "edeeeef3e76caa41b13c0a66762da95b90e989f09b8dff7d8480a9bd302ee084"
    );
    validate_image(&fixture(BASE), BASE).unwrap();
}

#[test]
fn image_guard_rejects_corruption_before_game_offsets() {
    let cases: &[(u32, &[u8])] = &[
        (0, b"ZZ"),
        (0x80, b"PX\0\0"),
        (0x84, &[0x64, 0x86]),
        (0x94, &[0x20, 0x00]),
        (0x98, &[0x0b, 0x02]),
        (0xd0, &[0, 0, 0, 0]),
        (0x3c, &[0xff, 0xff, 0xff, 0xff]),
    ];
    for &(offset, bytes) in cases {
        let memory = fixture(BASE);
        memory.put(at(BASE, offset), bytes);
        assert!(
            capture(&memory, BASE, Some(MANAGER)).is_err(),
            "accepted field {offset:#x}"
        );
        assert!(
            memory
                .reads
                .borrow()
                .iter()
                .all(|&(address, _)| address < at(BASE, FRAMEWORK_RVA))
        );
    }
}

#[test]
fn full_collection_retains_float_health_and_real_pixel_rect() {
    let memory = fixture(BASE);
    let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
    assert!(
        frame.status.starts_with("FullCollection:"),
        "{}",
        frame.status
    );
    assert_eq!(frame.manager, Some(MANAGER));
    assert_eq!((frame.width, frame.height), (1920.0, 1080.0));
    assert_eq!(frame.entities.len(), 1);
    let entity = &frame.entities[0];
    assert_eq!(entity.address, AVATAR);
    assert_eq!(entity.health, 132.25);
    assert_eq!(entity.position, [0.0, 0.0, 10.0]);
    let [left, top, width, height] = entity.screen.unwrap();
    near(left, 942.504);
    near(top, 442.8);
    near(width, 34.992);
    near(height, 97.2);
    // Blocks go directly to Memory, rather than being split into eight-byte reads.
    assert!(memory.reads.borrow().contains(&(u64::from(VIEW), 0x70)));
    assert!(
        memory
            .reads
            .borrow()
            .contains(&(u64::from(FRAMEWORK), 0xf0))
    );
}

#[test]
fn current_slot_is_explicitly_single_avatar() {
    let frame = capture(&fixture(BASE), BASE, None).unwrap();
    assert!(
        frame.status.starts_with("SingleAvatar:"),
        "{}",
        frame.status
    );
    assert!(frame.status.contains("full collection unresolved"));
    assert_eq!(frame.manager, None);
    assert_eq!(frame.entities.len(), 1);
}

#[test]
fn every_module_address_relocates_from_image_base() {
    let base = 0x0200_0000;
    let frame = capture(&fixture(base), base, Some(MANAGER)).unwrap();
    assert!(
        frame.status.starts_with("FullCollection:"),
        "{}",
        frame.status
    );
    assert_eq!(frame.entities.len(), 1);
}

#[test]
fn row_vector_view_translation_and_projection_are_preserved() {
    let mut view = identity();
    view[12] = -2.0;
    view[13] = 1.0;
    assert!(geometry::camera_matrices(
        &view,
        &perspective(),
        1920.0,
        1080.0
    ));
    let product = geometry::multiply(&view, &perspective()).unwrap();
    let pixel = geometry::project(&product, [2.0, -1.0, 10.0], 1920.0, 1080.0).unwrap();
    near(pixel[0], 960.0);
    near(pixel[1], 540.0);
    let memory = fixture(BASE);
    memory.matrix(at(VIEW, ATTRIBUTE_MATRIX), &view);
    memory.f32(at(MATRIX, MATRIX_TRANSLATION), 2.0);
    memory.f32(at(MATRIX, MATRIX_TRANSLATION + 4), -1.0);
    let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
    let rect = frame.entities[0].screen.unwrap();
    near(rect[0] + rect[2] / 2.0, 960.0);
}

#[test]
fn behind_camera_entity_retains_data_without_screen_rect() {
    let memory = fixture(BASE);
    memory.f32(at(MATRIX, MATRIX_TRANSLATION + 8), -10.0);
    let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
    assert_eq!(frame.entities.len(), 1);
    assert!(frame.entities[0].screen.is_none());
}

#[test]
fn camera_type_guards_fail_closed() {
    for object in [FRAMEWORK, CAMERA, VIEW, PROJECTION] {
        let memory = fixture(BASE);
        memory.u32(u64::from(object), 0);
        let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
        assert!(
            frame.status.starts_with("WaitingCamera:"),
            "{}",
            frame.status
        );
        assert!(frame.entities.is_empty());
        assert_eq!((frame.width, frame.height), (0.0, 0.0));
    }
}

#[test]
fn corrupt_pointers_never_wrap_to_low_memory() {
    for invalid in [0, 4, 0xffff, 0xffff_fff0] {
        let memory = fixture(BASE);
        memory.u32(at(BASE, FRAMEWORK_RVA), invalid);
        let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
        assert!(frame.entities.is_empty());
        assert!(frame.status.starts_with("WaitingCamera:"));
        assert!(memory.reads.borrow().iter().all(|&(address, length)| {
            address >= MIN_POINTER && address.checked_add(length as u64).unwrap() <= ADDRESS_LIMIT
        }));
    }
    let memory = Fixture::default();
    assert!(capture(&memory, 0xffff_fff0, None).is_err());
    assert!(memory.reads.borrow().is_empty());
}

#[test]
fn sparse_padding_gaps_use_field_fallbacks() {
    let memory = fixture(BASE);
    for address in [
        at(FRAMEWORK, 4),
        at(CAMERA, 4),
        at(VIEW, 4),
        at(PROJECTION, 4),
        at(MANAGER, 4),
        at(IMPLEMENTATION, 4),
        at(AVATAR, AVATAR_MATRIX + 4),
        at(MATRIX, 4),
    ] {
        memory.gap(address);
    }
    let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
    assert!(
        frame.status.starts_with("FullCollection:"),
        "{}",
        frame.status
    );
    assert_eq!(frame.entities.len(), 1);
}

#[test]
fn manager_is_typed_and_backlinked_without_silent_fallback() {
    for (address, value) in [
        (u64::from(MANAGER), 0),
        (at(MANAGER, CHARACTER_MANAGER_IMPL), 0xffff_fff8),
        (u64::from(IMPLEMENTATION), MANAGER + 4),
    ] {
        let memory = fixture(BASE);
        memory.u32(address, value);
        let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
        assert!(
            frame.status.starts_with("CollectionInvalid:"),
            "{}",
            frame.status
        );
        assert!(frame.entities.is_empty());
    }
}

#[test]
fn collection_bounds_and_duplicates_are_rejected_instead_of_deduplicated() {
    for (begin, end) in [
        (VECTOR, VECTOR + 129 * 4),
        (VECTOR + 1, VECTOR + 5),
        (VECTOR, VECTOR + 3),
        (VECTOR + 4, VECTOR),
        (0, VECTOR),
        (VECTOR, 0),
    ] {
        let memory = fixture(BASE);
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_BEGIN), begin);
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), end);
        let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
        assert!(
            frame.status.starts_with("CollectionInvalid:"),
            "{}",
            frame.status
        );
        assert!(frame.entities.is_empty());
    }
    let memory = fixture(BASE);
    memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 8);
    memory.u32(at(VECTOR, 4), AVATAR);
    let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
    assert!(frame.status.contains("duplicate"), "{}", frame.status);
    assert!(frame.entities.is_empty());
}

#[test]
fn stable_empty_collection_is_distinct_from_single_avatar() {
    for bounds in [(0, 0), (VECTOR, VECTOR)] {
        let memory = fixture(BASE);
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_BEGIN), bounds.0);
        memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), bounds.1);
        let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
        assert!(frame.status.starts_with("FullCollection: 0 typed entries"));
        assert!(frame.entities.is_empty());
    }
}

#[test]
fn maximum_128_unique_entries_are_read_as_one_vector_block() {
    let memory = fixture(BASE);
    for index in 0..128_u32 {
        let avatar = AVATAR.checked_add(index * 0x2000).unwrap();
        memory.avatar(BASE, avatar);
        memory.u32(at(VECTOR, index * 4), avatar);
    }
    memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 128 * 4);
    let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
    assert!(
        frame
            .status
            .starts_with("FullCollection: 128 typed entries"),
        "{}",
        frame.status
    );
    assert_eq!(frame.entities.len(), 128);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, length)| address == u64::from(VECTOR) && length == 512)
            .count(),
        2
    );
}

#[test]
fn source_health_float_accepts_zero_and_values_above_100() {
    for hp in [0.0, 51.0, 100.0, 132.25, 5000.5, 10_000.0] {
        let memory = fixture(BASE);
        memory.f32(at(HEALTH, HEALTH_VALUE), hp);
        let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
        assert_eq!(frame.entities[0].health, hp);
    }
    for hp in [-1.0, 10_000.5, f32::INFINITY, f32::NAN] {
        let memory = fixture(BASE);
        memory.f32(at(HEALTH, HEALTH_VALUE), hp);
        let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
        assert!(frame.entities.is_empty());
        assert!(frame.status.contains("1 rejected"));
    }
}

#[test]
fn invalid_health_id_matrix_type_or_translation_rejects_entity() {
    for (address, value) in [
        (at(HEALTH, HEALTH_ID), 128),
        (u64::from(MATRIX), 0),
        (at(AVATAR, AVATAR_MATRIX), 0xffff_fff0),
        (at(AVATAR, AVATAR_HEALTH), 0xffff_fff0),
        (at(MATRIX, MATRIX_TRANSLATION), f32::NAN.to_bits()),
        (at(MATRIX, MATRIX_TRANSLATION + 4), 1001.0_f32.to_bits()),
        (at(MATRIX, MATRIX_TRANSLATION + 8), 100_001.0_f32.to_bits()),
    ] {
        let memory = fixture(BASE);
        memory.u32(address, value);
        let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
        assert!(frame.entities.is_empty(), "accepted {address:#x}");
        assert!(frame.status.contains("1 rejected"), "{}", frame.status);
    }
}

#[test]
fn stronger_camera_proof_rejects_degenerate_or_nonorthonormal_matrices() {
    let mut singular = identity();
    singular[4] = singular[0];
    singular[5] = 0.0;
    assert!(!geometry::nonsingular(&singular));
    let mut almost_singular = perspective();
    almost_singular[0] = 1.0;
    almost_singular[10] = 0.011;
    almost_singular[14] = 1.1e-6;
    almost_singular[15] = 1e-4;
    assert!(!geometry::nonsingular(&almost_singular));
    assert!(!geometry::camera_matrices(
        &identity(),
        &almost_singular,
        1080.0,
        1080.0
    ));
    let memory = fixture(BASE);
    memory.f32(at(CAMERA, CAMERA_WIDTH), 1080.0);
    memory.matrix(at(PROJECTION, ATTRIBUTE_MATRIX), &almost_singular);
    let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
    assert!(frame.status.starts_with("WaitingCamera:"));
    assert!(frame.entities.is_empty());
    let memory = fixture(BASE);
    let mut view = identity();
    view[0] = 2.0;
    memory.matrix(at(VIEW, ATTRIBUTE_MATRIX), &view);
    assert!(
        capture(&memory, BASE, Some(MANAGER))
            .unwrap()
            .status
            .starts_with("WaitingCamera:")
    );
}

#[test]
fn invalid_camera_size_projection_shape_or_nonfinite_values_are_safe() {
    for (address, value) in [
        (at(CAMERA, CAMERA_WIDTH), 319.0),
        (at(CAMERA, CAMERA_HEIGHT), f32::NAN),
        (at(PROJECTION, ATTRIBUTE_MATRIX), 0.0),
        (at(PROJECTION, ATTRIBUTE_MATRIX + 4), 0.1),
        (at(PROJECTION, ATTRIBUTE_MATRIX + 14 * 4), 0.0),
        (at(VIEW, ATTRIBUTE_MATRIX), f32::INFINITY),
    ] {
        let memory = fixture(BASE);
        memory.f32(address, value);
        let frame = capture(&memory, BASE, Some(MANAGER)).unwrap();
        assert!(
            frame.status.starts_with("WaitingCamera:"),
            "{}",
            frame.status
        );
        assert!(frame.entities.is_empty());
    }
}

struct ChangeOnRead<'a> {
    inner: &'a Fixture,
    trigger: u64,
    nth: usize,
    hits: Cell<usize>,
    writes: Vec<(u64, Vec<u8>)>,
}

impl<'a> ChangeOnRead<'a> {
    fn u32(inner: &'a Fixture, trigger: u64, nth: usize, address: u64, value: u32) -> Self {
        Self {
            inner,
            trigger,
            nth,
            hits: Cell::new(0),
            writes: vec![(address, value.to_le_bytes().to_vec())],
        }
    }
}

impl Memory for ChangeOnRead<'_> {
    fn read(&self, address: u64, out: &mut [u8]) -> Result<()> {
        if address == self.trigger {
            self.hits.set(self.hits.get() + 1);
            if self.hits.get() == self.nth {
                for (address, bytes) in &self.writes {
                    self.inner.put(*address, bytes);
                }
            }
        }
        self.inner.read(address, out)
    }
}

#[test]
fn changed_collection_entries_or_bounds_discard_entire_frame() {
    let memory = fixture(BASE);
    memory.avatar(BASE, AVATAR2);
    let changed = ChangeOnRead::u32(&memory, u64::from(VECTOR), 2, u64::from(VECTOR), AVATAR2);
    let frame = capture(&changed, BASE, Some(MANAGER)).unwrap();
    assert!(
        frame.status.starts_with("CollectionChanged:"),
        "{}",
        frame.status
    );
    assert!(frame.entities.is_empty());

    let memory = fixture(BASE);
    memory.avatar(BASE, AVATAR2);
    memory.u32(at(VECTOR, 4), AVATAR2);
    let changed = ChangeOnRead::u32(
        &memory,
        u64::from(IMPLEMENTATION),
        2,
        at(IMPLEMENTATION, IMPL_AVATAR_END),
        VECTOR + 8,
    );
    let frame = capture(&changed, BASE, Some(MANAGER)).unwrap();
    assert!(frame.status.starts_with("CollectionChanged:"));
    assert!(frame.entities.is_empty());
}

#[test]
fn changed_camera_or_invalidated_matrix_discard_entire_frame() {
    for (trigger, address, value) in [
        (
            u64::from(PROJECTION),
            at(PROJECTION, ATTRIBUTE_MATRIX + 14 * 4),
            0,
        ),
        (u64::from(CAMERA), at(CAMERA, CAMERA_VIEW), 0xffff_fff0),
        (
            u64::from(VIEW),
            at(VIEW, ATTRIBUTE_MATRIX + 12 * 4),
            1.0_f32.to_bits(),
        ),
    ] {
        let memory = fixture(BASE);
        let changed = ChangeOnRead::u32(&memory, trigger, 2, address, value);
        let frame = capture(&changed, BASE, Some(MANAGER)).unwrap();
        assert!(
            frame.status.starts_with("CollectionChanged:"),
            "{}",
            frame.status
        );
        assert!(frame.entities.is_empty());
    }
}

#[test]
fn changed_current_avatar_slot_is_never_presented_as_full_collection() {
    let memory = fixture(BASE);
    memory.avatar(BASE, AVATAR2);
    let slot = at(BASE, CURRENT_AVATAR_RVA);
    let changed = ChangeOnRead::u32(&memory, slot, 2, slot, AVATAR2);
    let frame = capture(&changed, BASE, None).unwrap();
    assert!(frame.status.starts_with("SingleAvatarChanged:"));
    assert!(frame.entities.is_empty());
}

#[test]
fn avatar_field_swap_during_capture_rejects_the_entity() {
    let memory = fixture(BASE);
    let field = at(AVATAR, AVATAR_MATRIX);
    let changed = ChangeOnRead::u32(&memory, field, 2, field, 0xffff_fff0);
    let frame = capture(&changed, BASE, Some(MANAGER)).unwrap();
    assert!(frame.entities.is_empty());
    assert!(frame.status.contains("1 rejected"));
}

fn scan_region(memory: &Fixture) -> (u64, usize) {
    let start = u64::from(MANAGER) - 0x80;
    memory.zeros(start, 0x80);
    memory.zeros(u64::from(MANAGER) + 0x18, 0x68);
    (start, 0x100)
}

#[test]
fn bounded_scan_skips_false_positive_and_proves_typed_manager() {
    let memory = fixture(BASE);
    let region = scan_region(&memory);
    memory.u32(region.0, BASE + CHARACTER_MANAGER_VTABLE_RVA);
    let result = find_manager(&memory, BASE, &[(0, 0), (region.0 - 1, 1), region], 4096).unwrap();
    assert_eq!(result, Some(MANAGER));
    assert!(memory.reads.borrow().contains(&(region.0, 0x100)));
}

#[test]
fn scanner_budget_bounds_all_attempts_and_skips_non_x86_regions() {
    for byte_budget in [0, 1, 7, 64, 256] {
        let memory = fixture(BASE);
        let region = scan_region(&memory);
        assert_eq!(
            find_manager(&memory, BASE, &[region], byte_budget).unwrap(),
            None
        );
        let reads = memory.reads.borrow();
        assert!(
            reads
                .iter()
                .skip(2)
                .map(|&(_, length)| length)
                .sum::<usize>()
                <= byte_budget
        );
        assert!(
            reads.iter().all(|&(address, length)| address >= MIN_POINTER
                && address + length as u64 <= ADDRESS_LIMIT)
        );
    }
    let memory = fixture(BASE);
    assert_eq!(
        find_manager(
            &memory,
            BASE,
            &[(ADDRESS_LIMIT, 0x1000), (u64::MAX - 2, 64)],
            4096
        )
        .unwrap(),
        None
    );
    assert_eq!(memory.reads.borrow().len(), 2);
}

#[test]
fn scanner_rejects_duplicate_empty_or_changing_vectors() {
    let memory = fixture(BASE);
    let region = scan_region(&memory);
    memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR + 8);
    memory.u32(at(VECTOR, 4), AVATAR);
    assert_eq!(find_manager(&memory, BASE, &[region], 4096).unwrap(), None);

    let memory = fixture(BASE);
    let region = scan_region(&memory);
    memory.u32(at(IMPLEMENTATION, IMPL_AVATAR_END), VECTOR);
    assert_eq!(find_manager(&memory, BASE, &[region], 4096).unwrap(), None);

    let memory = fixture(BASE);
    let region = scan_region(&memory);
    memory.avatar(BASE, AVATAR2);
    let changed = ChangeOnRead::u32(&memory, u64::from(VECTOR), 2, u64::from(VECTOR), AVATAR2);
    assert_eq!(find_manager(&changed, BASE, &[region], 4096).unwrap(), None);
}

#[path = "profile_tests.rs"]
mod profile_tests;

#[path = "live_tests.rs"]
mod live_tests;

#[path = "candidate_tests.rs"]
mod candidate_tests;

#[path = "direct_tests.rs"]
mod direct_tests;
