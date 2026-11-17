// Adapted from amangly/palepale@6490070271354e2a13a30e107065338c530a7322
// crates/df/src/pubg/settings.rs: menu-open load, application storage and slots.
// PointBlank settings cover the external overlay and verified feature controls.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrosshairStyle {
    #[default]
    Segments,
    Dot,
    Cross,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    #[serde(skip)]
    pub menu: bool,
    pub esp_master: bool,
    pub boxes: bool,
    pub health: bool,
    pub names: bool,
    pub anti_kick: bool,
    pub crosshair: bool,
    pub crosshair_style: CrosshairStyle,
    pub macro_enabled: bool,
    pub macro_mode: u8,
    pub macro_delay_preset: u8,
    pub macro_custom_delay_ms: u32,
    pub theme: u8,
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
            names: false,
            anti_kick: false,
            crosshair: false,
            crosshair_style: CrosshairStyle::Segments,
            macro_enabled: false,
            macro_mode: 1,
            macro_delay_preset: 1,
            macro_custom_delay_ms: 120,
            theme: 0,
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
    ensure!(
        (1..=2).contains(&settings.macro_mode),
        "Macro mode is outside 1..=2"
    );
    ensure!(
        settings.macro_delay_preset <= 4,
        "Macro delay preset is outside 0..=4"
    );
    ensure!(
        settings.macro_custom_delay_ms <= 9999,
        "Macro custom delay exceeds 9999 ms"
    );
    ensure!(settings.theme <= 4, "Theme is outside 0..=4");
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

    pub fn feature_request(&self) -> crate::games::pointblank::patches::Requested {
        crate::games::pointblank::patches::Requested {
            names: self.esp_master && self.names,
            anti_kick: self.anti_kick,
        }
    }

    pub fn macro_delay_ms(&self) -> u32 {
        match self.macro_delay_preset {
            0 => 25,
            1 => 50,
            2 => 75,
            3 => 100,
            _ => self.macro_custom_delay_ms.min(9999),
        }
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
    fn source_feature_settings_preserve_modes_delays_and_themes() {
        let original = Settings {
            names: true,
            anti_kick: true,
            macro_enabled: true,
            macro_mode: 2,
            macro_delay_preset: 4,
            macro_custom_delay_ms: 1234,
            theme: 3,
            crosshair_style: CrosshairStyle::Dot,
            ..Default::default()
        };
        let loaded = decode(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert!(loaded.names && loaded.anti_kick && loaded.macro_enabled);
        assert_eq!(
            (loaded.macro_mode, loaded.macro_delay_ms(), loaded.theme),
            (2, 1234, 3)
        );
        assert_eq!(loaded.crosshair_style, CrosshairStyle::Dot);
        let mut settings = Settings::default();
        for (index, delay) in [25, 50, 75, 100].into_iter().enumerate() {
            settings.macro_delay_preset = index as u8;
            assert_eq!(settings.macro_delay_ms(), delay);
        }
        for invalid in [
            br#"{"macro_mode":0}"#.as_slice(),
            br#"{"macro_mode":3}"#,
            br#"{"macro_delay_preset":5}"#,
            br#"{"macro_custom_delay_ms":10000}"#,
            br#"{"theme":5}"#,
            br#"{"crosshair_style":"unknown"}"#,
        ] {
            assert!(decode(invalid).is_err());
        }
        let legacy = decode(br#"{"crosshair":true}"#).unwrap();
        assert!(legacy.crosshair);
        assert_eq!(legacy.crosshair_style, CrosshairStyle::Segments);
        assert!(!legacy.names && !legacy.anti_kick && !legacy.macro_enabled);
    }

    #[test]
    fn master_switch_disables_all_overlay_controls() {
        let mut settings = Settings::default();
        assert!(settings.draw_any());
        settings.names = true;
        settings.anti_kick = true;
        assert!(settings.feature_request().names);
        settings.esp_master = false;
        settings.crosshair = true;
        assert!(!settings.draw_any());
        assert!(!settings.feature_request().names);
        assert!(settings.feature_request().anti_kick);
    }
}
