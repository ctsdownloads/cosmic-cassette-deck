//! Multi-skin photo renderer + spool physics + interactive deck buttons.
//!
//! Every skin is a photograph plus a `SkinSpec`: pixel calibration for the
//! hubs, label strip, and buttons. Dynamic layers drawn per frame:
//! rotating hub sprites (fixed highlight on top), the pre-rasterized label,
//! and pressed-state sprites. Button roles are uniform across skins; a skin
//! may implement any subset (e.g. only red has Eject) and may map several
//! physical buttons to one role (red's PLAY key and PAUSE button both
//! toggle playback).

use cosmic::iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path};
use cosmic::iced::{mouse, Color, Point, Radians, Rectangle, Size, Vector};
use cosmic::widget::image;

// ── Physics ────────────────────────────────────────────────────────────────
const PACK_HUB_R: f32 = 0.34;
const PACK_FULL_R: f32 = 1.0;
const TAPE_SPEED: f32 = 3.4;
const INVERT_SPOOL_PHYSICS: bool = false;

pub fn pack_radius(f: f32) -> f32 {
    let f = f.clamp(0.0, 1.0);
    (PACK_HUB_R * PACK_HUB_R + f * (PACK_FULL_R * PACK_FULL_R - PACK_HUB_R * PACK_HUB_R)).sqrt()
}

pub fn spool_omegas(p: f32) -> (f32, f32) {
    let p = p.clamp(0.0, 1.0);
    let (mut w_src, mut w_tk) = (
        TAPE_SPEED / pack_radius(1.0 - p),
        TAPE_SPEED / pack_radius(p),
    );
    if INVERT_SPOOL_PHYSICS {
        std::mem::swap(&mut w_src, &mut w_tk);
    }
    (w_src, w_tk)
}

// ── Skin specification ─────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckButton {
    Play,
    Stop,
    Ff,
    Rew,
    /// Mode toggle (latching): FF/REW switch between wind and track-skip.
    Funct,
    /// Eject the tape -> open the Music Rack.
    Eject,
}

#[derive(Debug, Clone, Copy)]
pub enum PressStyle {
    /// Pre-composited patch, PRESS travel taller: key slides down behind
    /// the deck edge (protruding keys).
    Travel,
    /// Darkened crop drawn at the exact rect (recessed/round buttons).
    Darken,
}

pub struct ButtonSpec {
    pub rect: (f32, f32, f32, f32),
    pub role: DeckButton,
    pub style: PressStyle,
}

pub struct SkinSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub w: f32,
    pub h: f32,
    pub hub_src: (f32, f32),
    pub hub_tk: (f32, f32),
    pub hub_r: f32,
    /// Label strip (x0, y0, x1, y1).
    pub label: (f32, f32, f32, f32),
    /// True if label text reads bottom-to-top (portrait cassette).
    pub label_vertical: bool,
    pub travel: f32,
    pub buttons: &'static [ButtonSpec],
}


/// Belt/denim skin. Same portrait deck as silver, on a leather-belt-and-denim
/// background. Cassette flipped: label on the RIGHT, reels on the LEFT.
/// Coordinates verified against the 1024x1024 render via grid + pixel analysis
/// and approved via proof composites.
pub const BELT: SkinSpec = SkinSpec {
    id: "belt",
    name: "Denim Belt",
    w: 1024.0,
    h: 1024.0,
    // Flipped deck (body.png). Hub centers are the TRUE rotation centers, found
    // by 180° rotational-symmetry sweep and approved via proof composite.
    // The flip was HORIZONTAL, so top/bottom is unchanged:
    // source spool = TOP, take-up = BOTTOM. Reels are now on the LEFT.
    hub_src: (515.0, 420.0),
    hub_tk: (516.0, 668.0),
    hub_r: 56.0,
    // Label strip now on the RIGHT side of the window, still vertical.
    label: (592.0, 288.0, 682.0, 820.0),
    label_vertical: true,
    travel: 8.0,
    buttons: &[
        ButtonSpec { rect: (312.0, 100.0, 372.0, 148.0), role: DeckButton::Play, style: PressStyle::Travel },
        ButtonSpec { rect: (372.0, 100.0, 435.0, 148.0), role: DeckButton::Ff, style: PressStyle::Travel },
        ButtonSpec { rect: (438.0, 100.0, 512.0, 148.0), role: DeckButton::Rew, style: PressStyle::Travel },
        ButtonSpec { rect: (502.0, 100.0, 575.0, 148.0), role: DeckButton::Stop, style: PressStyle::Travel },
        ButtonSpec { rect: (600.0, 150.0, 700.0, 190.0), role: DeckButton::Funct, style: PressStyle::Darken },
    ],
};

/// Colour variants - identical deck to BELT (same calibration, buttons, label),
/// just a recoloured casing. Only the body/empty images differ per skin.
pub const RED: SkinSpec = SkinSpec {
    id: "red", name: "Red",
    w: BELT.w, h: BELT.h, hub_src: BELT.hub_src, hub_tk: BELT.hub_tk, hub_r: BELT.hub_r,
    label: BELT.label, label_vertical: BELT.label_vertical, travel: BELT.travel, buttons: BELT.buttons,
};
pub const BLUE: SkinSpec = SkinSpec {
    id: "blue", name: "Blue",
    w: BELT.w, h: BELT.h, hub_src: BELT.hub_src, hub_tk: BELT.hub_tk, hub_r: BELT.hub_r,
    label: BELT.label, label_vertical: BELT.label_vertical, travel: BELT.travel, buttons: BELT.buttons,
};
pub const BLACK: SkinSpec = SkinSpec {
    id: "black", name: "Black",
    w: BELT.w, h: BELT.h, hub_src: BELT.hub_src, hub_tk: BELT.hub_tk, hub_r: BELT.hub_r,
    label: BELT.label, label_vertical: BELT.label_vertical, travel: BELT.travel, buttons: BELT.buttons,
};
pub const WHITE: SkinSpec = SkinSpec {
    id: "white", name: "White",
    w: BELT.w, h: BELT.h, hub_src: BELT.hub_src, hub_tk: BELT.hub_tk, hub_r: BELT.hub_r,
    label: BELT.label, label_vertical: BELT.label_vertical, travel: BELT.travel, buttons: BELT.buttons,
};

// ── Loaded skins (assets embedded) ──────────────────────────────────────────
pub struct LoadedSkin {
    pub spec: &'static SkinSpec,
    pub body: image::Handle,
    /// Empty deck (open walkman) for the load + case-open animations.
    pub empty: image::Handle,
    pub hub: image::Handle,
    /// Parallel to spec.buttons.
    pub pressed: Vec<image::Handle>,
}

macro_rules! h {
    ($path:literal) => {
        image::Handle::from_bytes(include_bytes!($path).to_vec())
    };
}

pub fn load_skins() -> Vec<LoadedSkin> {
    // Colour skins reuse belt's silver hub + pressed-key sprites (the colour
    // decks keep silver top buttons, so these match); only body/empty differ.
    macro_rules! belt_hub { () => { h!("../assets/skins/belt/hub.png") }; }
    macro_rules! belt_btns {
        () => { vec![
            h!("../assets/skins/belt/btn_0.png"), h!("../assets/skins/belt/btn_1.png"),
            h!("../assets/skins/belt/btn_2.png"), h!("../assets/skins/belt/btn_3.png"),
            h!("../assets/skins/belt/btn_4.png"),
        ] };
    }
    vec![
        LoadedSkin {
            spec: &BELT,
            body: h!("../assets/skins/belt/body.png"),
            empty: h!("../assets/skins/belt/empty.png"),
            hub: belt_hub!(),
            pressed: belt_btns!(),
        },
        LoadedSkin {
            spec: &RED,
            body: h!("../assets/skins/red/body.png"),
            empty: h!("../assets/skins/red/empty.png"),
            hub: belt_hub!(),
            pressed: belt_btns!(),
        },
        LoadedSkin {
            spec: &BLUE,
            body: h!("../assets/skins/blue/body.png"),
            empty: h!("../assets/skins/blue/empty.png"),
            hub: belt_hub!(),
            pressed: belt_btns!(),
        },
        LoadedSkin {
            spec: &BLACK,
            body: h!("../assets/skins/black/body.png"),
            empty: h!("../assets/skins/black/empty.png"),
            hub: belt_hub!(),
            pressed: belt_btns!(),
        },
        LoadedSkin {
            spec: &WHITE,
            body: h!("../assets/skins/white/body.png"),
            empty: h!("../assets/skins/white/empty.png"),
            hub: belt_hub!(),
            pressed: belt_btns!(),
        },
    ]
}

pub fn case_handle() -> image::Handle {
    h!("../assets/case_blank.png")
}

pub fn bedroom_handle() -> image::Handle {
    h!("../assets/bedroom_bg.png")
}

// ── Coordinate mapping ──────────────────────────────────────────────────────
struct SkinMap {
    s: f32,
    ox: f32,
    oy: f32,
}

impl SkinMap {
    fn new(spec: &SkinSpec, size: Size) -> Self {
        let s = (size.width / spec.w).min(size.height / spec.h);
        Self {
            s,
            ox: (size.width - spec.w * s) / 2.0,
            oy: (size.height - spec.h * s) / 2.0,
        }
    }

    fn point(&self, x: f32, y: f32) -> Point {
        Point::new(self.ox + x * self.s, self.oy + y * self.s)
    }

    fn rect(&self, r: (f32, f32, f32, f32)) -> Rectangle {
        Rectangle::new(
            self.point(r.0, r.1),
            Size::new((r.2 - r.0) * self.s, (r.3 - r.1) * self.s),
        )
    }
}

// ── Interaction state ───────────────────────────────────────────────────────
#[derive(Debug, Default)]
pub struct DeckState {
    /// Index into spec.buttons of the currently held key.
    pressed: Option<usize>,
    /// True while a press landed on the label strip (pending release).
    label_armed: bool,
}

// ── Scene ───────────────────────────────────────────────────────────────────
pub struct CassetteScene<'a, Message> {
    pub skin: &'a LoadedSkin,
    pub label: Option<&'a image::Handle>,
    pub angle_src: f32,
    pub angle_tk: f32,
    pub on_toggle_play: Message,
    pub on_stop: Message,
    pub on_wind_fwd: Message,
    pub on_wind_back: Message,
    pub on_wind_stop: Message,
    pub on_next: Message,
    pub on_prev: Message,
    pub on_toggle_mode: Message,
    pub on_eject: Message,
    /// Clicking the cassette's label strip opens the album's art screen.
    pub on_label_click: Message,
    pub skip_mode: bool,
}

impl<'a, Message> CassetteScene<'a, Message> {
    fn hit(&self, map: &SkinMap, pos: Point) -> Option<usize> {
        self.skin
            .spec
            .buttons
            .iter()
            .position(|b| map.rect(b.rect).contains(pos))
    }
}

impl<'a, Message: Clone> canvas::Program<Message, cosmic::Theme, cosmic::Renderer>
    for CassetteScene<'a, Message>
{
    type State = DeckState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        let map = SkinMap::new(self.skin.spec, bounds.size());

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let pos = cursor.position_in(bounds)?;
                if let Some(idx) = self.hit(&map, pos) {
                    state.pressed = Some(idx);
                    let role = self.skin.spec.buttons[idx].role;
                    // Wind mode: FF/REW act on PRESS like held keys.
                    return Some(match role {
                        DeckButton::Ff if !self.skip_mode => {
                            Action::publish(self.on_wind_fwd.clone()).and_capture()
                        }
                        DeckButton::Rew if !self.skip_mode => {
                            Action::publish(self.on_wind_back.clone()).and_capture()
                        }
                        _ => Action::request_redraw().and_capture(),
                    });
                }
                // Not a transport button - is it the label strip? Clicking the
                // label opens this album's art screen.
                if map.rect(self.skin.spec.label).contains(pos) {
                    state.label_armed = true;
                    return Some(Action::request_redraw().and_capture());
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                // Label click: released inside the label strip after arming.
                if state.label_armed {
                    state.label_armed = false;
                    let on_label = cursor
                        .position_in(bounds)
                        .map(|p| map.rect(self.skin.spec.label).contains(p))
                        .unwrap_or(false);
                    if on_label {
                        return Some(
                            Action::publish(self.on_label_click.clone()).and_capture(),
                        );
                    }
                    return Some(Action::request_redraw());
                }
                if let Some(armed) = state.pressed.take() {
                    let released_on = cursor.position_in(bounds).and_then(|p| self.hit(&map, p));
                    let inside = released_on == Some(armed);
                    let role = self.skin.spec.buttons[armed].role;
                    let msg = match role {
                        // Wind keys: releasing ALWAYS stops the wind.
                        DeckButton::Ff | DeckButton::Rew if !self.skip_mode => {
                            Some(self.on_wind_stop.clone())
                        }
                        DeckButton::Ff if inside => Some(self.on_next.clone()),
                        DeckButton::Rew if inside => Some(self.on_prev.clone()),
                        DeckButton::Play if inside => Some(self.on_toggle_play.clone()),
                        DeckButton::Stop if inside => Some(self.on_stop.clone()),
                        DeckButton::Funct if inside => Some(self.on_toggle_mode.clone()),
                        DeckButton::Eject if inside => Some(self.on_eject.clone()),
                        _ => None,
                    };
                    return Some(match msg {
                        Some(m) => Action::publish(m).and_capture(),
                        None => Action::request_redraw(),
                    });
                }
            }
            _ => {}
        }
        None
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let map = SkinMap::new(self.skin.spec, bounds.size());
        if let Some(p) = cursor.position_in(bounds) {
            // Transport buttons or the clickable label strip.
            if self.hit(&map, p).is_some() || map.rect(self.skin.spec.label).contains(p) {
                return mouse::Interaction::Pointer;
            }
        }
        mouse::Interaction::default()
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &cosmic::Renderer,
        _theme: &cosmic::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<cosmic::Renderer>> {
        let spec = self.skin.spec;
        let mut frame = Frame::new(renderer, bounds.size());
        let map = SkinMap::new(spec, bounds.size());
        let s = map.s;

        // 0) Deliberate dark field behind the deck (fills the letterbox space
        //    around the fixed-size photo with an intentional charcoal instead
        //    of the theme's default grey).
        frame.fill_rectangle(
            cosmic::iced::Point::ORIGIN,
            bounds.size(),
            cosmic::iced::Color::from_rgb(0.09, 0.09, 0.11),
        );

        // 1) Photographic body.
        frame.draw_image(
            Rectangle::new(map.point(0.0, 0.0), Size::new(spec.w * s, spec.h * s)),
            canvas::Image::new(self.skin.body.clone()),
        );

        // 2) Depressed keys: transient presses, plus the latched Funct
        //    toggle (XOR previews the upcoming state while it's held).
        let funct_idx = spec.buttons.iter().position(|b| matches!(b.role, DeckButton::Funct));
        let mut down: Vec<usize> = Vec::new();
        if let Some(p) = state.pressed {
            if Some(p) != funct_idx {
                down.push(p);
            }
        }
        if let Some(f) = funct_idx {
            if self.skip_mode != (state.pressed == Some(f)) {
                down.push(f);
            }
        }
        for idx in down {
            let btn = &spec.buttons[idx];
            let sprite = canvas::Image::new(self.skin.pressed[idx].clone());
            match btn.style {
                PressStyle::Travel => {
                    // Patch is pre-composited `travel` px taller: key slides
                    // down, bottom clipped behind the deck edge.
                    let base = map.rect(btn.rect);
                    let dst = Rectangle::new(
                        Point::new(base.x, base.y - spec.travel * s),
                        Size::new(base.width, base.height + spec.travel * s),
                    );
                    frame.draw_image(dst, sprite);
                }
                PressStyle::Darken => {
                    frame.draw_image(map.rect(btn.rect), sprite);
                }
            }
        }

        // 3) Rotating hubs. Negative = counter-clockwise (y-down), the
        //    direction both reels turn during forward playback.
        for (center, angle) in [(spec.hub_src, self.angle_src), (spec.hub_tk, self.angle_tk)] {
            let c = map.point(center.0, center.1);
            let r = spec.hub_r * s;

            frame.with_save(|f| {
                f.translate(Vector::new(c.x, c.y));
                f.rotate(Radians(-angle));
                f.draw_image(
                    Rectangle::new(Point::new(-r, -r), Size::new(2.0 * r, 2.0 * r)),
                    canvas::Image::new(self.skin.hub.clone()),
                );
            });

            // Fixed highlight, unrotated.
            let hl = Point::new(c.x - 0.28 * r, c.y - 0.40 * r);
            for (hr, a) in [(0.60, 0.07), (0.40, 0.13), (0.22, 0.24), (0.10, 0.35)] {
                frame.fill(&Path::circle(hl, hr * r), Color::from_rgba(1.0, 1.0, 1.0, a));
            }
        }

        // 4) Label (pre-rasterized in label.rs, orientation per skin).
        if let Some(label) = self.label {
            frame.draw_image(map.rect(spec.label), canvas::Image::new(label.clone()));
        }

        vec![frame.into_geometry()]
    }
}
