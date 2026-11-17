// Adapted from amangly/palepale@6490070271354e2a13a30e107065338c530a7322
// crates/df/src/pubg/menu.rs: original Chrome shell, controls and configuration slots.
use super::settings::Settings;
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
        let dirty = self
            .chrome
            .shell(ui, &["ESP", "MISC", "CFG"], 560.0, |ui, chrome, tab| {
                body(ui, chrome, tab, settings, snapshot)
            });
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
            ui.spacing();

            Chrome::section(ui, "CAPTURE");
            ui.text_wrapped(&snapshot.status);
            ui.text(format!(
                "Entities: {}",
                snapshot
                    .frame
                    .as_ref()
                    .map_or(0, |frame| frame.entities.len())
            ));
            Chrome::foot(
                ui,
                &[
                    "INSERT hide   END quit",
                    "External Everything / read-only PointBlank",
                ],
            );
        }
        1 => {
            Chrome::section(ui, "MISC");
            dirty |= chrome.toggle(ui, "Crosshair", &mut settings.crosshair);
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
            Chrome::foot(
                ui,
                &[
                    "INSERT hide   END quit",
                    "Direct2D / DirectWrite ESP + D3D11 / ImGui menu",
                ],
            );
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
    dirty
}
