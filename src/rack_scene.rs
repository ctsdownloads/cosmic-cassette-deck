//! The browse view: flipping through cassette cases in an 80s bedroom.
//!
//! A single "current" album's case sits large and centred, its cover filling
//! the J-card; the neighbouring albums fan out behind it to either side,
//! receding and dimmed - like thumbing through a stack of tapes. The bedroom
//! photo is the app background (dimmed + behind the cases).
//!
//! Navigation is a single index into the (alphabetical) library; < > or the
//! side click-zones step through all albums. Clicking the front case plays it.
//!
//! Covers are composited onto each case by pre-baking a filled-case RGBA image
//! per visible slot on the app side (case_scene::compose), handed in as
//! `cases[]`; slots without art show the blank case.

use cosmic::iced::widget::canvas::{self, Action, Event, Frame, Geometry};
use cosmic::iced::{mouse, Point, Rectangle, Size};
use cosmic::widget::image;

use crate::app::Message;
use crate::library::Album;

/// How many neighbours fan out on each side of the centre case.
pub const FAN_EACH_SIDE: usize = 3;

pub struct RackScene<'a> {
    /// The bedroom background.
    pub background: &'a image::Handle,
    /// Pre-composed case images for the visible window, centre first, then
    /// alternating neighbours. Index 0 = centre; 1,2 = ±1; 3,4 = ±2; etc.
    /// (Built by the app so cover compositing happens once per change.)
    pub cases: &'a [image::Handle],
    pub albums: &'a [Album],
    pub current: usize,
}

/// Aspect ratio of a case sprite (from case_transparent.png, 480×735).
const CASE_AR: f32 = 480.0 / 735.0;

struct Layout {
    center: Point,
    front_h: f32,
}

impl Layout {
    fn new(size: Size) -> Self {
        // Front case is ~62% of the window height, leaving room for title/nav.
        let front_h = size.height * 0.58;
        Self {
            center: Point::new(size.width / 2.0, size.height * 0.46),
            front_h,
        }
    }

    /// Rectangle for the centre case (depth 0) or a neighbour at `depth`≥1 on
    /// `side` (−1 left, +1 right).
    fn case_rect(&self, depth: usize, side: f32) -> Rectangle {
        let scale = 1.0 - depth as f32 * 0.11;
        let h = self.front_h * scale;
        let w = h * CASE_AR;
        let dx = side * depth as f32 * (self.front_h * 0.24);
        let dy = depth as f32 * (self.front_h * 0.02);
        Rectangle::new(
            Point::new(
                self.center.x + dx - w / 2.0,
                self.center.y + dy - h / 2.0,
            ),
            Size::new(w, h),
        )
    }

    fn front_rect(&self) -> Rectangle {
        self.case_rect(0, 0.0)
    }

    /// Left / right click zones for stepping through the stack.
    fn left_zone(&self, size: Size) -> Rectangle {
        Rectangle::new(Point::ORIGIN, Size::new(size.width * 0.18, size.height))
    }
    fn right_zone(&self, size: Size) -> Rectangle {
        Rectangle::new(
            Point::new(size.width * 0.82, 0.0),
            Size::new(size.width * 0.18, size.height),
        )
    }
}

impl<'a> canvas::Program<Message, cosmic::Theme, cosmic::Renderer> for RackScene<'a> {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let pos = cursor.position_in(bounds)?;
            let l = Layout::new(bounds.size());
            // Side zones step; front case plays.
            if l.left_zone(bounds.size()).contains(pos) {
                return Some(Action::publish(Message::BrowseStep(-1)).and_capture());
            }
            if l.right_zone(bounds.size()).contains(pos) {
                return Some(Action::publish(Message::BrowseStep(1)).and_capture());
            }
            if l.front_rect().contains(pos) && self.current < self.albums.len() {
                return Some(
                    Action::publish(Message::AlbumSelected(self.current)).and_capture(),
                );
            }
        }
        None
    }

    fn mouse_interaction(
        &self,
        _state: &(),
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if let Some(pos) = cursor.position_in(bounds) {
            let l = Layout::new(bounds.size());
            if l.left_zone(bounds.size()).contains(pos)
                || l.right_zone(bounds.size()).contains(pos)
                || l.front_rect().contains(pos)
            {
                return mouse::Interaction::Pointer;
            }
        }
        mouse::Interaction::default()
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &cosmic::Renderer,
        _theme: &cosmic::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<cosmic::Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        let l = Layout::new(size);

        // Background: cover the window with the bedroom, dimmed for focus.
        // (Fill by scaling to cover; the image is square, window is wider.)
        let bg_scale = (size.width / 1024.0).max(size.height / 1024.0);
        let bw = 1024.0 * bg_scale;
        let bh = 1024.0 * bg_scale;
        frame.draw_image(
            Rectangle::new(
                Point::new((size.width - bw) / 2.0, (size.height - bh) / 2.0),
                Size::new(bw, bh),
            ),
            canvas::Image::new(self.background.clone()),
        );
        // Dim veil.
        frame.fill_rectangle(
            Point::ORIGIN,
            size,
            cosmic::iced::Color::from_rgba(0.05, 0.04, 0.06, 0.42),
        );

        // Fan the neighbours back-to-front: farthest depth first, both sides,
        // then the centre case last so it sits on top.
        // cases[] layout: 0=centre, 1=+1, 2=−1, 3=+2, 4=−2, ...
        for depth in (1..=FAN_EACH_SIDE).rev() {
            // right neighbour at this depth = index (depth*2 - 1)
            let ri = depth * 2 - 1;
            if let Some(h) = self.cases.get(ri) {
                frame.draw_image(l.case_rect(depth, 1.0), canvas::Image::new(h.clone()));
            }
            // left neighbour = index depth*2
            let li = depth * 2;
            if let Some(h) = self.cases.get(li) {
                frame.draw_image(l.case_rect(depth, -1.0), canvas::Image::new(h.clone()));
            }
        }
        // Centre case on top.
        if let Some(h) = self.cases.first() {
            frame.draw_image(l.front_rect(), canvas::Image::new(h.clone()));
        }

        vec![frame.into_geometry()]
    }
}
