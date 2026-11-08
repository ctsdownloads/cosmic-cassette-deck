//! J-card: the printed insert folded inside a store-bought cassette case,
//! listing the album's songs. One side is drawn at a time (SIDE A or SIDE B),
//! typewriter style on aged paper, with dot leaders and running times.
//!
//! The song list comes from the files' own tags and lengths. Songs are split
//! between the two sides by running time, like a real tape; the album still
//! plays straight through.

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use cosmic::widget::image;

pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/CourierPrime-Regular.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/CourierPrime-Bold.ttf");

/// Drawn at twice the panel's design size so it stays sharp when scaled.
const S: f32 = 2.0;
/// One panel, in design units (about 63.5 x 101.6 mm at 8 units per mm).
const PW: f32 = 508.0;
const PH: f32 = 812.0;
const MARGIN: f32 = 34.0;

const PAPER: [f32; 3] = [238.0, 232.0, 216.0];
const PAPER_EDGE: [f32; 3] = [205.0, 190.0, 150.0];
const INK: [u8; 4] = [28, 26, 30, 235];
const INK_LEADER: [u8; 4] = [28, 26, 30, 150];
const PENCIL: [u8; 4] = [90, 90, 95, 170];

/// Index of the first song on side B: the split falls where the running
/// time passes the halfway mark. Without lengths, it splits by count.
pub fn side_b_start(durations: &[f32]) -> usize {
    let total: f32 = durations.iter().sum();
    if total <= 0.0 {
        return durations.len().div_ceil(2);
    }
    let mut acc = 0.0;
    for (i, d) in durations.iter().enumerate() {
        if acc + d / 2.0 > total / 2.0 {
            return i;
        }
        acc += d;
    }
    durations.len()
}

/// "m:ss", or nothing when the length is unknown.
fn fmt(secs: f32) -> String {
    if secs <= 0.0 {
        return String::new();
    }
    let s = secs.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

struct Fonts<'a> {
    regular: FontRef<'a>,
    bold: FontRef<'a>,
    fallback: FontRef<'a>,
}

impl<'a> Fonts<'a> {
    /// The font that has this character: Courier Prime, else DejaVu Sans.
    fn pick(&self, bold: bool, c: char) -> &FontRef<'a> {
        let f = if bold { &self.bold } else { &self.regular };
        if f.glyph_id(c).0 != 0 || c.is_whitespace() {
            f
        } else {
            &self.fallback
        }
    }

    fn width(&self, text: &str, size: f32, bold: bool) -> f32 {
        let scale = PxScale::from(size);
        let mut w = 0.0;
        for c in text.chars() {
            let f = self.pick(bold, c).as_scaled(scale);
            w += f.h_advance(f.glyph_id(c));
        }
        w
    }
}

struct Canvas {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Canvas {
    fn blend(&mut self, x: i64, y: i64, c: [u8; 4], cov: f32) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let a = cov.clamp(0.0, 1.0) * c[3] as f32 / 255.0;
        if a <= 0.0 {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 4;
        for k in 0..3 {
            let d = self.px[i + k] as f32;
            self.px[i + k] = (d + (c[k] as f32 - d) * a).round() as u8;
        }
    }

    /// Text with its top at `y`. Returns its width.
    fn text(&mut self, fonts: &Fonts, x: f32, y: f32, text: &str, size: f32, bold: bool, c: [u8; 4]) -> f32 {
        let scale = PxScale::from(size);
        let base = fonts.regular.as_scaled(scale).ascent();
        let mut pen = x;
        for ch in text.chars() {
            let font = fonts.pick(bold, ch);
            let sf = font.as_scaled(scale);
            let id = sf.glyph_id(ch);
            let glyph = id.with_scale_and_position(scale, point(pen, y + base));
            if let Some(og) = font.outline_glyph(glyph) {
                let b = og.px_bounds();
                og.draw(|gx, gy, cov| {
                    self.blend(b.min.x as i64 + gx as i64, b.min.y as i64 + gy as i64, c, cov);
                });
            }
            pen += sf.h_advance(id);
        }
        pen - x
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [u8; 4]) {
        for y in y0.round() as i64..y1.round() as i64 {
            for x in x0.round() as i64..x1.round() as i64 {
                self.blend(x, y, c, 1.0);
            }
        }
    }

    /// A soft pencil stroke from a to b.
    fn stroke(&mut self, a: (f32, f32), b: (f32, f32), width: f32, c: [u8; 4]) {
        let r = width / 2.0;
        let (minx, maxx) = (a.0.min(b.0) - r - 1.0, a.0.max(b.0) + r + 1.0);
        let (miny, maxy) = (a.1.min(b.1) - r - 1.0, a.1.max(b.1) + r + 1.0);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        for y in miny as i64..=maxy as i64 {
            for x in minx as i64..=maxx as i64 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0);
                let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
                let d = ((px - qx).powi(2) + (py - qy).powi(2)).sqrt();
                self.blend(x, y, c, r + 0.5 - d);
            }
        }
    }
}

/// Small, fast, repeatable noise for the paper.
fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

/// Aged card stock: grain, faint fibres, and edges a little warmer and darker.
fn paper(w: usize, h: usize) -> Canvas {
    let mut px = vec![0u8; w * h * 4];
    let edge = 80.0;
    for y in 0..h {
        for x in 0..w {
            let n = (hash(x as u32, y as u32, 3) & 0xff) as f32 - 128.0;
            let d = (x.min(w - 1 - x).min(y).min(h - 1 - y)) as f32;
            let warm = if d < edge { 0.14 * (1.0 - d / edge).powi(2) } else { 0.0 };
            let i = (y * w + x) * 4;
            for k in 0..3 {
                let base = PAPER[k] + (PAPER_EDGE[k] - PAPER[k]) * warm;
                px[i + k] = (base + n * 0.05).clamp(0.0, 255.0) as u8;
            }
            px[i + 3] = 255;
        }
    }
    let mut c = Canvas { w, h, px };
    let fibres = w * h / 3600;
    for i in 0..fibres as u32 {
        let r = hash(i, 7, 11);
        let (x, y) = ((r % w as u32) as f32, ((r >> 8) % h as u32) as f32);
        let ang = (hash(i, 9, 13) % 314) as f32 / 100.0;
        let len = 8.0 + (hash(i, 5, 17) % 20) as f32;
        let alpha = 10 + (hash(i, 3, 19) % 18) as u8;
        c.stroke((x, y), (x + len * ang.cos(), y + len * ang.sin()), 1.2, [150, 140, 120, alpha]);
    }
    c
}

/// A blank sheet of the J-card's aged paper, for the lyric sheet.
pub fn paper_sheet() -> image::Handle {
    let (w, h) = (1100, 1400);
    image::Handle::from_rgba(w as u32, h as u32, paper(w, h).px)
}

/// Split `text` into lines no wider than `max` pixels.
fn wrap(fonts: &Fonts, text: &str, size: f32, bold: bool, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let next = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if cur.is_empty() || fonts.width(&next, size, bold) <= max {
            cur = next;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Draw one side of the J-card for an album.
///
/// `tracks` is every song on the album in play order: (title, seconds).
/// `side` is 0 for SIDE A, 1 for SIDE B. `playing` is the index of the song
/// in the deck, if it is on this album; it gets a pencil tick.
pub fn render(artist: &str, album: &str, tracks: &[(String, f32)], side: u8, playing: Option<usize>) -> image::Handle {
    let fonts = Fonts {
        regular: FontRef::try_from_slice(FONT_REGULAR).expect("embedded font"),
        bold: FontRef::try_from_slice(FONT_BOLD).expect("embedded font"),
        fallback: FontRef::try_from_slice(crate::label::FONT_SUB).expect("embedded font"),
    };
    let (w, h) = ((PW * S) as usize, (PH * S) as usize);
    let mut c = paper(w, h);

    let durations: Vec<f32> = tracks.iter().map(|t| t.1).collect();
    let split = side_b_start(&durations);
    let (first, list) = if side == 0 { (0, &tracks[..split]) } else { (split, &tracks[split..]) };

    // The biggest type size where both sides fit on the card, preferring one
    // where no title has to wrap. Both sides share it, so flipping the card
    // doesn't change the type.
    let x = MARGIN * S;
    let right = (PW - MARGIN) * S;
    let lines_at = |size: f32, part: &[(String, f32)]| -> usize {
        let avail = (right - x) - fonts.width("00.  00:00 ..", size, false);
        part.iter().map(|t| wrap(&fonts, &t.0, size, false, avail).len()).sum()
    };
    let (a, b) = tracks.split_at(split);
    let fits = |sz: f32| {
        let size = sz * S;
        let side_h = |part: &[(String, f32)]| lines_at(size, part) as f32 * 1.45 * sz + part.len() as f32 * 0.36 * sz;
        side_h(a).max(side_h(b)) + sz * 7.5 < PH * 0.78
    };
    let sizes: Vec<f32> = (13..=24).rev().map(|s| s as f32).collect();
    let sz = sizes
        .iter()
        .copied()
        .find(|&sz| lines_at(sz * S, a) == a.len() && lines_at(sz * S, b) == b.len() && fits(sz))
        .or_else(|| sizes.iter().copied().find(|&sz| fits(sz)))
        .unwrap_or(13.0);
    let size = sz * S;
    let head = sz * 1.35 * S;
    let side_size = sz * 1.15 * S;

    // Artist and album across the top, then a rule.
    let mut y = 40.0 * S;
    for line in [artist, album] {
        for l in wrap(&fonts, &line.to_uppercase(), head, true, right - x) {
            c.text(&fonts, x, y, &l, head, true, INK);
            y += sz * 1.6 * S;
        }
    }
    y += sz * 0.6 * S;
    c.rect(x, y, right, y + 2.0 * S, INK);
    y += sz * 1.2 * S;

    // SIDE A / SIDE B in a box, with that side's running time on the right.
    let label = if side == 0 { "SIDE A" } else { "SIDE B" };
    let lw = fonts.width(label, side_size, true);
    let (bx0, by0, bx1, by1) = (x - 6.0 * S, y - 4.0 * S, x + lw + 6.0 * S, y + sz * 1.3 * S);
    let t = 1.5 * S;
    c.rect(bx0, by0, bx1, by0 + t, INK);
    c.rect(bx0, by1 - t, bx1, by1, INK);
    c.rect(bx0, by0, bx0 + t, by1, INK);
    c.rect(bx1 - t, by0, bx1, by1, INK);
    c.text(&fonts, x, y, label, side_size, true, INK);
    let side_total = fmt(list.iter().map(|t| t.1).sum());
    c.text(&fonts, right - fonts.width(&side_total, size, false), y, &side_total, size, false, INK);
    y += sz * 2.3 * S;

    // The songs: number, title, dot leaders, length.
    let lh = sz * 1.45 * S;
    let num_w = fonts.width("00. ", size, false);
    let dot = fonts.width(".", size, false);
    for (i, (title, secs)) in list.iter().enumerate() {
        let no = first + i;
        let top = y;
        let num = format!("{:>2}.", no + 1);
        c.text(&fonts, x, y, &num, size, false, INK);
        let len = fmt(*secs);
        let len_w = fonts.width(&len, size, false);
        let nx = x + num_w;
        let avail = right - fonts.width(" 00:00", size, false) - nx;
        let lines = wrap(&fonts, title, size, false, avail);
        for (j, l) in lines.iter().enumerate() {
            let tw = c.text(&fonts, nx, y, l, size, false, INK);
            if j == lines.len() - 1 {
                let mut dx = nx + tw + dot;
                let end = right - len_w - dot;
                while dx + dot <= end {
                    c.text(&fonts, dx, y, ".", size, false, INK_LEADER);
                    dx += dot;
                }
                c.text(&fonts, right - len_w, y, &len, size, false, INK);
            }
            y += lh;
        }
        if playing == Some(no) {
            // A pencil tick in the margin beside the song in the deck.
            let (tx, ty) = (12.0 * S, top + 2.0 * S);
            let p = |fx: f32, fy: f32| (tx + sz * fx * S, ty + sz * fy * S);
            c.stroke(p(0.0, 0.55), p(0.35, 0.9), 1.6 * S, PENCIL);
            c.stroke(p(0.35, 0.9), p(0.95, 0.1), 1.6 * S, PENCIL);
        }
        y += lh * 0.25;
    }

    // Footer: a double rule and the small print of a printed insert.
    let fy = h as f32 - sz * 3.2 * S;
    c.rect(x, fy, right, fy + S, INK);
    c.rect(x, fy + 4.0 * S, right, fy + 5.0 * S, INK);
    let ty = h as f32 - sz * 2.4 * S;
    let print = "STEREO  ·  NORMAL POSITION  ·  TYPE I";
    let total = fmt(durations.iter().sum());
    let msg = if total.is_empty() { String::new() } else { format!("TOTAL TIME {total}") };
    // Shrink the small print if both pieces don't fit on one line.
    let mut small = sz * 0.72 * S;
    let need = fonts.width(print, small, false) + fonts.width(&msg, small, false) + 3.0 * small;
    if need > right - x {
        small *= (right - x) / need;
    }
    c.text(&fonts, x, ty, print, small, false, INK);
    c.text(&fonts, right - fonts.width(&msg, small, false), ty, &msg, small, false, INK);

    image::Handle::from_rgba(w as u32, h as u32, c.px)
}

#[cfg(test)]
mod tests {
    use super::side_b_start;

    #[test]
    fn splits_by_running_time() {
        assert_eq!(side_b_start(&[401.0, 584.0, 385.0, 444.0]), 2);
        assert_eq!(side_b_start(&[100.0]), 1);
        assert_eq!(side_b_start(&[]), 0);
        assert_eq!(side_b_start(&[0.0, 0.0, 0.0]), 2);
    }
}
