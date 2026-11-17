use std::cell::Cell;

use anyhow::{Context, Result, bail, ensure};

use super::{Entity, Frame, Memory, geometry, offsets::*};

fn pointer(address: u32) -> bool {
    (MIN_POINTER..ADDRESS_LIMIT).contains(&u64::from(address))
}

fn checked_range(address: u64, length: usize) -> Result<()> {
    let length = u64::try_from(length).context("read length exceeds x86 address space")?;
    let end = address
        .checked_add(length)
        .context("read address overflow")?;
    ensure!(
        (MIN_POINTER..ADDRESS_LIMIT).contains(&address) && end <= ADDRESS_LIMIT,
        "read outside x86 pointer bounds: {address:#x}+{length:#x}"
    );
    Ok(())
}

fn field(object: u32, offset: u32) -> Result<u64> {
    ensure!(pointer(object), "invalid x86 object pointer {object:#x}");
    let address = u64::from(object)
        .checked_add(u64::from(offset))
        .context("object field overflow")?;
    checked_range(address, 1)?;
    Ok(address)
}

fn module_address(base: u32, rva: u32, profile: &BuildProfile) -> Result<u64> {
    ensure!(
        rva < profile.image_size,
        "RVA exceeds selected profile image"
    );
    field(base, rva)
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed snapshot layout"),
    )
}

fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_bits(u32_at(bytes, offset))
}

struct Reader<'a, M: Memory> {
    memory: &'a M,
    profile: &'a BuildProfile,
}

impl<'a, M: Memory> Reader<'a, M> {
    fn new(memory: &'a M, profile: &'a BuildProfile) -> Self {
        Self { memory, profile }
    }

    fn read_into(&self, address: u64, bytes: &mut [u8]) -> Result<()> {
        checked_range(address, bytes.len())?;
        self.memory
            .read(address, bytes)
            .with_context(|| format!("remote read {address:#x}+{:#x}", bytes.len()))
    }

    fn block<const N: usize>(&self, address: u64) -> Result<[u8; N]> {
        let mut bytes = [0; N];
        self.read_into(address, &mut bytes)?;
        Ok(bytes)
    }

    fn u32(&self, address: u64) -> Result<u32> {
        Ok(u32::from_le_bytes(self.block(address)?))
    }

    fn f32(&self, address: u64) -> Result<f32> {
        Ok(f32::from_bits(self.u32(address)?))
    }

    fn matrix(&self, address: u64) -> Result<[f32; 16]> {
        let bytes = self.block::<64>(address)?;
        Ok(std::array::from_fn(|i| f32_at(&bytes, i * 4)))
    }
}

/// Checks the live DOS/PE signature, x86 machine, PE32 magic and exact image size.
/// The on-disk SHA-256 guard belongs to the caller and precedes offset reads.
pub fn validate_image(memory: &impl Memory, image_base: u32) -> Result<()> {
    validate_image_profile(memory, image_base, &LEGACY_PROFILE)
}

/// Checks remote PE identity against the selected profile's exact image size.
/// Hash verification remains the caller's responsibility and precedes this call.
pub fn validate_image_profile(
    memory: &impl Memory,
    image_base: u32,
    profile: &BuildProfile,
) -> Result<()> {
    checked_range(u64::from(image_base), profile.image_size as usize)
        .context("PointBlank image crosses x86 address bounds")?;
    let reader = Reader::new(memory, profile);
    let dos = reader.block::<0x40>(u64::from(image_base))?;
    ensure!(
        &dos[..2] == b"MZ",
        "PointBlank image guard: DOS MZ mismatch"
    );
    let nt_offset = u32_at(&dos, 0x3c);
    ensure!(
        (0x40..=0x10_0000).contains(&nt_offset),
        "PointBlank image guard: invalid e_lfanew"
    );
    let nt = reader.block::<0x54>(field(image_base, nt_offset)?)?;
    ensure!(
        &nt[..4] == b"PE\0\0",
        "PointBlank image guard: PE signature mismatch"
    );
    let machine = u16::from_le_bytes([nt[4], nt[5]]);
    ensure!(
        machine == 0x014c,
        "PointBlank image guard: expected x86 machine 0x14c"
    );
    let optional_size = u16::from_le_bytes([nt[0x14], nt[0x15]]);
    ensure!(
        optional_size >= 0x60,
        "PointBlank image guard: truncated optional header"
    );
    let magic = u16::from_le_bytes([nt[0x18], nt[0x19]]);
    ensure!(
        magic == 0x010b,
        "PointBlank image guard: expected PE32 optional header"
    );
    ensure!(
        u32_at(&nt, 0x50) == profile.image_size,
        "PointBlank image guard: SizeOfImage differs from {:#x}",
        profile.image_size
    );
    Ok(())
}

#[derive(Clone, Copy)]
struct FrameworkHeader {
    vtable: u32,
    camera: u32,
}

fn framework<M: Memory>(reader: &Reader<'_, M>, address: u32) -> Result<FrameworkHeader> {
    ensure!(pointer(address), "invalid framework pointer");
    if let Ok(bytes) = reader.block::<0xf0>(u64::from(address)) {
        return Ok(FrameworkHeader {
            vtable: u32_at(&bytes, 0),
            camera: u32_at(&bytes, FRAMEWORK_CAMERA as usize),
        });
    }
    // An unreadable padding gap falls back to the real fields, as in C++.
    Ok(FrameworkHeader {
        vtable: reader.u32(u64::from(address))?,
        camera: reader.u32(field(address, FRAMEWORK_CAMERA)?)?,
    })
}

#[derive(Clone, Copy)]
struct CameraHeader {
    vtable: u32,
    width: f32,
    height: f32,
    projection: u32,
    view: u32,
}

fn camera_header<M: Memory>(reader: &Reader<'_, M>, address: u32) -> Result<CameraHeader> {
    ensure!(pointer(address), "invalid camera pointer");
    if let Ok(bytes) = reader.block::<0xd8>(u64::from(address)) {
        return Ok(CameraHeader {
            vtable: u32_at(&bytes, 0),
            width: f32_at(&bytes, CAMERA_WIDTH as usize),
            height: f32_at(&bytes, CAMERA_HEIGHT as usize),
            projection: u32_at(&bytes, CAMERA_PROJECTION as usize),
            view: u32_at(&bytes, CAMERA_VIEW as usize),
        });
    }
    Ok(CameraHeader {
        vtable: reader.u32(u64::from(address))?,
        width: reader.f32(field(address, CAMERA_WIDTH)?)?,
        height: reader.f32(field(address, CAMERA_HEIGHT)?)?,
        projection: reader.u32(field(address, CAMERA_PROJECTION)?)?,
        view: reader.u32(field(address, CAMERA_VIEW)?)?,
    })
}

fn attribute<M: Memory>(reader: &Reader<'_, M>, address: u32) -> Result<(u32, [f32; 16])> {
    ensure!(pointer(address), "invalid camera attribute pointer");
    if let Ok(bytes) = reader.block::<0x70>(u64::from(address)) {
        return Ok((
            u32_at(&bytes, 0),
            std::array::from_fn(|i| f32_at(&bytes, 0x30 + i * 4)),
        ));
    }
    Ok((
        reader.u32(u64::from(address))?,
        reader.matrix(field(address, ATTRIBUTE_MATRIX)?)?,
    ))
}

fn require_type<M: Memory>(reader: &Reader<'_, M>, base: u32, object: u32, rva: u32) -> Result<()> {
    ensure!(pointer(object), "invalid typed pointer {object:#x}");
    let vtable = reader.u32(u64::from(object))?;
    check_type(reader, base, vtable, rva)
}

fn check_type<M: Memory>(reader: &Reader<'_, M>, base: u32, vtable: u32, rva: u32) -> Result<()> {
    ensure!(
        u64::from(vtable) == module_address(base, rva, reader.profile)?,
        "vtable mismatch for RVA {rva:#x}"
    );
    Ok(())
}

#[derive(PartialEq)]
struct Camera {
    framework: u32,
    camera: u32,
    view: u32,
    projection: u32,
    width: f32,
    height: f32,
    view_matrix: [f32; 16],
    projection_matrix: [f32; 16],
    product: [f32; 16],
}

fn load_camera<M: Memory>(reader: &Reader<'_, M>, base: u32) -> Result<Camera> {
    let profile = reader.profile;
    let framework_address = reader.u32(module_address(base, profile.framework_rva, profile)?)?;
    let framework = framework(reader, framework_address)?;
    check_type(reader, base, framework.vtable, profile.framework_vtable)?;
    let camera = camera_header(reader, framework.camera)?;
    check_type(reader, base, camera.vtable, profile.camera_vtable)?;
    let (view_type, view) = attribute(reader, camera.view)?;
    check_type(reader, base, view_type, profile.view_vtable)?;
    let (projection_type, projection) = attribute(reader, camera.projection)?;
    check_type(reader, base, projection_type, profile.projection_vtable)?;
    ensure!(
        geometry::camera_matrices(&view, &projection, camera.width, camera.height),
        "camera matrix proof guard failed"
    );
    let product =
        geometry::multiply(&view, &projection).context("nonfinite view/projection product")?;
    Ok(Camera {
        framework: framework_address,
        camera: framework.camera,
        view: camera.view,
        projection: camera.projection,
        width: camera.width,
        height: camera.height,
        view_matrix: view,
        projection_matrix: projection,
        product,
    })
}

#[derive(PartialEq)]
struct Collection {
    implementation: u32,
    begin: u32,
    end: u32,
    entries: Vec<u32>,
}

fn manager_header<M: Memory>(reader: &Reader<'_, M>, address: u32) -> Result<(u32, u32)> {
    ensure!(pointer(address), "invalid character-manager pointer");
    if let Ok(bytes) = reader.block::<0x18>(u64::from(address)) {
        return Ok((
            u32_at(&bytes, 0),
            u32_at(&bytes, CHARACTER_MANAGER_IMPL as usize),
        ));
    }
    Ok((
        reader.u32(u64::from(address))?,
        reader.u32(field(address, CHARACTER_MANAGER_IMPL)?)?,
    ))
}

fn implementation<M: Memory>(reader: &Reader<'_, M>, address: u32) -> Result<(u32, u32, u32)> {
    ensure!(pointer(address), "invalid manager implementation pointer");
    if let Ok(bytes) = reader.block::<0x10>(u64::from(address)) {
        return Ok((u32_at(&bytes, 0), u32_at(&bytes, 8), u32_at(&bytes, 12)));
    }
    let bounds = reader.block::<8>(field(address, IMPL_AVATAR_BEGIN)?)?;
    Ok((
        reader.u32(u64::from(address))?,
        u32_at(&bounds, 0),
        u32_at(&bounds, 4),
    ))
}

fn load_collection<M: Memory>(
    reader: &Reader<'_, M>,
    base: u32,
    manager: u32,
) -> Result<Collection> {
    let (vtable, implementation_address) = manager_header(reader, manager)?;
    check_type(
        reader,
        base,
        vtable,
        reader.profile.character_manager_vtable,
    )?;
    let (backlink, begin, end) = implementation(reader, implementation_address)?;
    ensure!(
        backlink == manager,
        "manager implementation backlink mismatch"
    );
    let mut collection = Collection {
        implementation: implementation_address,
        begin,
        end,
        entries: Vec::new(),
    };
    if begin == 0 && end == 0 {
        return Ok(collection);
    }
    ensure!(
        pointer(begin) && begin & 3 == 0,
        "invalid or unaligned Avatar-vector begin"
    );
    let span = end
        .checked_sub(begin)
        .context("reversed Avatar-vector bounds")?;
    ensure!(
        span & 3 == 0 && span as usize <= MAX_COLLECTION * 4,
        "invalid Avatar-vector extent"
    );
    if span == 0 {
        return Ok(collection);
    }
    let mut bytes = vec![0; span as usize];
    reader.read_into(u64::from(begin), &mut bytes)?;
    for bytes in bytes.as_chunks::<4>().0.iter() {
        let avatar = u32_at(bytes, 0);
        require_type(reader, base, avatar, reader.profile.avatar_vtable)?;
        ensure!(
            !collection.entries.contains(&avatar),
            "duplicate Avatar-vector entry"
        );
        collection.entries.push(avatar);
    }
    Ok(collection)
}

fn avatar_fields<M: Memory>(reader: &Reader<'_, M>, avatar: u32) -> Result<(u32, u32)> {
    let address = field(avatar, AVATAR_MATRIX)?;
    if let Ok(bytes) = reader.block::<0xb4>(address) {
        return Ok((
            u32_at(&bytes, 0),
            u32_at(&bytes, (AVATAR_HEALTH - AVATAR_MATRIX) as usize),
        ));
    }
    Ok((
        reader.u32(address)?,
        reader.u32(field(avatar, AVATAR_HEALTH)?)?,
    ))
}

fn body_matrix<M: Memory>(reader: &Reader<'_, M>, address: u32) -> Result<(u32, [f32; 3])> {
    ensure!(pointer(address), "invalid body-matrix pointer");
    if let Ok(bytes) = reader.block::<0x5c>(u64::from(address)) {
        return Ok((
            u32_at(&bytes, 0),
            std::array::from_fn(|i| f32_at(&bytes, 0x50 + i * 4)),
        ));
    }
    let position = reader.block::<12>(field(address, MATRIX_TRANSLATION)?)?;
    Ok((
        reader.u32(u64::from(address))?,
        std::array::from_fn(|i| f32_at(&position, i * 4)),
    ))
}

fn load_entity<M: Memory>(
    reader: &Reader<'_, M>,
    base: u32,
    avatar: u32,
    camera: &Camera,
) -> Result<Entity> {
    require_type(reader, base, avatar, reader.profile.avatar_vtable)?;
    let (matrix, health) = avatar_fields(reader, avatar)?;
    let record = reader.block::<8>(field(health, HEALTH_ID)?)?;
    let id = u32_at(&record, 0);
    let hp = f32_at(&record, 4);
    ensure!(
        id < MAX_COLLECTION as u32,
        "health ID exceeds collection limit"
    );
    ensure!(
        hp.is_finite() && (0.0..=10_000.0).contains(&hp),
        "invalid health float"
    );
    let (matrix_type, position) = body_matrix(reader, matrix)?;
    check_type(reader, base, matrix_type, reader.profile.matrix_vtable)?;
    for (i, value) in position.iter().enumerate() {
        let limit = if i == 1 { 1000.0 } else { 100_000.0 };
        ensure!(
            value.is_finite() && value.abs() <= limit,
            "invalid body-transform translation"
        );
    }
    require_type(reader, base, avatar, reader.profile.avatar_vtable)?;
    ensure!(
        avatar_fields(reader, avatar)? == (matrix, health),
        "Avatar fields changed during capture"
    );
    require_type(reader, base, matrix, reader.profile.matrix_vtable)?;
    Ok(Entity {
        address: avatar,
        health: hp,
        position,
        screen: geometry::rectangle(&camera.product, position, camera.width, camera.height),
    })
}

/// Captures either a proven full typed collection, or a single observed Avatar.
/// Passing a manager never silently falls back to the unrelated current-Avatar
/// slot. Invalid objects produce explicit empty/rejected states; a PE/build or
/// initial image-read failure is returned as an error.
pub fn capture(memory: &impl Memory, image_base: u32, manager: Option<u32>) -> Result<Frame> {
    capture_profile(memory, image_base, manager, &LEGACY_PROFILE)
}

/// Captures using explicit module RVAs and image identity, with shared verified
/// object-field offsets. The caller verifies the selected profile's hash first.
pub fn capture_profile(
    memory: &impl Memory,
    image_base: u32,
    manager: Option<u32>,
    profile: &BuildProfile,
) -> Result<Frame> {
    validate_image_profile(memory, image_base, profile)?;
    let reader = Reader::new(memory, profile);
    let mut frame = Frame {
        manager,
        ..Frame::default()
    };
    let camera = match load_camera(&reader, image_base) {
        Ok(camera) => camera,
        Err(error) => {
            frame.status = format!("WaitingCamera: {error:#}");
            return Ok(frame);
        }
    };
    frame.width = camera.width;
    frame.height = camera.height;
    if let Some(manager) = manager {
        let collection = match load_collection(&reader, image_base, manager) {
            Ok(collection) => collection,
            Err(error) => {
                frame.status = format!("CollectionInvalid: {error:#}");
                return Ok(frame);
            }
        };
        let mut rejected = 0;
        for &avatar in &collection.entries {
            match load_entity(&reader, image_base, avatar, &camera) {
                Ok(entity) => frame.entities.push(entity),
                Err(_) => rejected += 1,
            }
        }
        // Reread bounds AND every entry without deduplication. Reject the whole
        // frame on mutation, including retyping or a changed camera snapshot.
        let stable_collection =
            load_collection(&reader, image_base, manager).is_ok_and(|again| again == collection);
        let stable_camera = load_camera(&reader, image_base).is_ok_and(|again| again == camera);
        if !stable_collection || !stable_camera {
            frame.entities.clear();
            frame.status = "CollectionChanged: frame discarded after consistency recheck".into();
            return Ok(frame);
        }
        frame.status = format!(
            "FullCollection: {} typed entries, {} entities, {rejected} rejected; estimated boxes",
            collection.entries.len(),
            frame.entities.len()
        );
        return Ok(frame);
    }
    let slot = module_address(image_base, profile.current_avatar_rva, profile)?;
    let avatar = match reader.u32(slot) {
        Ok(avatar) => avatar,
        Err(error) => {
            frame.status = format!(
                "WaitingManager: collection root unresolved; current-Avatar read: {error:#}"
            );
            return Ok(frame);
        }
    };
    let entity = match load_entity(&reader, image_base, avatar, &camera) {
        Ok(entity) => entity,
        Err(error) => {
            frame.status = format!(
                "WaitingManager: collection root unresolved; current-Avatar invalid: {error:#}"
            );
            return Ok(frame);
        }
    };
    if reader.u32(slot).ok() != Some(avatar)
        || !load_camera(&reader, image_base).is_ok_and(|again| again == camera)
    {
        frame.status = "SingleAvatarChanged: observed slot/camera changed; frame discarded".into();
        return Ok(frame);
    }
    frame.entities.push(entity);
    frame.status =
        "SingleAvatar: one observed callback slot; full collection unresolved; estimated box"
            .into();
    Ok(frame)
}

struct BudgetMemory<'a, M: Memory> {
    inner: &'a M,
    remaining: Cell<usize>,
}

impl<M: Memory> Memory for BudgetMemory<'_, M> {
    fn read(&self, address: u64, out: &mut [u8]) -> Result<()> {
        let Some(remaining) = self.remaining.get().checked_sub(out.len()) else {
            bail!("manager scan byte budget exhausted");
        };
        // Failed attempts also consume budget, including snapshot fallbacks.
        self.remaining.set(remaining);
        self.inner.read(address, out)
    }
}

/// Bounded dynamic-vtable/RTTI-backed manager discovery, ported from
/// old game::discover_manager. The host supplies committed, private, readable
/// regions (guard/no-access excluded); this module never queries a process.
///
/// `byte_budget` bounds all attempted scan and candidate-validation bytes,
/// excluding the mandatory two PE-identity reads. Slabs are at most 64 KiB and
/// reserve half the remaining budget for typed validation. Exhaustion or stale
/// regions returns `Ok(None)`. A candidate requires a nonempty, stable, unique,
/// max-128 typed Avatar vector and manager-implementation backlink.
pub fn find_manager(
    memory: &impl Memory,
    base: u32,
    regions: &[(u64, usize)],
    byte_budget: usize,
) -> Result<Option<u32>> {
    find_manager_profile(memory, base, regions, byte_budget, &LEGACY_PROFILE)
}

/// Profile-aware manager discovery with the same total attempted-byte budget.
/// Mandatory PE identity reads remain outside the scan budget, including zero.
pub fn find_manager_profile(
    memory: &impl Memory,
    base: u32,
    regions: &[(u64, usize)],
    byte_budget: usize,
    profile: &BuildProfile,
) -> Result<Option<u32>> {
    validate_image_profile(memory, base, profile)?;
    let wanted = u32::try_from(module_address(
        base,
        profile.character_manager_vtable,
        profile,
    )?)?;
    let budget = BudgetMemory {
        inner: memory,
        remaining: Cell::new(byte_budget),
    };
    let reader = Reader::new(&budget, profile);
    for &(region_base, region_size) in regions {
        let Ok(region_size) = u64::try_from(region_size) else {
            continue;
        };
        let Some(end) = region_base.checked_add(region_size) else {
            continue;
        };
        let end = end.min(ADDRESS_LIMIT);
        let Some(aligned) = region_base.max(MIN_POINTER).checked_add(3) else {
            continue;
        };
        let mut cursor = aligned & !3;
        while cursor < end {
            let available = usize::try_from(end - cursor).unwrap_or(usize::MAX);
            if available < 4 {
                break;
            }
            let length = available.min(0x1_0000).min(budget.remaining.get() / 2) & !3;
            if length < 4 {
                return Ok(None);
            }
            let mut bytes = vec![0; length];
            if reader.read_into(cursor, &mut bytes).is_ok() {
                for (index, entry) in bytes.as_chunks::<4>().0.iter().enumerate() {
                    if u32_at(entry, 0) != wanted {
                        continue;
                    }
                    let offset = u64::try_from(index)?
                        .checked_mul(4)
                        .context("scan index overflow")?;
                    let candidate = cursor
                        .checked_add(offset)
                        .context("scan address overflow")?;
                    let Ok(candidate) = u32::try_from(candidate) else {
                        continue;
                    };
                    let Ok(collection) = load_collection(&reader, base, candidate) else {
                        continue;
                    };
                    if !collection.entries.is_empty()
                        && load_collection(&reader, base, candidate)
                            .is_ok_and(|again| again == collection)
                    {
                        return Ok(Some(candidate));
                    }
                }
            }
            cursor = cursor
                .checked_add(u64::try_from(length)?)
                .context("scan cursor overflow")?;
        }
    }
    Ok(None)
}
