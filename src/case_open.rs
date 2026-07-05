//! Cassette-case open animation scene.
//!
//! On the reveal screen the user clicks the case; it swings open on its spine
//! hinge, the tape lifts out of the tray, the scene crosses to the walkman
//! deck, and the tape drops into the well - handing off to the player.
//!
//! Self-contained canvas `Program` with no interaction, driven by an external
//! progress value `t` in 0.0..=1.0. All geometry is in the belt skin's
//! 1024×1024 space, letterboxed to the bounds exactly like `loading.rs`. The
//! tape stays portrait (label-right) the whole way - it lifts straight out and
//! drops straight down, no rotation - so nothing wobbles.

use cosmic::iced::widget::canvas::{self, Frame, Geometry};
use cosmic::iced::{Point, Radians, Rectangle, Size, Vector};
use cosmic::widget::image;

const DECK: f32 = 1024.0;

// Keyed case_open.png geometry (measured off the sprite).
const SPINE: f32 = 478.0;
const LID: (f32, f32, f32, f32) = (106.0, 142.0, 478.0, 887.0); // open-lid crop
const TRAY: (f32, f32, f32, f32) = (478.0, 171.0, 924.0, 858.0); // tray crop
const TRAY_RECT: (f32, f32, f32, f32) = (478.0, 172.0, 923.0, 856.0); // where the tape sits

// Walkman seat - identical to loading.rs (SEAT_TALL + reel-offset at 90°).
const SEAT_TALL: f32 = 598.0;
const SPRITE_RATIO: f32 = 761.0 / 496.0;
const WELL_CX: f32 = 515.0;
const WELL_CY: f32 = 544.0;
const REEL_OFF_FX: f32 = 0.0005;
const REEL_OFF_FY: f32 = -0.0294;

// Animation phase boundaries (fractions of the whole `t`).
const CLOSED_END: f32 = 0.08; // brief closed hold before the swing
const SWING_END: f32 = 0.40; // lid finishes opening
const LIFT_START: f32 = 0.50; // tape starts lifting out of the tray
const HANDOFF: f32 = 0.66; // background switches case -> walkman; tape at top
const TOP_Y: f32 = 40.0; // tape centre y at the top, where the drop begins

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

pub struct CaseOpenScene {
    /// 80s bedroom, shown (stacked behind in view()) during the case phase.
    pub bedroom: image::Handle,
    /// Empty walkman deck, drawn during the drop phase.
    pub empty: image::Handle,
    pub tray: image::Handle,
    pub lid: image::Handle,
    pub lid_flipped: image::Handle,
    /// The landscape cassette sprite (rotated 90° when drawn = label-right).
    pub tape: image::Handle,
    /// Progress, 0.0 (closed case) -> 1.0 (seated in the walkman).
    pub t: f32,
}

fn ease_out_cubic(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}
fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x.clamp(0.0, 1.0)
}

impl<Message> canvas::Program<Message, cosmic::Theme, cosmic::Renderer> for CaseOpenScene {
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

        // Fit the 1024×1024 deck space into the bounds, letterboxed & centered
        // (same convention as loading.rs / player_view).
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

        let t = self.t.clamp(0.0, 1.0);

        // Tape geometry.
        let tray_tall = (TRAY_RECT.3 - TRAY_RECT.1) * 0.985;
        let tray_cx = (TRAY_RECT.0 + TRAY_RECT.2) * 0.5;
        let tray_cy = (TRAY_RECT.1 + TRAY_RECT.3) * 0.5;
        // Seated centre folds in the reel-offset at 90° (rx = -oy, ry = ox), so
        // placing the tape centre here lands its reels on the deck hubs - this
        // is algebraically identical to loading.rs's seat.
        let ox = REEL_OFF_FX * SEAT_TALL;
        let oy = REEL_OFF_FY * (SEAT_TALL / SPRITE_RATIO);
        let seat_cx = WELL_CX - (-oy);
        let seat_cy = WELL_CY - ox;

        let lid_w = LID.2 - LID.0;
        let lid_h = LID.3 - LID.1;
        let tray_w = TRAY.2 - TRAY.0;
        let tray_h = TRAY.3 - TRAY.1;

        if t < HANDOFF {
            // ── CASE PHASE ── bedroom shows through (stacked behind in view()).
            img(&mut frame, &self.tray, TRAY.0, TRAY.1, tray_w, tray_h);

            // Tape sits in the tray until it lifts.
            if t < LIFT_START {
                tape(&mut frame, tray_cx, tray_cy, tray_tall);
            } else {
                let k = ease_out_cubic(((t - LIFT_START) / (HANDOFF - LIFT_START)).clamp(0.0, 1.0));
                let tall = lerp(tray_tall, SEAT_TALL, k);
                let cx = lerp(tray_cx, seat_cx, k);
                let cy = lerp(tray_cy, TOP_Y, k);
                tape(&mut frame, cx, cy, tall);
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
        } else {
            // ── WALKMAN PHASE ── cover the bedroom with charcoal + the deck.
            frame.fill_rectangle(
                Point::new(0.0, 0.0),
                bounds.size(),
                cosmic::iced::Color::from_rgb(0.09, 0.09, 0.11),
            );
            frame.draw_image(
                Rectangle::new(Point::new(off_x, off_y), Size::new(DECK * s, DECK * s)),
                canvas::Image::new(self.empty.clone()),
            );
            // Tape drops straight down from the top into the well (no rotation).
            let k = ease_out_cubic(((t - HANDOFF) / (1.0 - HANDOFF)).clamp(0.0, 1.0));
            let cy = lerp(TOP_Y, seat_cy, k);
            tape(&mut frame, seat_cx, cy, SEAT_TALL);
        }

        vec![frame.into_geometry()]
    }
}
