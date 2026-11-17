// Adapted from Palepale crates/df/src/common/paths.rs at
// 6490070271354e2a13a30e107065338c530a7322 (build/reference/palepale).
// External Everything owns its settings directory; legacy settings stay separate.
use std::path::PathBuf;
use std::sync::OnceLock;

static DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn app_dir() -> Option<PathBuf> {
    if let Some(dir) = DIR.get() {
        return Some(dir.clone());
    }
    let dest = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("External Everything");
    std::fs::create_dir_all(&dest).ok()?;
    let _ = DIR.set(dest.clone());
    Some(dest)
}
