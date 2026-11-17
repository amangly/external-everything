// Adapted from amangly/palepale@6490070271354e2a13a30e107065338c530a7322
// crates/df/src/pubg/settings.rs: menu-open load, application storage and slots.
// PointBlank persists only controls supported by the verified read-only frame.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    #[serde(skip)]
    pub menu: bool,
    pub esp_master: bool,
    pub boxes: bool,
    pub health: bool,
    pub crosshair: bool,
    pub col_box: [f32; 4],
    #[serde(skip)]
    pub config_message: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            menu: true,
            esp_master: true,
            boxes: true,
            health: true,
            crosshair: false,
            col_box: [1.0, 1.0, 0.0, 1.0],
            config_message: String::new(),
        }
    }
}

fn directory() -> Option<PathBuf> {
    #[cfg(windows)]
    let root = crate::common::paths::app_dir()?;
    #[cfg(not(windows))]
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("External Everything");
    Some(root.join("pointblank"))
}

fn path_named(name: &str) -> Result<PathBuf> {
    Ok(directory()
        .context("LOCALAPPDATA is unavailable for PointBlank settings")?
        .join(name))
}

fn slot_name(slot: u8) -> Result<String> {
    ensure!(
        (1..=3).contains(&slot),
        "Configuration slot is outside 1..=3"
    );
    Ok(format!("settings{slot}.json"))
}

fn decode(bytes: &[u8]) -> Result<Settings> {
    ensure!(
        bytes.len() as u64 <= MAX_CONFIG_BYTES,
        "Settings exceed the 1 MiB limit"
    );
    let mut settings: Settings =
        serde_json::from_slice(bytes).context("Read PointBlank settings JSON")?;
    ensure!(
        settings
            .col_box
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value)),
        "Box color components must be finite values between 0 and 1"
    );
    // Startup and slot loading always leave the control window accessible.
    settings.menu = true;
    Ok(settings)
}

fn read_settings(path: &Path) -> Result<Settings> {
    let file = fs::File::open(path).context("Open PointBlank settings")?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_CONFIG_BYTES,
        "Settings exceed the 1 MiB limit"
    );
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    decode(&bytes)
}

impl Settings {
    pub fn load() -> Self {
        let mut settings = Self::default();
        if let Some(path) = directory().map(|directory| directory.join(SETTINGS_FILE)) {
            match read_settings(&path) {
                Ok(loaded) => settings = loaded,
                Err(error) if !path.exists() => {
                    let _ = error;
                }
                Err(error) => settings.config_message = format!("Settings: {error:#}"),
            }
        }
        settings.menu = true;
        settings
    }

    pub fn save(&mut self) {
        self.config_message = match self.save_named(SETTINGS_FILE) {
            Ok(()) => "Settings saved".into(),
            Err(error) => format!("Save: {error:#}"),
        };
    }

    pub fn save_slot(&mut self, slot: u8) {
        self.config_message = match slot_name(slot).and_then(|name| self.save_named(&name)) {
            Ok(()) => format!("Slot {slot} saved"),
            Err(error) => format!("Save slot: {error:#}"),
        };
    }

    pub fn load_slot(&mut self, slot: u8) {
        match slot_name(slot)
            .and_then(|name| path_named(&name))
            .and_then(|path| read_settings(&path))
        {
            Ok(mut loaded) => {
                loaded.config_message = format!("Slot {slot} loaded");
                *self = loaded;
            }
            Err(error) => self.config_message = format!("Load slot: {error:#}"),
        }
    }

    fn save_named(&self, name: &str) -> Result<()> {
        let path = path_named(name)?;
        let bytes = serde_json::to_vec_pretty(self).context("Encode PointBlank settings")?;
        // Keep writes and reads under the identical validation contract.
        decode(&bytes)?;
        fs::create_dir_all(path.parent().context("Settings directory is absent")?)?;
        fs::write(path, bytes).context("Write PointBlank settings")
    }

    pub fn draw_any(&self) -> bool {
        self.esp_master && (self.boxes || self.health || self.crosshair)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_preserves_supported_controls_and_opens_menu() {
        let original = Settings {
            menu: false,
            boxes: false,
            health: false,
            crosshair: true,
            col_box: [0.2, 0.3, 0.4, 0.5],
            ..Settings::default()
        };
        let encoded = serde_json::to_vec(&original).unwrap();
        let loaded = decode(&encoded).unwrap();
        assert!(loaded.menu);
        assert!(!loaded.boxes);
        assert!(!loaded.health);
        assert!(loaded.crosshair);
        assert_eq!(loaded.col_box, original.col_box);
        assert!(!String::from_utf8(encoded).unwrap().contains("menu"));
    }

    #[test]
    fn settings_reject_unknown_controls_invalid_colors_and_oversize_input() {
        assert!(decode(br#"{"aim":true}"#).is_err());
        assert!(decode(br#"{"col_box":[2,0,0,1]}"#).is_err());
        assert!(decode(br#"{"col_box":[null,0,0,1]}"#).is_err());
        assert!(decode(&vec![b' '; MAX_CONFIG_BYTES as usize + 1]).is_err());
    }

    #[test]
    fn settings_default_opens_menu_and_limits_named_slots() {
        assert!(Settings::default().menu);
        assert_eq!(slot_name(1).unwrap(), "settings1.json");
        assert_eq!(slot_name(3).unwrap(), "settings3.json");
        assert!(slot_name(0).is_err());
        assert!(slot_name(4).is_err());
    }

    #[test]
    fn master_switch_disables_all_overlay_controls() {
        let mut settings = Settings::default();
        assert!(settings.draw_any());
        settings.esp_master = false;
        settings.crosshair = true;
        assert!(!settings.draw_any());
    }
}
