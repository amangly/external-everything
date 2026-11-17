//! Private, content-addressed boot sources retained until journal-based Restore.
//!
//! The vendor may need the original bytes to prove ownership of an interrupted
//! ESP copy. Runtime staging stays temporary; these two boot images do not.
use anyhow::{Context, Result, ensure};
use hyperrev_loader::files::{self, DLL_NAME, EFI_NAME, Identity, Journal, PlannedFile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const KIND: &str = "external-everything-boot-recovery";
const MARKER: &str = ".external-everything-boot-recovery.json";
const MAX_IMAGE: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST: u64 = 4096;
const NAMES: [&str; 2] = [EFI_NAME, DLL_NAME];

// Four fields, including the exact two permitted payload filenames.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    kind: String,
    #[serde(rename = "uefi-boot-standalone.efi")]
    efi: Identity,
    #[serde(rename = "hyperv-attachment.dll")]
    dll: Identity,
}

impl Manifest {
    fn new(identities: &[Identity]) -> Result<Self> {
        ensure!(
            identities.len() == 2,
            "Expected exactly two boot identities"
        );
        let manifest = Self {
            schema: 1,
            kind: KIND.into(),
            efi: identities[0].clone(),
            dll: identities[1].clone(),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    fn identities(&self) -> [&Identity; 2] {
        [&self.efi, &self.dll]
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1 && self.kind == KIND,
            "Foreign recovery marker"
        );
        for identity in self.identities() {
            ensure!(
                identity.bytes > 0 && identity.bytes <= MAX_IMAGE && digest_name(&identity.sha256),
                "Malformed boot recovery identity"
            );
        }
        Ok(())
    }

    fn digest(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(KIND.as_bytes());
        digest.update([0]);
        digest.update(self.schema.to_le_bytes());
        for (name, identity) in NAMES.into_iter().zip(self.identities()) {
            digest.update((name.len() as u64).to_le_bytes());
            digest.update(name.as_bytes());
            digest.update(identity.bytes.to_le_bytes());
            digest.update(identity.sha256.as_bytes());
        }
        format!("{:x}", digest.finalize())
    }
}

fn digest_name(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn checked_path(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir)),
        "Recovery requires an absolute path without traversal: {}",
        path.display()
    );
    files::reject_links(path)
}

fn default_root() -> Result<PathBuf> {
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is missing")?);
    checked_path(&local)?;
    let local = local
        .canonicalize()
        .context("Resolve LOCALAPPDATA for private boot recovery")?;
    checked_path(&local)?;
    Ok(local
        .join("External Everything")
        .join("recovery")
        .join("boot"))
}

/// Retain only the two validated boot images; returns their persistent directory.
pub fn retain(source: &Path) -> Result<PathBuf> {
    retain_at(&default_root()?, source)
}

/// Call only after vendor Restore succeeds, with its original loaded journal.
/// Legacy source paths outside our private namespace remain untouched.
pub fn cleanup(journal: &Journal) -> Result<()> {
    cleanup_at(&default_root()?, journal)
}

fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>> {
    checked_path(path)?;
    let file =
        File::open(path).with_context(|| format!("Open recovery file {}", path.display()))?;
    let before = file.metadata()?;
    ensure!(
        before.is_file() && before.len() <= limit,
        "Invalid recovery file: {}",
        path.display()
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == before.len(),
        "Recovery file changed while reading: {}",
        path.display()
    );
    checked_path(path)?;
    let after = fs::symlink_metadata(path)?;
    ensure!(
        after.is_file() && after.len() == before.len(),
        "Recovery file changed: {}",
        path.display()
    );
    Ok(bytes)
}

fn byte_identity(bytes: &[u8]) -> Identity {
    Identity {
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

fn check_directory(root: &Path, directory: &Path, expected: &Manifest) -> Result<()> {
    checked_path(root)?;
    checked_path(directory)?;
    ensure!(
        directory.parent() == Some(root)
            && directory.file_name().and_then(|part| part.to_str())
                == Some(expected.digest().as_str()),
        "Recovery directory differs from its content-addressed identity: {}",
        directory.display()
    );
    ensure!(
        fs::symlink_metadata(root)?.is_dir(),
        "Recovery root is a foreign file"
    );
    ensure!(root.canonicalize()? == root, "Recovery root changed");
    ensure!(
        fs::symlink_metadata(directory)?.is_dir(),
        "Recovery entry is a foreign file: {}",
        directory.display()
    );
    ensure!(
        directory.canonicalize()? == directory,
        "Recovery directory changed: {}",
        directory.display()
    );
    Ok(())
}

fn inventory(directory: &Path, expected: &[&str]) -> Result<()> {
    checked_path(directory)?;
    let mut found = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .context("Foreign non-Unicode filename in recovery cache")?;
        ensure!(
            expected.contains(&name),
            "Unexpected recovery entry {}; preserve it and inspect this cache before retrying",
            entry.path().display()
        );
        checked_path(&entry.path())?;
        ensure!(
            entry.file_type()?.is_file(),
            "Foreign recovery entry: {}",
            entry.path().display()
        );
        found.push(name.to_owned());
    }
    ensure!(
        found.len() == expected.len(),
        "Recovery cache is incomplete: {}; preserve this directory for inspection",
        directory.display()
    );
    Ok(())
}

fn check_marker(directory: &Path, expected: &Manifest) -> Result<()> {
    let marker: Manifest =
        serde_json::from_slice(&read_regular(&directory.join(MARKER), MAX_MANIFEST)?)
            .context("Parse boot recovery ownership marker")?;
    marker.validate()?;
    ensure!(
        marker == *expected,
        "Recovery marker differs from the source or installation journal"
    );
    Ok(())
}

fn check_cache(root: &Path, directory: &Path, expected: &Manifest) -> Result<()> {
    check_directory(root, directory, expected)?;
    inventory(directory, &[EFI_NAME, DLL_NAME, MARKER])?;
    check_marker(directory, expected)?;
    let identities =
        files::source_identities(directory).context("Revalidate both cached boot PE images")?;
    ensure!(
        identities.iter().eq(expected.identities()),
        "Cached boot bytes differ from the source or installation journal"
    );
    Ok(())
}

fn publish(
    root: &Path,
    directory: &Path,
    manifest: &Manifest,
    name: &str,
    bytes: &[u8],
) -> Result<()> {
    ensure!(
        NAMES.contains(&name) || name == MARKER,
        "Unexpected recovery publication name"
    );
    check_directory(root, directory, manifest)?;
    let target = directory.join(name);
    checked_path(&target)?;
    // tempfile uses CreateNew. On Windows persist_noclobber atomically renames
    // within this directory without replacing a pre-existing destination.
    let mut temporary = tempfile::Builder::new()
        .prefix(".recovery-")
        .tempfile_in(directory)?;
    checked_path(temporary.path())?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    check_directory(root, directory, manifest)?;
    checked_path(&target)?;
    temporary
        .persist_noclobber(&target)
        .map_err(|error| error.error)
        .with_context(|| {
            format!(
                "Publish recovery file without replacing existing content: {}",
                target.display()
            )
        })?;
    Ok(())
}

fn retain_at(root: &Path, source: &Path) -> Result<PathBuf> {
    checked_path(root)?;
    checked_path(source)?;
    // Vendor order is EFI_NAME, DLL_NAME, with bounded x64 PE/class checks.
    let expected = Manifest::new(&files::source_identities(source)?)?;
    fs::create_dir_all(root).context("Create private boot recovery namespace")?;
    checked_path(root)?;
    let root = root.canonicalize()?;
    checked_path(&root)?;
    let directory = root.join(expected.digest());
    checked_path(&directory)?;
    match fs::create_dir(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            check_cache(&root, &directory, &expected).with_context(|| format!(
                "Existing boot recovery cache was preserved: {}; inspect foreign or incomplete content before retrying",
                directory.display()
            ))?;
            return Ok(directory);
        }
        Err(error) => return Err(error).context("Create fresh boot recovery directory"),
    }
    let result = (|| -> Result<()> {
        check_directory(&root, &directory, &expected)?;
        inventory(&directory, &[])?;
        for (name, identity) in NAMES.into_iter().zip(expected.identities()) {
            let bytes = read_regular(&source.join(name), MAX_IMAGE)?;
            files::validate_pe(&bytes, name == EFI_NAME)?;
            ensure!(
                byte_identity(&bytes) == *identity,
                "Source image changed before recovery retention: {name}"
            );
            publish(&root, &directory, &expected, name, &bytes)?;
        }
        let identities = files::source_identities(&directory)?;
        ensure!(
            identities.iter().eq(expected.identities()),
            "Recovery copy differs from source identity"
        );
        inventory(&directory, &NAMES)?;
        // The ownership marker is deferred until both complete images validate.
        publish(
            &root,
            &directory,
            &expected,
            MARKER,
            &serde_json::to_vec_pretty(&expected)?,
        )?;
        check_cache(&root, &directory, &expected)
    })();
    result.with_context(|| {
        format!(
            "Recovery data preserved at {}; inspect this directory before retrying",
            directory.display()
        )
    })?;
    Ok(directory)
}

fn journal_manifest(directory: &Path, entries: &[&PlannedFile]) -> Result<Manifest> {
    ensure!(
        entries.len() == 2,
        "Recovery journal requires exactly two source files per cache"
    );
    let mut identities = Vec::new();
    for name in NAMES {
        let matching: Vec<_> = entries.iter().filter(|entry| entry.name == name).collect();
        ensure!(
            matching.len() == 1,
            "Recovery journal has a missing or duplicate boot filename: {name}"
        );
        let entry = matching[0];
        checked_path(&entry.source)?;
        ensure!(
            entry.source.parent() == Some(directory)
                && entry.source.file_name() == Some(std::ffi::OsStr::new(name)),
            "Recovery journal source/name differs from its exact cache path"
        );
        identities.push(entry.identity.clone());
    }
    let manifest = Manifest::new(&identities)?;
    ensure!(
        directory.file_name().and_then(|part| part.to_str()) == Some(manifest.digest().as_str()),
        "Recovery journal identities differ from the cache directory digest"
    );
    Ok(manifest)
}

fn cleanup_at(root: &Path, journal: &Journal) -> Result<()> {
    checked_path(root)?;
    if !root.try_exists()? {
        return Ok(());
    }
    let root = root.canonicalize()?;
    checked_path(&root)?;
    let mut groups: BTreeMap<PathBuf, Vec<&PlannedFile>> = BTreeMap::new();
    for entry in &journal.files {
        // Inspect filesystem paths only after lexical namespace classification.
        // This excludes legacy files, other drives, hosts, and sibling prefixes.
        if !entry.source.starts_with(&root) {
            continue;
        }
        checked_path(&entry.source)?;
        let relative = entry.source.strip_prefix(&root)?;
        let components: Vec<_> = relative.components().collect();
        ensure!(
            components.len() == 2
                && components
                    .iter()
                    .all(|part| matches!(part, Component::Normal(_))),
            "Unexpected path inside boot recovery namespace: {}",
            entry.source.display()
        );
        let parent = entry
            .source
            .parent()
            .context("Recovery source requires a cache parent")?;
        ensure!(
            parent.parent() == Some(root.as_path()),
            "Recovery source is outside an immediate digest directory"
        );
        groups.entry(parent.to_owned()).or_default().push(entry);
    }
    let mut validated = Vec::new();
    // Validate every referenced group before deleting anything from any group.
    for (directory, entries) in groups {
        let expected = journal_manifest(&directory, &entries)?;
        checked_path(&directory)?;
        if !directory.try_exists()? {
            continue; // Already cleaned; never create a directory during cleanup.
        }
        check_cache(&root, &directory, &expected).with_context(|| format!(
            "Restore succeeded; recovery data preserved at {} because ownership validation failed. Inspect the unexpected content before removing it manually",
            directory.display()
        ))?;
        for entry in entries {
            ensure!(
                entry.source.canonicalize()? == entry.source,
                "Journal recovery source is an alias rather than its canonical cache file"
            );
        }
        validated.push((directory, expected));
    }
    for (directory, expected) in validated {
        check_cache(&root, &directory, &expected)?;
        for (index, (name, identity)) in NAMES.into_iter().zip(expected.identities()).enumerate() {
            check_directory(&root, &directory, &expected)?;
            let mut remaining = NAMES[index..].to_vec();
            remaining.push(MARKER);
            inventory(&directory, &remaining)?;
            check_marker(&directory, &expected)?;
            let path = directory.join(name);
            checked_path(&path)?;
            ensure!(
                path.canonicalize()? == path && files::identity(&path)? == *identity,
                "Recovery image changed before removal: {}",
                path.display()
            );
            fs::remove_file(&path)
                .with_context(|| format!("Remove verified recovery image {}", path.display()))?;
        }
        check_directory(&root, &directory, &expected)?;
        inventory(&directory, &[MARKER])?;
        check_marker(&directory, &expected)?;
        let marker = directory.join(MARKER);
        checked_path(&marker)?;
        fs::remove_file(&marker).context("Remove verified boot recovery marker")?;
        check_directory(&root, &directory, &expected)?;
        inventory(&directory, &[])?;
        fs::remove_dir(&directory)
            .context("Remove empty verified boot recovery digest directory")?;
        // Namespace parents are deliberately retained; recursive removal is absent.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyperrev_loader::{boot::Variable, files::Phase};
    use tempfile::TempDir;

    fn pe(efi: bool, tag: u8) -> Vec<u8> {
        let mut bytes = vec![tag; 512];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        bytes[84..86].copy_from_slice(&240u16.to_le_bytes());
        bytes[86..88].copy_from_slice(&(if efi { 0x22u16 } else { 0x2022u16 }).to_le_bytes());
        bytes[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        bytes[156..158].copy_from_slice(&(if efi { 10u16 } else { 1u16 }).to_le_bytes());
        bytes
    }

    struct Fixture {
        _temp: TempDir,
        root: PathBuf,
        source: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let source = base.join("source");
            fs::create_dir(&source).unwrap();
            for (name, efi) in [(EFI_NAME, true), (DLL_NAME, false)] {
                fs::write(source.join(name), pe(efi, 0)).unwrap();
            }
            Self {
                _temp: temp,
                root: base.join("private/recovery/boot"),
                source,
            }
        }
        fn retain(&self) -> PathBuf {
            retain_at(&self.root, &self.source).unwrap()
        }
        fn journal(&self, directory: &Path) -> Journal {
            let variable = Variable {
                attributes: 7,
                data: vec![],
            };
            Journal {
                schema: 2,
                transaction_id: Default::default(),
                phase: Phase::Applied,
                esp_guid: Default::default(),
                esp_hint: self.source.clone(),
                boot_number: 1,
                boot_name: "Boot0001".into(),
                safe_number: 2,
                safe_name: "Boot0002".into(),
                safe_option: variable.clone(),
                boot_order_after: variable.clone(),
                reference_name: "Boot0000".into(),
                reference_before: variable.clone(),
                boot_order_before: variable.clone(),
                boot_next_before: None,
                created_option: variable,
                windows_boot_manager: files::identity(&directory.join(EFI_NAME)).unwrap(),
                product_directory_existed: false,
                files: NAMES
                    .into_iter()
                    .zip(files::source_identities(directory).unwrap())
                    .map(|(name, identity)| PlannedFile {
                        name: name.into(),
                        source: directory.join(name).canonicalize().unwrap(),
                        identity,
                        existed_before: false,
                    })
                    .collect(),
            }
        }
    }

    #[test]
    fn repeated_retention_is_stable_and_revalidates_both_images() {
        let fixture = Fixture::new();
        let first = fixture.retain();
        assert!(first.is_absolute());
        assert_eq!(fixture.retain(), first);
        assert_eq!(
            files::source_identities(&first).unwrap(),
            files::source_identities(&fixture.source).unwrap()
        );
        assert!(digest_name(first.file_name().unwrap().to_str().unwrap()));
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(first.join(MARKER)).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 4);
        fs::write(first.join(DLL_NAME), pe(false, 7)).unwrap();
        assert!(retain_at(&fixture.root, &fixture.source).is_err());
        assert_eq!(fs::read(first.join(DLL_NAME)).unwrap(), pe(false, 7));
    }

    #[test]
    fn retained_sources_survive_temporary_stage_removal() {
        let fixture = Fixture::new();
        let cache = fixture.retain();
        for name in NAMES {
            fs::remove_file(fixture.source.join(name)).unwrap();
        }
        fs::remove_dir(&fixture.source).unwrap();
        let identities = files::source_identities(&cache).unwrap();
        assert_eq!(identities.len(), 2);
        assert!(fs::read(cache.join(EFI_NAME)).unwrap().starts_with(b"MZ"));
    }

    #[test]
    fn invalid_source_is_rejected_before_cache_creation() {
        let fixture = Fixture::new();
        fs::write(fixture.source.join(EFI_NAME), b"foreign").unwrap();
        assert!(retain_at(&fixture.root, &fixture.source).is_err());
        assert!(!fixture.root.exists());
    }

    #[test]
    fn unmarked_foreign_digest_directory_is_preserved() {
        let fixture = Fixture::new();
        let manifest = Manifest::new(&files::source_identities(&fixture.source).unwrap()).unwrap();
        let directory = fixture.root.join(manifest.digest());
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("foreign.txt"), b"keep").unwrap();
        assert!(retain_at(&fixture.root, &fixture.source).is_err());
        assert_eq!(fs::read(directory.join("foreign.txt")).unwrap(), b"keep");
        assert!(!directory.join(MARKER).exists());
        assert!(!directory.join(EFI_NAME).exists());
    }

    #[test]
    fn successful_cleanup_removes_only_journal_referenced_cache() {
        let fixture = Fixture::new();
        let old = fixture.retain();
        let journal = fixture.journal(&old);
        fs::write(fixture.source.join(DLL_NAME), pe(false, 1)).unwrap();
        let updated = fixture.retain();
        assert_ne!(old, updated);
        cleanup_at(&fixture.root, &journal).unwrap();
        assert!(!old.exists());
        assert!(updated.join(MARKER).exists());
        assert!(fixture.root.exists());
        cleanup_at(&fixture.root, &journal).unwrap();
    }

    #[test]
    fn legacy_sources_remain_unchanged() {
        let fixture = Fixture::new();
        let cache = fixture.retain();
        let journal = fixture.journal(&fixture.source);
        cleanup_at(&fixture.root, &journal).unwrap();
        assert!(cache.join(MARKER).exists());
        assert!(fixture.source.join(EFI_NAME).exists());
        assert!(fixture.source.join(DLL_NAME).exists());
    }

    #[test]
    fn unknown_file_prevents_all_cache_removal() {
        let fixture = Fixture::new();
        let cache = fixture.retain();
        let journal = fixture.journal(&cache);
        fs::write(cache.join("foreign.txt"), b"keep").unwrap();
        assert!(cleanup_at(&fixture.root, &journal).is_err());
        for name in [EFI_NAME, DLL_NAME, MARKER, "foreign.txt"] {
            assert!(cache.join(name).exists());
        }
    }

    #[test]
    fn journal_identity_and_filename_mismatches_preserve_everything() {
        let fixture = Fixture::new();
        let cache = fixture.retain();
        let mut journal = fixture.journal(&cache);
        journal.files[0].identity.sha256 = "a".repeat(64);
        assert!(cleanup_at(&fixture.root, &journal).is_err());
        let mut journal = fixture.journal(&cache);
        journal.files[0].source = cache.join(DLL_NAME);
        assert!(cleanup_at(&fixture.root, &journal).is_err());
        assert!(cache.join(EFI_NAME).exists());
        assert!(cache.join(DLL_NAME).exists());
        assert!(cache.join(MARKER).exists());
    }

    #[test]
    fn forged_marker_and_changed_bytes_do_not_establish_ownership() {
        let fixture = Fixture::new();
        let cache = fixture.retain();
        let journal = fixture.journal(&cache);
        fs::write(cache.join(DLL_NAME), pe(false, 8)).unwrap();
        let forged = Manifest::new(&files::source_identities(&cache).unwrap()).unwrap();
        fs::write(cache.join(MARKER), serde_json::to_vec(&forged).unwrap()).unwrap();
        assert!(cleanup_at(&fixture.root, &journal).is_err());
        assert!(retain_at(&fixture.root, &fixture.source).is_err());
        assert_eq!(fs::read(cache.join(DLL_NAME)).unwrap(), pe(false, 8));
    }

    #[test]
    fn noncanonical_digest_and_nested_source_paths_are_rejected() {
        let fixture = Fixture::new();
        let cache = fixture.retain();
        let mut journal = fixture.journal(&cache);
        for entry in &mut journal.files {
            entry.source = fixture.root.join("a".repeat(64)).join(&entry.name);
        }
        assert!(cleanup_at(&fixture.root, &journal).is_err());
        let mut journal = fixture.journal(&cache);
        journal.files[0].source = cache.join("nested").join(EFI_NAME);
        assert!(cleanup_at(&fixture.root, &journal).is_err());
        assert!(cache.join(MARKER).exists());
    }

    #[test]
    fn publication_never_replaces_existing_target() {
        let fixture = Fixture::new();
        let cache = fixture.retain();
        let manifest = Manifest::new(&files::source_identities(&cache).unwrap()).unwrap();
        let root = fixture.root.canonicalize().unwrap();
        assert!(publish(&root, &cache, &manifest, EFI_NAME, b"replacement").is_err());
        assert_eq!(fs::read(cache.join(EFI_NAME)).unwrap(), pe(true, 0));
        inventory(&cache, &[EFI_NAME, DLL_NAME, MARKER]).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_cache_is_preserved() {
        let fixture = Fixture::new();
        let cache = fixture.retain();
        let journal = fixture.journal(&cache);
        let outside = fixture.source.join("outside.dll");
        fs::write(&outside, pe(false, 0)).unwrap();
        fs::remove_file(cache.join(DLL_NAME)).unwrap();
        std::os::unix::fs::symlink(&outside, cache.join(DLL_NAME)).unwrap();
        assert!(cleanup_at(&fixture.root, &journal).is_err());
        assert!(outside.exists());
        assert!(cache.join(EFI_NAME).exists());
        assert!(cache.join(MARKER).exists());
    }
}
