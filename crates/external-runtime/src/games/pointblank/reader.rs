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

impl Camera {
    /// Matrix values may move during a live read; the typed object chain and
    /// source viewport must still identify the same camera throughout capture.
    fn same_identity(&self, other: &Self) -> bool {
        self.framework == other.framework
            && self.camera == other.camera
            && self.view == other.view
            && self.projection == other.projection
            && self.width == other.width
            && self.height == other.height
    }
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

#[derive(Clone, Copy)]
enum CaptureMode {
    Strict,
    Live,
}

impl CaptureMode {
    fn accepts_camera(self, initial: &Camera, final_camera: &Camera) -> bool {
        match self {
            Self::Strict => initial == final_camera,
            Self::Live => initial.same_identity(final_camera),
        }
    }

    fn finish(self, frame: &mut Frame, camera: &Camera) {
        if let Self::Live = self {
            // Reproject the positions read in this capture, never cached world
            // data or extrapolated positions, with the last valid typed camera.
            for entity in &mut frame.entities {
                entity.screen = geometry::rectangle(
                    &camera.product,
                    entity.position,
                    camera.width,
                    camera.height,
                );
            }
        }
    }
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
    capture_internal(memory, image_base, manager, profile, CaptureMode::Strict)
}

/// Captures live positions while permitting natural view/projection movement.
/// Both camera reads retain every typed/matrix proof guard; camera-chain identity
/// and dimensions, collection entries, and Avatar field pointers remain stable.
/// Every accepted entity is reprojected using the final validated camera. The
/// caller verifies the selected profile's exact image hash before capture.
pub fn capture_live_profile(
    memory: &impl Memory,
    image_base: u32,
    manager: Option<u32>,
    profile: &BuildProfile,
) -> Result<Frame> {
    capture_internal(memory, image_base, manager, profile, CaptureMode::Live)
}

fn capture_internal(
    memory: &impl Memory,
    image_base: u32,
    manager: Option<u32>,
    profile: &BuildProfile,
    mode: CaptureMode,
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
        // Reread bounds AND every entry without deduplication. Both modes reject
        // collection mutation/retyping. Live mode permits matrix movement only
        // within the same fully validated camera chain and source dimensions.
        let stable_collection =
            load_collection(&reader, image_base, manager).is_ok_and(|again| again == collection);
        let final_camera = load_camera(&reader, image_base);
        if !stable_collection
            || !final_camera
                .as_ref()
                .is_ok_and(|again| mode.accepts_camera(&camera, again))
        {
            frame.entities.clear();
            frame.status = "CollectionChanged: frame discarded after consistency recheck".into();
            return Ok(frame);
        }
        if let Ok(final_camera) = final_camera {
            mode.finish(&mut frame, &final_camera);
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
    let stable_slot = reader.u32(slot).ok() == Some(avatar);
    // Preserve the strict path's short-circuit read order for its diagnostic
    // contract while applying the same live camera proof as full collections.
    let final_camera = if stable_slot {
        load_camera(&reader, image_base).ok()
    } else {
        None
    };
    let Some(final_camera) = final_camera.filter(|again| mode.accepts_camera(&camera, again))
    else {
        frame.status = "SingleAvatarChanged: observed slot/camera changed; frame discarded".into();
        return Ok(frame);
    };
    frame.entities.push(entity);
    mode.finish(&mut frame, &final_camera);
    frame.status =
        "SingleAvatar: one observed callback slot; full collection unresolved; estimated box"
            .into();
    Ok(frame)
}

/// A freshly proven manager, including a valid lobby-time empty collection.
#[derive(Clone, Debug)]
pub struct ManagerCandidate {
    pub address: u32,
    pub count: usize,
}

fn probe_manager<M: Memory>(
    reader: &Reader<'_, M>,
    base: u32,
    address: u32,
) -> Result<ManagerCandidate> {
    let collection = load_collection(reader, base, address)?;
    ensure!(
        load_collection(reader, base, address)? == collection,
        "manager collection changed during probe"
    );
    Ok(ManagerCandidate {
        address,
        count: collection.entries.len(),
    })
}

/// Revalidates a cached address using live PE identity, manager type, backlink,
/// vector bounds, unique typed entries, and a second identical collection read.
/// An empty vector is valid; this proof establishes a manager rather than players.
pub fn probe_manager_profile(
    memory: &impl Memory,
    base: u32,
    address: u32,
    profile: &BuildProfile,
) -> Result<ManagerCandidate> {
    validate_image_profile(memory, base, profile)?;
    probe_manager(&Reader::new(memory, profile), base, address)
}

/// Resolves the observed split-pointer manager singleton without a heap scan.
///
/// The caller first selects the profile by its full executable hash. A profile
/// without proven wrapper RVAs performs zero reads. For supported profiles, PE
/// identity failures remain errors; uninitialized, unsupported, unreadable or
/// changing singleton state returns `None` for a fresh bounded retry. Only the
/// observed initialized/zero-flags wrapper layout is supported, rather than a
/// general decoder for the game's protected-value API.
pub fn direct_manager_profile(
    memory: &impl Memory,
    base: u32,
    profile: &BuildProfile,
) -> Result<Option<ManagerCandidate>> {
    let Some(rvas) = profile.manager_wrappers else {
        return Ok(None);
    };
    validate_image_profile(memory, base, profile)?;
    let reader = Reader::new(memory, profile);
    let proof = || -> Result<ManagerCandidate> {
        let address = |rva: u32| -> Result<u64> {
            ensure!(
                rva & 3 == 0
                    && rva
                        .checked_add(0x14)
                        .is_some_and(|end| end <= profile.image_size),
                "manager wrapper exceeds selected profile image"
            );
            module_address(base, rva, profile)
        };
        let addresses = [address(rvas[0])?, address(rvas[1])?];
        let wrappers = [
            reader.block::<0x14>(addresses[0])?,
            reader.block::<0x14>(addresses[1])?,
        ];
        let payload_pointer = |wrapper: &[u8; 0x14]| -> Result<u32> {
            ensure!(
                u32_at(wrapper, 0) == 0
                    && u32_at(wrapper, 4) == 0
                    && u32_at(wrapper, 8) == 1
                    && u32_at(wrapper, 0x10) == 0,
                "uninitialized or unsupported manager wrapper"
            );
            let payload = u32_at(wrapper, 0x0c);
            ensure!(
                pointer(payload) && payload & 3 == 0,
                "invalid manager wrapper payload pointer"
            );
            Ok(payload)
        };
        let pointers = [
            payload_pointer(&wrappers[0])?,
            payload_pointer(&wrappers[1])?,
        ];
        let payloads = [
            reader.u32(u64::from(pointers[0]))?,
            reader.u32(u64::from(pointers[1]))?,
        ];
        let manager = payloads[0].wrapping_add(payloads[1]);
        ensure!(
            pointer(manager) && manager & 3 == 0,
            "invalid decoded manager pointer"
        );
        // Reuse the existing two-pass type/backlink/vector/unique-entry proof;
        // a stable empty collection is a legitimate lobby-time owner.
        let candidate = probe_manager(&reader, base, manager)?;
        let final_wrappers = [
            reader.block::<0x14>(addresses[0])?,
            reader.block::<0x14>(addresses[1])?,
        ];
        let final_payloads = [
            reader.u32(u64::from(pointers[0]))?,
            reader.u32(u64::from(pointers[1]))?,
        ];
        ensure!(
            final_wrappers == wrappers && final_payloads == payloads,
            "manager singleton changed during direct proof"
        );
        Ok(candidate)
    };
    Ok(proof().ok())
}

struct BudgetMemory<'a, M: Memory> {
    inner: &'a M,
    remaining: Cell<usize>,
    exhausted: Cell<bool>,
}

impl<M: Memory> Memory for BudgetMemory<'_, M> {
    fn read(&self, address: u64, out: &mut [u8]) -> Result<()> {
        let Some(remaining) = self.remaining.get().checked_sub(out.len()) else {
            self.exhausted.set(true);
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
    Ok(
        scan_managers(memory, base, regions, byte_budget, profile, false, 1)?
            .0
            .first()
            .map(|candidate| candidate.address),
    )
}

/// Finds up to 64 stable typed/backlinked managers, including valid empty vectors.
/// Attempted reads share the same byte budget as strict discovery. Empty managers
/// establish a cacheable address, never a claim that a live player was found.
pub fn manager_candidates_profile(
    memory: &impl Memory,
    base: u32,
    regions: &[(u64, usize)],
    byte_budget: usize,
    profile: &BuildProfile,
) -> Result<Vec<ManagerCandidate>> {
    Ok(scan_manager_candidates_profile(memory, base, regions, byte_budget, profile)?.0)
}

/// The consumed prefix is measured across the supplied region lengths, including
/// skipped/unreadable ranges. A budget stop before a proof leaves that candidate
/// unconsumed, so incremental callers retry it instead of skipping a window tail.
pub(crate) fn scan_manager_candidates_profile(
    memory: &impl Memory,
    base: u32,
    regions: &[(u64, usize)],
    byte_budget: usize,
    profile: &BuildProfile,
) -> Result<(Vec<ManagerCandidate>, usize)> {
    scan_managers(memory, base, regions, byte_budget, profile, true, 64)
}

fn scan_managers(
    memory: &impl Memory,
    base: u32,
    regions: &[(u64, usize)],
    byte_budget: usize,
    profile: &BuildProfile,
    allow_empty: bool,
    limit: usize,
) -> Result<(Vec<ManagerCandidate>, usize)> {
    validate_image_profile(memory, base, profile)?;
    let wanted = u32::try_from(module_address(
        base,
        profile.character_manager_vtable,
        profile,
    )?)?;
    let budget = BudgetMemory {
        inner: memory,
        remaining: Cell::new(byte_budget),
        exhausted: Cell::new(false),
    };
    let reader = Reader::new(&budget, profile);
    let mut candidates: Vec<ManagerCandidate> = Vec::new();
    let mut consumed = 0usize;
    for &(region_base, region_length) in regions {
        let region_consumed = |cursor: u64| {
            consumed.saturating_add(
                usize::try_from(cursor.saturating_sub(region_base))
                    .unwrap_or(region_length)
                    .min(region_length),
            )
        };
        let end = u64::try_from(region_length)
            .ok()
            .and_then(|length| region_base.checked_add(length));
        let Some(end) = end else {
            consumed = consumed.saturating_add(region_length);
            continue;
        };
        let end = end.min(ADDRESS_LIMIT);
        let Some(aligned) = region_base.max(MIN_POINTER).checked_add(3) else {
            consumed = consumed.saturating_add(region_length);
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
                return Ok((candidates, region_consumed(cursor)));
            }
            let mut bytes = vec![0; length];
            if reader.read_into(cursor, &mut bytes).is_ok() {
                for (index, entry) in bytes.as_chunks::<4>().0.iter().enumerate() {
                    if u32_at(entry, 0) != wanted {
                        continue;
                    }
                    let address = cursor
                        .checked_add(u64::try_from(index)? * 4)
                        .context("scan address overflow")?;
                    let Ok(candidate) = u32::try_from(address) else {
                        continue;
                    };
                    if candidates.iter().any(|known| known.address == candidate) {
                        continue;
                    }
                    let proof = load_collection(&reader, base, candidate).and_then(|collection| {
                        if !allow_empty && collection.entries.is_empty() {
                            return Ok(None);
                        }
                        ensure!(
                            load_collection(&reader, base, candidate)? == collection,
                            "manager collection changed during discovery"
                        );
                        Ok(Some(ManagerCandidate {
                            address: candidate,
                            count: collection.entries.len(),
                        }))
                    });
                    if budget.exhausted.get() {
                        return Ok((candidates, region_consumed(address)));
                    }
                    if let Ok(Some(candidate)) = proof {
                        candidates.push(candidate);
                        if candidates.len() == limit {
                            return Ok((candidates, region_consumed(address + 4)));
                        }
                    }
                }
            }
            cursor = cursor
                .checked_add(u64::try_from(length)?)
                .context("scan cursor overflow")?;
        }
        consumed = consumed.saturating_add(region_length);
    }
    Ok((candidates, consumed))
}
