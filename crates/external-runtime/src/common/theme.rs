// Copied unchanged from Palepale crates/df/src/common/theme.rs at
// 6490070271354e2a13a30e107065338c530a7322 (build/reference/palepale).
use imgui::{Style, StyleColor};

pub const ACCENT: [f32; 4] = [0.00, 1.00, 0.255, 1.00];
pub const ACCENT_DIM: [f32; 4] = [0.051, 0.149, 0.078, 1.00];
pub const ACCENT_HOVER: [f32; 4] = [0.196, 1.00, 0.404, 1.00];
pub const BG_DEEP: [f32; 4] = [0.00, 0.00, 0.00, 0.99];
pub const BG_PANEL: [f32; 4] = [0.020, 0.020, 0.020, 1.00];
pub const BORDER: [f32; 4] = [0.141, 0.141, 0.141, 1.00];
pub const TEXT: [f32; 4] = [0.910, 0.910, 0.910, 1.00];
pub const TEXT_DIM: [f32; 4] = [0.541, 0.541, 0.541, 1.00];

pub fn apply(s: &mut Style) {
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
    s[StyleColor::ScrollbarGrab] = ACCENT_DIM;
    s[StyleColor::ScrollbarGrabHovered] = ACCENT_HOVER;
    s[StyleColor::ScrollbarGrabActive] = ACCENT;
    s[StyleColor::CheckMark] = ACCENT;
    s[StyleColor::SliderGrab] = ACCENT_DIM;
    s[StyleColor::SliderGrabActive] = ACCENT;
    s[StyleColor::Button] = BG_PANEL;
    s[StyleColor::ButtonHovered] = BORDER;
    s[StyleColor::ButtonActive] = ACCENT_DIM;
    s[StyleColor::Header] = BG_PANEL;
    s[StyleColor::HeaderHovered] = BORDER;
    s[StyleColor::HeaderActive] = ACCENT_DIM;
    s[StyleColor::Separator] = BORDER;
    s[StyleColor::Tab] = BG_DEEP;
    s[StyleColor::TabHovered] = BORDER;
    s[StyleColor::TabActive] = BG_PANEL;
    s[StyleColor::Text] = TEXT;
    s[StyleColor::TextDisabled] = TEXT_DIM;
}
