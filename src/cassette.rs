//! Multi-skin photo renderer + spool physics + interactive deck buttons.
//!
//! Every skin is a photograph plus a `SkinSpec`: pixel calibration for the
//! hubs, label strip, buttons, and cassette window. Dynamic layers drawn per
//! frame: rotating hub sprites (fixed highlight on top), the pre-rasterized
//! label, the tape sliding in or out of the window, and the keys and FUNCT
//! switch, whose moving parts are cut from each skin's own photo at load.

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
}

#[derive(Debug, Clone, Copy)]
pub enum PressStyle {
    /// Piano-style key on the top edge: sinks straight down into the deck by
    /// `SkinSpec::travel`, its bottom disappearing behind the deck's edge.
    Travel,
    /// Slide switch: the knob slides right along its slot by `SkinSpec::slide`.
    Slide,
}

pub struct ButtonSpec {
    /// Click target.
    pub rect: (f32, f32, f32, f32),
    pub role: DeckButton,
    pub style: PressStyle,
    /// The moving part as photographed (key face, or switch knob with its
    /// shadow), cut from each skin's own body.png at load.
    pub face: (f32, f32, f32, f32),
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
    /// How far a fully pressed key sinks (skin px).
    pub travel: f32,
    /// How far the slide switch knob moves (skin px).
    pub slide: f32,
    pub buttons: &'static [ButtonSpec],
    /// Cassette window opening (x0, y0, x1, y1): the area inside the door
    /// frame where the tape shows. The insert animation slides the tape in
    /// here, clipped, so it comes down from behind the top of the frame.
    pub window: (f32, f32, f32, f32),
}

/// A latched key (Play while the tape plays) rests this far down, as a
/// fraction of full travel: it catches a little above where a full press
/// bottoms out, like a real deck's latch.
pub const LATCH_DEPTH: f32 = 0.72;

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
    travel: 14.0,
    slide: 14.0,
    // Key faces run from their top edge (y 105) down to the deck's top edge
    // (y 146); the FUNCT knob box includes its shadow so it moves with it.
    // Measured off body.png; every colour deck has them at the same pixels.
    buttons: &[
        ButtonSpec { rect: (312.0, 100.0, 372.0, 148.0), role: DeckButton::Play, style: PressStyle::Travel, face: (317.0, 105.0, 377.0, 146.0) },
        ButtonSpec { rect: (372.0, 100.0, 435.0, 148.0), role: DeckButton::Ff, style: PressStyle::Travel, face: (381.0, 105.0, 437.0, 146.0) },
        ButtonSpec { rect: (438.0, 100.0, 512.0, 148.0), role: DeckButton::Rew, style: PressStyle::Travel, face: (443.0, 105.0, 500.0, 146.0) },
        ButtonSpec { rect: (502.0, 100.0, 575.0, 148.0), role: DeckButton::Stop, style: PressStyle::Travel, face: (504.0, 105.0, 562.0, 146.0) },
        ButtonSpec { rect: (600.0, 150.0, 700.0, 190.0), role: DeckButton::Funct, style: PressStyle::Slide, face: (617.0, 163.0, 687.0, 193.0) },
    ],
    // Inside edge of the window's dark border, measured off body.png. The
    // colour decks share this frame position (same seam/lip pixels).
    window: (337.0, 271.0, 694.0, 853.0),
};

/// Colour variants - identical deck to BELT (same calibration, buttons, label),
/// just a recoloured casing. Only the body/empty images differ per skin.
pub const RED: SkinSpec = SkinSpec {
    id: "red", name: "Red",
    w: BELT.w, h: BELT.h, hub_src: BELT.hub_src, hub_tk: BELT.hub_tk, hub_r: BELT.hub_r,
    label: BELT.label, label_vertical: BELT.label_vertical, travel: BELT.travel, slide: BELT.slide,
    buttons: BELT.buttons, window: BELT.window,
};
pub const BLUE: SkinSpec = SkinSpec {
    id: "blue", name: "Blue",
    w: BELT.w, h: BELT.h, hub_src: BELT.hub_src, hub_tk: BELT.hub_tk, hub_r: BELT.hub_r,
    label: BELT.label, label_vertical: BELT.label_vertical, travel: BELT.travel, slide: BELT.slide,
    buttons: BELT.buttons, window: BELT.window,
};
pub const BLACK: SkinSpec = SkinSpec {
    id: "black", name: "Black",
    w: BELT.w, h: BELT.h, hub_src: BELT.hub_src, hub_tk: BELT.hub_tk, hub_r: BELT.hub_r,
    label: BELT.label, label_vertical: BELT.label_vertical, travel: BELT.travel, slide: BELT.slide,
    buttons: BELT.buttons, window: BELT.window,
};
pub const WHITE: SkinSpec = SkinSpec {
    id: "white", name: "White",
    w: BELT.w, h: BELT.h, hub_src: BELT.hub_src, hub_tk: BELT.hub_tk, hub_r: BELT.hub_r,
    label: BELT.label, label_vertical: BELT.label_vertical, travel: BELT.travel, slide: BELT.slide,
    buttons: BELT.buttons, window: BELT.window,
};

// ── Loaded skins (assets embedded) ──────────────────────────────────────────
/// The moving part of one control, cut from the skin's own photo.
pub struct KeyArt {
    /// What shows where the part used to be once it moves: the belt or clip
    /// behind a key, or empty slot behind the switch knob.
    pub fill: image::Handle,
    /// The key face / switch knob itself.
    pub face: image::Handle,
    /// The face shaded as if sunk into the deck; faded in with key depth.
    pub face_dark: image::Handle,
}

pub struct LoadedSkin {
    pub spec: &'static SkinSpec,
    pub body: image::Handle,
    /// Empty deck (open walkman) for the load + case-open animations.
    pub empty: image::Handle,
    pub hub: image::Handle,
    /// Parallel to spec.buttons.
    pub keys: Vec<KeyArt>,
}

macro_rules! h {
    ($path:literal) => {
        image::Handle::from_bytes(include_bytes!($path).to_vec())
    };
}

/// Cut each control's moving part, and what's behind it, out of a skin's
/// body.png. Done per skin so a blue deck shows blue behind its keys.
fn key_art(png: &[u8], spec: &SkinSpec) -> Vec<KeyArt> {
    use ::image as ic;
    let Ok(img) = ic::load_from_memory(png) else {
        return Vec::new();
    };
    let img = img.to_rgba8();
    let handle =
        |im: ic::RgbaImage| image::Handle::from_rgba(im.width(), im.height(), im.into_raw());

    spec.buttons
        .iter()
        .map(|b| {
            let (x0, y0, x1, y1) = (b.face.0 as u32, b.face.1 as u32, b.face.2 as u32, b.face.3 as u32);
            let (w, h) = (x1 - x0, y1 - y0);
            let face = ic::imageops::crop_imm(&img, x0, y0, w, h).to_image();
            let fill = match b.style {
                // Behind a key: the strip just above its top edge (belt or
                // clip), stretched down over the face area.
                PressStyle::Travel => {
                    let strip = ic::imageops::crop_imm(&img, x0, y0 - 10, w, 10).to_image();
                    ic::imageops::resize(&strip, w, h, ic::imageops::FilterType::Triangle)
                }
                // Behind the knob: empty slot. Each row is the average of a
                // shadow-free stretch of slot to the left, repeated across.
                PressStyle::Slide => {
                    let (sx0, sx1) = (x0 - 12, x0 - 2);
                    let mut out = ic::RgbaImage::new(w, h);
                    for y in 0..h {
                        let mut acc = [0u32; 4];
                        for x in sx0..sx1 {
                            let p = img.get_pixel(x, y0 + y).0;
                            for c in 0..4 {
                                acc[c] += p[c] as u32;
                            }
                        }
                        let n = sx1 - sx0;
                        let px = ic::Rgba([
                            (acc[0] / n) as u8,
                            (acc[1] / n) as u8,
                            (acc[2] / n) as u8,
                            (acc[3] / n) as u8,
                        ]);
                        for x in 0..w {
                            out.put_pixel(x, y, px);
                        }
                    }
                    out
                }
            };
            let mut face_dark = face.clone();
            for p in face_dark.pixels_mut() {
                for c in 0..3 {
                    p.0[c] = (p.0[c] as f32 * 0.86) as u8;
                }
            }
            KeyArt { fill: handle(fill), face: handle(face), face_dark: handle(face_dark) }
        })
        .collect()
}

pub fn load_skins() -> Vec<LoadedSkin> {
    // Colour skins reuse belt's silver hub; body/empty (and the key art cut
    // from each body) are per skin.
    macro_rules! belt_hub { () => { h!("../assets/skins/belt/hub.png") }; }
    macro_rules! skin {
        ($spec:expr, $dir:literal) => {{
            let body_png: &[u8] = include_bytes!(concat!("../assets/skins/", $dir, "/body.png"));
            LoadedSkin {
                spec: $spec,
                body: image::Handle::from_bytes(body_png.to_vec()),
                empty: image::Handle::from_bytes(
                    include_bytes!(concat!("../assets/skins/", $dir, "/empty.png")).to_vec(),
                ),
                hub: belt_hub!(),
                keys: key_art(body_png, $spec),
            }
        }};
    }
    vec![
        skin!(&BELT, "belt"),
        skin!(&RED, "red"),
        skin!(&BLUE, "blue"),
        skin!(&BLACK, "black"),
        skin!(&WHITE, "white"),
    ]
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
/// Most controls a skin can have (sizes the per-control depth array).
const MAX_BUTTONS: usize = 8;

#[derive(Debug, Default)]
pub struct DeckState {
    /// Index into spec.buttons of the currently held key.
    pressed: Option<usize>,
    /// True while a press landed on the label strip (pending release).
    label_armed: bool,
    /// How far each control has moved, 0.0 (rest) to 1.0 (fully pressed /
    /// slid). Eased toward its target each frame so keys travel.
    depth: [f32; MAX_BUTTONS],
    /// Time of the last animation frame (None while everything is at rest).
    last: Option<cosmic::iced::time::Instant>,
    /// False until the first frame: a fresh deck (e.g. returning from the
    /// rack while a tape plays) starts with latched controls already in place
    /// instead of animating them down.
    primed: bool,
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
    /// Clicking the cassette's label strip opens the album's art screen.
    pub on_label_click: Message,
    pub skip_mode: bool,
    /// Fill the letterbox with the charcoal field. False when a backdrop sits
    /// behind the deck -- then leave it transparent so the backdrop shows.
    pub fill_letterbox: bool,
    /// How far the tape is out of the window: 0.0 seated, 1.0 gone (empty
    /// deck). The eject and tape-swap animations run this between the two.
    pub drop: f32,
    /// False while an animation owns the deck: clicks are ignored.
    pub interactive: bool,
    /// Hold the Play key down while the tape plays, like a real deck.
    pub play_latched: bool,
}

impl<'a, Message> CassetteScene<'a, Message> {
    /// Where each control should be right now, 0.0 (rest) to 1.0 (fully
    /// pressed / slid): held keys go all the way down, the Play key rests at
    /// the latch while the tape plays, and the FUNCT knob sits on its side
    /// (previewing the other side while it's held).
    fn key_target(&self, state: &DeckState, idx: usize) -> f32 {
        let b = &self.skin.spec.buttons[idx];
        let held = state.pressed == Some(idx);
        match (b.style, b.role) {
            (PressStyle::Slide, _) => {
                if self.skip_mode != held { 1.0 } else { 0.0 }
            }
            (PressStyle::Travel, _) if held => 1.0,
            (PressStyle::Travel, DeckButton::Play) if self.play_latched => LATCH_DEPTH,
            _ => 0.0,
        }
    }

    fn animate_keys<M>(&self, state: &mut DeckState, now: cosmic::iced::time::Instant) -> Option<Action<M>> {
        let n = self.skin.spec.buttons.len().min(MAX_BUTTONS);
        if !state.primed {
            // First frame of a fresh deck: latched controls start in place.
            for i in 0..n {
                if state.pressed != Some(i) {
                    state.depth[i] = self.key_target(state, i);
                }
            }
            state.primed = true;
        }
        let dt = state
            .last
            .map(|t| now.saturating_duration_since(t).as_secs_f32())
            .unwrap_or(1.0 / 60.0)
            .min(0.05);
        let mut moving = false;
        for i in 0..n {
            let target = self.key_target(state, i);
            let d = state.depth[i];
            if (target - d).abs() < 0.004 {
                state.depth[i] = target;
                continue;
            }
            // Keys go down fast and come back up a touch slower (spring).
            let tau = if target > d { 0.020 } else { 0.035 };
            state.depth[i] = d + (target - d) * (1.0 - (-dt / tau).exp());
            moving = true;
        }
        if moving {
            state.last = Some(now);
            Some(Action::request_redraw())
        } else {
            state.last = None;
            None
        }
    }

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
        // Key travel: every frame, ease each control toward where it should
        // be (held down, latched, slid, or at rest) and keep asking for frames
        // until they all settle. Runs even while input is blocked.
        if let Event::Window(cosmic::iced::window::Event::RedrawRequested(now)) = event {
            return self.animate_keys(state, *now);
        }
        if !self.interactive {
            if let Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) = event {
                state.pressed = None;
                state.label_armed = false;
            }
            return None;
        }
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
        if !self.interactive {
            return mouse::Interaction::default();
        }
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

        // Controls away from rest, with how far they've moved. Before the
        // first animation frame, draw them where they belong.
        let n = spec.buttons.len().min(MAX_BUTTONS);
        let down: Vec<(usize, f32)> = (0..n)
            .map(|i| {
                let d = if state.primed { state.depth[i] } else { self.key_target(state, i) };
                (i, d)
            })
            .filter(|&(_, d)| d > 0.0)
            .collect();

        draw_deck(
            &mut frame,
            bounds.size(),
            self.skin,
            self.label,
            self.angle_src,
            self.angle_tk,
            self.drop,
            self.fill_letterbox,
            &down,
        );

        vec![frame.into_geometry()]
    }
}

/// The per-frame parts of the seated tape: rotating hubs (with their fixed
/// highlight) and the pre-rasterized label. `dy` shifts everything down in
/// skin space; the seated tape draws at 0, the animations while it moves.
fn draw_tape_parts(
    frame: &mut Frame,
    map: &SkinMap,
    skin: &LoadedSkin,
    label: Option<&image::Handle>,
    angle_src: f32,
    angle_tk: f32,
    dy: f32,
) {
    let spec = skin.spec;
    let s = map.s;

    // Negative = counter-clockwise (y-down), the direction both reels turn
    // during forward playback.
    for (center, angle) in [(spec.hub_src, angle_src), (spec.hub_tk, angle_tk)] {
        let c = map.point(center.0, center.1 + dy);
        let r = spec.hub_r * s;

        frame.with_save(|f| {
            f.translate(Vector::new(c.x, c.y));
            f.rotate(Radians(-angle));
            f.draw_image(
                Rectangle::new(Point::new(-r, -r), Size::new(2.0 * r, 2.0 * r)),
                canvas::Image::new(skin.hub.clone()),
            );
        });

        // Fixed highlight, unrotated.
        let hl = Point::new(c.x - 0.28 * r, c.y - 0.40 * r);
        for (hr, a) in [(0.60, 0.07), (0.40, 0.13), (0.22, 0.24), (0.10, 0.35)] {
            frame.fill(&Path::circle(hl, hr * r), Color::from_rgba(1.0, 1.0, 1.0, a));
        }
    }

    // Label (pre-rasterized in label.rs, orientation per skin).
    if let Some(label) = label {
        let l = spec.label;
        frame.draw_image(map.rect((l.0, l.1 + dy, l.2, l.3 + dy)), canvas::Image::new(label.clone()));
    }
}

/// Draw controls that have moved: each `(button index, amount)` with amount
/// 0.0 (rest) to 1.0 (fully pressed / slid).
fn draw_pressed_keys(frame: &mut Frame, map: &SkinMap, skin: &LoadedSkin, keys: &[(usize, f32)]) {
    let spec = skin.spec;
    for &(idx, amount) in keys {
        let (Some(btn), Some(art)) = (spec.buttons.get(idx), skin.keys.get(idx)) else {
            continue;
        };
        let face = map.rect(btn.face);
        match btn.style {
            PressStyle::Travel => {
                // What's behind the key shows where it used to be; the key
                // sinks, its bottom passing behind the deck's top edge (the
                // clip), and it darkens a touch as it goes into the body.
                let dy = spec.travel * amount * map.s;
                frame.with_clip(face, |f| {
                    f.draw_image(face, canvas::Image::new(art.fill.clone()));
                    let moved = Rectangle::new(Point::new(face.x, face.y + dy), face.size());
                    f.draw_image(moved, canvas::Image::new(art.face.clone()));
                    f.draw_image(moved, canvas::Image::new(art.face_dark.clone()).opacity(amount));
                });
            }
            PressStyle::Slide => {
                // Empty slot where the knob was, knob (with its shadow) moved.
                let dx = spec.slide * amount * map.s;
                frame.draw_image(face, canvas::Image::new(art.fill.clone()));
                let moved = Rectangle::new(Point::new(face.x + dx, face.y), face.size());
                frame.draw_image(moved, canvas::Image::new(art.face.clone()));
            }
        }
    }
}

/// Draw the deck. `drop` is how far the tape is out of the cassette window:
/// 0.0 seated (the normal player), 1.0 fully out (empty deck), anything
/// between is the tape partway in or out, sliding behind the top of the
/// window frame. The player, the insert, eject, and tape-swap animations all
/// draw through here, so they line up pixel for pixel.
///
/// The moving tape is the skin's own seated cassette from body.png, clipped
/// to the part of the window it covers.
pub fn draw_deck(
    frame: &mut Frame,
    size: Size,
    skin: &LoadedSkin,
    label: Option<&image::Handle>,
    angle_src: f32,
    angle_tk: f32,
    drop: f32,
    fill_letterbox: bool,
    pressed: &[(usize, f32)],
) {
    let spec = skin.spec;
    let map = SkinMap::new(spec, size);
    let s = map.s;
    let drop = drop.clamp(0.0, 1.0);

    // Deliberate dark field behind the deck (fills the letterbox space around
    // the fixed-size photo with an intentional charcoal instead of the
    // theme's default grey). Skipped when a backdrop sits behind the deck, so
    // the letterbox stays transparent and it shows through.
    if fill_letterbox {
        frame.fill_rectangle(Point::ORIGIN, size, Color::from_rgb(0.09, 0.09, 0.11));
    }

    // Photographic body, tape seated: frame, keys, and everything outside the
    // window come from here.
    let deck_rect = Rectangle::new(map.point(0.0, 0.0), Size::new(spec.w * s, spec.h * s));
    frame.draw_image(deck_rect, canvas::Image::new(skin.body.clone()));

    draw_pressed_keys(frame, &map, skin, pressed);

    if drop <= 0.0 {
        // Seated: hubs and label straight on top.
        draw_tape_parts(frame, &map, skin, label, angle_src, angle_tk, 0.0);
        return;
    }

    let (wx0, wy0, wx1, wy1) = spec.window;
    let dy = -(wy1 - wy0) * drop;

    // Empty well inside the window, where the tape isn't.
    frame.with_clip(map.rect(spec.window), |f| {
        f.draw_image(deck_rect, canvas::Image::new(skin.empty.clone()));
    });

    // The tape: the window area of body.png shifted down by dy, clipped to
    // the part of the window it covers.
    let visible_bottom = wy1 + dy;
    if visible_bottom > wy0 {
        frame.with_clip(map.rect((wx0, wy0, wx1, visible_bottom)), |f| {
            let shifted = Rectangle::new(map.point(0.0, dy), Size::new(spec.w * s, spec.h * s));
            f.draw_image(shifted, canvas::Image::new(skin.body.clone()));
            draw_tape_parts(f, &map, skin, label, angle_src, angle_tk, dy);
        });
    }
}

/// The empty deck at partial opacity, for cross-fading into the deck from
/// another scene (the area around it is left clear). Drawn as the body around the window plus the empty well
/// inside it (never the body under the well), so no ghost of the seated tape
/// shows through mid-fade.
pub fn draw_empty_deck_faded(frame: &mut Frame, size: Size, skin: &LoadedSkin, alpha: f32) {
    let spec = skin.spec;
    let map = SkinMap::new(spec, size);
    let s = map.s;
    let a = alpha.clamp(0.0, 1.0);
    // No fill around the deck: the page behind fades on its own (see view()).

    let deck_rect = Rectangle::new(map.point(0.0, 0.0), Size::new(spec.w * s, spec.h * s));
    let (wx0, wy0, wx1, wy1) = spec.window;
    // Four strips around the window: above, below, left, right.
    for r in [
        (0.0, 0.0, spec.w, wy0),
        (0.0, wy1, spec.w, spec.h),
        (0.0, wy0, wx0, wy1),
        (wx1, wy0, spec.w, wy1),
    ] {
        frame.with_clip(map.rect(r), |f| {
            f.draw_image(deck_rect, canvas::Image::new(skin.body.clone()).opacity(a));
        });
    }
    frame.with_clip(map.rect(spec.window), |f| {
        f.draw_image(deck_rect, canvas::Image::new(skin.empty.clone()).opacity(a));
    });
}
