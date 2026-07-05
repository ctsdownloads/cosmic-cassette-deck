//! Cassette-load animation scene.
//!
//! When the user hits Play, before the player appears we show the empty deck
//! and animate a cassette dropping in from the top, rotating from landscape to
//! vertical, seating into the well. Then the app cross-fades to the real deck.
//!
//! This is a self-contained canvas Program with no interaction - purely visual,
//! driven by an externally-supplied progress value in 0.0..=1.0.

use cosmic::iced::widget::canvas::{self, Frame, Geometry};
use cosmic::iced::{Point, Radians, Rectangle, Size, Vector};
use cosmic::widget::image;

/// Deck geometry constants (same 1024x1024 space as the belt skin).
const DECK_W: f32 = 1024.0;
const DECK_H: f32 = 1024.0;
/// Well center = midpoint of the two hub spindles (515,420)/(516,668).
const WELL_CX: f32 = 515.0;
const WELL_CY: f32 = 544.0;
/// Seated cassette long-dimension (tall), sized so its reels match the deck's
/// 248px hub gap. Measured/approved via proof composites.
const SEAT_TALL: f32 = 598.0;
/// The cassette sprite's native aspect (landscape, w/h) - cropped to the
/// opaque cassette with no transparent margins, so the draw rect matches.
const SPRITE_RATIO: f32 = 761.0 / 496.0;

pub struct LoadingScene {
    pub empty: image::Handle,
    pub sprite: image::Handle,
    /// Animation progress, 0.0 (cassette high above) -> 1.0 (seated).
    pub t: f32,
}

fn ease_out_cubic(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}

impl<Message> canvas::Program<Message, cosmic::Theme, cosmic::Renderer> for LoadingScene {
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

        // Fill the letterbox background with the SAME deliberate charcoal the
        // player deck uses (cassette.rs), so the area around the square deck
        // matches and there's no colour jump at the animation handoff.
        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            bounds.size(),
            cosmic::iced::Color::from_rgb(0.09, 0.09, 0.11),
        );

        // Fit the 1024x1024 deck into the bounds, letterboxed and centered
        // (same convention as the player deck).
        let s = (bounds.width / DECK_W).min(bounds.height / DECK_H);
        let off_x = (bounds.width - DECK_W * s) * 0.5;
        let off_y = (bounds.height - DECK_H * s) * 0.5;
        let map = |x: f32, y: f32| Point::new(off_x + x * s, off_y + y * s);

        // 1) Empty deck background.
        frame.draw_image(
            Rectangle::new(
                Point::new(off_x, off_y),
                Size::new(DECK_W * s, DECK_H * s),
            ),
            canvas::Image::new(self.empty.clone()),
        );

        // 2) The dropping/rotating cassette.
        let e = ease_out_cubic(self.t.clamp(0.0, 1.0));
        // Descend: from well above the deck (y ~ 40) down to the well center.
        let start_y = 40.0;
        let cy = start_y + (WELL_CY - start_y) * e;
        let cx = WELL_CX;
        // Rotate: 0° (landscape) -> 90° (vertical seated).
        let angle = (std::f32::consts::FRAC_PI_2) * e;

        // Seated size: long side = SEAT_TALL. The sprite is landscape, so its
        // width maps to the eventual tall dimension after a 90° turn.
        let sprite_w = SEAT_TALL * s;
        let sprite_h = sprite_w / SPRITE_RATIO;

        // The cassette's reel-midpoint is NOT the sprite's geometric centre -
        // the reels sit slightly below centre (the label offsets them). Place
        // by the reel-midpoint so, when seated, the reels land exactly on the
        // deck's hub spindles (no shift at the animation handoff). The offset
        // rotates with the sprite so it tracks through the whole drop.
        // Reel-mid offset from centre, as fraction of the landscape sprite:
        const REEL_OFF_FX: f32 = 0.0005;
        const REEL_OFF_FY: f32 = -0.0294;
        let ox = REEL_OFF_FX * sprite_w;
        let oy = REEL_OFF_FY * sprite_h;
        // Rotate that offset by the current angle and subtract, so the target
        // point (reel-mid) sits at the well centre.
        let (sa, ca) = angle.sin_cos();
        let rx = ox * ca - oy * sa;
        let ry = ox * sa + oy * ca;

        let c = map(cx, cy);
        frame.with_save(|f| {
            f.translate(Vector::new(c.x - rx, c.y - ry));
            f.rotate(Radians(angle));
            f.draw_image(
                Rectangle::new(
                    Point::new(-sprite_w * 0.5, -sprite_h * 0.5),
                    Size::new(sprite_w, sprite_h),
                ),
                canvas::Image::new(self.sprite.clone()),
            );
        });

        vec![frame.into_geometry()]
    }
}
