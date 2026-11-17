use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fs,
    io::{Read, Write},
    path::{Component, Path},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const BUNDLE_ENV: &str = "EXTERNAL_EVERYTHING_BUNDLE_DIR";
const MAX_IMAGE: u64 = 64 * 1024 * 1024;
const MAX_CATALOG: u64 = 1024 * 1024;
const NAMES: [&str; 4] = [
    "uefi-boot-standalone.efi",
    "hyperv-attachment.dll",
    "games/pointblank/external-runtime.exe",
    "games/catalog.json",
];

fn main() {
    if let Err(error) = generate() {
        panic!("Embedded bundle validation failed: {error}");
    }
}

fn generate() -> Result<()> {
    println!("cargo:rerun-if-env-changed={BUNDLE_ENV}");
    println!("cargo:rerun-if-changed=build.rs");
    let output =
        Path::new(&env::var_os("OUT_DIR").ok_or("OUT_DIR is absent")?).join("embedded_bundle.rs");
    let mut generated =
        String::from("// Generated from validated build inputs; keep out of source control.\n");
    let Some(directory) = env::var_os(BUNDLE_ENV) else {
        generated.push_str("const EMBEDDED: Option<[Asset<'static>; 4]> = None;\n");
        fs::write(output, generated)?;
        return Ok(());
    };
    let directory = Path::new(&directory);
    reject_links(directory)?;
    if !directory.is_dir() {
        return Err("Bundle directory is absent or is not a directory".into());
    }
    println!("cargo:rerun-if-changed={}", directory.display());
    let mut assets = Vec::new();
    for (index, name) in NAMES.iter().enumerate() {
        let path = directory.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = read_bounded(&path, if index == 3 { MAX_CATALOG } else { MAX_IMAGE })?;
        if index != 3 {
            validate_pe(&bytes, index)?;
        }
        let hash = format!("{:x}", Sha256::digest(&bytes));
        reject_links(&path)?;
        let absolute = path.canonicalize()?;
        reject_links(&absolute)?;
        assets.push((absolute, bytes, hash));
    }
    validate_catalog(&assets[3].1, &assets[2].2)?;
    generated.push_str("const EMBEDDED: Option<[Asset<'static>; 4]> = Some([\n");
    for (index, (path, bytes, hash)) in assets.iter().enumerate() {
        let kind = ["Efi", "Dll", "Runtime", "Catalog"][index];
        let path = path.to_str().ok_or("Bundle asset path requires Unicode")?;
        generated.push_str(&format!(
            "    Asset {{ name: {:?}, bytes: include_bytes!({path:?}), size: {}, sha256: {hash:?}, kind: Kind::{kind} }},\n",
            NAMES[index], bytes.len(),
        ));
    }
    generated.push_str("]);\n");
    let mut file = fs::File::create(output)?;
    file.write_all(generated.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn reject_links(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("Bundle paths require an absolute path without parent traversal".into());
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if metadata.file_type().is_symlink() {
            return Err("Bundle path contains a symbolic link".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("Bundle path contains a reparse point".into());
            }
        }
    }
    Ok(())
}

fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    reject_links(path)?;
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum {
        return Err(format!("Asset must be a nonempty regular file within {maximum} bytes").into());
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > maximum {
        return Err("Bundle asset changed while reading".into());
    }
    reject_links(path)?;
    Ok(bytes)
}

// Keep the bounded checks in sync with bundle::validate_pe. The launcher also
// verifies include_bytes! output against the independent build-time digest.
fn validate_pe(bytes: &[u8], kind: usize) -> Result<()> {
    if bytes.len() < 64 || bytes.len() as u64 > MAX_IMAGE || &bytes[..2] != b"MZ" {
        return Err("Expected a bounded PE image".into());
    }
    let pe = u32::from_le_bytes(bytes[60..64].try_into()?) as usize;
    let optional = pe.checked_add(24).ok_or("PE offset overflow")?;
    if optional > bytes.len() || &bytes[pe..pe + 4] != b"PE\0\0" {
        return Err("Invalid PE signature".into());
    }
    if u16::from_le_bytes(bytes[pe + 4..pe + 6].try_into()?) != 0x8664 {
        return Err("Expected an x64 PE image".into());
    }
    let size = u16::from_le_bytes(bytes[pe + 20..pe + 22].try_into()?) as usize;
    if size < 112
        || optional
            .checked_add(size)
            .is_none_or(|end| end > bytes.len())
        || u16::from_le_bytes(bytes[optional..optional + 2].try_into()?) != 0x20b
    {
        return Err("Expected complete PE32+ optional headers".into());
    }
    let flags = u16::from_le_bytes(bytes[pe + 22..pe + 24].try_into()?);
    let subsystem = u16::from_le_bytes(bytes[optional + 68..optional + 70].try_into()?);
    if flags & 2 == 0 {
        return Err("PE image lacks the executable flag".into());
    }
    match kind {
        0 if subsystem == 10 => Ok(()),
        1 if flags & 0x2000 != 0 => Ok(()),
        2 if flags & 0x2000 == 0 && matches!(subsystem, 2 | 3) => Ok(()),
        _ => Err("PE image type differs from its bundle role".into()),
    }
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
    if bytes.is_empty() || bytes.len() as u64 > MAX_CATALOG {
        return Err("Catalog exceeds its bounded size".into());
    }
    let catalog: Catalog = serde_json::from_slice(bytes)?;
    if catalog.schema != 1 || catalog.games.is_empty() || catalog.games.len() > 128 {
        return Err("Unsupported bundle catalog".into());
    }
    let mut ids = BTreeSet::new();
    let mut pointblank = false;
    for entry in catalog.games {
        if entry.id.is_empty()
            || entry.id.len() > 64
            || !entry
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || !ids.insert(entry.id.clone())
            || entry.name.trim().is_empty()
            || entry.name.len() > 128
            || entry.name.chars().any(char::is_control)
        {
            return Err("Invalid or duplicate catalog identity".into());
        }
        if entry.id == "pointblank" {
            if entry.executable.as_deref() != Some("pointblank/external-runtime.exe")
                || entry.sha256.as_deref() != Some(runtime_hash)
            {
                return Err(
                    "PointBlank catalog path/hash differs from its embedded runtime".into(),
                );
            }
            pointblank = true;
        } else if entry.executable.is_some() || entry.sha256.is_some() {
            return Err("Only the packaged PointBlank runtime is supported".into());
        }
    }
    if !pointblank {
        return Err("Packaged PointBlank entry is absent".into());
    }
    Ok(())
}
