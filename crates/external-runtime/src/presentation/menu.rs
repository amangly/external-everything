// Adapted from amangly/palepale@6490070271354e2a13a30e107065338c530a7322
// crates/df/src/pubg/menu.rs: shared Chrome shell and configuration slots.
// PointBlank feature controls follow the supplied PBZPTID source.
use super::settings::{CrosshairStyle, Settings};
use crate::{common::chrome::Chrome, engine::Snapshot};
use imgui::Ui;

#[derive(Default)]
pub struct Menu {
    chrome: Chrome,
}
impl Menu {
    pub fn poll(&mut self, settings: &mut Settings) {
        self.chrome.poll_insert(&mut settings.menu);
    }
    pub fn draw(&mut self, ui: &Ui, settings: &mut Settings, snapshot: &Snapshot) {
        let dirty = self.chrome.shell(
            ui,
            &["ESP", "WEAPON", "PLAYER", "MISC", "CFG"],
            560.,
            |ui, chrome, tab| body(ui, chrome, tab, settings, snapshot),
        );
        if dirty {
            settings.save();
        }
    }
}

fn body(
    ui: &Ui,
    chrome: &mut Chrome,
    tab: usize,
    settings: &mut Settings,
    snapshot: &Snapshot,
) -> bool {
    let mut dirty = false;
    match tab {
        0 => {
            Chrome::section(ui, "POINTBLANK / ESP");
            dirty |= chrome.toggle(ui, "Master", &mut settings.esp_master);
            dirty |= chrome.toggle(ui, "Box", &mut settings.boxes);
            dirty |= chrome.toggle(ui, "Health", &mut settings.health);
            dirty |= chrome.color(ui, "Box color", &mut settings.col_box);
            Chrome::section(ui, "NATIVE NAME TAGS");
            {
                let _disabled = ui.begin_disabled(!snapshot.features.available && !settings.names);
                dirty |= chrome.toggle(ui, "ESP names", &mut settings.names);
            }
            ui.text(format!(
                "Name tags: {}",
                if snapshot.features.names { "on" } else { "off" }
            ));
            feature_message(ui, snapshot);
            Chrome::section(ui, "CAPTURE");
            ui.text_wrapped(&snapshot.status);
            ui.text(format!(
                "Entities: {}",
                snapshot
                    .frame
                    .as_ref()
                    .map_or(0, |frame| frame.entities.len())
            ));
        }
        1 => {
            Chrome::section(ui, "WEAPON / MACRO");
            dirty |= chrome.toggle(ui, "Macro", &mut settings.macro_enabled);
            let mut mode = usize::from(settings.macro_mode.saturating_sub(1)).min(1);
            if ui.combo_simple_string("Mode", &mut mode, &["Scope 3Q", "3Q"]) {
                settings.macro_mode = mode as u8 + 1;
                dirty = true;
            }
            let mut delay = usize::from(settings.macro_delay_preset).min(4);
            if ui.combo_simple_string(
                "Delay",
                &mut delay,
                &["25 ms", "50 ms", "75 ms", "100 ms", "Custom"],
            ) {
                settings.macro_delay_preset = delay as u8;
                dirty = true;
            }
            if settings.macro_delay_preset == 4 {
                let mut ms = settings.macro_custom_delay_ms as i32;
                if chrome.slider_i(
                    ui,
                    "Custom delay (ms)",
                    "##macro_custom_delay",
                    &mut ms,
                    0,
                    9999,
                ) {
                    settings.macro_custom_delay_ms = ms as u32;
                    dirty = true;
                }
            }
            ui.text(format!("Active delay: {} ms", settings.macro_delay_ms()));
            ui.text_wrapped("Scope 3Q: right click, then fire, 3, Q. 3Q: left click, then 3, Q. Runs once per press while PointBlank has focus. Opening the menu cancels the sequence.");
        }
        2 => {
            Chrome::section(ui, "PLAYER");
            {
                let _disabled =
                    ui.begin_disabled(!snapshot.features.available && !settings.anti_kick);
                dirty |= chrome.toggle(ui, "Anti kick", &mut settings.anti_kick);
            }
            ui.text(format!(
                "Client kick handler: {}",
                if snapshot.features.anti_kick {
                    "patched"
                } else {
                    "original"
                }
            ));
            ui.text_wrapped("Uses the verified client room-kick handler. Disabling it or closing the runtime restores the original instruction.");
            feature_message(ui, snapshot);
        }
        3 => {
            Chrome::section(ui, "MISC");
            dirty |= chrome.toggle(ui, "Crosshair", &mut settings.crosshair);
            let mut style = match settings.crosshair_style {
                CrosshairStyle::Segments => 0,
                CrosshairStyle::Dot => 1,
                CrosshairStyle::Cross => 2,
            };
            if ui.combo_simple_string("Crosshair style", &mut style, &["Segments", "Dot", "Cross"])
            {
                settings.crosshair_style = match style {
                    1 => CrosshairStyle::Dot,
                    2 => CrosshairStyle::Cross,
                    _ => CrosshairStyle::Segments,
                };
                dirty = true;
            }
            let mut theme = usize::from(settings.theme).min(4);
            if ui.combo_simple_string(
                "Theme",
                &mut theme,
                &["Mint", "Cyan", "Crimson", "Purple", "Gold"],
            ) {
                settings.theme = theme as u8;
                dirty = true;
            }
            Chrome::section(ui, "RUNTIME");
            ui.text_wrapped(&snapshot.status);
            if let Some(pid) = snapshot.pid {
                ui.text(format!("Process: {pid}"));
            }
            if let Some(base) = snapshot.base {
                ui.text(format!("Image: {base:#x}"));
            }
            if let Some(cr3) = snapshot.cr3 {
                ui.text(format!("CR3: {cr3:#x}"));
            }
            if !snapshot.detail.is_empty() {
                ui.text_wrapped(&snapshot.detail);
            }
            feature_message(ui, snapshot);
        }
        _ => {
            dirty |= Chrome::cfg_tab(
                ui,
                r"%LOCALAPPDATA%\External Everything\pointblank\settings.json",
                "INSERT hide   END quit",
                |slot, load| {
                    if load {
                        settings.load_slot(slot);
                    } else {
                        settings.save_slot(slot);
                    }
                },
            );
            if !settings.config_message.is_empty() {
                ui.text_wrapped(&settings.config_message);
            }
        }
    }
    if tab < 4 {
        Chrome::foot(
            ui,
            &[
                "INSERT hide   END quit",
                "External Everything / hypercall transport",
            ],
        );
    }
    dirty
}

fn feature_message(ui: &Ui, snapshot: &Snapshot) {
    if !snapshot.features.available {
        ui.text_wrapped("Waiting for a supported PointBlank build.");
    }
    if !snapshot.features.message.is_empty() {
        ui.text_wrapped(&snapshot.features.message);
    }
}
