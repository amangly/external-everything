// Copied from Palepale crates/df/src/common/theme.rs at
// 6490070271354e2a13a30e107065338c530a7322 (build/reference/palepale).
// Accent selection added without changing the original default or layout.
use imgui::{Style, StyleColor};
use std::cell::Cell;

pub const ACCENT: [f32; 4] = [0.00, 1.00, 0.255, 1.00];
pub const ACCENT_DIM: [f32; 4] = [0.051, 0.149, 0.078, 1.00];
pub const ACCENT_HOVER: [f32; 4] = [0.196, 1.00, 0.404, 1.00];
pub const BG_DEEP: [f32; 4] = [0.00, 0.00, 0.00, 0.99];
pub const BG_PANEL: [f32; 4] = [0.020, 0.020, 0.020, 1.00];
pub const BORDER: [f32; 4] = [0.141, 0.141, 0.141, 1.00];
pub const TEXT: [f32; 4] = [0.910, 0.910, 0.910, 1.00];
pub const TEXT_DIM: [f32; 4] = [0.541, 0.541, 0.541, 1.00];

#[derive(Clone, Copy, Debug, PartialEq)]
struct Palette {
    accent: [f32; 4],
    dim: [f32; 4],
    hover: [f32; 4],
}

impl Palette {
    const fn rgb(r: u8, g: u8, b: u8) -> Self {
        let r = r as f32 / 255.0;
        let g = g as f32 / 255.0;
        let b = b as f32 / 255.0;
        Self {
            accent: [r, g, b, 1.0],
            dim: [r * 0.149, g * 0.149, b * 0.149, 1.0],
            hover: [
                r + (1.0 - r) * 0.196,
                g + (1.0 - g) * 0.196,
                b + (1.0 - b) * 0.196,
                1.0,
            ],
        }
    }
}

const fn palette(index: u8) -> Palette {
    // PBZPTID/Dll/Menu.cpp GetThemeColor: Cyan, Crimson, Purple, Gold.
    // Mint deliberately retains this UI's original #00FF41, DIM and HOVER.
    match index {
        1 => Palette::rgb(0, 180, 255),
        2 => Palette::rgb(230, 40, 60),
        3 => Palette::rgb(160, 50, 255),
        4 => Palette::rgb(255, 190, 0),
        _ => Palette {
            accent: ACCENT,
            dim: ACCENT_DIM,
            hover: ACCENT_HOVER,
        },
    }
}

thread_local! {
    static SELECTED: Cell<u8> = const { Cell::new(0) };
}

/// Select Mint/Cyan/Crimson/Purple/Gold for the calling UI thread.
/// Call before each GUI frame; indices outside 0..=4 select Mint.
pub fn select(index: u8) {
    SELECTED.with(|selected| selected.set(if index <= 4 { index } else { 0 }));
}

pub fn accent() -> [f32; 4] {
    SELECTED.with(|selected| palette(selected.get()).accent)
}

pub fn accent_dim() -> [f32; 4] {
    SELECTED.with(|selected| palette(selected.get()).dim)
}

pub fn accent_hover() -> [f32; 4] {
    SELECTED.with(|selected| palette(selected.get()).hover)
}

pub fn apply(s: &mut Style) {
    let colors = SELECTED.with(|selected| palette(selected.get()));
    s.window_rounding = 0.0;
    s.child_rounding = 0.0;
    s.frame_rounding = 0.0;
    s.popup_rounding = 0.0;
    s.scrollbar_rounding = 0.0;
    s.grab_rounding = 0.0;
    s.tab_rounding = 0.0;
    s.window_border_size = 1.0;
    s.child_border_size = 1.0;
    s.frame_border_size = 1.0;
    s.window_padding = [8.0, 8.0];
    s.frame_padding = [5.0, 3.0];
    s.item_spacing = [6.0, 5.0];
    s.item_inner_spacing = [3.0, 3.0];
    s[StyleColor::WindowBg] = BG_DEEP;
    s[StyleColor::ChildBg] = BG_PANEL;
    s[StyleColor::PopupBg] = BG_DEEP;
    s[StyleColor::Border] = BORDER;
    s[StyleColor::BorderShadow] = [0.0, 0.0, 0.0, 0.0];
    s[StyleColor::FrameBg] = BG_DEEP;
    s[StyleColor::FrameBgHovered] = BG_PANEL;
    s[StyleColor::FrameBgActive] = BORDER;
    s[StyleColor::TitleBg] = BG_DEEP;
    s[StyleColor::TitleBgActive] = BG_DEEP;
    s[StyleColor::TitleBgCollapsed] = BG_DEEP;
    s[StyleColor::ScrollbarBg] = BG_DEEP;
    s[StyleColor::ScrollbarGrab] = colors.dim;
    s[StyleColor::ScrollbarGrabHovered] = colors.hover;
    s[StyleColor::ScrollbarGrabActive] = colors.accent;
    s[StyleColor::CheckMark] = colors.accent;
    s[StyleColor::SliderGrab] = colors.dim;
    s[StyleColor::SliderGrabActive] = colors.accent;
    s[StyleColor::Button] = BG_PANEL;
    s[StyleColor::ButtonHovered] = BORDER;
    s[StyleColor::ButtonActive] = colors.dim;
    s[StyleColor::Header] = BG_PANEL;
    s[StyleColor::HeaderHovered] = BORDER;
    s[StyleColor::HeaderActive] = colors.dim;
    s[StyleColor::Separator] = BORDER;
    s[StyleColor::Tab] = BG_DEEP;
    s[StyleColor::TabHovered] = BORDER;
    s[StyleColor::TabActive] = BG_PANEL;
    s[StyleColor::Text] = TEXT;
    s[StyleColor::TextDisabled] = TEXT_DIM;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(color: [f32; 4]) -> [u8; 4] {
        color.map(|channel| (channel * 255.0).round() as u8)
    }

    #[test]
    fn mint_preserves_original_palette() {
        let mint = palette(0);
        assert_eq!(mint.accent, [0.00, 1.00, 0.255, 1.00]);
        assert_eq!(mint.dim, [0.051, 0.149, 0.078, 1.00]);
        assert_eq!(mint.hover, [0.196, 1.00, 0.404, 1.00]);
        assert_eq!(bytes(mint.accent), [0, 255, 65, 255]);
        assert_eq!(bytes(mint.dim), [13, 38, 20, 255]);
        assert_eq!(bytes(mint.hover), [50, 255, 103, 255]);
    }

    #[test]
    fn source_accents_are_exact_and_distinct() {
        let expected = [
            [0, 255, 65, 255],
            [0, 180, 255, 255],
            [230, 40, 60, 255],
            [160, 50, 255, 255],
            [255, 190, 0, 255],
        ];
        for (index, color) in expected.into_iter().enumerate() {
            assert_eq!(bytes(palette(index as u8).accent), color);
            for other in 0..index {
                assert_ne!(palette(index as u8), palette(other as u8));
            }
        }
    }

    #[test]
    fn every_palette_channel_is_bounded_and_opaque() {
        for index in 0..=u8::MAX {
            let colors = palette(index);
            for color in [colors.accent, colors.dim, colors.hover] {
                assert_eq!(color[3], 1.0);
                assert!(color.into_iter().all(|value| (0.0..=1.0).contains(&value)));
            }
            if index > 4 {
                assert_eq!(colors, palette(0));
            }
        }
    }

    #[test]
    fn selection_updates_all_getters_and_invalid_values_restore_mint() {
        for index in 0..=u8::MAX {
            select(index);
            let colors = palette(index);
            assert_eq!(accent(), colors.accent);
            assert_eq!(accent_dim(), colors.dim);
            assert_eq!(accent_hover(), colors.hover);
        }
        select(0);
    }

    #[test]
    fn selection_is_local_to_the_ui_thread() {
        select(3);
        let other = std::thread::spawn(|| {
            assert_eq!(accent(), ACCENT);
            select(1);
            accent()
        })
        .join()
        .expect("palette test thread completed");
        assert_eq!(other, palette(1).accent);
        assert_eq!(accent(), palette(3).accent);
        select(0);
    }
}
