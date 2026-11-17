// Drawing primitives adapted from amangly/palepale@6490070271354e2a13a30e107065338c530a7322
// crates/df/src/pubg/esp.rs: box, shadowed label and four-segment crosshair.
// The frozen PointBlank reader supplies the only entity and projection data.
#[cfg(any(windows, test))]
use super::settings::CrosshairStyle;
#[cfg(windows)]
use super::settings::Settings;
#[cfg(windows)]
use crate::{common::overlay::Overlay, games::pointblank::Frame};

#[derive(Clone, Copy, Debug)]
struct Viewport {
    source: [f32; 2],
    offset: [f32; 2],
    scale: f32,
}

impl Viewport {
    /// Preserve the camera aspect ratio, centering any letterboxing inside the
    /// physical game-client viewport selected by the original Overlay::sync.
    fn fit(source: [f32; 2], target: [f32; 2]) -> Option<Self> {
        if source
            .into_iter()
            .chain(target)
            .any(|value| !value.is_finite() || value <= 0.0)
        {
            return None;
        }
        let scale = (target[0] / source[0]).min(target[1] / source[1]);
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        Some(Self {
            source,
            offset: [
                (target[0] - source[0] * scale) * 0.5,
                (target[1] - source[1] * scale) * 0.5,
            ],
            scale,
        })
    }

    fn rect(self, screen: [f32; 4]) -> Option<[f32; 4]> {
        let [left, top, width, height] = screen;
        if screen.into_iter().any(|value| !value.is_finite()) || width <= 0.0 || height <= 0.0 {
            return None;
        }
        let right = left + width;
        let bottom = top + height;
        if !right.is_finite()
            || !bottom.is_finite()
            || right <= 0.0
            || bottom <= 0.0
            || left >= self.source[0]
            || top >= self.source[1]
        {
            return None;
        }
        // Keep every primitive within the source camera viewport, including
        // partially visible boxes at the edges of a letterboxed game window.
        let x0 = left.max(0.0);
        let y0 = top.max(0.0);
        let x1 = right.min(self.source[0]);
        let y1 = bottom.min(self.source[1]);
        Some([
            self.offset[0] + x0 * self.scale,
            self.offset[1] + y0 * self.scale,
            (x1 - x0) * self.scale,
            (y1 - y0) * self.scale,
        ])
    }

    fn center(self) -> [f32; 2] {
        [
            self.offset[0] + self.source[0] * self.scale * 0.5,
            self.offset[1] + self.source[1] * self.scale * 0.5,
        ]
    }
}

#[cfg(windows)]
fn label(overlay: &Overlay, x: f32, y: f32, text: &str) {
    overlay.text(x + 1.0, y + 1.0, text, 0.0, 0.0, 0.0, 0.85, false);
    overlay.text(x, y, text, 0.2, 0.9, 0.3, 1.0, false);
}

#[cfg(windows)]
pub fn draw(overlay: &Overlay, frame: &Frame, settings: &Settings) -> usize {
    if !settings.draw_any() {
        return 0;
    }
    let target = [overlay.w as f32, overlay.h as f32];
    let source = if frame.width > 0. && frame.height > 0. {
        [frame.width, frame.height]
    } else {
        target
    };
    let Some(view) = Viewport::fit(source, target) else {
        return 0;
    };
    let mut projected = 0;
    let [r, g, b, a] = settings.col_box;
    for entity in &frame.entities {
        if !entity.health.is_finite() || entity.health <= 0.0 {
            continue;
        }
        let Some(screen) = entity.screen else {
            continue;
        };
        let Some([x, y, width, height]) = view.rect(screen) else {
            continue;
        };
        if settings.boxes || settings.health {
            projected += 1;
        }
        if settings.boxes {
            overlay.rect(x, y, width, height, r, g, b, a, 2.0);
        }
        if settings.health {
            // Maximum health has yet to be established for this build. Preserve
            // the observed float instead of inventing a 100-HP percentage bar.
            let text = format!("{:.1} HP", entity.health);
            let text_width = overlay.measure(&text, false);
            let right = view.offset[0] + view.source[0] * view.scale;
            let bottom = view.offset[1] + view.source[1] * view.scale;
            let tx = (x + width + 4.0)
                .min(right - text_width)
                .max(view.offset[0]);
            let ty = y
                .min((bottom - 18.0).max(view.offset[1]))
                .max(view.offset[1]);
            label(overlay, tx, ty, &text);
        }
    }
    if settings.crosshair {
        let [cx, cy] = view.center();
        if settings.crosshair_style != CrosshairStyle::Segments {
            let (rectangles, count, color) =
                crosshair_rectangles(settings.crosshair_style, [cx, cy]);
            let [r, g, b, a] = color;
            for [x, y, w, h] in rectangles.into_iter().take(count) {
                overlay.fill(x, y, w, h, r, g, b, a);
            }
            return projected;
        }
        let ox = view.source[0] * view.scale * 0.007875;
        let oy = view.source[1] * view.scale * 0.014;
        let size = 5.0;
        overlay.line(cx, cy - oy, cx, cy - oy - size, 0.0, 1.0, 0.0, 1.0, 2.0);
        overlay.line(cx, cy + oy, cx, cy + oy + size, 0.0, 1.0, 0.0, 1.0, 2.0);
        overlay.line(cx - ox, cy, cx - ox - size, cy, 0.0, 1.0, 0.0, 1.0, 2.0);
        overlay.line(cx + ox, cy, cx + ox + size, cy, 0.0, 1.0, 0.0, 1.0, 2.0);
    }
    projected
}

#[cfg(any(windows, test))]
fn crosshair_rectangles(
    style: CrosshairStyle,
    [cx, cy]: [f32; 2],
) -> ([[f32; 4]; 2], usize, [f32; 4]) {
    match style {
        CrosshairStyle::Dot => ([[cx - 2., cy - 2., 4., 4.], [0.; 4]], 1, [1., 0., 0., 1.]),
        CrosshairStyle::Cross => (
            [[cx - 10., cy - 1., 20., 2.], [cx - 1., cy - 10., 2., 20.]],
            2,
            [0., 1., 0., 1.],
        ),
        CrosshairStyle::Segments => ([[0.; 4]; 2], 0, [0., 1., 0., 1.]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_dot_and_cross_have_exact_centered_pixel_dimensions() {
        assert_eq!(
            crosshair_rectangles(CrosshairStyle::Dot, [800., 600.]),
            ([[798., 598., 4., 4.], [0.; 4]], 1, [1., 0., 0., 1.])
        );
        assert_eq!(
            crosshair_rectangles(CrosshairStyle::Cross, [800., 600.]),
            (
                [[790., 599., 20., 2.], [799., 590., 2., 20.]],
                2,
                [0., 1., 0., 1.]
            )
        );
        assert_eq!(
            crosshair_rectangles(CrosshairStyle::Segments, [800., 600.]).1,
            0
        );
    }

    #[test]
    fn same_aspect_scales_the_actual_camera_box() {
        let viewport = Viewport::fit([1024.0, 768.0], [2048.0, 1536.0]).unwrap();
        assert_eq!(
            viewport.rect([100.0, 200.0, 20.0, 60.0]),
            Some([200.0, 400.0, 40.0, 120.0])
        );
        assert_eq!(viewport.center(), [1024.0, 768.0]);
    }

    #[test]
    fn wide_target_keeps_four_by_three_camera_letterboxed() {
        let viewport = Viewport::fit([800.0, 600.0], [1600.0, 900.0]).unwrap();
        assert_eq!(
            viewport.rect([0.0, 0.0, 800.0, 600.0]),
            Some([200.0, 0.0, 1200.0, 900.0])
        );
        assert_eq!(viewport.center(), [800.0, 450.0]);
    }

    #[test]
    fn tall_target_preserves_vertical_letterboxing() {
        let viewport = Viewport::fit([1600.0, 900.0], [800.0, 600.0]).unwrap();
        assert_eq!(
            viewport.rect([0.0, 0.0, 1600.0, 900.0]),
            Some([0.0, 75.0, 800.0, 450.0])
        );
    }

    #[test]
    fn projection_rejects_invalid_and_fully_offscreen_rectangles() {
        let viewport = Viewport::fit([800.0, 600.0], [800.0, 600.0]).unwrap();
        for invalid in [
            [f32::NAN, 0.0, 10.0, 20.0],
            [0.0, 0.0, -1.0, 1.0],
            [801.0, 1.0, 10.0, 10.0],
            [-20.0, 0.0, 10.0, 10.0],
            [0.0, 601.0, 10.0, 10.0],
            [0.0, -20.0, 10.0, 10.0],
        ] {
            assert!(viewport.rect(invalid).is_none());
        }
        assert!(Viewport::fit([0.0, 600.0], [800.0, 600.0]).is_none());
        assert!(Viewport::fit([800.0, 600.0], [f32::INFINITY, 600.0]).is_none());
    }

    #[test]
    fn partial_boxes_are_clipped_inside_the_camera_content() {
        let viewport = Viewport::fit([800.0, 600.0], [1600.0, 900.0]).unwrap();
        assert_eq!(
            viewport.rect([-20.0, -10.0, 40.0, 30.0]),
            Some([200.0, 0.0, 30.0, 30.0])
        );
        assert_eq!(
            viewport.rect([790.0, 590.0, 50.0, 50.0]),
            Some([1385.0, 885.0, 15.0, 15.0])
        );
    }
}
