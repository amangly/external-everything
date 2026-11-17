//! Validated embedded payloads and operation-owned temporary staging.
//!
//! Build inputs are selected only by EXTERNAL_EVERYTHING_BUNDLE_DIR. Checking
//! metadata never extracts assets. Staging owns a fresh TempDir, with no global
//! cache: callers retain the game stage until its owned child has terminated.
use anyhow::{Context, Result, ensure};
use hyperrev_loader::files::reject_links;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};
use tempfile::TempDir;

const MAX_IMAGE: u64 = 64 * 1024 * 1024;
const MAX_CATALOG: u64 = 1024 * 1024;
const EFI: &str = "uefi-boot-standalone.efi";
const DLL: &str = "hyperv-attachment.dll";
const RUNTIME: &str = "games/pointblank/external-runtime.exe";
const CATALOG: &str = "games/catalog.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Efi,
    Dll,
    Runtime,
    Catalog,
}

#[derive(Clone, Copy)]
struct Asset<'a> {
    name: &'static str,
    bytes: &'a [u8],
    size: u64,
    sha256: &'a str,
    kind: Kind,
}

include!(concat!(env!("OUT_DIR"), "/embedded_bundle.rs"));

#[derive(Debug, Serialize)]
pub struct AssetReport {
    pub name: &'static str,
    pub size: u64,
    pub sha256: String,
    pub verified: bool,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub single_file: bool,
    pub assets: Vec<AssetReport>,
}

pub fn available() -> bool {
    EMBEDDED.is_some()
}

pub fn catalog_bytes() -> Option<&'static [u8]> {
    EMBEDDED.as_ref().map(|assets| assets[3].bytes)
}

/// Verify every embedded digest, PE role, and catalog binding without writes.
pub fn inspect() -> Result<Report> {
    let Some(assets) = EMBEDDED.as_ref() else {
        return Ok(Report {
            single_file: false,
            assets: Vec::new(),
        });
    };
    verify_bundle(assets)?;
    Ok(Report {
        single_file: true,
        assets: assets
            .iter()
            .map(|asset| AssetReport {
                name: asset.name,
                size: asset.size,
                sha256: asset.sha256.to_owned(),
                verified: true,
            })
            .collect(),
    })
}

#[derive(Clone, Copy, Debug)]
enum Scope {
    Boot,
    Game,
}

#[derive(Debug)]
struct Expected {
    name: &'static str,
    size: u64,
    sha256: String,
    kind: Kind,
}

/// A single operation's staging owner. Deliberately neither Clone nor static.
/// The root is an implementation detail used for vendor source/catalog APIs.
pub struct Staged {
    directory: Option<TempDir>,
    root: PathBuf,
    created: Option<SystemTime>,
    expected: Vec<Expected>,
    scope: Scope,
}

impl std::fmt::Debug for Staged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Staged")
            .field("scope", &self.scope)
            .field("assets", &self.expected.len())
            .finish_non_exhaustive()
    }
}

impl Staged {
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn verify(&self) -> Result<()> {
        self.verify_root()?;
        for expected in &self.expected {
            let path = self.root.join(expected.name);
            let bytes = read_bounded(&path, maximum(expected.kind))?;
            verify_asset(&Asset {
                name: expected.name,
                bytes: &bytes,
                size: expected.size,
                sha256: &expected.sha256,
                kind: expected.kind,
            })?;
            if expected.kind == Kind::Catalog {
                let runtime = self
                    .expected
                    .iter()
                    .find(|asset| asset.kind == Kind::Runtime)
                    .context("Staged runtime identity is absent")?;
                validate_catalog(&bytes, &runtime.sha256)?;
            }
        }
        reject_tree_links(&self.root, 0, &mut 0)?;
        Ok(())
    }

    fn verify_root(&self) -> Result<()> {
        reject_links(&self.root)?;
        ensure!(self.root.is_dir(), "Staging root is absent");
        ensure!(
            self.root.canonicalize()? == self.root,
            "Staging root identity changed"
        );
        if let Some(created) = self.created {
            ensure!(
                fs::symlink_metadata(&self.root)?.created()? == created,
                "Staging directory ownership changed"
            );
        }
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let Some(directory) = self.directory.take() else {
            return;
        };
        // Cleanup is limited to the exact private root created by this owner.
        // Fail closed when any ancestor/tree entry has become a link, rather
        // than allowing recursive deletion through an externally replaced path.
        let safe = self
            .verify_root()
            .and_then(|()| reject_tree_links(&self.root, 0, &mut 0));
        if safe.is_ok() {
            let _ = directory.close();
        } else {
            let _ = directory.keep();
        }
    }
}

pub fn stage_boot() -> Result<Option<Staged>> {
    let Some(assets) = EMBEDDED.as_ref() else {
        return Ok(None);
    };
    stage(assets, Scope::Boot, &std::env::temp_dir()).map(Some)
}

pub fn stage_game(game_id: &str) -> Result<Option<Staged>> {
    let Some(assets) = EMBEDDED.as_ref() else {
        return Ok(None);
    };
    ensure!(game_id == "pointblank", "Game has no embedded runtime");
    stage(assets, Scope::Game, &std::env::temp_dir()).map(Some)
}

fn stage(assets: &[Asset<'_>; 4], scope: Scope, parent: &Path) -> Result<Staged> {
    // Validate the entire source bundle before creating a directory, including
    // the catalog/runtime binding even for a boot-only operation.
    verify_bundle(assets)?;
    reject_links(parent)?;
    let parent = parent.canonicalize()?;
    reject_links(&parent)?;
    let executable = std::env::current_exe()?;
    let executable_directory = executable
        .parent()
        .context("Application directory is absent")?
        .canonicalize()?;
    ensure!(
        !parent.starts_with(&executable_directory),
        "Temporary staging requires a directory outside the application directory"
    );
    let mut directory = tempfile::Builder::new()
        .prefix("ExternalEverything-")
        .rand_bytes(16)
        .tempdir_in(&parent)
        .context("Create private payload staging")?;
    // Disable TempDir's unconditional destructor first. Our Staged destructor
    // validates the exact root and every ancestor before explicit close().
    directory.disable_cleanup(true);
    let root = directory.path().to_path_buf();
    let mut staged = Staged {
        directory: Some(directory),
        root,
        created: None,
        expected: assets
            .iter()
            .filter(|asset| in_scope(asset.kind, scope))
            .map(|asset| Expected {
                name: asset.name,
                size: asset.size,
                sha256: asset.sha256.to_owned(),
                kind: asset.kind,
            })
            .collect(),
        scope,
    };
    staged.verify_root()?;
    staged.created = fs::symlink_metadata(&staged.root)?.created().ok();
    if matches!(scope, Scope::Game) {
        for relative in ["games", "games/pointblank"] {
            let destination = staged.root.join(relative);
            reject_links(&destination)?;
            fs::create_dir(&destination)?;
            reject_links(&destination)?;
        }
    }
    for asset in assets.iter().filter(|asset| in_scope(asset.kind, scope)) {
        let path = staged.root.join(asset.name);
        reject_links(&path)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("Stage {}", asset.name))?;
        file.write_all(asset.bytes)?;
        file.sync_all()?;
        drop(file);
        reject_links(&path)?;
        let readback = read_bounded(&path, maximum(asset.kind))?;
        verify_asset(&Asset {
            bytes: &readback,
            ..*asset
        })?;
    }
    staged.verify()?;
    Ok(staged)
}

fn in_scope(kind: Kind, scope: Scope) -> bool {
    matches!(
        (kind, scope),
        (Kind::Efi | Kind::Dll, Scope::Boot) | (Kind::Runtime | Kind::Catalog, Scope::Game)
    )
}

fn maximum(kind: Kind) -> u64 {
    if kind == Kind::Catalog {
        MAX_CATALOG
    } else {
        MAX_IMAGE
    }
}

fn verify_bundle(assets: &[Asset<'_>; 4]) -> Result<()> {
    for (asset, (name, kind)) in assets.iter().zip([
        (EFI, Kind::Efi),
        (DLL, Kind::Dll),
        (RUNTIME, Kind::Runtime),
        (CATALOG, Kind::Catalog),
    ]) {
        ensure!(
            asset.name == name && asset.kind == kind,
            "Unexpected embedded asset role"
        );
        verify_asset(asset)?;
    }
    validate_catalog(assets[3].bytes, assets[2].sha256)
}

fn verify_asset(asset: &Asset<'_>) -> Result<()> {
    ensure!(
        !asset.bytes.is_empty()
            && asset.bytes.len() as u64 <= maximum(asset.kind)
            && asset.bytes.len() as u64 == asset.size,
        "Embedded asset size differs: {}",
        asset.name
    );
    ensure!(
        asset.sha256.len() == 64
            && asset
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && format!("{:x}", Sha256::digest(asset.bytes)) == asset.sha256,
        "Embedded asset SHA-256 differs: {}",
        asset.name
    );
    if asset.kind != Kind::Catalog {
        validate_pe(asset.bytes, asset.kind).with_context(|| format!("Validate {}", asset.name))?;
    }
    Ok(())
}

fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    reject_links(path)?;
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() > 0 && metadata.len() <= maximum,
        "Staged asset exceeds its size limit"
    );
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == metadata.len() && bytes.len() as u64 <= maximum,
        "Staged asset changed while reading"
    );
    reject_links(path)?;
    Ok(bytes)
}

fn reject_tree_links(path: &Path, depth: usize, count: &mut usize) -> Result<()> {
    ensure!(depth <= 4 && *count <= 16, "Unexpected staging tree growth");
    reject_links(path)?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        *count += 1;
        ensure!(*count <= 16, "Unexpected staging tree growth");
        let path = entry.path();
        reject_links(&path)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            reject_tree_links(&path, depth + 1, count)?;
        } else {
            ensure!(metadata.is_file(), "Unexpected staging entry type");
        }
    }
    Ok(())
}

// Keep the bounded role checks in sync with build.rs.
fn validate_pe(bytes: &[u8], kind: Kind) -> Result<()> {
    ensure!(
        bytes.len() >= 64 && bytes.len() as u64 <= MAX_IMAGE && &bytes[..2] == b"MZ",
        "Expected a bounded PE image"
    );
    let pe = u32::from_le_bytes(bytes[60..64].try_into()?) as usize;
    let optional = pe.checked_add(24).context("PE offset overflow")?;
    ensure!(
        optional <= bytes.len() && &bytes[pe..pe + 4] == b"PE\0\0",
        "Invalid PE signature"
    );
    ensure!(
        u16::from_le_bytes(bytes[pe + 4..pe + 6].try_into()?) == 0x8664,
        "Expected an x64 PE image"
    );
    let size = u16::from_le_bytes(bytes[pe + 20..pe + 22].try_into()?) as usize;
    ensure!(
        size >= 112
            && optional
                .checked_add(size)
                .is_some_and(|end| end <= bytes.len())
            && u16::from_le_bytes(bytes[optional..optional + 2].try_into()?) == 0x20b,
        "Expected complete PE32+ optional headers"
    );
    let flags = u16::from_le_bytes(bytes[pe + 22..pe + 24].try_into()?);
    let subsystem = u16::from_le_bytes(bytes[optional + 68..optional + 70].try_into()?);
    ensure!(flags & 2 != 0, "PE image lacks the executable flag");
    ensure!(
        match kind {
            Kind::Efi => subsystem == 10,
            Kind::Dll => flags & 0x2000 != 0,
            Kind::Runtime => flags & 0x2000 == 0 && matches!(subsystem, 2 | 3),
            Kind::Catalog => false,
        },
        "PE image type differs from its bundle role"
    );
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema: u32,
    games: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    name: String,
    executable: Option<String>,
    sha256: Option<String>,
}

fn validate_catalog(bytes: &[u8], runtime_hash: &str) -> Result<()> {
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= MAX_CATALOG,
        "Catalog exceeds its bounded size"
    );
    let catalog: Catalog = serde_json::from_slice(bytes).context("Read embedded catalog")?;
    ensure!(
        catalog.schema == 1 && !catalog.games.is_empty() && catalog.games.len() <= 128,
        "Unsupported bundle catalog"
    );
    let mut ids = BTreeSet::new();
    let mut pointblank = false;
    for entry in catalog.games {
        ensure!(
            !entry.id.is_empty()
                && entry.id.len() <= 64
                && entry
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && ids.insert(entry.id.clone()),
            "Invalid or duplicate catalog identifier"
        );
        ensure!(
            !entry.name.trim().is_empty()
                && entry.name.len() <= 128
                && !entry.name.chars().any(char::is_control),
            "Invalid catalog name"
        );
        if entry.id == "pointblank" {
            ensure!(
                entry.executable.as_deref() == Some("pointblank/external-runtime.exe")
                    && entry.sha256.as_deref() == Some(runtime_hash),
                "PointBlank catalog path/hash differs from its embedded runtime"
            );
            pointblank = true;
        } else {
            ensure!(
                entry.executable.is_none() && entry.sha256.is_none(),
                "Only the packaged PointBlank runtime is supported"
            );
        }
    }
    ensure!(pointblank, "Packaged PointBlank entry is absent");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        bytes: [Vec<u8>; 4],
        hashes: [String; 4],
    }

    impl Fixture {
        fn new() -> Self {
            let runtime = image(Kind::Runtime);
            let runtime_hash = format!("{:x}", Sha256::digest(&runtime));
            let catalog = serde_json::to_vec(&serde_json::json!({
                "schema": 1,
                "games": [{"id":"pointblank", "name":"PointBlank",
                    "executable":"pointblank/external-runtime.exe", "sha256":runtime_hash}]
            }))
            .unwrap();
            let bytes = [image(Kind::Efi), image(Kind::Dll), runtime, catalog];
            let hashes =
                std::array::from_fn(|index| format!("{:x}", Sha256::digest(&bytes[index])));
            Self { bytes, hashes }
        }

        fn assets(&self) -> [Asset<'_>; 4] {
            let names = [EFI, DLL, RUNTIME, CATALOG];
            let kinds = [Kind::Efi, Kind::Dll, Kind::Runtime, Kind::Catalog];
            std::array::from_fn(|index| Asset {
                name: names[index],
                bytes: &self.bytes[index],
                size: self.bytes[index].len() as u64,
                sha256: &self.hashes[index],
                kind: kinds[index],
            })
        }
    }

    fn image(kind: Kind) -> Vec<u8> {
        let mut bytes = vec![0; 256];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&0x8664_u16.to_le_bytes());
        bytes[84..86].copy_from_slice(&112_u16.to_le_bytes());
        let flags = 2_u16 | if kind == Kind::Dll { 0x2000 } else { 0 };
        bytes[86..88].copy_from_slice(&flags.to_le_bytes());
        bytes[88..90].copy_from_slice(&0x20b_u16.to_le_bytes());
        let subsystem = if kind == Kind::Efi { 10_u16 } else { 3_u16 };
        bytes[156..158].copy_from_slice(&subsystem.to_le_bytes());
        bytes
    }

    #[test]
    fn owned_boot_stage_contains_only_boot_assets_and_drops_cleanly() {
        let fixture = Fixture::new();
        let parent = tempfile::tempdir().unwrap();
        let staged = stage(&fixture.assets(), Scope::Boot, parent.path()).unwrap();
        let root = staged.root().to_owned();
        assert!(root.join(EFI).is_file());
        assert!(root.join(DLL).is_file());
        assert!(!root.join("games").exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        staged.verify().unwrap();
        drop(staged);
        assert!(!root.exists());
        assert!(parent.path().is_dir());
    }

    #[test]
    fn owned_game_stage_has_separate_roots_and_cleans_dirty_owned_files() {
        let fixture = Fixture::new();
        let parent = tempfile::tempdir().unwrap();
        let first = stage(&fixture.assets(), Scope::Game, parent.path()).unwrap();
        let second = stage(&fixture.assets(), Scope::Game, parent.path()).unwrap();
        let root = first.root().to_owned();
        assert_ne!(root, second.root());
        assert!(root.join(CATALOG).is_file());
        assert!(root.join(RUNTIME).is_file());
        assert!(!root.join(EFI).exists());
        assert!(!root.join(DLL).exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        first.verify().unwrap();
        fs::write(root.join("operation-owned-note"), b"temporary").unwrap();
        drop(first);
        assert!(!root.exists());
        assert!(second.root().is_dir());
        let second_root = second.root().to_owned();
        drop(second);
        assert!(!second_root.exists());
    }

    #[test]
    fn touched_staged_payload_is_rejected_and_still_cleaned() {
        let fixture = Fixture::new();
        let parent = tempfile::tempdir().unwrap();
        let staged = stage(&fixture.assets(), Scope::Game, parent.path()).unwrap();
        let root = staged.root().to_owned();
        fs::write(root.join(RUNTIME), image(Kind::Dll)).unwrap();
        assert!(staged.verify().is_err());
        drop(staged);
        assert!(!root.exists());
    }

    #[test]
    fn wrong_hash_size_or_role_never_creates_a_stage() {
        let fixture = Fixture::new();
        let parent = tempfile::tempdir().unwrap();
        for mode in 0..3 {
            let mut assets = fixture.assets();
            match mode {
                0 => assets[0].sha256 = assets[1].sha256,
                1 => assets[0].size += 1,
                _ => assets[0].name = "../outside.efi",
            }
            assert!(stage(&assets, Scope::Boot, parent.path()).is_err());
            assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn bounded_pe_checks_reject_truncation_flags_architecture_and_wrong_role() {
        for kind in [Kind::Efi, Kind::Dll, Kind::Runtime] {
            let valid = image(kind);
            validate_pe(&valid, kind).unwrap();
            for length in [0, 63, 64, 87, 199] {
                assert!(validate_pe(&valid[..length], kind).is_err());
            }
            for (offset, bytes) in [
                (60, u32::MAX.to_le_bytes().to_vec()),
                (68, 0x14c_u16.to_le_bytes().to_vec()),
                (84, 1_u16.to_le_bytes().to_vec()),
                (86, 0_u16.to_le_bytes().to_vec()),
                (88, 0x10b_u16.to_le_bytes().to_vec()),
            ] {
                let mut invalid = valid.clone();
                invalid[offset..offset + bytes.len()].copy_from_slice(&bytes);
                assert!(validate_pe(&invalid, kind).is_err());
            }
        }
        assert!(validate_pe(&image(Kind::Dll), Kind::Runtime).is_err());
        assert!(validate_pe(&image(Kind::Runtime), Kind::Efi).is_err());
        assert!(validate_pe(&image(Kind::Efi), Kind::Dll).is_err());
        assert!(validate_pe(&image(Kind::Runtime), Kind::Catalog).is_err());
    }

    #[test]
    fn standalone_efi_accepts_the_pinned_vendors_linker_flags() {
        let mut efi = image(Kind::Efi);
        // The pinned native EFI uses PE flags 0x2022, including IMAGE_FILE_DLL.
        // Its firmware role is EFI_APPLICATION (subsystem 10), as in the vendor.
        efi[86..88].copy_from_slice(&0x2022_u16.to_le_bytes());
        validate_pe(&efi, Kind::Efi).unwrap();
        assert!(validate_pe(&efi, Kind::Runtime).is_err());
    }

    #[test]
    fn strict_catalog_rejects_schema_identity_hash_and_path_substitutions() {
        let fixture = Fixture::new();
        for mutation in [
            "schema",
            "path",
            "hash",
            "duplicate",
            "unknown",
            "future-runtime",
        ] {
            let mut catalog: serde_json::Value = serde_json::from_slice(&fixture.bytes[3]).unwrap();
            match mutation {
                "schema" => catalog["schema"] = 2.into(),
                "path" => catalog["games"][0]["executable"] = "../outside.exe".into(),
                "hash" => catalog["games"][0]["sha256"] = "f".repeat(64).into(),
                "duplicate" => {
                    let duplicate = catalog["games"][0].clone();
                    catalog["games"].as_array_mut().unwrap().push(duplicate);
                }
                "unknown" => catalog["unexpected"] = true.into(),
                _ => catalog["games"][0]["id"] = "another-game".into(),
            }
            assert!(
                validate_catalog(&serde_json::to_vec(&catalog).unwrap(), &fixture.hashes[2])
                    .is_err(),
                "{mutation}"
            );
        }
        let mut catalog: serde_json::Value = serde_json::from_slice(&fixture.bytes[3]).unwrap();
        catalog["games"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"id":"future-game", "name":"Future Game"}));
        validate_catalog(&serde_json::to_vec(&catalog).unwrap(), &fixture.hashes[2]).unwrap();
    }

    #[test]
    fn catalog_hash_binding_is_checked_even_for_boot_staging() {
        let mut fixture = Fixture::new();
        fixture.bytes[2][255] = 1;
        fixture.hashes[2] = format!("{:x}", Sha256::digest(&fixture.bytes[2]));
        let parent = tempfile::tempdir().unwrap();
        assert!(stage(&fixture.assets(), Scope::Boot, parent.path()).is_err());
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
    }

    #[test]
    fn traversal_parent_is_rejected_before_extraction() {
        let fixture = Fixture::new();
        let parent = tempfile::tempdir().unwrap();
        let invalid = parent.path().join("..").join("external-everything-unowned");
        assert!(stage(&fixture.assets(), Scope::Boot, &invalid).is_err());
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
    }

    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) -> std::io::Result<()> {
        use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};
        // Directory junctions use the mount-point tag and require directory
        // ownership, rather than the machine's symlink privilege or cmd.exe.
        #[link(name = "kernel32")]
        unsafe extern "system" {
            #[link_name = "CreateFileW"]
            fn create_file(
                path: *const u16,
                access: u32,
                share: u32,
                security: *mut c_void,
                disposition: u32,
                flags: u32,
                template: *mut c_void,
            ) -> *mut c_void;
            #[link_name = "DeviceIoControl"]
            fn device_control(
                handle: *mut c_void,
                code: u32,
                input: *const c_void,
                input_size: u32,
                output: *mut c_void,
                output_size: u32,
                returned: *mut u32,
                overlapped: *mut c_void,
            ) -> i32;
            #[link_name = "CloseHandle"]
            fn close_handle(handle: *mut c_void) -> i32;
        }
        let target = target.canonicalize()?;
        reject_links(&target).map_err(std::io::Error::other)?;
        let printable = target.to_string_lossy();
        let printable = printable.strip_prefix(r"\\?\").unwrap_or(&printable);
        let substitute: Vec<u16> = format!(r"\??\{printable}").encode_utf16().collect();
        let print: Vec<u16> = printable.encode_utf16().collect();
        let substitute_bytes = (substitute.len() * 2) as u16;
        let print_bytes = (print.len() * 2) as u16;
        let data_length = 8 + substitute_bytes + 2 + print_bytes + 2;
        let mut data = Vec::new();
        data.extend_from_slice(&0xa0000003_u32.to_le_bytes());
        for word in [
            data_length,
            0,
            0,
            substitute_bytes,
            substitute_bytes + 2,
            print_bytes,
        ] {
            data.extend_from_slice(&word.to_le_bytes());
        }
        for word in substitute.into_iter().chain([0]).chain(print).chain([0]) {
            data.extend_from_slice(&word.to_le_bytes());
        }
        fs::create_dir(link)?;
        let path: Vec<u16> = link.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: paths/buffers remain live for synchronous Win32 operations;
        // the single directory handle is closed on both success and failure.
        unsafe {
            let handle = create_file(
                path.as_ptr(),
                0x40000000,
                7,
                ptr::null_mut(),
                3,
                0x02200000,
                ptr::null_mut(),
            );
            if handle == -1_isize as *mut c_void {
                return Err(std::io::Error::last_os_error());
            }
            let mut returned = 0;
            let success = device_control(
                handle,
                0x900a4,
                data.as_ptr().cast(),
                data.len() as u32,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            );
            let result = if success == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            };
            close_handle(handle);
            result
        }
    }

    #[cfg(windows)]
    #[test]
    fn reparse_points_neither_admit_staging_nor_redirect_cleanup() {
        let fixture = Fixture::new();
        let parent = tempfile::tempdir().unwrap();
        let outside = parent.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let marker = outside.join("keep-user-content");
        fs::write(&marker, b"preserve").unwrap();
        let alias = parent.path().join("alias");
        junction(&alias, &outside).unwrap();
        assert!(stage(&fixture.assets(), Scope::Boot, &alias).is_err());
        let staged = stage(&fixture.assets(), Scope::Boot, parent.path()).unwrap();
        let root = staged.root().to_owned();
        let redirect = root.join("redirect");
        junction(&redirect, &outside).unwrap();
        assert!(staged.verify().is_err());
        drop(staged);
        assert_eq!(fs::read(&marker).unwrap(), b"preserve");
        // A changed tree fails closed. Remove only the two fixture junction
        // directory entries after validating their exact generated locations.
        assert_eq!(redirect.parent().unwrap(), root);
        assert_eq!(alias.parent().unwrap(), parent.path());
        {
            use std::os::windows::fs::MetadataExt;
            assert_ne!(
                fs::symlink_metadata(&redirect).unwrap().file_attributes() & 0x400,
                0
            );
        }
        fs::remove_dir(&redirect).unwrap();
        fs::remove_dir(&alias).unwrap();
        reject_links(&root).unwrap();
        assert_eq!(root.canonicalize().unwrap(), root);
        assert!(root.starts_with(parent.path().canonicalize().unwrap()));
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(fs::read(&marker).unwrap(), b"preserve");
    }

    #[test]
    fn stages_are_send_and_metadata_inspection_stays_extraction_free() {
        fn assert_send<T: Send>() {}
        assert_send::<Staged>();
        let report = inspect().unwrap();
        assert_eq!(report.single_file, available());
        assert_eq!(catalog_bytes().is_some(), available());
        if available() {
            assert_eq!(report.assets.len(), 4);
            assert!(report.assets.iter().all(|asset| asset.verified));
            assert!(stage_game("unknown-game").is_err());
        } else {
            assert!(report.assets.is_empty());
            assert!(stage_boot().unwrap().is_none());
            assert!(stage_game("pointblank").unwrap().is_none());
        }
        serde_json::to_vec(&report).unwrap();
    }
}
