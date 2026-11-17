// Copied from Palepale crates/df/src/common/chrome.rs at
// 6490070271354e2a13a30e107065338c530a7322 (build/reference/palepale).
// Product labels adapted; original shell and controls retained.
use super::theme::{BG_DEEP, BORDER, TEXT, TEXT_DIM, accent, accent_dim, accent_hover};
use imgui::{Condition, ImColor32, MouseButton, StyleColor, StyleVar, TextureId, Ui, WindowFlags};
use std::collections::HashMap;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

const SIDEBAR_W: f32 = 50.0;
const WIN_W: f32 = 780.0;
const HEADER_H: f32 = 36.0;
const LOGO_PX: f32 = 28.0;

pub struct EspPreview {
    pub box_on: bool,
    pub skel_on: bool,
    pub health_on: bool,
    pub name_on: bool,
    pub dist_on: bool,
    pub col_box: [f32; 4],
    pub col_skel: [f32; 4],
    pub col_name: [f32; 4],
    pub col_dist: [f32; 4],
    pub thick: f32,
}

pub struct Chrome {
    tab: usize,
    render_tab: usize,
    tab_alpha: f32,
    tab_slide: f32,
    wx: f32,
    wy: f32,
    toggles: HashMap<String, f32>,
    hovers: HashMap<String, f32>,
    bind: Option<String>,
    bind_wait_up: bool,
    preview_ox: f32,
    preview_oy: f32,
}

impl Default for Chrome {
    fn default() -> Self {
        Self {
            tab: 0,
            render_tab: 0,
            tab_alpha: 1.0,
            tab_slide: 0.0,
            wx: 100.0,
            wy: 64.0,
            toggles: HashMap::new(),
            hovers: HashMap::new(),
            bind: None,
            bind_wait_up: false,
            preview_ox: 110.0,
            preview_oy: 28.0,
        }
    }
}

fn down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) < 0 }
}

fn vk_label(vk: i32) -> String {
    match vk {
        0x01 => "LMB".into(),
        0x02 => "RMB".into(),
        0x04 => "MMB".into(),
        0x05 => "Mouse4".into(),
        0x06 => "Mouse5".into(),
        0x08 => "Backspace".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x10 => "Shift".into(),
        0x11 => "Ctrl".into(),
        0x12 => "Alt".into(),
        0x14 => "Caps".into(),
        0x1B => "Esc".into(),
        0x20 => "Space".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        c @ 0x30..=0x39 => format!("{}", (b'0' + (c - 0x30) as u8) as char),
        c @ 0x41..=0x5A => format!("{}", (b'A' + (c - 0x41) as u8) as char),
        c @ 0x70..=0x7B => format!("F{}", c - 0x6F),
        _ => format!("VK {vk}"),
    }
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> ImColor32 {
    ImColor32::from_rgba(r, g, b, a)
}

#[inline]
fn color_with_alpha([r, g, b, _]: [f32; 4], alpha: u8) -> ImColor32 {
    rgba(
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
        alpha,
    )
}

#[inline]
fn accent_color(alpha: u8) -> ImColor32 {
    color_with_alpha(accent(), alpha)
}

#[inline]
fn accent_shade_color(alpha: u8) -> ImColor32 {
    let [r, g, b, a] = accent();
    // Retain the original upper-gradient RGB (0, 217, 54) for Mint.
    color_with_alpha(
        [
            r * (217.0 / 255.0),
            g * (217.0 / 255.0),
            b * (54.0 / 65.0),
            a,
        ],
        alpha,
    )
}

fn draw_tab_icon(dl: &imgui::DrawListMut, cx: f32, cy: f32, s: f32, col: ImColor32, key: &str) {
    let th = 1.6;
    match key.to_ascii_uppercase().as_str() {
        "AIM" => {
            dl.add_circle([cx, cy], s * 0.62, col).thickness(th).build();
            dl.add_line([cx - s, cy], [cx - s * 0.28, cy], col)
                .thickness(th)
                .build();
            dl.add_line([cx + s * 0.28, cy], [cx + s, cy], col)
                .thickness(th)
                .build();
            dl.add_line([cx, cy - s], [cx, cy - s * 0.28], col)
                .thickness(th)
                .build();
            dl.add_line([cx, cy + s * 0.28], [cx, cy + s], col)
                .thickness(th)
                .build();
            dl.add_circle([cx, cy], 1.4, col).filled(true).build();
        }
        "VIS" | "ESP" => {
            dl.add_circle([cx, cy], s * 0.42, col).thickness(th).build();
            dl.add_circle([cx, cy], s * 0.16, col).filled(true).build();
            dl.add_bezier_curve(
                [cx - s, cy],
                [cx - s * 0.45, cy - s * 0.85],
                [cx + s * 0.45, cy - s * 0.85],
                [cx + s, cy],
                col,
            )
            .thickness(th)
            .build();
            dl.add_bezier_curve(
                [cx - s, cy],
                [cx - s * 0.45, cy + s * 0.85],
                [cx + s * 0.45, cy + s * 0.85],
                [cx + s, cy],
                col,
            )
            .thickness(th)
            .build();
        }
        "WLD" => {
            dl.add_circle([cx, cy], s * 0.85, col).thickness(th).build();
            dl.add_line([cx, cy - s * 0.85], [cx, cy + s * 0.85], col)
                .thickness(th)
                .build();
            dl.add_bezier_curve(
                [cx - s * 0.85, cy],
                [cx - s * 0.25, cy - s * 0.55],
                [cx + s * 0.25, cy - s * 0.55],
                [cx + s * 0.85, cy],
                col,
            )
            .thickness(th)
            .build();
            dl.add_bezier_curve(
                [cx - s * 0.85, cy],
                [cx - s * 0.25, cy + s * 0.55],
                [cx + s * 0.25, cy + s * 0.55],
                [cx + s * 0.85, cy],
                col,
            )
            .thickness(th)
            .build();
        }
        "GLOW" => {
            dl.add_line([cx, cy - s], [cx, cy + s], col)
                .thickness(th)
                .build();
            dl.add_line([cx - s, cy], [cx + s, cy], col)
                .thickness(th)
                .build();
            dl.add_line(
                [cx - s * 0.68, cy - s * 0.68],
                [cx + s * 0.68, cy + s * 0.68],
                col,
            )
            .thickness(th)
            .build();
            dl.add_line(
                [cx + s * 0.68, cy - s * 0.68],
                [cx - s * 0.68, cy + s * 0.68],
                col,
            )
            .thickness(th)
            .build();
            dl.add_circle([cx, cy], s * 0.28, col).filled(true).build();
        }
        "PREV" => {
            dl.add_rect(
                [cx - s * 0.7, cy - s * 0.85],
                [cx + s * 0.7, cy + s * 0.85],
                col,
            )
            .thickness(th)
            .build();
            dl.add_circle([cx, cy - s * 0.28], s * 0.22, col)
                .filled(true)
                .build();
            dl.add_line(
                [cx - s * 0.38, cy + s * 0.55],
                [cx + s * 0.38, cy + s * 0.55],
                col,
            )
            .thickness(th)
            .build();
            dl.add_line([cx, cy - s * 0.02], [cx, cy + s * 0.55], col)
                .thickness(th)
                .build();
        }
        "COL" => {
            dl.add_circle([cx - s * 0.42, cy + s * 0.18], s * 0.32, col)
                .filled(true)
                .build();
            dl.add_circle([cx + s * 0.42, cy + s * 0.18], s * 0.32, col)
                .thickness(th)
                .build();
            dl.add_circle([cx, cy - s * 0.42], s * 0.32, col)
                .thickness(th)
                .build();
        }
        "MISC" => {
            dl.add_line(
                [cx - s * 0.85, cy - s * 0.32],
                [cx + s * 0.85, cy - s * 0.32],
                col,
            )
            .thickness(th)
            .build();
            dl.add_circle([cx - s * 0.22, cy - s * 0.32], s * 0.22, col)
                .filled(true)
                .build();
            dl.add_line(
                [cx - s * 0.85, cy + s * 0.38],
                [cx + s * 0.85, cy + s * 0.38],
                col,
            )
            .thickness(th)
            .build();
            dl.add_circle([cx + s * 0.28, cy + s * 0.38], s * 0.22, col)
                .filled(true)
                .build();
        }
        "CFG" => {
            dl.add_circle([cx, cy], s * 0.38, col).thickness(th).build();
            for i in 0..6 {
                let a = i as f32 * std::f32::consts::PI / 3.0;
                let inner = s * 0.42;
                let outer = s * 0.92;
                dl.add_line(
                    [cx + a.cos() * inner, cy + a.sin() * inner],
                    [cx + a.cos() * outer, cy + a.sin() * outer],
                    col,
                )
                .thickness(th)
                .build();
            }
        }
        "POWER" => {
            dl.add_circle([cx, cy + s * 0.08], s * 0.78, col)
                .thickness(th)
                .build();
            dl.add_line([cx, cy - s], [cx, cy + s * 0.08], col)
                .thickness(th + 0.4)
                .build();
        }
        _ => {
            dl.add_rect(
                [cx - s * 0.7, cy - s * 0.7],
                [cx + s * 0.7, cy + s * 0.7],
                col,
            )
            .thickness(th)
            .build();
        }
    }
}

fn lerp_col(a: [f32; 4], b: [f32; 4], t: f32) -> ImColor32 {
    ImColor32::from_rgba_f32s(
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    )
}

impl Chrome {
    pub fn poll_insert(&mut self, open: &mut bool) {
        *open = crate::common::keys::menu_open();
    }

    pub fn shell(
        &mut self,
        ui: &Ui,
        tabs: &[&str],
        height: f32,
        body: impl FnOnce(&Ui, &mut Self, usize) -> bool,
    ) -> bool {
        let mut dirty = false;
        let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _spc = ui.push_style_var(StyleVar::ItemSpacing([5.0, 5.0]));
        let wx = self.wx;
        let wy = self.wy;
        let ds = ui.io().display_size;
        let win_w = WIN_W.min((ds[0] - 24.0).max(520.0));
        let win_h = height.max(860.0).min((ds[1] - 32.0).max(480.0));
        ui.window("EXTERNAL EVERYTHING")
            .flags(WindowFlags::NO_TITLE_BAR | WindowFlags::NO_RESIZE | WindowFlags::NO_SCROLLBAR)
            .size([win_w, win_h], Condition::Always)
            .position([wx, wy], Condition::Always)
            .collapsed(false, Condition::Always)
            .build(|| {
                dirty |= self.frame(ui, tabs, body);
            });
        dirty
    }

    fn frame(
        &mut self,
        ui: &Ui,
        tabs: &[&str],
        body: impl FnOnce(&Ui, &mut Self, usize) -> bool,
    ) -> bool {
        let mut wpos = ui.window_pos();
        let wsz = ui.window_size();
        {
            ui.set_cursor_screen_pos(wpos);
            ui.invisible_button("##paleDrag", [wsz[0], HEADER_H]);
            if ui.is_item_active() && ui.is_mouse_dragging(MouseButton::Left) {
                let d = ui.io().mouse_delta;
                let ds = ui.io().display_size;
                self.wx = (self.wx + d[0]).clamp(0.0, (ds[0] - wsz[0]).max(0.0));
                self.wy = (self.wy + d[1]).clamp(0.0, (ds[1] - 80.0).max(0.0));
                wpos = [self.wx, self.wy];
            }
            let hov = ui.is_item_hovered();
            let strip = if hov {
                accent_color(48)
            } else {
                accent_color(18)
            };
            let dl = ui.get_window_draw_list();
            dl.add_rect(wpos, [wpos[0] + wsz[0], wpos[1] + HEADER_H], strip)
                .filled(true)
                .build();
        }
        let t = (ui.time() % 2.0) as f32 / 2.0;
        let pulse = (t * 2.0 * std::f32::consts::PI).sin() * 0.1 + 0.9;
        {
            let dl = ui.get_window_draw_list();
            dl.add_rect_filled_multicolor(
                wpos,
                [wpos[0] + wsz[0], wpos[1] + wsz[1]],
                accent_color(10),
                accent_shade_color(8),
                rgba(0, 0, 0, 10),
                rgba(0, 0, 0, 10),
            );
            for i in 1..=4 {
                let a = ((0.16 - i as f32 * 0.032) * pulse * 255.0) as u8;
                let o = i as f32;
                dl.add_rect(
                    [wpos[0] - o, wpos[1] - o],
                    [wpos[0] + wsz[0] + o, wpos[1] + wsz[1] + o],
                    accent_color(a),
                )
                .thickness(1.0)
                .build();
            }
            dl.add_rect(
                wpos,
                [wpos[0] + wsz[0], wpos[1] + wsz[1]],
                rgba(36, 36, 36, 255),
            )
            .thickness(1.0)
            .build();
            dl.add_rect(
                wpos,
                [wpos[0] + SIDEBAR_W, wpos[1] + wsz[1]],
                rgba(0, 0, 0, 255),
            )
            .filled(true)
            .build();
            dl.add_rect_filled_multicolor(
                wpos,
                [wpos[0] + SIDEBAR_W, wpos[1] + wsz[1]],
                accent_color(40),
                color_with_alpha(accent_dim(), 50),
                rgba(0, 0, 0, 40),
                rgba(0, 0, 0, 40),
            );
            dl.add_line(
                [wpos[0] + SIDEBAR_W, wpos[1]],
                [wpos[0] + SIDEBAR_W, wpos[1] + wsz[1]],
                rgba(36, 36, 36, 255),
            )
            .build();
            let lx = wpos[0] + (SIDEBAR_W - LOGO_PX) * 0.5;
            let ly = wpos[1] + (HEADER_H - LOGO_PX) * 0.5;
            dl.add_image(TextureId::new(2), [lx, ly], [lx + LOGO_PX, ly + LOGO_PX])
                .build();
            dl.add_text(
                [wpos[0] + SIDEBAR_W + 10.0, wpos[1] + 10.0],
                accent_color(210),
                "EXTERNAL EVERYTHING",
            );
            dl.add_line(
                [wpos[0] + 6.0, wpos[1] + HEADER_H + 4.0],
                [wpos[0] + SIDEBAR_W - 6.0, wpos[1] + HEADER_H + 4.0],
                rgba(255, 255, 255, 60),
            )
            .build();
        }
        for (i, label) in tabs.iter().enumerate() {
            ui.set_cursor_screen_pos([wpos[0], wpos[1] + HEADER_H + 8.0 + i as f32 * 36.0]);
            self.nav_button(ui, label, i, SIDEBAR_W);
        }
        if self.render_tab != self.tab {
            self.tab_alpha += (0.0 - self.tab_alpha) * 0.25;
            if self.tab_alpha < 0.05 {
                self.render_tab = self.tab;
                self.tab_slide = 22.0;
            }
        } else {
            self.tab_alpha += (1.0 - self.tab_alpha) * 0.20;
            self.tab_slide += (0.0 - self.tab_slide) * 0.16;
        }
        ui.set_cursor_screen_pos([wpos[0] + SIDEBAR_W + 10.0, wpos[1] + HEADER_H + 4.0]);
        let _cpad = ui.push_style_var(StyleVar::WindowPadding([8.0, 8.0]));
        let _bar = ui.push_style_var(StyleVar::ScrollbarSize(16.0));
        let tab = self.render_tab;
        let mut dirty = false;
        ui.child_window("##paleContent")
            .size([wsz[0] - SIDEBAR_W - 20.0, wsz[1] - HEADER_H - 12.0])
            .scroll_bar(true)
            .scrollable(true)
            .always_vertical_scrollbar(true)
            .build(|| {
                let _a = ui.push_style_var(StyleVar::Alpha(self.tab_alpha.clamp(0.05, 1.0)));
                ui.set_cursor_pos([ui.cursor_pos()[0], ui.cursor_pos()[1] + self.tab_slide]);
                dirty = body(ui, self, tab);
            });
        dirty
    }

    fn nav_button(&mut self, ui: &Ui, label: &str, idx: usize, width: f32) {
        let pos = ui.cursor_screen_pos();
        let h = 36.0;
        ui.invisible_button(label, [width, h]);
        if ui.is_item_clicked() {
            self.tab = idx;
        }
        let active = idx == self.tab;
        let hovered = ui.is_item_hovered();
        let bg = if active {
            rgba(0, 0, 0, 130)
        } else if hovered {
            rgba(0, 0, 0, 70)
        } else {
            rgba(0, 0, 0, 0)
        };
        let tx = if active {
            accent_color(255)
        } else if hovered {
            color_with_alpha(accent_hover(), 255)
        } else {
            rgba(138, 138, 138, 255)
        };
        let dl = ui.get_window_draw_list();
        dl.add_rect(pos, [pos[0] + width, pos[1] + h], bg)
            .filled(true)
            .build();
        if active {
            dl.add_rect(
                [pos[0], pos[1] + 4.0],
                [pos[0] + 3.0, pos[1] + h - 4.0],
                accent_color(255),
            )
            .filled(true)
            .build();
        }
        let cx = pos[0] + width * 0.5;
        let cy = pos[1] + h * 0.5;
        draw_tab_icon(&dl, cx, cy, 8.0, tx, label);
        if hovered {
            ui.tooltip_text(label);
        }
    }

    pub fn esp_preview(&mut self, ui: &Ui, spec: EspPreview) {
        self.esp_preview_with_health_mode(ui, spec, false);
    }

    pub fn esp_preview_with_health_mode(&mut self, ui: &Ui, spec: EspPreview, health_number: bool) {
        const CW: f32 = 360.0;
        const CH: f32 = 268.0;
        const DW: f32 = 72.0;
        const DH: f32 = 196.0;
        let origin = ui.cursor_screen_pos();
        ui.invisible_button("##espPreview", [CW, CH]);
        if ui.is_item_active() && ui.is_mouse_dragging(MouseButton::Left) {
            let d = ui.io().mouse_delta;
            self.preview_ox = (self.preview_ox + d[0]).clamp(12.0, CW - DW - 12.0);
            self.preview_oy = (self.preview_oy + d[1]).clamp(24.0, CH - DH - 28.0);
        }
        let dl = ui.get_window_draw_list();
        dl.add_rect(origin, [origin[0] + CW, origin[1] + CH], rgba(0, 0, 0, 170))
            .filled(true)
            .build();
        dl.add_rect(
            origin,
            [origin[0] + CW, origin[1] + CH],
            rgba(36, 36, 36, 255),
        )
        .thickness(1.0)
        .build();
        let hint = if ui.is_item_hovered() {
            "drag dummy  ·  live ESP toggles / colors"
        } else {
            "drag dummy"
        };
        dl.add_text([origin[0] + 10.0, origin[1] + 8.0], accent_color(160), hint);
        let ox = origin[0] + self.preview_ox;
        let oy = origin[1] + self.preview_oy;
        let pt = |x: f32, y: f32| [ox + x, oy + y];
        let box_c = ImColor32::from_rgba_f32s(
            spec.col_box[0],
            spec.col_box[1],
            spec.col_box[2],
            spec.col_box[3],
        );
        let skel_c = ImColor32::from_rgba_f32s(
            spec.col_skel[0],
            spec.col_skel[1],
            spec.col_skel[2],
            spec.col_skel[3],
        );
        let name_c = ImColor32::from_rgba_f32s(
            spec.col_name[0],
            spec.col_name[1],
            spec.col_name[2],
            spec.col_name[3],
        );
        let dist_c = ImColor32::from_rgba_f32s(
            spec.col_dist[0],
            spec.col_dist[1],
            spec.col_dist[2],
            spec.col_dist[3],
        );
        if spec.box_on {
            dl.add_rect(pt(0.0, 0.0), pt(DW, DH), box_c)
                .thickness(1.4)
                .build();
        }
        if spec.skel_on {
            let th = spec.thick.clamp(0.6, 3.0);
            let head = pt(DW * 0.5, 18.0);
            let neck = pt(DW * 0.5, 32.0);
            let chest = pt(DW * 0.5, 58.0);
            let pelvis = pt(DW * 0.5, 108.0);
            let l_sh = pt(18.0, 42.0);
            let r_sh = pt(DW - 18.0, 42.0);
            let l_el = pt(12.0, 72.0);
            let r_el = pt(DW - 12.0, 72.0);
            let l_h = pt(10.0, 100.0);
            let r_h = pt(DW - 10.0, 100.0);
            let l_hip = pt(28.0, 112.0);
            let r_hip = pt(DW - 28.0, 112.0);
            let l_kn = pt(26.0, 150.0);
            let r_kn = pt(DW - 26.0, 150.0);
            let l_ft = pt(24.0, 186.0);
            let r_ft = pt(DW - 24.0, 186.0);
            dl.add_circle(head, 10.0, skel_c).thickness(th).build();
            for (a, b) in [
                (head, neck),
                (neck, chest),
                (chest, pelvis),
                (neck, l_sh),
                (neck, r_sh),
                (l_sh, l_el),
                (r_sh, r_el),
                (l_el, l_h),
                (r_el, r_h),
                (pelvis, l_hip),
                (pelvis, r_hip),
                (l_hip, l_kn),
                (r_hip, r_kn),
                (l_kn, l_ft),
                (r_kn, r_ft),
            ] {
                dl.add_line(a, b, skel_c).thickness(th).build();
            }
        }
        if spec.health_on && health_number {
            dl.add_text(pt(DW + 6.0, 0.0), rgba(51, 230, 77, 230), "82 HP");
        } else if spec.health_on {
            let bw = 3.0;
            let bx = ox - bw - 3.0;
            let frac = 0.82;
            dl.add_rect([bx, oy], [bx + bw, oy + DH], rgba(20, 20, 20, 180))
                .filled(true)
                .build();
            let fh = DH * frac;
            dl.add_rect(
                [bx, oy + (DH - fh)],
                [bx + bw, oy + DH],
                rgba(51, 230, 77, 230),
            )
            .filled(true)
            .build();
        }
        if spec.name_on {
            dl.add_text(pt(8.0, -16.0), name_c, "bot");
        }
        if spec.dist_on {
            dl.add_text(pt(18.0, DH + 4.0), dist_c, "12m");
        }
        if !spec.box_on && !spec.skel_on && !spec.health_on && !spec.name_on && !spec.dist_on {
            dl.add_text(
                [origin[0] + 18.0, origin[1] + CH * 0.5],
                rgba(138, 138, 138, 200),
                "enable Box / Skeleton on VIS",
            );
        }
        ui.dummy([0.0, 4.0]);
        let _t = ui.push_style_color(StyleColor::Text, TEXT_DIM);
        ui.text("preview uses VIS toggles and COL colors  ·  drag the dummy");
    }

    pub fn section(ui: &Ui, label: &str) {
        let _c = ui.push_style_color(StyleColor::Text, accent());
        ui.text(label);
        ui.separator();
        ui.spacing();
    }

    pub fn toggle(&mut self, ui: &Ui, label: &str, val: &mut bool) -> bool {
        let pos = ui.cursor_screen_pos();
        let w = 26.0;
        let h = 14.0;
        ui.invisible_button(label, [w, h]);
        let clicked = ui.is_item_clicked();
        let hovered = ui.is_item_hovered();
        if clicked {
            *val = !*val;
        }
        let anim = self.toggles.entry(label.to_string()).or_insert(0.0);
        *anim += ((*val as u8 as f32) - *anim) * 0.3;
        let a = *anim;
        let hv = self.hovers.entry(label.to_string()).or_insert(0.0);
        if hovered {
            *hv += (1.0 - *hv) * 0.20;
        } else {
            *hv += (0.0 - *hv) * 0.20;
        }
        let scale = 1.0 + *hv * 0.08;
        let fw = w * scale;
        let fh = h * scale;
        let ox = (w - fw) * 0.5;
        let oy = (h - fh) * 0.5;
        let track = lerp_col(BG_DEEP, accent_dim(), a);
        let rim = lerp_col(BORDER, accent(), a);
        let knob = lerp_col([0.2, 0.2, 0.2, 1.0], accent(), a);
        let kx = pos[0] + ox + 3.0 * scale + a * (fw - fh);
        {
            let dl = ui.get_window_draw_list();
            dl.add_rect(
                [pos[0] + ox, pos[1] + oy],
                [pos[0] + ox + fw, pos[1] + oy + fh],
                track,
            )
            .filled(true)
            .build();
            dl.add_rect(
                [pos[0] + ox, pos[1] + oy],
                [pos[0] + ox + fw, pos[1] + oy + fh],
                rim,
            )
            .thickness(1.0)
            .build();
            dl.add_rect(
                [kx, pos[1] + oy + 2.0 * scale],
                [kx + fh - 4.0 * scale, pos[1] + oy + fh - 2.0 * scale],
                knob,
            )
            .filled(true)
            .build();
        }
        ui.same_line_with_spacing(0.0, 8.0);
        let _c = ui.push_style_color(StyleColor::Text, if *val { TEXT } else { TEXT_DIM });
        ui.text(label);
        clicked
    }

    pub fn slider(
        &mut self,
        ui: &Ui,
        caption: &str,
        id: &str,
        val: &mut f32,
        mn: f32,
        mx: f32,
        fmt: &str,
    ) -> bool {
        let _c = ui.push_style_color(StyleColor::Text, TEXT_DIM);
        ui.text(caption);
        let _b = ui.push_style_color(StyleColor::Border, BORDER);
        ui.set_next_item_width(ui.content_region_avail()[0]);
        ui.slider_config(id, mn, mx).display_format(fmt).build(val)
    }

    pub fn slider_i(
        &mut self,
        ui: &Ui,
        caption: &str,
        id: &str,
        val: &mut i32,
        mn: i32,
        mx: i32,
    ) -> bool {
        let _c = ui.push_style_color(StyleColor::Text, TEXT_DIM);
        ui.text(caption);
        ui.set_next_item_width(ui.content_region_avail()[0]);
        ui.slider_config(id, mn, mx).build(val)
    }

    pub fn color(&mut self, ui: &Ui, label: &str, c: &mut [f32; 4]) -> bool {
        let _t = ui.push_style_color(StyleColor::Text, TEXT_DIM);
        ui.text(label);
        ui.same_line();
        ui.color_edit4_config(label, c)
            .alpha(true)
            .inputs(false)
            .build()
    }

    pub fn bind(&mut self, ui: &Ui, label: &str, vk: &mut i32) -> bool {
        let armed = self.bind.as_deref() == Some(label);
        let caption = if armed {
            format!("{label}: press key…")
        } else if *vk <= 0 {
            format!("{label}: none")
        } else {
            format!("{label}: {}", vk_label(*vk))
        };
        if ui.button(&caption) {
            self.bind = Some(label.to_string());
            self.bind_wait_up = true;
        }
        if armed {
            if self.bind_wait_up {
                if !down(0x01) {
                    self.bind_wait_up = false;
                }
                return false;
            }
            // Esc clears
            if down(0x1B) {
                *vk = 0;
                self.bind = None;
                return true;
            }
            for i in 1..=254 {
                if i == 0x2D || i == 0x1B {
                    continue;
                }
                if down(i) {
                    *vk = i;
                    self.bind = None;
                    return true;
                }
            }
        }
        false
    }

    pub fn foot(ui: &Ui, lines: &[&str]) {
        ui.spacing();
        if ui.button("QUIT overlay") {
            crate::common::keys::request_quit();
        }
        ui.spacing();
        let _t = ui.push_style_color(StyleColor::Text, TEXT_DIM);
        for line in lines {
            ui.text(*line);
        }
    }

    pub fn cfg_tab(ui: &Ui, path: &str, extra: &str, mut slot: impl FnMut(u8, bool)) -> bool {
        let mut dirty = false;
        Chrome::section(ui, "CONFIG");
        for n in 1u8..=3 {
            if ui.button(format!("Save slot {n}")) {
                slot(n, false);
            }
            ui.same_line();
            if ui.button(format!("Load {n}")) {
                slot(n, true);
                dirty = true;
            }
        }
        Chrome::foot(ui, &[path, extra]);
        dirty
    }
}
