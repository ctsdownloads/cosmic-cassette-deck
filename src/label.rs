//! Label rasterizer, per-skin orientation.
//!
//! Vertical labels (portrait cassette) are drawn into a horizontal strip
//! and rotated 90° CCW so they read bottom-to-top. Horizontal labels are
//! drawn directly. Regenerated only on track or skin change.

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use cosmic::widget::image;

use crate::cassette::SkinSpec;

const FONT_TITLE: &[u8] = include_bytes!("../assets/DejaVuSans-Bold.ttf");
pub const FONT_SUB: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/DejaVuSans-Bold.ttf");

/// Supersample so the label stays crisp above native skin resolution.
const SS: u32 = 2;

const INK_TITLE: [u8; 3] = [32, 31, 33];
const INK_SUB: [u8; 3] = [58, 56, 58];
const INK_RULE: [u8; 4] = [70, 68, 70, 255];

pub fn render(song: &str, album: &str, spec: &SkinSpec) -> image::Handle {
    let label_w = (spec.label.2 - spec.label.0) as u32;
    let label_h = (spec.label.3 - spec.label.1) as u32;

    if spec.label_vertical {
        render_vertical(song, album, label_w, label_h)
    } else {
        render_horizontal(song, album, label_w, label_h)
    }
}

/// Horizontal strip rotated 90° CCW: reads bottom-to-top.
fn render_vertical(song: &str, album: &str, label_w: u32, label_h: u32) -> image::Handle {
    // Strip: width = label height, height = label width.
    let sw = label_h * SS;
    let sh = label_w * SS;
    let mut strip = vec![0u8; (sw * sh * 4) as usize];

    let title_font = FontRef::try_from_slice(FONT_TITLE).expect("embedded title font");
    let sub_font = FontRef::try_from_slice(FONT_SUB).expect("embedded sub font");

    let margin = 30.0 * SS as f32;
    let max_w = sw as f32 - 2.0 * margin;

    draw_centered(&mut strip, sw, sh, &title_font, 52.0 * SS as f32, song, 54.0 * SS as f32, INK_TITLE, max_w);

    let rule_y = (74 * SS) as usize;
    for dy in 0..(2 * SS as usize) {
        let y = rule_y + dy;
        for x in (margin as usize)..(sw as usize - margin as usize) {
            put(&mut strip, sw, x, y, INK_RULE);
        }
    }

    draw_centered(&mut strip, sw, sh, &sub_font, 26.0 * SS as f32, album, 103.0 * SS as f32, INK_SUB, max_w);

    // Rotate 90° CW: dst(x, y) = src(y, sh − 1 − x).
    // (The label sits on the RIGHT of the deck now; CW rotation makes the
    // title read top-to-bottom correctly. CCW would render it upside down.)
    let (ow, oh) = (sh, sw);
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for y in 0..oh {
        for x in 0..ow {
            let si = (((sh - 1 - x) * sw + (y)) * 4) as usize;
            let di = ((y * ow + x) * 4) as usize;
            out[di..di + 4].copy_from_slice(&strip[si..si + 4]);
        }
    }

    image::Handle::from_rgba(ow, oh, out)
}

/// Drawn directly. Tall labels get two lines; short strips get one
/// combined "Song - Album" line.
fn render_horizontal(song: &str, album: &str, label_w: u32, label_h: u32) -> image::Handle {
    let w = label_w * SS;
    let h = label_h * SS;
    let mut buf = vec![0u8; (w * h * 4) as usize];

    let title_font = FontRef::try_from_slice(FONT_TITLE).expect("embedded title font");
    let sub_font = FontRef::try_from_slice(FONT_SUB).expect("embedded sub font");

    let margin = 12.0 * SS as f32;
    let max_w = w as f32 - 2.0 * margin;

    if label_h >= 100 {
        let title_px = (label_h as f32 * 0.42) * SS as f32;
        let sub_px = (label_h as f32 * 0.24) * SS as f32;
        draw_centered(&mut buf, w, h, &title_font, title_px, song, title_px * 1.05, INK_TITLE, max_w);
        draw_centered(
            &mut buf, w, h, &sub_font, sub_px, album,
            title_px * 1.05 + sub_px * 1.35, INK_SUB, max_w,
        );
    } else {
        let px = (label_h as f32 * 0.52) * SS as f32;
        let line = format!("{song} - {album}");
        // Vertically centered baseline ≈ mid + 0.35·px.
        draw_centered(&mut buf, w, h, &title_font, px, &line, h as f32 / 2.0 + px * 0.35, INK_TITLE, max_w);
    }

    image::Handle::from_rgba(w, h, buf)
}

fn draw_centered(
    buf: &mut [u8],
    w: u32,
    h: u32,
    font: &FontRef,
    px: f32,
    text: &str,
    baseline: f32,
    ink: [u8; 3],
    max_w: f32,
) {
    let scaled = font.as_scaled(PxScale::from(px));

    let measure = |s: &str| -> f32 {
        let mut width = 0.0;
        let mut last = None;
        for c in s.chars() {
            let id = scaled.glyph_id(c);
            if let Some(prev) = last {
                width += scaled.kern(prev, id);
            }
            width += scaled.h_advance(id);
            last = Some(id);
        }
        width
    };

    let mut line: String = text.to_string();
    if measure(&line) > max_w {
        while !line.is_empty() && measure(&format!("{line}...")) > max_w {
            line.pop();
        }
        line.push_str("...");
    }

    let width = measure(&line);
    let mut caret = (w as f32 - width) / 2.0;
    let mut last = None;

    for c in line.chars() {
        let id = scaled.glyph_id(c);
        if let Some(prev) = last {
            caret += scaled.kern(prev, id);
        }
        let glyph = id.with_scale_and_position(PxScale::from(px), point(caret, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let pb = outlined.px_bounds();
            outlined.draw(|gx, gy, cov| {
                let x = pb.min.x as i32 + gx as i32;
                let y = pb.min.y as i32 + gy as i32;
                if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h && cov > 0.0 {
                    let a = (cov * 255.0) as u8;
                    blend_max(buf, w, x as u32, y as u32, [ink[0], ink[1], ink[2], a]);
                }
            });
        }
        caret += scaled.h_advance(id);
        last = Some(id);
    }
}

#[inline]
fn put(buf: &mut [u8], w: u32, x: usize, y: usize, rgba: [u8; 4]) {
    let i = (y * w as usize + x) * 4;
    buf[i..i + 4].copy_from_slice(&rgba);
}

#[inline]
fn blend_max(buf: &mut [u8], w: u32, x: u32, y: u32, rgba: [u8; 4]) {
    let i = ((y * w + x) * 4) as usize;
    if rgba[3] >= buf[i + 3] {
        buf[i..i + 4].copy_from_slice(&rgba);
    }
}
