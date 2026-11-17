use crate::games::pointblank::{
    self, Frame, Memory,
    patches::{Controller, Requested},
};
use anyhow::{Context, Result, ensure};
use hyperrev_transport::{Connection, Process};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

const ATTACH_RETRY_INTERVAL: Duration = Duration::from_millis(200);
const MANAGER_SCAN_INTERVAL: Duration = Duration::from_millis(100);
const MANAGER_PROBE_INTERVAL: Duration = Duration::from_millis(40);
const MAX_CACHED_MANAGERS: usize = 64;
const REGION_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const MANAGER_SCAN_WINDOWS: usize = 8;
const MANAGER_SCAN_WINDOW_BYTES: usize = 256 * 1024;

/// Wall-clock gates keep discovery work independent of the caller's poll rate.
struct Cadence {
    interval: Duration,
    next: Option<Instant>,
}

impl Cadence {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            next: None,
        }
    }

    fn take(&mut self, now: Instant) -> bool {
        if self.next.is_some_and(|next| now < next) {
            return false;
        }
        self.defer(now);
        true
    }

    fn defer(&mut self, now: Instant) {
        self.next = now.checked_add(self.interval);
    }

    fn reset(&mut self) {
        self.next = None;
    }
}

/// Only live reader proofs enter this bounded cache. Counts are selection hints;
/// every emitted frame independently revalidates its collection and camera.
#[derive(Default)]
struct ManagerCache {
    candidates: Vec<pointblank::ManagerCandidate>,
    active: Option<u32>,
}

impl ManagerCache {
    fn select_active(&mut self) {
        self.active = self
            .candidates
            .iter()
            .filter(|candidate| candidate.count > 0)
            .max_by_key(|candidate| (candidate.count, Some(candidate.address) == self.active))
            .map(|candidate| candidate.address);
    }

    fn remember(&mut self, candidate: pointblank::ManagerCandidate) {
        if let Some(known) = self
            .candidates
            .iter_mut()
            .find(|known| known.address == candidate.address)
        {
            *known = candidate;
        } else {
            if self.candidates.len() == MAX_CACHED_MANAGERS {
                let (index, lowest) = self
                    .candidates
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, known)| known.count)
                    .expect("a full manager cache is nonempty");
                if candidate.count < lowest.count || (candidate.count == 0 && lowest.count > 0) {
                    return;
                }
                // Rotate equally empty candidates instead of pinning the first
                // addresses forever when a scene has more than the cache bound.
                self.candidates.remove(index);
            }
            self.candidates.push(candidate);
        }
        self.select_active();
    }

    fn reprobe(&mut self, mut probe: impl FnMut(u32) -> Result<pointblank::ManagerCandidate>) {
        self.candidates.retain_mut(|candidate| {
            if let Ok(current) = probe(candidate.address)
                && current.address == candidate.address
            {
                *candidate = current;
                true
            } else {
                false
            }
        });
        self.select_active();
    }

    fn invalidate(&mut self, address: u32) {
        self.candidates
            .retain(|candidate| candidate.address != address);
        self.select_active();
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct FeatureState {
    pub available: bool,
    pub names: bool,
    pub anti_kick: bool,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub status: String,
    pub detail: String,
    pub pid: Option<u32>,
    pub base: Option<u64>,
    pub cr3: Option<u64>,
    pub frame: Option<Frame>,
    pub features: FeatureState,
}

impl Snapshot {
    pub fn waiting(message: impl Into<String>) -> Self {
        Self {
            status: message.into(),
            detail: String::new(),
            pid: None,
            base: None,
            cr3: None,
            frame: None,
            features: FeatureState::default(),
        }
    }
}

struct TargetMemory<'a> {
    connection: &'a Connection,
    cr3: u64,
}
impl Memory for TargetMemory<'_> {
    fn read(&self, address: u64, out: &mut [u8]) -> Result<()> {
        self.connection.read(self.cr3, address, out)
    }
}

impl pointblank::patches::Memory for TargetMemory<'_> {
    fn read(&self, address: u64, out: &mut [u8]) -> Result<()> {
        self.connection.read(self.cr3, address, out)
    }
    fn write(&self, address: u64, bytes: &[u8]) -> Result<()> {
        self.connection.write(self.cr3, address, bytes)
    }
}

pub struct Engine {
    connection: Connection,
    process: Option<Process>,
    lifetime: Option<native::ProcessLifetime>,
    profile: Option<pointblank::BuildProfile>,
    observed: Option<Process>,
    managers: ManagerCache,
    regions: Vec<(u64, usize)>,
    region_index: usize,
    region_offset: usize,
    region_cycle_complete: bool,
    attach_retry: Cadence,
    manager_scan: Cadence,
    manager_probe: Cadence,
    region_refresh: Cadence,
    build_detail: String,
    patches: Controller,
    requested_features: Requested,
    attempted_features: Option<Requested>,
    feature_message: String,
}

impl Engine {
    pub fn new(connection: Connection) -> Self {
        Self {
            connection,
            process: None,
            lifetime: None,
            profile: None,
            observed: None,
            managers: ManagerCache::default(),
            regions: Vec::new(),
            region_index: 0,
            region_offset: 0,
            region_cycle_complete: false,
            attach_retry: Cadence::new(ATTACH_RETRY_INTERVAL),
            manager_scan: Cadence::new(MANAGER_SCAN_INTERVAL),
            manager_probe: Cadence::new(MANAGER_PROBE_INTERVAL),
            region_refresh: Cadence::new(REGION_REFRESH_INTERVAL),
            build_detail: String::new(),
            patches: Controller::default(),
            requested_features: Requested::default(),
            attempted_features: None,
            feature_message: String::new(),
        }
    }

    pub fn memory_check(&self) -> Result<()> {
        self.connection.self_test()
    }

    pub fn set_features(&mut self, requested: Requested) {
        if self.requested_features.names != requested.names
            || self.requested_features.anti_kick != requested.anti_kick
        {
            self.requested_features = requested;
            self.attempted_features = None;
        }
    }

    pub fn restore_features(&mut self) -> Result<()> {
        if !self.patches.owns_patches() {
            return Ok(());
        }
        if let (Some(process), Some(lifetime)) = (&self.process, &self.lifetime) {
            if lifetime.alive()? {
                self.patches.restore(&TargetMemory {
                    connection: &self.connection,
                    cr3: process.cr3,
                })?;
            } else {
                self.patches = Controller::default();
            }
        }
        ensure!(
            !self.patches.owns_patches(),
            "Feature restoration lost its process lifetime"
        );
        self.attempted_features = None;
        Ok(())
    }

    pub fn verify_feature_roundtrip(&mut self) -> Result<()> {
        ensure!(
            !self.patches.owns_patches(),
            "Feature self-test requires original target instructions"
        );
        let process = self
            .process
            .as_ref()
            .context("Feature self-test requires an attached PointBlank process")?;
        let profile = self
            .profile
            .as_ref()
            .context("Feature self-test requires a supported game build")?;
        ensure!(
            profile.sha256 == pointblank::CURRENT_PROFILE.sha256,
            "Feature self-test requires the verified current profile"
        );
        let memory = TargetMemory {
            connection: &self.connection,
            cr3: process.cr3,
        };
        let result = (|| -> Result<()> {
            for requested in [
                Requested {
                    names: true,
                    anti_kick: false,
                },
                Requested {
                    names: false,
                    anti_kick: true,
                },
                Requested {
                    names: true,
                    anti_kick: true,
                },
            ] {
                self.patches
                    .sync(&memory, process.image_base as u32, profile, requested)?;
                let active = self.patches.active();
                ensure!(
                    active.names == requested.names && active.anti_kick == requested.anti_kick,
                    "Feature patch readback differs from requested state"
                );
                self.patches.sync(
                    &memory,
                    process.image_base as u32,
                    profile,
                    Requested::default(),
                )?;
                ensure!(
                    !self.patches.owns_patches(),
                    "Feature self-test restoration is incomplete"
                );
            }
            Ok(())
        })();
        let restore = self.patches.restore(&memory);
        self.attempted_features = None;
        result?;
        restore?;
        Ok(())
    }

    fn feature_state(&self) -> FeatureState {
        let active = self.patches.active();
        FeatureState {
            available: self
                .profile
                .as_ref()
                .is_some_and(|profile| profile.sha256 == pointblank::CURRENT_PROFILE.sha256),
            names: active.names,
            anti_kick: active.anti_kick,
            message: self.feature_message.clone(),
        }
    }

    pub fn poll(&mut self) -> Snapshot {
        match self.capture() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let process = self.process.as_ref().or(self.observed.as_ref());
                let snapshot = Snapshot {
                    status: "PointBlank check failed".into(),
                    detail: format!("{error:#}"),
                    pid: process.map(|p| p.pid),
                    base: process.map(|p| p.image_base),
                    cr3: process.map(|p| p.cr3),
                    frame: None,
                    features: self.feature_state(),
                };
                // A returned error is an attachment/image failure. Typed game
                // transitions are successful waiting frames and retain the CR3.
                let mut snapshot = snapshot;
                if let Err(restore) = self.clear_target() {
                    self.requested_features = Requested::default();
                    self.feature_message = format!("Restore pending: {restore:#}");
                    snapshot
                        .detail
                        .push_str(&format!("\n{}", self.feature_message));
                    snapshot.features = self.feature_state();
                }
                self.attach_retry.defer(Instant::now());
                snapshot
            }
        }
    }

    fn clear_target(&mut self) -> Result<()> {
        self.restore_features()?;
        self.patches = Controller::default();
        self.attempted_features = None;
        self.feature_message.clear();
        self.process = None;
        self.lifetime = None;
        self.profile = None;
        self.observed = None;
        self.managers = ManagerCache::default();
        self.regions.clear();
        self.region_index = 0;
        self.region_offset = 0;
        self.region_cycle_complete = false;
        self.manager_scan.reset();
        self.manager_probe.reset();
        self.region_refresh.reset();
        self.build_detail.clear();
        Ok(())
    }

    fn capture(&mut self) -> Result<Snapshot> {
        let now = Instant::now();
        if self.process.is_some()
            && !self
                .lifetime
                .as_ref()
                .context("PointBlank process lifetime is absent")?
                .alive()?
        {
            // The held process object remains signaled even if its PID is reused.
            self.clear_target()?;
            self.attach_retry.defer(now);
            return Ok(Snapshot::waiting("Waiting for PointBlank"));
        }
        if self.process.is_none() {
            if !self.attach_retry.take(now) {
                return Ok(Snapshot::waiting("Waiting for PointBlank"));
            }
            let Some(pid) = native::game_pid(pointblank::PROCESS_NAMES)? else {
                return Ok(Snapshot::waiting("Waiting for PointBlank"));
            };
            // Bind a native lifetime before the expensive checked kernel walk.
            // Holding this object and checking it again prevents PID-reuse races.
            let lifetime = native::ProcessLifetime::open(pid)?;
            let Some(process) = hyperrev_transport::process_by_pid(&self.connection, pid)? else {
                self.attach_retry.defer(Instant::now());
                return Ok(Snapshot::waiting("Waiting for PointBlank"));
            };
            ensure!(
                lifetime.pid() == process.pid && lifetime.alive()?,
                "PointBlank process exited during attachment"
            );
            ensure!(
                process.image_base <= u32::MAX as u64,
                "PointBlank offsets require a PE32 process"
            );
            self.observed = Some(process.clone());
            let path = lifetime.image_path()?;
            let profile = verify_image(&path)?;
            ensure!(
                process.image_size == profile.image_size,
                "PointBlank image size differs from the selected verified build"
            );
            // A supported direct singleton path needs neither a region walk nor
            // a heap scan, even when the match's player vector is still empty.
            self.regions = if profile.manager_wrappers.is_some() {
                Vec::new()
            } else {
                native::regions(process.pid).unwrap_or_default()
            };
            ensure!(
                lifetime.alive()?,
                "PointBlank process exited during build verification"
            );
            self.build_detail = format!(
                "Offset build: {}\nExecutable SHA-256: {}\nProcess creation FILETIME: {}\nReads: hyper-reV CPUID virtual-copy",
                profile.id,
                profile.sha256,
                lifetime.creation_time()
            );
            self.profile = Some(profile);
            self.lifetime = Some(lifetime);
            self.region_index = 0;
            self.region_offset = 0;
            self.region_cycle_complete = false;
            self.region_refresh.defer(Instant::now());
            self.managers = ManagerCache::default();
            self.process = Some(process);
        }
        let process = self
            .process
            .as_ref()
            .context("PointBlank target disappeared")?;
        let profile = self
            .profile
            .as_ref()
            .context("PointBlank build profile is absent")?;
        let memory = TargetMemory {
            connection: &self.connection,
            cr3: process.cr3,
        };
        if self.attempted_features.is_none() {
            let requested = self.requested_features;
            self.feature_message.clear();
            if (requested.names || requested.anti_kick || self.patches.owns_patches())
                && let Err(error) =
                    self.patches
                        .sync(&memory, process.image_base as u32, profile, requested)
            {
                self.feature_message = format!("Feature patch: {error:#}");
            }
            self.attempted_features = Some(requested);
        }
        let direct = profile.manager_wrappers.is_some();
        let mut direct_address = None;
        if direct {
            // Current-build singleton storage is resolved again on every poll.
            // Its fixed-size read path never waits on a heap-discovery cadence.
            self.managers = ManagerCache::default();
            if let Some(candidate) =
                pointblank::direct_manager_profile(&memory, process.image_base as u32, profile)?
            {
                direct_address = Some(candidate.address);
                self.managers.remember(candidate);
            }
        }
        // Legacy profiles retain their independently throttled bounded scan.
        if !direct && self.manager_probe.take(Instant::now()) {
            self.managers.reprobe(|address| {
                pointblank::probe_manager_profile(
                    &memory,
                    process.image_base as u32,
                    address,
                    profile,
                )
            });
            self.manager_probe.defer(Instant::now());
        }
        // Steady-state capture performs no kernel process walks or region queries.
        // Unresolved discovery scans up to 2 MiB per 100ms, with all candidate
        // validation bounded by the reader's attempted-byte budget as well.
        if !direct && self.managers.active.is_none() && self.manager_scan.take(Instant::now()) {
            if (self.regions.is_empty() || self.region_cycle_complete)
                && self.region_refresh.take(Instant::now())
                && let Ok(regions) = native::regions(process.pid)
            {
                self.regions = regions;
                self.region_index = 0;
                self.region_offset = 0;
                self.region_cycle_complete = false;
            }
            for _ in 0..MANAGER_SCAN_WINDOWS {
                if self.managers.active.is_some() || self.regions.is_empty() {
                    break;
                }
                let (address, region_length) = self.regions[self.region_index];
                let length = region_length
                    .saturating_sub(self.region_offset)
                    .min(MANAGER_SCAN_WINDOW_BYTES);
                let consumed = if length > 0 {
                    let (candidates, consumed) = pointblank::scan_manager_candidates_profile(
                        &memory,
                        process.image_base as u32,
                        &[(address + self.region_offset as u64, length)],
                        2 * length + 4096,
                        profile,
                    )?;
                    for candidate in candidates {
                        self.managers.remember(candidate);
                    }
                    consumed.min(length)
                } else {
                    0
                };
                self.region_offset = self.region_offset.saturating_add(consumed);
                if self.region_offset >= region_length {
                    self.region_offset = 0;
                    self.region_index = (self.region_index + 1) % self.regions.len();
                    self.region_cycle_complete |= self.region_index == 0;
                } else if consumed == 0 {
                    break;
                }
            }
            self.manager_scan.defer(Instant::now());
        }
        let mut frame = if direct && direct_address.is_none() {
            Frame {
                status: "WaitingManager: direct singleton is uninitialized or changing".into(),
                ..Frame::default()
            }
        } else {
            pointblank::capture_live_profile(
                &memory,
                process.image_base as u32,
                if direct {
                    direct_address
                } else {
                    self.managers.active
                },
                profile,
            )?
        };
        if !direct
            && (frame.status.starts_with("CollectionInvalid:")
                || frame.status.starts_with("FullCollection: 0 typed entries,"))
        {
            if let Some(address) = frame.manager {
                if frame.status.starts_with("CollectionInvalid:") {
                    self.managers.invalidate(address);
                } else {
                    self.managers
                        .remember(pointblank::ManagerCandidate { address, count: 0 });
                }
            }
            // Immediately discard invalid/cleared active worlds. Another freshly
            // proven cached collection may take over without resetting scan progress.
            self.managers.reprobe(|address| {
                pointblank::probe_manager_profile(
                    &memory,
                    process.image_base as u32,
                    address,
                    profile,
                )
            });
            self.manager_probe.defer(Instant::now());
            frame = pointblank::capture_live_profile(
                &memory,
                process.image_base as u32,
                self.managers.active,
                profile,
            )?;
        }
        let status = if self.managers.active.is_none() && !self.managers.candidates.is_empty() {
            format!(
                "Waiting for match · {} managers",
                self.managers.candidates.len()
            )
        } else {
            frame.status.clone()
        };
        Ok(Snapshot {
            status,
            detail: format!(
                "Capture: {}\n{}\nManager path: {}\nManager: {:?}\nEntities: {}\nManagers found: {}; scan region {}/{} (+{:#x} bytes)",
                frame.status,
                self.build_detail,
                if direct {
                    "direct singleton"
                } else {
                    "bounded legacy scan"
                },
                frame.manager,
                frame.entities.len(),
                self.managers.candidates.len(),
                if self.regions.is_empty() {
                    0
                } else {
                    self.region_index + 1
                },
                self.regions.len(),
                self.region_offset,
            ),
            pid: Some(process.pid),
            base: Some(process.image_base),
            cr3: Some(process.cr3),
            frame: Some(frame),
            features: self.feature_state(),
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Err(error) = self.restore_features() {
            eprintln!("Restore PointBlank features: {error:#}");
        }
    }
}

pub fn target_image_path(pid: u32) -> Result<std::path::PathBuf> {
    native::image_path(pid)
}

pub fn verify_image(path: &Path) -> Result<pointblank::BuildProfile> {
    let file = fs::File::open(path)
        .with_context(|| format!("Open PointBlank build {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= 128 * 1024 * 1024,
        "PointBlank executable is outside its size limit"
    );
    let mut bytes = Vec::new();
    file.take(128 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 128 * 1024 * 1024,
        "PointBlank executable exceeds its size limit"
    );
    profile_for_hash(&format!("{:x}", Sha256::digest(&bytes)))
}

fn profile_for_hash(hash: &str) -> Result<pointblank::BuildProfile> {
    [pointblank::LEGACY_PROFILE, pointblank::CURRENT_PROFILE]
        .into_iter()
        .find(|profile| profile.sha256 == hash)
        .context("PointBlank executable SHA-256 is outside the verified build profiles")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_known_pointblank_hashes_select_profiles() {
        assert_eq!(
            profile_for_hash(pointblank::LEGACY_PROFILE.sha256)
                .unwrap()
                .id,
            pointblank::LEGACY_PROFILE.id
        );
        assert_eq!(
            profile_for_hash(pointblank::CURRENT_PROFILE.sha256)
                .unwrap()
                .id,
            pointblank::CURRENT_PROFILE.id
        );
        assert!(profile_for_hash(&"0".repeat(64)).is_err());
        assert!(profile_for_hash("").is_err());
    }

    #[test]
    fn wall_clock_cadence_ignores_fast_poll_counts() {
        let start = Instant::now();
        let mut cadence = Cadence::new(MANAGER_SCAN_INTERVAL);
        assert!(cadence.take(start));
        for _ in 0..1000 {
            assert!(!cadence.take(start));
        }
        for tick in 1..=12 {
            assert!(!cadence.take(start + Duration::from_millis(tick * 8)));
        }
        assert!(!cadence.take(start + Duration::from_millis(99)));
        assert!(cadence.take(start + Duration::from_millis(100)));
    }

    #[test]
    fn retry_deadline_includes_completed_work_and_resets_explicitly() {
        let start = Instant::now();
        let mut cadence = Cadence::new(ATTACH_RETRY_INTERVAL);
        assert!(cadence.take(start));
        let completed = start + Duration::from_secs(1);
        cadence.defer(completed);
        assert!(!cadence.take(completed + Duration::from_millis(199)));
        assert!(cadence.take(completed + ATTACH_RETRY_INTERVAL));
        cadence.reset();
        assert!(cadence.take(completed + ATTACH_RETRY_INTERVAL));
    }

    #[test]
    fn cadence_skips_missed_intervals_without_catch_up_bursts() {
        let start = Instant::now();
        let mut cadence = Cadence::new(REGION_REFRESH_INTERVAL);
        assert!(cadence.take(start));
        let much_later = start + Duration::from_secs(60);
        assert!(cadence.take(much_later));
        assert!(!cadence.take(much_later));
        assert!(!cadence.take(much_later + Duration::from_secs(4)));
        assert!(cadence.take(much_later + REGION_REFRESH_INTERVAL));
    }

    fn candidate(address: u32, count: usize) -> pointblank::ManagerCandidate {
        pointblank::ManagerCandidate { address, count }
    }

    #[test]
    fn cached_empty_manager_populates_on_its_own_probe_cadence() {
        let mut cache = ManagerCache::default();
        cache.remember(candidate(0x10000, 0));
        cache.remember(candidate(0x20000, 0));
        assert_eq!(cache.active, None);
        let start = Instant::now();
        let mut scan = Cadence::new(MANAGER_SCAN_INTERVAL);
        let mut probe = Cadence::new(MANAGER_PROBE_INTERVAL);
        assert!(scan.take(start));
        assert!(probe.take(start));
        assert!(!scan.take(start + MANAGER_PROBE_INTERVAL));
        assert!(probe.take(start + MANAGER_PROBE_INTERVAL));
        let mut probes = 0;
        cache.reprobe(|address| {
            probes += 1;
            Ok(candidate(address, if address == 0x10000 { 12 } else { 0 }))
        });
        assert_eq!(probes, 2);
        assert_eq!(cache.active, Some(0x10000));
        assert_eq!(cache.candidates.len(), 2);
    }

    #[test]
    fn cache_prunes_overwritten_addresses_and_clears_ended_matches() {
        let mut cache = ManagerCache::default();
        cache.remember(candidate(0x10000, 12));
        cache.remember(candidate(0x20000, 1));
        cache.remember(candidate(0x30000, 0));
        assert_eq!(cache.active, Some(0x10000));
        cache.reprobe(|address| {
            if address == 0x10000 {
                anyhow::bail!("manager was overwritten")
            }
            Ok(candidate(address, if address == 0x20000 { 8 } else { 0 }))
        });
        assert_eq!(cache.active, Some(0x20000));
        assert_eq!(cache.candidates.len(), 2);
        cache.reprobe(|address| Ok(candidate(address, 0)));
        assert_eq!(cache.active, None);
        assert_eq!(cache.candidates.len(), 2);
        cache.invalidate(0x20000);
        assert_eq!(cache.candidates.len(), 1);
        assert_eq!(cache.active, None);
    }

    #[test]
    fn cache_is_bounded_deduplicated_and_prefers_largest_stable_population() {
        let mut cache = ManagerCache::default();
        for index in 0..MAX_CACHED_MANAGERS + 20 {
            cache.remember(candidate(0x10000 + index as u32 * 4, 0));
        }
        assert_eq!(cache.candidates.len(), MAX_CACHED_MANAGERS);
        cache.remember(candidate(0x30000, 12));
        assert_eq!(cache.active, Some(0x30000));
        cache.remember(candidate(0x30000, 12));
        cache.remember(candidate(0x40000, 12));
        assert_eq!(cache.active, Some(0x30000));
        cache.remember(candidate(0x50000, 14));
        assert_eq!(cache.active, Some(0x50000));
        assert_eq!(cache.candidates.len(), MAX_CACHED_MANAGERS);
        let mut probes = 0;
        cache.reprobe(|address| {
            probes += 1;
            Ok(candidate(address, 0))
        });
        assert_eq!(probes, MAX_CACHED_MANAGERS);
        assert_eq!(cache.active, None);
    }

    #[test]
    fn cache_rejects_probe_results_for_another_address() {
        let mut cache = ManagerCache::default();
        cache.remember(candidate(0x10000, 12));
        cache.reprobe(|_| Ok(candidate(0x20000, 12)));
        assert!(cache.candidates.is_empty());
        assert_eq!(cache.active, None);
    }

    #[test]
    fn engine_remains_send_for_the_capture_worker() {
        fn assert_send<T: Send>() {}
        assert_send::<Engine>();
    }
}

#[cfg(windows)]
mod native {
    use anyhow::{Context, Result, ensure};
    use std::{
        ffi::OsString,
        mem::size_of,
        os::windows::{
            ffi::OsStringExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
        path::PathBuf,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::{
            Memory::{
                MEM_COMMIT, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS,
                VirtualQueryEx,
            },
            Threading::{
                GetProcessId, GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION,
                PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW,
                WaitForSingleObject,
            },
        },
    };

    /// OwnedHandle supplies CloseHandle and Send for the engine's worker handoff.
    /// Query/synchronize access binds the original process object without VM rights.
    pub struct ProcessLifetime {
        handle: OwnedHandle,
        pid: u32,
        creation_time: u64,
    }

    impl ProcessLifetime {
        pub fn open(pid: u32) -> Result<Self> {
            let raw = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    pid,
                )
            };
            ensure!(
                !raw.is_null(),
                "Open PointBlank process lifetime: {}",
                std::io::Error::last_os_error()
            );
            // SAFETY: OpenProcess returned a valid, uniquely owned handle above.
            let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
            let actual_pid = unsafe { GetProcessId(handle.as_raw_handle()) };
            ensure!(
                actual_pid == pid,
                "PointBlank lifetime PID differs from the selected process"
            );
            let mut creation = FILETIME::default();
            let mut exit = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            ensure!(
                unsafe {
                    GetProcessTimes(
                        handle.as_raw_handle(),
                        &mut creation,
                        &mut exit,
                        &mut kernel,
                        &mut user,
                    )
                } != 0,
                "Read PointBlank process creation time: {}",
                std::io::Error::last_os_error()
            );
            let creation_time =
                (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
            ensure!(
                creation_time != 0,
                "PointBlank process creation time is empty"
            );
            let lifetime = Self {
                handle,
                pid: actual_pid,
                creation_time,
            };
            ensure!(lifetime.alive()?, "PointBlank process has exited");
            Ok(lifetime)
        }

        pub fn pid(&self) -> u32 {
            self.pid
        }

        pub fn creation_time(&self) -> u64 {
            self.creation_time
        }

        pub fn alive(&self) -> Result<bool> {
            // A process handle becomes permanently signaled on exit. PID reuse
            // therefore never turns this original process object's wait live again.
            match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) } {
                WAIT_TIMEOUT => Ok(true),
                WAIT_OBJECT_0 => Ok(false),
                _ => anyhow::bail!(
                    "Check PointBlank process lifetime: {}",
                    std::io::Error::last_os_error()
                ),
            }
        }

        pub fn image_path(&self) -> Result<PathBuf> {
            image_path_for_handle(self.handle.as_raw_handle())
        }
    }

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub fn game_pid(names: &[&str]) -> Result<Option<u32>> {
        use std::mem::size_of;
        use windows_sys::Win32::{
            Foundation::{HWND, INVALID_HANDLE_VALUE, LPARAM},
            System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            UI::WindowsAndMessaging::{
                EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible,
            },
        };
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        ensure!(
            raw != INVALID_HANDLE_VALUE,
            "List game processes: {}",
            std::io::Error::last_os_error()
        );
        let snapshot = Handle(raw);
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut candidates = Vec::new();
        let mut available = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while available {
            let end = entry
                .szExeFile
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if names.iter().any(|wanted| name.eq_ignore_ascii_case(wanted)) {
                candidates.push(entry.th32ProcessID);
            }
            available = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        struct Find<'a> {
            candidates: &'a [u32],
            pid: Option<u32>,
        }
        unsafe extern "system" fn visible(window: HWND, param: LPARAM) -> i32 {
            let find = unsafe { &mut *(param as *mut Find<'_>) };
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(window, &mut pid);
            }
            if find.candidates.contains(&pid)
                && unsafe { IsWindowVisible(window) } != 0
                && unsafe { GetWindow(window, GW_OWNER) }.is_null()
            {
                find.pid = Some(pid);
                return 0;
            }
            1
        }
        let mut find = Find {
            candidates: &candidates,
            pid: None,
        };
        unsafe {
            EnumWindows(Some(visible), (&mut find as *mut Find<'_>) as LPARAM);
        }
        Ok(find.pid.or_else(|| candidates.first().copied()))
    }
    pub fn image_path(pid: u32) -> Result<PathBuf> {
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        ensure!(
            !raw.is_null(),
            "Query PointBlank process path: {}",
            std::io::Error::last_os_error()
        );
        let handle = Handle(raw);
        image_path_for_handle(handle.0)
    }
    fn image_path_for_handle(handle: HANDLE) -> Result<PathBuf> {
        let mut data = vec![0u16; 32768];
        let mut size = data.len() as u32;
        ensure!(
            unsafe { QueryFullProcessImageNameW(handle, 0, data.as_mut_ptr(), &mut size) } != 0,
            "Read PointBlank image path: {}",
            std::io::Error::last_os_error()
        );
        Ok(PathBuf::from(OsString::from_wide(&data[..size as usize])))
    }
    pub fn regions(pid: u32) -> Result<Vec<(u64, usize)>> {
        let raw = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid) };
        ensure!(
            !raw.is_null(),
            "Query PointBlank memory regions: {}",
            std::io::Error::last_os_error()
        );
        let handle = Handle(raw);
        let mut address = 0x10000u64;
        let mut regions = Vec::new();
        for _ in 0..65536 {
            if address >= 0x1_0000_0000 {
                break;
            }
            let mut info = MEMORY_BASIC_INFORMATION::default();
            let count = unsafe {
                VirtualQueryEx(
                    handle.0,
                    address as *const _,
                    &mut info,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if count == 0 {
                break;
            }
            ensure!(
                count >= size_of::<MEMORY_BASIC_INFORMATION>(),
                "Truncated PointBlank region metadata"
            );
            let base = info.BaseAddress as u64;
            let length = info.RegionSize;
            let next = base
                .checked_add(length as u64)
                .context("PointBlank region overflow")?;
            ensure!(
                next > address,
                "PointBlank memory-region walk stopped advancing"
            );
            if info.State == MEM_COMMIT
                && info.Type == MEM_PRIVATE
                && info.Protect & (PAGE_GUARD | PAGE_NOACCESS) == 0
                && matches!(info.Protect & 0xff, 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80)
            {
                let length = (length as u64).min(0x1_0000_0000 - base) as usize;
                if length > 0 {
                    regions.push((base, length));
                }
            }
            address = next;
        }
        Ok(regions)
    }
}

#[cfg(not(windows))]
mod native {
    use anyhow::Result;
    use std::path::PathBuf;

    pub struct ProcessLifetime;
    impl ProcessLifetime {
        pub fn open(_pid: u32) -> Result<Self> {
            anyhow::bail!("PointBlank process lifetime requires Windows")
        }
        pub fn pid(&self) -> u32 {
            0
        }
        pub fn creation_time(&self) -> u64 {
            0
        }
        pub fn alive(&self) -> Result<bool> {
            anyhow::bail!("PointBlank process lifetime requires Windows")
        }
        pub fn image_path(&self) -> Result<PathBuf> {
            anyhow::bail!("PointBlank process discovery requires Windows")
        }
    }

    pub fn game_pid(_names: &[&str]) -> Result<Option<u32>> {
        anyhow::bail!("PointBlank discovery requires Windows")
    }
    pub fn image_path(_pid: u32) -> Result<PathBuf> {
        anyhow::bail!("PointBlank process discovery requires Windows")
    }
    pub fn regions(_pid: u32) -> Result<Vec<(u64, usize)>> {
        anyhow::bail!("PointBlank region discovery requires Windows")
    }
}
