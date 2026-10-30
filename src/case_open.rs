//! Cassette-case open animation scene.
//!
//! On the reveal screen the user clicks the case; it swings open on its spine
//! hinge, the tape lifts up out of the tray and off the top of the view, the
//! scene cross-fades to the walkman deck, and the tape slides down into the
//! cassette window - handing off to the player.
//!
//! Self-contained canvas `Program` with no interaction, driven by an external
//! progress value `t` in 0.0..=1.0. All geometry is in the belt skin's
//! 1024×1024 space, letterboxed to the bounds. The tape stays portrait
//! (label-right) the whole way, no rotation, so nothing wobbles.
//!
//! The deck phase draws the skin's own seated cassette (see
//! `cassette::draw_deck`), so its last frame is the player's first frame.
//! The case tape leaves the view before the fade, so the two are never seen
//! swapping.

use cosmic::iced::widget::canvas::{self, Frame, Geometry};
use cosmic::iced::{Point, Radians, Rectangle, Size, Vector};
use cosmic::widget::image;

use crate::cassette::{draw_deck, draw_empty_deck_faded, LoadedSkin};

const DECK: f32 = 1024.0;

// Keyed case_open.png geometry (measured off the sprite).
const SPINE: f32 = 478.0;
const LID: (f32, f32, f32, f32) = (106.0, 142.0, 478.0, 887.0); // open-lid crop
const TRAY: (f32, f32, f32, f32) = (478.0, 171.0, 924.0, 858.0); // tray crop
const TRAY_RECT: (f32, f32, f32, f32) = (478.0, 172.0, 923.0, 856.0); // where the tape sits

/// The landscape cassette sprite's native aspect (w/h).
const SPRITE_RATIO: f32 = 761.0 / 496.0;

// Animation phase boundaries (fractions of the whole `t`).
const CLOSED_END: f32 = 0.08; // brief closed hold before the swing
const SWING_END: f32 = 0.40; // lid finishes opening
const LIFT_START: f32 = 0.46; // tape starts lifting out of the tray
const LIFT_END: f32 = 0.58; // tape is off the top; cross-fade to the deck begins
const FADE_END: f32 = 0.68; // deck fully shown; tape starts sliding into the window

/// Split the keyed open-case PNG into (tray, lid, lid_flipped) RGBA handles.
/// The lid is drawn separately so it can be horizontally compressed to fake the
/// hinge; the flipped copy drapes over the tray for the closed/early frames.
pub fn split(png: &[u8]) -> Option<(image::Handle, image::Handle, image::Handle)> {
    use ::image as ic;
    let img = ic::ImageReader::new(std::io::Cursor::new(png))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?
        .to_rgba8();
    let crop = |x0: f32, y0: f32, x1: f32, y1: f32| {
        let (x0, y0) = (x0 as u32, y0 as u32);
        let (w, h) = ((x1 as u32) - x0, (y1 as u32) - y0);
        ic::imageops::crop_imm(&img, x0, y0, w, h).to_image()
    };
    let tray = crop(TRAY.0, TRAY.1, TRAY.2, TRAY.3);
    let lid = crop(LID.0, LID.1, LID.2, LID.3);
    let lid_f = ic::imageops::flip_horizontal(&lid);
    let to_handle =
        |im: ic::RgbaImage| image::Handle::from_rgba(im.width(), im.height(), im.into_raw());
    Some((to_handle(tray), to_handle(lid), to_handle(lid_f)))
}

pub struct CaseOpenScene<'a> {
    /// Deck skin the tape goes into.
    pub skin: &'a LoadedSkin,
    /// Label for the track being loaded.
    pub label: Option<&'a image::Handle>,
    /// Current reel angles, so the hubs match the player at the hand-off.
    pub angle_src: f32,
    pub angle_tk: f32,
    pub tray: image::Handle,
    pub lid: image::Handle,
    pub lid_flipped: image::Handle,
    /// The landscape cassette sprite (rotated 90° when drawn = label-right),
    /// shown in the case.
    pub tape: image::Handle,
    /// Progress, 0.0 (closed case) -> 1.0 (seated in the walkman).
    pub t: f32,
}

/// Opacity of the room photo behind the case-open view at progress `t`: full
/// through the case phase, fading out with the cross-fade to the deck, so the
/// surroundings match the player by the time the tape slides in.
pub fn backdrop_alpha(t: f32) -> f32 {
    1.0 - ((t - LIFT_END) / (FADE_END - LIFT_END)).clamp(0.0, 1.0)
}

fn ease_out_cubic(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}
fn ease_in_quad(x: f32) -> f32 {
    x * x
}
fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x.clamp(0.0, 1.0)
}

impl<'a, Message> canvas::Program<Message, cosmic::Theme, cosmic::Renderer> for CaseOpenScene<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &cosmic::Renderer,
        _theme: &cosmic::Theme,
        bounds: Rectangle,
        _cursor: cosmic::iced::mouse::Cursor,
    ) -> Vec<Geometry<cosmic::Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let t = self.t.clamp(0.0, 1.0);

        if t >= FADE_END {
            // ── WALKMAN PHASE ── the deck (charcoal letterbox covers the
            // bedroom), tape sliding down into the window.
            let k = ((t - FADE_END) / (1.0 - FADE_END)).clamp(0.0, 1.0);
            draw_deck(
                &mut frame,
                bounds.size(),
                self.skin,
                self.label,
                self.angle_src,
                self.angle_tk,
                1.0 - ease_in_quad(k),
                // Letterbox left clear: the page behind is charcoal, or the
                // new album's wallpaper fading in.
                false,
                &[],
            );
            return vec![frame.into_geometry()];
        }

        // ── CASE PHASE ── bedroom shows through (stacked behind in view()).

        // Pre-load the deck-phase textures. The renderer decodes images on a
        // background worker and skips them until they are ready, so a texture
        // first drawn at the hand-off would be missing for a frame or more (the
        // empty well showed up late, flashing the seated tape). Drawing them
        // invisibly here starts the decode well before they are needed.
        let warm = Rectangle::new(Point::ORIGIN, Size::new(1.0, 1.0));
        for h in [&self.skin.body, &self.skin.empty, &self.skin.hub]
            .into_iter()
            .chain(self.label)
        {
            frame.draw_image(warm, canvas::Image::new(h.clone()).opacity(0.0_f32));
        }

        // Fit the 1024×1024 space into the bounds, letterboxed & centered.
        let s = (bounds.width / DECK).min(bounds.height / DECK);
        let off_x = (bounds.width - DECK * s) * 0.5;
        let off_y = (bounds.height - DECK * s) * 0.5;
        let map = |x: f32, y: f32| Point::new(off_x + x * s, off_y + y * s);

        // Draw a whole handle into a 1024-space rect.
        let img = |f: &mut Frame, h: &image::Handle, x: f32, y: f32, w: f32, ht: f32| {
            f.draw_image(
                Rectangle::new(map(x, y), Size::new(w * s, ht * s)),
                canvas::Image::new(h.clone()),
            );
        };

        // Draw the portrait tape (landscape sprite turned +90° => label-right),
        // centre at (cx, cy) in 1024-space, long side = `tall`.
        let tape = |f: &mut Frame, cx: f32, cy: f32, tall: f32| {
            let c = map(cx, cy);
            f.with_save(|f| {
                f.translate(Vector::new(c.x, c.y));
                f.rotate(Radians(std::f32::consts::FRAC_PI_2));
                let lw = tall * s; // becomes the tape's height after the 90° turn
                let lh = tall / SPRITE_RATIO * s; // becomes the tape's width
                f.draw_image(
                    Rectangle::new(Point::new(-lw * 0.5, -lh * 0.5), Size::new(lw, lh)),
                    canvas::Image::new(self.tape.clone()),
                );
            });
        };

        // Tape geometry in the tray.
        let tray_tall = (TRAY_RECT.3 - TRAY_RECT.1) * 0.985;
        let tray_cx = (TRAY_RECT.0 + TRAY_RECT.2) * 0.5;
        let tray_cy = (TRAY_RECT.1 + TRAY_RECT.3) * 0.5;
        // Lift target: centre high enough that the whole tape is above the
        // top edge of the canvas (including any letterbox above the 1024
        // space), so it is gone before the scene switches to the deck.
        let off_top_cy = -off_y / s - tray_tall * 0.5 - 20.0;

        let lid_w = LID.2 - LID.0;
        let lid_h = LID.3 - LID.1;
        let tray_w = TRAY.2 - TRAY.0;
        let tray_h = TRAY.3 - TRAY.1;

        img(&mut frame, &self.tray, TRAY.0, TRAY.1, tray_w, tray_h);

        // Tape sits in the tray until it lifts, then rises out of view.
        if t < LIFT_START {
            tape(&mut frame, tray_cx, tray_cy, tray_tall);
        } else if t < LIFT_END {
            let k = ease_in_quad(((t - LIFT_START) / (LIFT_END - LIFT_START)).clamp(0.0, 1.0));
            tape(&mut frame, tray_cx, lerp(tray_cy, off_top_cy, k), tray_tall);
        }

        // Lid swing: 180° (closed, draped over tray) -> 0° (open, flat left).
        let angle = if t < CLOSED_END {
            std::f32::consts::PI
        } else {
            let k = ease_out_cubic(((t - CLOSED_END) / (SWING_END - CLOSED_END)).clamp(0.0, 1.0));
            lerp(std::f32::consts::PI, 0.0, k)
        };
        let c = angle.cos();
        let w = (lid_w * c.abs()).max(1.0);
        if c < 0.0 {
            // Flipped lid over the tray (hinge on the left, extends right).
            img(&mut frame, &self.lid_flipped, SPINE, LID.1, w, lid_h);
        } else {
            // Open lid to the left of the spine (hinge on the right).
            img(&mut frame, &self.lid, SPINE - w, LID.1, w, lid_h);
        }

        // Cross-fade: the empty deck fades in over the open case.
        if t > LIFT_END {
            let a = (t - LIFT_END) / (FADE_END - LIFT_END);
            draw_empty_deck_faded(&mut frame, bounds.size(), self.skin, a);
        }

        vec![frame.into_geometry()]
    }
}
