//! The deck eats the tape: a hidden extra.
//!
//! Slam the wind keys back and forth (FF, REW, FF, REW, FF: four reversals
//! inside about six seconds, each key pressed soon after the last one let
//! go) and hit Play straight after. On a real deck that throws a slack loop
//! of tape into the transport, and the pinch roller grabs it. Here the song
//! warbles, loses its treble and drags down while the take-up reel stutters
//! to a stop, the deck crunches, and the chewed tape bursts out of the
//! cassette. Then a close-up: the cassette on the bedroom rug, a pencil in
//! the hub winding the slack back in. Fade to black, and the deck comes back
//! up playing from just before where it chewed.
//!
//! Normal button behaviour is untouched: the trigger only watches the keys,
//! and only a Play press right after the reversals is taken over.

use std::time::{Duration, Instant};

use cosmic::iced::widget::canvas;
use cosmic::iced::{mouse, Color, Point, Rectangle, Size};

// ── Trigger ─────────────────────────────────────────────────────────────────

/// Direction reversals needed (FF, REW, FF, REW, FF is four).
pub const FLIPS_NEEDED: usize = 4;
/// The next wind key must go down within this long of the last one letting
/// go, while the reels are still spinning.
const FLIP_GAP: Duration = Duration::from_millis(1200);
/// All the reversals fit inside this long.
const CHAIN_WINDOW: Duration = Duration::from_millis(6000);
/// Play must come this soon after the last wind key lets go.
const PLAY_GAP: Duration = Duration::from_millis(2500);

/// Watches the wind keys for the reversals. It never changes what a key
/// does; `play` only says whether this Play press sets the extra off.
#[derive(Debug, Default)]
pub struct Trigger {
    /// Wind key presses in the current run of reversals: (when, direction).
    presses: Vec<(Instant, i8)>,
    /// When the last wind key let go (None while one is held).
    released: Option<Instant>,
}

impl Trigger {
    pub fn wind_start(&mut self, now: Instant, dir: i8) {
        let continues = match (self.presses.last(), self.released) {
            (Some(&(_, last)), Some(r)) => last != dir && now.saturating_duration_since(r) <= FLIP_GAP,
            _ => false,
        };
        if !continues {
            self.presses.clear();
        }
        self.presses.push((now, dir));
        self.released = None;
        while self
            .presses
            .first()
            .is_some_and(|&(t, _)| now.saturating_duration_since(t) > CHAIN_WINDOW)
        {
            self.presses.remove(0);
        }
    }

    pub fn wind_stop(&mut self, now: Instant) {
        if !self.presses.is_empty() {
            self.released = Some(now);
        }
    }

    /// Play was pressed. True when it comes right after the reversals. Any
    /// Play press starts the watch over.
    pub fn play(&mut self, now: Instant) -> bool {
        let fired = self.presses.len() > FLIPS_NEEDED
            && self
                .released
                .is_some_and(|r| now.saturating_duration_since(r) <= PLAY_GAP);
        self.reset();
        fired
    }

    pub fn reset(&mut self) {
        self.presses.clear();
        self.released = None;
    }
}

// ── Timeline (seconds from the Play press) ─────────────────────────────────

/// The song goes wrong for this long, then it crunches.
pub const WARBLE: f32 = 2.8;
const CRUNCH_END: f32 = 3.15;
/// Hard cut to the close-up.
pub const CUT: f32 = 4.45;
const WIND_END: f32 = 8.05;
const FADE_END: f32 = 8.75;
/// The deck fades back up from black.
const UP: f32 = 9.15;
/// The music comes back.
pub const MUSIC: f32 = 9.5;
pub const END: f32 = 10.05;
/// Sounds: the crunch, the deck's clunk, the pencil, the tape going back in.
pub const CRUNCH_SFX: f32 = WARBLE - 0.03;
pub const CLACK_SFX: f32 = WARBLE + 0.02;
pub const PENCIL_SFX: f32 = CUT + 0.15;
pub const SEAT_SFX: f32 = UP + 0.1;
/// It plays on from this far before where it chewed.
pub const BACK_UP: f32 = 1.5;
/// Pencil turns while it winds: about one every two seconds, a hand's pace.
const TURNS: f32 = 1.75;
/// The right reel turns a little slower than the left one.
const RIGHT_REEL: f32 = 0.82;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// Playing, going wrong. 0..1 through it.
    Wrong(f32),
    /// Stopped dead, the chewed tape out. 0..1 through the thunk.
    Crunch(f32),
    /// A beat on the chewed tape.
    Hold,
    /// The close-up, winding. 0..1 through it.
    Wind(f32),
    /// The close-up fading to black. 0..1.
    Fade(f32),
    Black,
    /// The deck fading back up. 0..1.
    Up(f32),
    Done,
}

pub fn phase(t: f32) -> Phase {
    let k = |a: f32, b: f32| ((t - a) / (b - a)).clamp(0.0, 1.0);
    if t < WARBLE {
        Phase::Wrong(k(0.0, WARBLE))
    } else if t < CRUNCH_END {
        Phase::Crunch(k(WARBLE, CRUNCH_END))
    } else if t < CUT {
        Phase::Hold
    } else if t < WIND_END {
        Phase::Wind(k(CUT, WIND_END))
    } else if t < FADE_END {
        Phase::Fade(k(WIND_END, FADE_END))
    } else if t < UP {
        Phase::Black
    } else if t < END {
        Phase::Up(k(UP, END))
    } else {
        Phase::Done
    }
}

pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// While it goes wrong, how fast each reel turns compared with normal play:
/// the supply reel keeps feeding, slowing a little; the take-up reel jerks
/// and stops as the tape wraps the capstan instead.
pub fn reel_factors(t: f32) -> (f32, f32) {
    let u = (t / WARBLE).clamp(0.0, 1.0);
    let src = 1.0 - 0.15 * u;
    let jerk = if (std::f32::consts::TAU * 6.5 * t).sin() > -0.2 { 1.0 } else { 0.15 };
    let tk = (1.0 - u / 0.7).max(0.0).powf(1.5) * jerk;
    (src, tk)
}

/// The deck's small thunk at the crunch, in deck-photo pixels.
pub fn thunk(t: f32) -> (f32, f32) {
    let Phase::Crunch(u) = phase(t) else { return (0.0, 0.0) };
    if u >= 0.25 {
        return (0.0, 0.0);
    }
    let a = 2.5 / 0.88;
    (
        a * ((t * 91.0).sin() + 0.6 * (t * 53.0).sin()),
        a * ((t * 77.0).cos() + 0.5 * (t * 131.0).sin()),
    )
}

// ── What the song does ─────────────────────────────────────────────────────
// Shared by the audio thread (which plays it) and the app (which needs to
// know how much tape went past the head while it did).

/// Smooth random wobble, 0..1, changing about every `per` seconds.
fn wobble(t: f32, per: f32, seed: u32) -> f32 {
    let hash = |i: i64| -> f32 {
        let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (seed as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        x ^= x >> 31;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 29;
        (x >> 40) as f32 / (1u64 << 24) as f32
    };
    let p = t / per;
    let i = p.floor();
    let f = p - i;
    let f = f * f * (3.0 - 2.0 * f);
    let i = i as i64;
    hash(i) * (1.0 - f) + hash(i + 1) * f
}

/// Going wrong, `t` seconds in: (tape speed, treble cutoff in Hz, loudness).
/// Wow and flutter build, the pitch drags down at the end, treble fades as
/// the tape lifts off the head, and the sound drops in and out.
pub fn chew(t: f32) -> (f32, f32, f32) {
    use std::f32::consts::TAU;
    let k = (t / WARBLE).clamp(0.0, 1.0);
    let wow = (0.006 + 0.05 * k.powf(1.3)) * (TAU * (3.2 + 1.5 * k) * t).sin()
        + 0.012 * k * (TAU * 11.0 * t).sin();
    let drag = ((k - 0.55) / 0.45).clamp(0.0, 1.0);
    let rate = 1.0 + wow - 0.30 * drag * drag;
    let cutoff = 9000.0 * (1.0 - k) * (1.0 - k) + 600.0;
    let dips = 1.0 - 0.75 * k * ((wobble(t, 0.12, 5) - 0.45) * 2.2).clamp(0.0, 1.0);
    (rate, cutoff, dips * (1.0 - 0.2 * k))
}

/// The motor straining in the last half: (pitch in Hz, loudness).
pub fn motor(t: f32) -> (f32, f32) {
    let k = (t / WARBLE).clamp(0.0, 1.0);
    let m = ((k - 0.45) / 0.55).clamp(0.0, 1.0);
    (160.0 - 55.0 * m, 0.05 * m.powf(1.5))
}

/// Back in and playing, `t` seconds after it starts: (tape speed, treble
/// cutoff in Hz). A short warble as the crinkled stretch passes the head.
pub fn warble(t: f32) -> (f32, f32) {
    let k = if t > 1.2 { (1.0 - (t - 1.5) / 1.2).clamp(0.0, 1.0) } else { 0.0 };
    (
        1.0 + 0.03 * k * (std::f32::consts::TAU * 4.5 * t).sin(),
        9000.0 - 6500.0 * k,
    )
}

/// After this long the warble is over and the song plays untouched.
pub const WARBLE_DONE: f32 = 2.8;

/// How far the tape has moved `t` seconds into going wrong, in seconds of
/// song.
pub fn tape_moved(t: f32) -> f32 {
    let t = t.clamp(0.0, WARBLE);
    let steps = ((t * 1000.0) as usize).max(1);
    let dt = t / steps as f32;
    (0..steps).map(|i| chew((i as f32 + 0.5) * dt).0 * dt).sum()
}

// ── The chewed tape on the deck ─────────────────────────────────────────────

/// The tangle photo's top-left corner and size, in deck-photo pixels.
pub const TANGLE_AT: (f32, f32) = (-53.36, 471.97);
pub const TANGLE_SIZE: (f32, f32) = (612.0, 528.0);
pub const TANGLE_PNG: &[u8] = include_bytes!("../assets/egg/tangle.png");

/// The deck photo with less tape on both reels: the outer turns are pulled
/// out, and the clear shell behind them shows (dark, colourless).
pub fn eaten_body(png: &[u8], hubs: [(f32, f32); 2]) -> Option<::image::RgbaImage> {
    let mut img = ::image::load_from_memory(png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let lum: Vec<f32> = img
        .pixels()
        .map(|p| (p.0[0] as f32 + p.0[1] as f32 + p.0[2] as f32) / 3.0)
        .collect();
    // Pack edge now (top reel, bottom reel), the full pack, and how far right
    // the reels reach before the label: deck-photo pixels.
    let left = [68.2_f32, 79.5];
    let full = 97.7_f32;
    let x_max = 570.5_f32;
    let (soft_w, rim_w) = (2.5 / 0.88, 2.5 / 0.88);
    for ((cx, cy), r_now) in hubs.into_iter().zip(left) {
        let reach = full + 6.8;
        let (x0, x1) = ((cx - reach).max(0.0) as u32, ((cx + reach) as u32).min(w - 1));
        let (y0, y1) = ((cy - reach).max(0.0) as u32, ((cy + reach) as u32).min(h - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                if x as f32 >= x_max {
                    continue;
                }
                let r = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                let l = lum[(y * w + x) as usize];
                let p = img.get_pixel_mut(x, y);
                if r > r_now && r <= reach && l < 150.0 {
                    let wgt = ((r - r_now) / soft_w).clamp(0.0, 1.0);
                    let g = (34.0 + (l - 60.0) * 0.35).clamp(22.0, 90.0);
                    for (c, e) in [g, g + 1.0, g + 5.0].into_iter().enumerate() {
                        p.0[c] = (p.0[c] as f32 * (1.0 - wgt) + e * wgt).round() as u8;
                    }
                }
                if r > r_now - rim_w && r <= r_now {
                    for c in 0..3 {
                        p.0[c] = (p.0[c] as f32 * 0.75) as u8;
                    }
                }
            }
        }
    }
    Some(img)
}

// ── The close-up ────────────────────────────────────────────────────────────
// The cassette lying flat on the rug, seen close: only the label, the reels
// and the pencil. Drawn here, a frame at a time, on a worker thread: the
// cassette is warped into perspective, its reels turned and their packs
// filled, and the pencil spun on its own axis.

pub const FRAME_W: u32 = 1100;
pub const FRAME_H: u32 = 1200;
/// Where the cassette photo's corners land in the close-up.
const CLOSE: [(f64, f64); 4] = [(57.697, 676.363), (1104.616, 676.363), (1385.498, 1316.251), (-192.403, 1316.251)];
/// The left hub's centre in the close-up, and the size of its hole there.
const HOLE_C: (f32, f32) = (294.332, 930.796);
const HOLE_R: (f32, f32) = (55.670, 43.030);
/// The pencil picture: where its bottom end sits, which columns are its body,
/// and how far down into the hub it's pushed.
const PENCIL_ER: (f32, f32) = (60.5, 1353.646);
const PENCIL_COLS: (f32, f32) = (6.0, 115.0);
const PENCIL_SINK: f32 = 301.6;
/// The cassette: the same one that's in the deck, cut from the deck photo
/// and laid flat. Hub centres, the hub disc that turns, the tape packs, and
/// the area the packs can show in (left, right, bottom).
const TAPE_PNG: &[u8] = include_bytes!("../assets/egg/walkman_tape.png");
const HUB_L: (f32, f32) = (207.0, 243.9);
const HUB_R: (f32, f32) = (522.0, 242.6);
const HUB_DISC: f32 = 80.0;
const PACK_FULL: f32 = 124.1;
const PACK_EATEN: (f32, f32) = (86.6, 101.0);
const PACK_AREA: (u32, u32, u32) = (18, 757, 400);
const LABEL_BOTTOM: f32 = 146.1;
/// Where the song's label text goes on the cassette photo's white label
/// strip (x0, y0, x1, y1): the same shape as the deck's label, so it reads
/// the same.
const LABEL_TEXT: (u32, u32, u32, u32) = (32, 32, 729, 146);

const BG_JPG: &[u8] = include_bytes!("../assets/egg/closeup_bg.jpg");
const SHADE_PNG: &[u8] = include_bytes!("../assets/egg/closeup_shade.png");
const PENCIL_PNG: &[u8] = include_bytes!("../assets/egg/pencil.png");

/// A 3x3 projective map, row-major.
type Mat3 = [f64; 9];

/// The map that takes `src` corners to `dst` corners.
#[allow(clippy::needless_range_loop)]
fn homography(src: [(f64, f64); 4], dst: [(f64, f64); 4]) -> Mat3 {
    let mut a = [[0.0f64; 9]; 8];
    for (i, ((x, y), (u, v))) in src.into_iter().zip(dst).enumerate() {
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    for c in 0..8 {
        let p = (c..8).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs())).unwrap();
        a.swap(c, p);
        for r in 0..8 {
            if r != c {
                let f = a[r][c] / a[c][c];
                for k in c..9 {
                    a[r][k] -= f * a[c][k];
                }
            }
        }
    }
    let mut h = [1.0; 9];
    for i in 0..8 {
        h[i] = a[i][8] / a[i][i];
    }
    h
}

fn invert(m: Mat3) -> Mat3 {
    let [a, b, c, d, e, f, g, h, i] = m;
    let co = [e * i - f * h, -(b * i - c * h), b * f - c * e, -(d * i - f * g), a * i - c * g, -(a * f - c * d), d * h - e * g, -(a * h - b * g), a * e - b * d];
    let det = a * co[0] + b * co[3] + c * co[6];
    co.map(|v| v / det)
}

/// Sample an RGBA picture between pixels (premultiplied by alpha, so the
/// cassette's edges blend cleanly). Outside the picture is clear.
fn sample(img: &::image::RgbaImage, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let mut out = [0.0f32; 4];
    for (dx, dy, wgt) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
        let (px, py) = (x0 + dx, y0 + dy);
        if px < 0 || py < 0 || px >= w || py >= h || wgt == 0.0 {
            continue;
        }
        let p = img.get_pixel(px as u32, py as u32).0;
        let a = p[3] as f32 / 255.0;
        out[0] += p[0] as f32 * a * wgt;
        out[1] += p[1] as f32 * a * wgt;
        out[2] += p[2] as f32 * a * wgt;
        out[3] += a * wgt;
    }
    out
}

pub struct Closeup {
    bg: ::image::RgbaImage,
    shade: ::image::GrayImage,
    pencil: ::image::RgbaImage,
    sprite: ::image::RgbaImage,
    /// The cassette photo's brightness, for telling plastic from tape.
    lum: Vec<f32>,
    /// Close-up pixel -> cassette photo pixel.
    to_sprite: Mat3,
    /// Close-up rows and columns the cassette covers.
    span: (u32, u32, u32, u32),
}

impl Closeup {
    /// `label`: the song's label laid flat (label::flat_strip), written onto
    /// the cassette's label strip so it's the same tape as in the deck.
    pub fn new(label: Option<&::image::RgbaImage>) -> Option<Closeup> {
        let mut sprite = ::image::load_from_memory(TAPE_PNG).ok()?.to_rgba8();
        if let Some(label) = label {
            let (x0, y0, x1, y1) = LABEL_TEXT;
            let text = ::image::imageops::resize(label, x1 - x0, y1 - y0, ::image::imageops::FilterType::Triangle);
            ::image::imageops::overlay(&mut sprite, &text, x0 as i64, y0 as i64);
        }
        let bg = ::image::load_from_memory(BG_JPG).ok()?.to_rgba8();
        let shade = ::image::load_from_memory(SHADE_PNG).ok()?.to_luma8();
        let pencil = ::image::load_from_memory(PENCIL_PNG).ok()?.to_rgba8();
        let (cw, ch) = (sprite.width() as f64, sprite.height() as f64);
        let to_screen = homography([(0.0, 0.0), (cw, 0.0), (cw, ch), (0.0, ch)], CLOSE);
        let to_sprite = invert(to_screen);
        let xs = CLOSE.map(|p| p.0);
        let ys = CLOSE.map(|p| p.1);
        let fold = |v: [f64; 4], f: fn(f64, f64) -> f64| v.into_iter().reduce(f).unwrap();
        let span = (
            fold(xs, f64::min).max(0.0) as u32,
            fold(ys, f64::min).max(0.0) as u32,
            (fold(xs, f64::max).ceil() as u32).min(FRAME_W),
            (fold(ys, f64::max).ceil() as u32).min(FRAME_H),
        );
        let lum = sprite
            .pixels()
            .map(|p| (p.0[0] as f32 + p.0[1] as f32 + p.0[2] as f32) / 3.0)
            .collect();
        Some(Closeup { bg, shade, pencil, sprite, lum, to_sprite, span })
    }

    /// The cassette photo for this moment: packs `wound` (0..1) of the way
    /// back to full, reels turned clockwise by `turn` radians.
    fn cassette(&self, wound: f32, turn: f32) -> ::image::RgbaImage {
        let mut c = self.sprite.clone();
        let w = c.width();
        let radii = [
            PACK_EATEN.0 + (PACK_FULL - PACK_EATEN.0) * wound,
            PACK_EATEN.1 + (PACK_FULL - PACK_EATEN.1) * wound,
        ];
        // Less tape on the reels: the clear shell shows where the outer turns were.
        for (hub, radius) in [HUB_L, HUB_R].into_iter().zip(radii) {
            if radius >= PACK_FULL - 0.5 {
                continue;
            }
            let x0 = ((hub.0 - PACK_FULL - 8.0) as u32).max(PACK_AREA.0);
            let x1 = ((hub.0 + PACK_FULL + 8.0) as u32).min(PACK_AREA.1);
            for y in (LABEL_BOTTOM as u32 + 2)..PACK_AREA.2.min(c.height()) {
                for x in x0..=x1 {
                    let r = ((x as f32 - hub.0).powi(2) + (y as f32 - hub.1).powi(2)).sqrt();
                    let l = self.lum[(y * w + x) as usize];
                    let p = c.get_pixel_mut(x, y);
                    if r > radius && r <= PACK_FULL + 8.0 && l <= 150.0 {
                        let k = ((r - radius) / 2.5).clamp(0.0, 1.0);
                        let g = (34.0 + (l - 60.0) * 0.35).clamp(22.0, 90.0) + (l - 130.0).clamp(0.0, 255.0) * 0.6;
                        for (i, e) in [g, g + 1.0, g + 5.0].into_iter().enumerate() {
                            p.0[i] = (p.0[i] as f32 * (1.0 - k) + e * k).clamp(0.0, 255.0) as u8;
                        }
                    }
                    if r > radius - 2.5 && r <= radius {
                        for i in 0..3 {
                            p.0[i] = (p.0[i] as f32 * 0.75) as u8;
                        }
                    }
                }
            }
        }
        // The hubs turn (clockwise: the rewind direction, pulling the slack
        // onto the left reel), the right one a little slower.
        for (hub, a) in [(HUB_L, turn), (HUB_R, turn * RIGHT_REEL)] {
            let (s, co) = a.sin_cos();
            let r = HUB_DISC as i32 + 1;
            for dy in -r..=r {
                for dx in -r..=r {
                    let (x, y) = (hub.0 as i32 + dx, hub.1 as i32 + dy);
                    let (ox, oy) = (x as f32 + 0.5 - hub.0, y as f32 + 0.5 - hub.1);
                    let m = ((HUB_DISC - (ox * ox + oy * oy).sqrt()) / 3.0).clamp(0.0, 1.0);
                    if m <= 0.0 {
                        continue;
                    }
                    // Where this pixel was before the turn.
                    let (sx, sy) = (ox * co + oy * s, -ox * s + oy * co);
                    let v = sample(&self.sprite, hub.0 + sx, hub.1 + sy);
                    let p = c.get_pixel_mut(x as u32, y as u32);
                    if v[3] > 0.0 {
                        for i in 0..3 {
                            p.0[i] = (p.0[i] as f32 * (1.0 - m) + v[i] / v[3] * m) as u8;
                        }
                    }
                }
            }
        }
        c
    }

    /// One frame: `wound` 0..1 through the winding, `dark` how strong the
    /// vignette is.
    pub fn render(&self, wound: f32, dark: f32) -> ::image::RgbaImage {
        let wound = wound.clamp(0.0, 1.0);
        let turn = std::f32::consts::TAU * TURNS * wound;
        let cas = self.cassette(wound, turn);
        let mut out = self.bg.clone();
        let (sx0, sy0, sx1, sy1) = self.span;
        let h = self.to_sprite;
        let (cw, ch) = (cas.width() as f32, cas.height() as f32);
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
        let row_bytes = FRAME_W as usize * 4;
        let rows_per = (FRAME_H as usize).div_ceil(threads);
        std::thread::scope(|scope| {
            for (chunk_i, chunk) in out.chunks_mut(row_bytes * rows_per).enumerate() {
                let cas = &cas;
                let shade = &self.shade;
                scope.spawn(move || {
                    let y_base = chunk_i * rows_per;
                    for (ry, row) in chunk.chunks_mut(row_bytes).enumerate() {
                        let y = (y_base + ry) as u32;
                        let fy = y as f64 + 0.5;
                        for x in 0..FRAME_W {
                            let px = &mut row[x as usize * 4..x as usize * 4 + 4];
                            // The cassette, warped flat onto the rug.
                            if y >= sy0 && y < sy1 && x >= sx0 && x < sx1 {
                                let fx = x as f64 + 0.5;
                                let wq = h[6] * fx + h[7] * fy + h[8];
                                let u = ((h[0] * fx + h[1] * fy + h[2]) / wq) as f32;
                                let v = ((h[3] * fx + h[4] * fy + h[5]) / wq) as f32;
                                if u > -1.0 && v > -1.0 && u < cw + 1.0 && v < ch + 1.0 {
                                    let s = sample(cas, u, v);
                                    for i in 0..3 {
                                        px[i] = (s[i] + px[i] as f32 * (1.0 - s[3])).clamp(0.0, 255.0) as u8;
                                    }
                                }
                            }
                            // The pencil's shadow and the dark of the hole.
                            let d = shade.get_pixel(x, y).0[0];
                            if d > 0 {
                                let k = 1.0 - d as f32 / 255.0;
                                for c in px.iter_mut().take(3) {
                                    *c = (*c as f32 * k) as u8;
                                }
                            }
                        }
                    }
                });
            }
        });
        self.draw_pencil(&mut out, turn);
        if dark > 0.0 {
            for (x, y, p) in out.enumerate_pixels_mut() {
                let rx = (x as f32 - FRAME_W as f32 / 2.0) / (FRAME_W as f32 / 2.0);
                let ry = (y as f32 - FRAME_H as f32 / 2.0) / (FRAME_H as f32 / 2.0);
                let r = (rx * rx + ry * ry).sqrt();
                let m = (1.0 - dark * (0.35 + 0.9 * (r - 0.3).clamp(0.0, 1.0))).clamp(0.0, 1.0);
                for c in p.0.iter_mut().take(3) {
                    *c = (*c as f32 * m) as u8;
                }
            }
        }
        out
    }

    /// The pencil standing in the hub, turning with it: its six painted faces
    /// roll round, and its top dips toward you and away a touch as it turns.
    fn draw_pencil(&self, out: &mut ::image::RgbaImage, turn: f32) {
        let spin = -turn.to_degrees();
        let (x0, x1) = PENCIL_COLS;
        let pw = self.pencil.width();
        let bright: Vec<f32> = (0..pw)
            .map(|x| {
                let u = ((x as f32 - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0);
                let across = (1.0 - 2.0 * u).acos().to_degrees();
                let face = ((across - spin) / 60.0).floor();
                let normal = spin + (face + 0.5) * 60.0;
                let b = 0.78 + 0.40 * (normal - 65.0).to_radians().cos().clamp(-0.3, 1.0);
                let edge = (((across - spin).rem_euclid(60.0)) - 30.0).abs() > 27.5;
                if edge { b * 0.8 } else { b }
            })
            .collect();
        let fore = 1.0 - 0.035 * spin.to_radians().sin();
        let ph = self.pencil.height() as f32 * fore;
        let er_y = PENCIL_ER.1 * fore;
        let left = (HOLE_C.0 - PENCIL_ER.0) as i32;
        let top = (HOLE_C.1 - er_y + PENCIL_SINK) as i32;
        let (hx, hy) = HOLE_C;
        let (rx, ry) = HOLE_R;
        for oy in 0..ph as i32 {
            let y = top + oy;
            if y < 0 || y >= FRAME_H as i32 {
                continue;
            }
            let fy = y as f32;
            let sy = ((oy as f32 + 0.5) / fore) as u32;
            if sy >= self.pencil.height() {
                continue;
            }
            let lit = 0.55 + 0.45 * ((hy + ry - fy) / (2.2 * ry)).clamp(0.0, 1.0);
            for sx in 0..pw {
                let x = left + sx as i32;
                if x < 0 || x >= FRAME_W as i32 {
                    continue;
                }
                let fx = x as f32;
                // Down in the hole, only what's inside the opening shows.
                if fy > hy && ((fx - hx) / rx).powi(2) + ((fy - hy) / ry).powi(2) > 1.0 {
                    continue;
                }
                let p = self.pencil.get_pixel(sx, sy).0;
                if p[3] == 0 {
                    continue;
                }
                let a = p[3] as f32 / 255.0;
                let k = bright[sx as usize] * lit;
                let q = out.get_pixel_mut(x as u32, y as u32);
                for (qc, &pc) in q.0.iter_mut().zip(p.iter()).take(3) {
                    let c = (pc as f32 * k).min(255.0);
                    *qc = (c * a + *qc as f32 * (1.0 - a)) as u8;
                }
            }
        }
    }
}

/// A horizontal strip of a close-up frame: its first and last row (with two
/// spare rows past each cut edge), and its picture.
#[derive(Clone)]
pub struct Band {
    pub rows: (u32, u32),
    pub image: cosmic::widget::image::Handle,
}

/// Rows per strip. A picture of 2 MB or more reaches the screen a frame or
/// more late (it's handed to a loader thread), so a whole frame at a time
/// never shows while the close-up moves; strips under that go up at once.
const BAND_ROWS: u32 = 400;
const BAND_SPARE: u32 = 2;

/// Cut a close-up frame into strips that each go to the screen at once.
pub fn bands(frame: &::image::RgbaImage) -> Vec<Band> {
    let (w, h) = frame.dimensions();
    (0..h.div_ceil(BAND_ROWS))
        .map(|i| {
            let y0 = (i * BAND_ROWS).saturating_sub(BAND_SPARE);
            let y1 = ((i + 1) * BAND_ROWS + BAND_SPARE).min(h);
            let strip = ::image::imageops::crop_imm(frame, 0, y0, w, y1 - y0).to_image();
            Band { rows: (y0, y1), image: cosmic::widget::image::Handle::from_rgba(w, y1 - y0, strip.into_raw()) }
        })
        .collect()
}

/// The close-up on screen: its strips, fitted to the window on black.
pub struct CloseupScene {
    pub bands: Vec<Band>,
}

impl<Message> canvas::Program<Message, cosmic::Theme, cosmic::Renderer> for CloseupScene {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &cosmic::Renderer,
        _theme: &cosmic::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<cosmic::Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), Color::BLACK);
        let s = (bounds.width / FRAME_W as f32).min(bounds.height / FRAME_H as f32);
        let size = Size::new(FRAME_W as f32 * s, FRAME_H as f32 * s);
        let at = Point::new((bounds.width - size.width) / 2.0, (bounds.height - size.height) / 2.0);
        for band in &self.bands {
            let (y0, y1) = band.rows;
            let dest = Rectangle::new(
                Point::new(at.x, at.y + y0 as f32 * s),
                Size::new(size.width, (y1 - y0) as f32 * s),
            );
            // Show only the strip's own rows (plus a pixel of overlap), so
            // no cut edge is ever on screen.
            let own0 = if y0 == 0 { 0 } else { y0 + BAND_SPARE };
            let own1 = if y1 == FRAME_H { y1 } else { y1 - BAND_SPARE };
            let clip = Rectangle::new(
                Point::new(at.x, at.y + own0 as f32 * s - 1.0),
                Size::new(size.width, (own1 - own0) as f32 * s + 2.0),
            );
            frame.with_clip(clip, |f| f.draw_image(dest, canvas::Image::new(band.image.clone())));
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// Press and let go of a wind key: `dir` down at `ms`, up `hold` later.
    fn tap(tr: &mut Trigger, t0: Instant, ms: u64, hold: u64, dir: i8) {
        tr.wind_start(at(t0, ms), dir);
        tr.wind_stop(at(t0, ms + hold));
    }

    #[test]
    fn four_quick_reversals_then_play_sets_it_off() {
        let t0 = Instant::now();
        let mut tr = Trigger::default();
        for (i, dir) in [1, -1, 1, -1, 1].into_iter().enumerate() {
            tap(&mut tr, t0, i as u64 * 450, 300, dir);
        }
        assert!(tr.play(at(t0, 4 * 450 + 300 + 400)));
    }

    #[test]
    fn a_hand_on_a_mouse_can_set_it_off() {
        // Clicking back and forth by hand: each key held about a third of a
        // second, about a second between presses, then over to Play.
        let t0 = Instant::now();
        let mut tr = Trigger::default();
        for (i, dir) in [1, -1, 1, -1, 1].into_iter().enumerate() {
            tap(&mut tr, t0, i as u64 * 1100, 350, dir);
        }
        assert!(tr.play(at(t0, 4 * 1100 + 350 + 2000)));
    }

    #[test]
    fn three_reversals_are_not_enough() {
        let t0 = Instant::now();
        let mut tr = Trigger::default();
        for (i, dir) in [1, -1, 1, -1].into_iter().enumerate() {
            tap(&mut tr, t0, i as u64 * 450, 300, dir);
        }
        assert!(!tr.play(at(t0, 3 * 450 + 300 + 400)));
    }

    #[test]
    fn same_direction_twice_starts_over() {
        let t0 = Instant::now();
        let mut tr = Trigger::default();
        for (i, dir) in [1, -1, -1, 1, -1, 1].into_iter().enumerate() {
            tap(&mut tr, t0, i as u64 * 450, 300, dir);
        }
        // Only -1, 1, -1, 1 after the repeat: three reversals.
        assert!(!tr.play(at(t0, 5 * 450 + 300 + 300)));
    }

    #[test]
    fn slow_reversals_do_not_count() {
        let t0 = Instant::now();
        let mut tr = Trigger::default();
        for (i, dir) in [1, -1, 1, -1, 1].into_iter().enumerate() {
            tap(&mut tr, t0, i as u64 * 2000, 300, dir);
        }
        assert!(!tr.play(at(t0, 4 * 2000 + 300 + 300)));
    }

    #[test]
    fn play_too_late_does_nothing() {
        let t0 = Instant::now();
        let mut tr = Trigger::default();
        for (i, dir) in [1, -1, 1, -1, 1].into_iter().enumerate() {
            tap(&mut tr, t0, i as u64 * 450, 300, dir);
        }
        assert!(!tr.play(at(t0, 4 * 450 + 300 + 3000)));
    }

    #[test]
    fn ordinary_play_presses_never_set_it_off() {
        let t0 = Instant::now();
        let mut tr = Trigger::default();
        assert!(!tr.play(t0));
        tap(&mut tr, t0, 100, 2000, 1); // one long fast-forward
        assert!(!tr.play(at(t0, 2300)));
        // And a Play press resets: the same keys again don't fire twice.
        for (i, dir) in [1, -1, 1, -1, 1].into_iter().enumerate() {
            tap(&mut tr, t0, 3000 + i as u64 * 450, 300, dir);
        }
        assert!(tr.play(at(t0, 3000 + 4 * 450 + 600)));
        assert!(!tr.play(at(t0, 3000 + 4 * 450 + 700)));
    }

    #[test]
    fn timeline_runs_in_order() {
        assert!(matches!(phase(0.0), Phase::Wrong(_)));
        assert!(matches!(phase(3.0), Phase::Crunch(_)));
        assert_eq!(phase(4.0), Phase::Hold);
        assert!(matches!(phase(5.0), Phase::Wind(_)));
        assert!(matches!(phase(8.3), Phase::Fade(_)));
        assert_eq!(phase(9.0), Phase::Black);
        assert!(matches!(phase(9.6), Phase::Up(_)));
        assert_eq!(phase(END), Phase::Done);
    }

    #[test]
    fn the_tape_drags_while_it_chews() {
        let moved = tape_moved(WARBLE);
        assert!(moved > 2.4 && moved < WARBLE, "moved {moved}");
        // Clean again once the crinkled stretch has passed.
        assert_eq!(warble(3.0), (1.0, 9000.0));
        assert_eq!(warble(0.5), (1.0, 9000.0));
    }

    #[test]
    fn closeup_renders() {
        let strip = crate::label::flat_strip("Midnight Arcade", "The Cassette Kids", &crate::cassette::BELT);
        assert!(strip.is_some());
        let c = Closeup::new(strip.as_ref()).expect("assets");
        let t = Instant::now();
        let f = c.render(0.5, 0.4);
        eprintln!("frame in {:?}", t.elapsed());
        // Every strip is small enough to reach the screen the same frame,
        // and together they cover every row.
        let b = bands(&f);
        assert!(b.iter().all(|b| (FRAME_W * (b.rows.1 - b.rows.0) * 4) < 2 * 1024 * 1024));
        assert_eq!(b.first().unwrap().rows.0, 0);
        assert_eq!(b.last().unwrap().rows.1, FRAME_H);
        assert!(b.windows(2).all(|w| w[0].rows.1 > w[1].rows.0));
        assert_eq!(f.dimensions(), (FRAME_W, FRAME_H));
        if let Ok(dir) = std::env::var("EGG_DUMP") {
            for (i, u) in [0.0f32, 0.33, 0.66, 1.0].into_iter().enumerate() {
                c.render(u, 0.25 + 0.35 * u).save(format!("{dir}/closeup_{i}.png")).unwrap();
            }
            let body = include_bytes!("../assets/skins/belt/body.png");
            eaten_body(body, [(515.0, 420.0), (516.0, 668.0)]).unwrap().save(format!("{dir}/eaten.png")).unwrap();
        }
    }
}
