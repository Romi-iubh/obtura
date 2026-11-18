//! Choosing a part of the screen: one borderless fullscreen window per monitor, with ONE
//! selection shared by all of them in desktop coordinates (so a drag may cross monitors).
//!
//! The overlay shows a still picture of the screen taken just before (on GNOME a fullscreen
//! window cannot be see-through: the compositor paints black behind it). The picture is taken
//! through screen sharing, so GNOME does not flash (see `capture.rs`).
//!
//! The region chosen last time is proposed: Enter takes it again, a drag elsewhere starts a
//! new one. Drag a handle to resize, drag inside to move, arrow keys nudge by one pixel,
//! Shift+arrows resize by one pixel. Enter or double-click accepts, Escape or right-click
//! cancels. When recording, a click (no drag) chooses the whole screen under the pointer,
//! and a countdown runs on the overlay before it goes.

use gtk::prelude::*;
use gtk::{cairo, gdk, gdk_pixbuf::Pixbuf, glib};
use obtura_image::{Bitmap, Rect};
use std::{
    cell::RefCell,
    rc::Rc,
    time::Duration,
};

/// What the overlay is for: the hints differ, and recording counts down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Capture,
    Record,
    /// Record a whole screen: the screen under the pointer lights up, a click chooses it (no
    /// region to drag).
    RecordScreen,
    /// A scrolling capture: the part of the screen that scrolls is chosen like a region.
    Scroll,
    /// Measure on the screen: a drag draws a ruler (length, width, height, angle in pixels);
    /// Enter copies the measurement, Escape closes.
    Measure,
}

/// A ruler drawn on the screen: from `a` to `b`, desktop (logical) coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ruler {
    pub a: (f64, f64),
    pub b: (f64, f64),
}

impl Ruler {
    /// Width, height (signed), length and angle (degrees, 0 to the right, counter-clockwise as
    /// on paper) in physical pixels.
    pub fn measure(&self, fx: f64, fy: f64) -> (f64, f64, f64, f64) {
        let (dx, dy) = (((self.b.0 - self.a.0) * fx).round(), ((self.b.1 - self.a.1) * fy).round());
        let len = dx.hypot(dy);
        let deg = (-dy).atan2(dx).to_degrees();
        (dx, dy, len, if deg < 0.0 { deg + 360.0 } else { deg })
    }

    /// The measurement as copied: "300 × 86 px, 312 px long at 16°".
    pub fn said(&self, fx: f64, fy: f64) -> String {
        let (dx, dy, len, deg) = self.measure(fx, fy);
        format!("{} × {} px, {} px long at {}°", dx.abs(), dy.abs(), len.round(), deg.round() as i64 % 360)
    }

    /// Shift: the end goes straight across, straight down, or at 45°.
    fn straightened(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let step = std::f64::consts::FRAC_PI_4;
        let angle = (dy.atan2(dx) / step).round() * step;
        let len = dx.hypot(dy);
        let (c, s) = (angle.cos(), angle.sin());
        // Projected onto the chosen direction.
        let along = dx * c + dy * s;
        let _ = len;
        (a.0 + along * c, a.1 + along * s)
    }
}

/// The part of the screen chosen, in desktop (logical) coordinates from the top-left corner of
/// the whole desktop, with the desktop's logical size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub desktop: (f64, f64),
}

impl Area {
    /// This area in pixels of a picture of the whole desktop that is `width` x `height`
    /// (physical pixels), or `None` if it lies outside.
    pub fn in_pixels(&self, width: u32, height: u32) -> Option<Rect> {
        let fx = width as f64 / self.desktop.0;
        let fy = height as f64 / self.desktop.1;
        Rect::new((self.x * fx).round() as i32, (self.y * fy).round() as i32, (self.w * fx).round().max(1.0) as u32, (self.h * fy).round().max(1.0) as u32).clamp_to(width, height)
    }
}

/// The overlay's answer: the chosen area, or `None` if the user cancelled.
pub type Done = Box<dyn FnOnce(Option<Area>)>;

/// Handles are drawn this big and grabbed within this many logical pixels.
const HANDLE: f64 = 8.0;
const GRAB: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct R {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl R {
    fn from_points(x0: f64, y0: f64, x1: f64, y1: f64) -> R {
        R { x: x0.min(x1), y: y0.min(y1), w: (x1 - x0).abs(), h: (y1 - y0).abs() }
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }

    /// Whether this rectangle holds all of `o` but for a pixel or two at the edges.
    fn covers(&self, o: &R) -> bool {
        self.x <= o.x + 2.0 && self.y <= o.y + 2.0 && self.x + self.w >= o.x + o.w - 2.0 && self.y + self.h >= o.y + o.h - 2.0
    }

    fn intersects(&self, o: &R) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    /// The part of this rectangle inside a `w` x `h` desktop, if at least two pixels square.
    fn inside(&self, w: f64, h: f64) -> Option<R> {
        let (x0, y0) = (self.x.max(0.0), self.y.max(0.0));
        let (x1, y1) = ((self.x + self.w).min(w), (self.y + self.h).min(h));
        (x1 - x0 >= 2.0 && y1 - y0 >= 2.0).then_some(R { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }
    /// The eight resize handles as (hx, hy) in {-1, 0, 1}, with their centre.
    fn handles(&self) -> [((i8, i8), (f64, f64)); 8] {
        let (l, c, r) = (self.x, self.x + self.w / 2.0, self.x + self.w);
        let (t, m, b) = (self.y, self.y + self.h / 2.0, self.y + self.h);
        [((-1, -1), (l, t)), ((0, -1), (c, t)), ((1, -1), (r, t)), ((-1, 0), (l, m)), ((1, 0), (r, m)), ((-1, 1), (l, b)), ((0, 1), (c, b)), ((1, 1), (r, b))]
    }

    fn handle_at(&self, x: f64, y: f64) -> Option<(i8, i8)> {
        self.handles().into_iter().find(|(_, (hx, hy))| (x - hx).abs() <= GRAB && (y - hy).abs() <= GRAB).map(|(h, _)| h)
    }

    /// This rectangle with the edges named by `handle` moved by (dx, dy).
    fn resized(&self, handle: (i8, i8), dx: f64, dy: f64) -> R {
        let (mut x0, mut y0, mut x1, mut y1) = (self.x, self.y, self.x + self.w, self.y + self.h);
        match handle.0 {
            -1 => x0 += dx,
            1 => x1 += dx,
            _ => {}
        }
        match handle.1 {
            -1 => y0 += dy,
            1 => y1 += dy,
            _ => {}
        }
        R::from_points(x0, y0, x1, y1)
    }
}

enum Drag {
    New { start: (f64, f64) },
    Move { origin: R },
    Resize { origin: R, handle: (i8, i8) },
}

struct Mon {
    /// Top-left of this monitor in desktop (logical) coordinates.
    origin: (f64, f64),
    /// Its logical size.
    size: (f64, f64),
    area: gtk::DrawingArea,
    window: gtk::Window,
}

struct State {
    sel: Option<R>,
    /// The selection before a click that did not drag, restored on release.
    before_click: Option<R>,
    drag: Option<Drag>,
    mons: Vec<Mon>,
    /// Physical pixels per logical pixel.
    fx: f64,
    fy: f64,
    /// The desktop in physical pixels (estimated from the monitors' scale when live).
    image: (u32, u32),
    /// The desktop in logical pixels.
    desktop: (f64, f64),
    done: Option<Done>,
    /// Colour picking: the shot (for the loupe) and where the pointer is (desktop coordinates).
    pick: Option<Rc<Bitmap>>,
    pointer: Option<(f64, f64)>,
    purpose: Purpose,
    /// The selection shown is still the one proposed (the region chosen last time).
    proposed: bool,
    /// Seconds to count down after accepting (recording), and the seconds left once started.
    count_from: u32,
    countdown: Option<u32>,
    /// When the picture holds only some screens: their parts of the desktop.
    shared: Option<Vec<R>>,
    /// Asks GNOME again which screens to share (S, when some are not).
    share_again: Option<Box<dyn FnOnce()>>,
    /// Measuring: the ruler drawn.
    ruler: Option<Ruler>,
}

impl State {
    /// Whether some screen is not in the picture.
    fn screen_missing(&self) -> bool {
        self.shared.as_ref().is_some_and(|shared| self.mons.iter().any(|m| !shared.iter().any(|r| r.intersects(&R { x: m.origin.0, y: m.origin.1, w: m.size.0, h: m.size.1 }))))
    }
}

impl State {
    fn redraw(&self) {
        for m in &self.mons {
            m.area.queue_draw();
        }
    }

    /// The selection in physical pixels, clamped to the desktop.
    fn physical(&self) -> Option<Rect> {
        let r = self.sel?;
        let rect = Rect::new((r.x * self.fx).round() as i32, (r.y * self.fy).round() as i32, (r.w * self.fx).round().max(1.0) as u32, (r.h * self.fy).round().max(1.0) as u32);
        rect.clamp_to(self.image.0, self.image.1)
    }

    fn area(&self) -> Option<Area> {
        self.sel.map(|r| Area { x: r.x, y: r.y, w: r.w, h: r.h, desktop: self.desktop })
    }

    /// Screen `i` as a whole, in desktop coordinates.
    fn screen_rect(&self, i: usize) -> R {
        let m = &self.mons[i];
        R { x: m.origin.0, y: m.origin.1, w: m.size.0, h: m.size.1 }
    }

    fn select_screen(&mut self, i: usize) {
        self.proposed = false;
        self.sel = Some(self.screen_rect(i));
    }
}

/// Accepting (with the selection) or cancelling. When recording, accepting first counts down
/// on the overlay; Escape during the countdown still cancels.
fn finish(st: &Rc<RefCell<State>>, accept: bool) {
    if accept {
        let mut s = st.borrow_mut();
        if s.purpose == Purpose::Measure {
            // The ruler as the answer (see `measure`).
            let Some(r) = s.ruler else { return };
            s.sel = Some(R { x: r.a.0, y: r.a.1, w: r.b.0 - r.a.0, h: r.b.1 - r.a.1 });
        } else if s.sel.is_none() && s.pick.is_none() {
            return;
        }
        match s.countdown {
            None if s.count_from > 0 => {
                s.countdown = Some(s.count_from);
                s.drag = None;
                for m in &s.mons {
                    m.area.set_cursor_from_name(Some("default"));
                }
                s.redraw();
                drop(s);
                return tick(st);
            }
            Some(left) if left > 0 => return,
            _ => {}
        }
    }
    let (answer, done, windows) = {
        let mut s = st.borrow_mut();
        let answer = if accept { s.area() } else { None };
        (answer, s.done.take(), s.mons.iter().map(|m| m.window.clone()).collect::<Vec<_>>())
    };
    let Some(done) = done else { return };
    for w in windows {
        w.destroy();
    }
    done(answer);
}

fn tick(st: &Rc<RefCell<State>>) {
    let st = st.clone();
    glib::timeout_add_local_once(Duration::from_secs(1), move || {
        let left = {
            let mut s = st.borrow_mut();
            if s.done.is_none() {
                return; // cancelled meanwhile
            }
            let left = s.countdown.unwrap_or(1).saturating_sub(1);
            s.countdown = Some(left);
            s.redraw();
            left
        };
        if left == 0 {
            finish(&st, true);
        } else {
            tick(&st);
        }
    });
}

fn list_monitors() -> Result<Vec<(gdk::Monitor, gdk::Rectangle)>, String> {
    let display = gdk::Display::default().ok_or("no display")?;
    let list = display.monitors();
    let mut monitors = Vec::new();
    for i in 0..list.n_items() {
        let m = list.item(i).and_downcast::<gdk::Monitor>().ok_or("monitor list")?;
        let g = m.geometry();
        monitors.push((m, g));
    }
    if monitors.is_empty() {
        return Err("no monitor".into());
    }
    Ok(monitors)
}

/// One monitor in desktop (logical) coordinates from the desktop's top-left corner, with its
/// scale (physical pixels per logical pixel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

/// The monitors, and where the desktop's top-left corner is in the compositor's coordinates.
pub struct Desktop {
    pub screens: Vec<Screen>,
    pub origin: (i32, i32),
}

pub fn desktop() -> Result<Desktop, String> {
    let monitors = list_monitors()?;
    let min_x = monitors.iter().map(|(_, g)| g.x()).min().unwrap_or(0);
    let min_y = monitors.iter().map(|(_, g)| g.y()).min().unwrap_or(0);
    let screens = monitors.iter().map(|(m, g)| Screen { x: (g.x() - min_x) as f64, y: (g.y() - min_y) as f64, w: g.width() as f64, h: g.height() as f64, scale: m.scale() }).collect();
    Ok(Desktop { screens, origin: (min_x, min_y) })
}

/// Shows the overlay over `shot` (a picture of the whole desktop, physical pixels), proposing
/// `last` (the region chosen last time, physical pixels of the shot). When recording,
/// `countdown` seconds are counted on the overlay after accepting. `shared` (desktop logical
/// rectangles) are the only screens the picture holds, if not all; then S calls `share_again`
/// (the overlay goes without answering). If the overlay cannot be shown, `done` is never called
/// and the reason is returned.
pub fn select_region(shot: &Bitmap, purpose: Purpose, last: Option<Rect>, countdown: u32, shared: Option<Vec<(f64, f64, f64, f64)>>, share_again: Option<Box<dyn FnOnce()>>, done: Done) -> Result<(), String> {
    show(shot, purpose, last, countdown, shared, None, share_again, done).map(|_| ())
}

/// Counts `seconds` down over `area` (x, y, width, height in desktop logical coordinates) on
/// `shot`, then calls `done` with that area (`None` if Escape was pressed).
pub fn count_down(shot: &Bitmap, area: (f64, f64, f64, f64), seconds: u32, done: Done) -> Result<(), String> {
    let st = show(shot, Purpose::Record, None, seconds, None, None, None, done)?;
    {
        let mut s = st.borrow_mut();
        s.sel = Some(R { x: area.0, y: area.1, w: area.2, h: area.3 });
        s.proposed = false;
    }
    finish(&st, true);
    Ok(())
}

/// Shows the frozen `shot` with a loupe under the pointer; a click answers with the pixel
/// clicked (a 1 x 1 area), Escape with `None`.
pub fn pick_color(shot: Rc<Bitmap>, done: Done) -> Result<(), String> {
    show(&shot.clone(), Purpose::Capture, None, 0, None, Some(shot), None, done).map(|_| ())
}

/// The region proposed, in logical pixels: `last` (physical) as far as it lies on the desktop.
fn proposal(last: Rect, fx: f64, fy: f64, desktop: (f64, f64)) -> Option<R> {
    R { x: last.x as f64 / fx, y: last.y as f64 / fy, w: last.w as f64 / fx, h: last.h as f64 / fy }.inside(desktop.0, desktop.1)
}

#[allow(clippy::too_many_arguments)]
fn show(shot: &Bitmap, purpose: Purpose, last: Option<Rect>, countdown: u32, shared: Option<Vec<(f64, f64, f64, f64)>>, pick: Option<Rc<Bitmap>>, share_again: Option<Box<dyn FnOnce()>>, done: Done) -> Result<Rc<RefCell<State>>, String> {
    let monitors = list_monitors()?;
    let min_x = monitors.iter().map(|(_, g)| g.x()).min().unwrap_or(0);
    let min_y = monitors.iter().map(|(_, g)| g.y()).min().unwrap_or(0);
    let max_x = monitors.iter().map(|(_, g)| g.x() + g.width()).max().unwrap_or(1);
    let max_y = monitors.iter().map(|(_, g)| g.y() + g.height()).max().unwrap_or(1);
    let desktop = ((max_x - min_x) as f64, (max_y - min_y) as f64);
    let full = Pixbuf::from_bytes(&glib::Bytes::from(&shot.rgba[..]), gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, shot.width as i32, shot.height as i32, shot.width as i32 * 4);
    let (fx, fy, image) = (shot.width as f64 / desktop.0, shot.height as f64 / desktop.1, (shot.width, shot.height));

    let picking = pick.is_some();
    let sel = last.and_then(|r| proposal(r, fx, fy, desktop));
    let shared = shared.map(|v| v.into_iter().map(|(x, y, w, h)| R { x, y, w, h }).collect());
    let st = Rc::new(RefCell::new(State {
        sel,
        before_click: None,
        drag: None,
        mons: Vec::new(),
        fx,
        fy,
        image,
        desktop,
        done: Some(done),
        pick,
        pointer: None,
        purpose,
        proposed: sel.is_some(),
        count_from: countdown,
        countdown: None,
        shared,
        share_again,
        ruler: None,
    }));

    for (i, (monitor, g)) in monitors.iter().enumerate() {
        let (ox, oy) = ((g.x() - min_x) as f64, (g.y() - min_y) as f64);
        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_cursor_from_name(Some("crosshair"));
        let window = gtk::Window::new();
        window.set_decorated(false);
        // The size is also set so that it works where fullscreen is not honoured (no window manager).
        window.set_default_size(g.width(), g.height());
        window.set_title(Some(match (picking, purpose) {
            (true, _) => "Obtura: pick a colour",
            (false, Purpose::Capture) => "Obtura: select a region",
            (false, Purpose::Record) => "Obtura: choose what to record",
            (false, Purpose::RecordScreen) => "Obtura: choose the screen to record",
            (false, Purpose::Measure) => "Obtura: measure",
            (false, Purpose::Scroll) => "Obtura: select what scrolls",
        }));
        let (px, py) = ((ox * fx).round() as i32, (oy * fy).round() as i32);
        let pw = ((g.width() as f64 * fx).round() as i32).min(full.width() - px).max(1);
        let ph = ((g.height() as f64 * fy).round() as i32).min(full.height() - py).max(1);
        #[allow(deprecated)]
        let texture = gdk::Texture::for_pixbuf(&full.new_subpixbuf(px, py, pw, ph));
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_content_fit(gtk::ContentFit::Fill);
        picture.set_can_shrink(true);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&picture));
        overlay.add_overlay(&area);
        window.set_child(Some(&overlay));

        {
            let st = st.clone();
            area.set_draw_func(move |_, cr, w, h| draw(&st.borrow(), i, cr, w, h));
        }
        if purpose == Purpose::Measure {
            area.add_controller(ruler_controller(&st, i));
        } else if !picking && purpose != Purpose::RecordScreen {
            area.add_controller(drag_controller(&st, i));
        }
        area.add_controller(click_controller(&st, i));
        area.add_controller(motion_controller(&st, i));
        window.add_controller(key_controller(&st));
        {
            // Closed from outside (Alt+F4, the window manager): the same as Escape, so the
            // capture ends and Obtura's other windows come back.
            let st = st.clone();
            window.connect_close_request(move |_| {
                if st.borrow().done.is_none() {
                    return glib::Propagation::Proceed;
                }
                finish(&st, false);
                glib::Propagation::Stop
            });
        }
        window.fullscreen_on_monitor(monitor);
        st.borrow_mut().mons.push(Mon { origin: (ox, oy), size: (g.width() as f64, g.height() as f64), area, window });
    }
    let windows: Vec<_> = st.borrow().mons.iter().map(|m| m.window.clone()).collect();
    for w in windows {
        w.present();
    }
    Ok(st)
}

/// Measuring: a drag draws the ruler (Shift: straight or at 45°). It stays shown after the drag.
fn ruler_controller(st: &Rc<RefCell<State>>, i: usize) -> gtk::GestureDrag {
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    {
        let st = st.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let mut s = st.borrow_mut();
            let (ox, oy) = s.mons[i].origin;
            let a = (ox + x, oy + y);
            s.before_click = s.ruler.map(|r| R { x: r.a.0, y: r.a.1, w: r.b.0 - r.a.0, h: r.b.1 - r.a.1 });
            s.ruler = Some(Ruler { a, b: a });
            s.redraw();
        });
    }
    {
        // A click without a drag keeps the ruler that was there.
        let st = st.clone();
        drag.connect_drag_end(move |_, dx, dy| {
            let mut s = st.borrow_mut();
            if dx.abs() < 3.0 && dy.abs() < 3.0 {
                s.ruler = s.before_click.map(|r| Ruler { a: (r.x, r.y), b: (r.x + r.w, r.y + r.h) });
                s.redraw();
            }
        });
    }
    {
        let st = st.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            let mut s = st.borrow_mut();
            let Some(r) = s.ruler else { return };
            let b = (r.a.0 + dx, r.a.1 + dy);
            let shift = g.current_event_state().contains(gdk::ModifierType::SHIFT_MASK);
            s.ruler = Some(Ruler { a: r.a, b: if shift { Ruler::straightened(r.a, b) } else { b } });
            s.redraw();
        });
    }
    drag
}

/// Measuring is done: the ruler as an answer (`x`, `y` its start; `w`, `h` how far its end is
/// from it, which may be negative), or `None`.
pub fn measure(shot: &Bitmap, done: Done) -> Result<(), String> {
    show(shot, Purpose::Measure, None, 0, None, None, None, done).map(|_| ())
}

fn drag_controller(st: &Rc<RefCell<State>>, i: usize) -> gtk::GestureDrag {
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    {
        let st = st.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let mut s = st.borrow_mut();
            if s.countdown.is_some() {
                return;
            }
            let (ox, oy) = s.mons[i].origin;
            let (x, y) = (ox + x, oy + y);
            s.before_click = s.sel;
            // A selection as big as the screen cannot be moved: dragging in it starts a new one.
            let m = &s.mons[i];
            let screen = R { x: m.origin.0, y: m.origin.1, w: m.size.0, h: m.size.1 };
            s.drag = Some(match s.sel {
                Some(r) => match r.handle_at(x, y) {
                    Some(handle) => Drag::Resize { origin: r, handle },
                    None if r.contains(x, y) && !r.covers(&screen) => Drag::Move { origin: r },
                    None => Drag::New { start: (x, y) },
                },
                None => Drag::New { start: (x, y) },
            });
            if matches!(s.drag, Some(Drag::New { .. })) {
                s.sel = None;
            }
            s.redraw();
        });
    }
    {
        let st = st.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            let mut s = st.borrow_mut();
            if s.countdown.is_some() || s.drag.is_none() {
                return;
            }
            s.proposed = false;
            let (fx, fy) = (s.fx, s.fy);
            let new_sel = match &s.drag {
                Some(Drag::New { start }) => Some(R::from_points(start.0, start.1, start.0 + dx, start.1 + dy)),
                Some(Drag::Move { origin }) => Some(R { x: origin.x + dx, y: origin.y + dy, ..*origin }),
                Some(Drag::Resize { origin, handle }) => Some(origin.resized(*handle, dx, dy)),
                None => None,
            };
            if let Some(r) = new_sel {
                // Never smaller than one physical pixel.
                s.sel = Some(R { w: r.w.max(1.0 / fx), h: r.h.max(1.0 / fy), ..r });
            }
            let _ = g;
            s.redraw();
        });
    }
    {
        let st = st.clone();
        drag.connect_drag_end(move |_, dx, dy| {
            let mut s = st.borrow_mut();
            if s.countdown.is_some() {
                return;
            }
            // A plain click (no drag) keeps the selection that was there; when recording it
            // chooses the whole screen under the pointer.
            if dx.abs() < 3.0 && dy.abs() < 3.0 && matches!(s.drag, Some(Drag::New { .. })) {
                s.sel = match s.purpose {
                    Purpose::Capture | Purpose::Measure | Purpose::Scroll => s.before_click,
                    Purpose::Record | Purpose::RecordScreen => {
                        s.proposed = false;
                        let m = &s.mons[i];
                        Some(R { x: m.origin.0, y: m.origin.1, w: m.size.0, h: m.size.1 })
                    }
                };
            }
            s.drag = None;
            s.redraw();
        });
    }
    drag
}

fn click_controller(st: &Rc<RefCell<State>>, _i: usize) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let st = st.clone();
    click.connect_pressed(move |g, n, x, y| match g.current_button() {
        3 => finish(&st, false),
        1 if st.borrow().pick.is_some() => {
            let (ox, oy) = st.borrow().mons[_i].origin;
            pick_at(&st, ox + x, oy + y);
        }
        1 if st.borrow().purpose == Purpose::RecordScreen => {
            if st.borrow().countdown.is_none() {
                st.borrow_mut().select_screen(_i);
                finish(&st, true);
            }
        }
        1 if n == 2 && st.borrow().purpose != Purpose::Measure => finish(&st, true),
        _ => {}
    });
    click
}

/// Answers with the pixel under desktop point (x, y).
fn pick_at(st: &Rc<RefCell<State>>, x: f64, y: f64) {
    {
        let mut s = st.borrow_mut();
        let (fx, fy) = (s.fx, s.fy);
        s.sel = Some(R { x: (x * fx).floor() / fx, y: (y * fy).floor() / fy, w: 1.0 / fx, h: 1.0 / fy });
    }
    finish(st, true);
}

/// The cursor shows what a drag will do: resize arrows on a handle, a grabbing hand inside.
fn motion_controller(st: &Rc<RefCell<State>>, i: usize) -> gtk::EventControllerMotion {
    let motion = gtk::EventControllerMotion::new();
    let st = st.clone();
    motion.connect_motion(move |_, x, y| {
        if let Ok(mut s) = st.try_borrow_mut() {
            if s.purpose == Purpose::RecordScreen {
                // The screen under the pointer is the one a click records.
                if s.countdown.is_none() && s.sel != Some(s.screen_rect(i)) {
                    s.select_screen(i);
                    s.redraw();
                }
                return;
            }
            if s.pick.is_some() {
                let (ox, oy) = s.mons[i].origin;
                s.pointer = Some((ox + x, oy + y));
                s.redraw();
                return;
            }
        }
        let Ok(s) = st.try_borrow() else { return };
        let (ox, oy) = s.mons[i].origin;
        let (x, y) = (ox + x, oy + y);
        let name = match s.sel {
            Some(r) => match r.handle_at(x, y) {
                Some((-1, -1)) => "nw-resize",
                Some((1, -1)) => "ne-resize",
                Some((-1, 1)) => "sw-resize",
                Some((1, 1)) => "se-resize",
                Some((0, _)) => "ns-resize",
                Some(_) => "ew-resize",
                None if r.contains(x, y) => "move",
                None => "crosshair",
            },
            None => "crosshair",
        };
        for m in &s.mons {
            m.area.set_cursor_from_name(Some(name));
        }
    });
    motion
}

fn key_controller(st: &Rc<RefCell<State>>) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let st = st.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let (dx, dy) = match key {
            gdk::Key::Escape => {
                finish(&st, false);
                return glib::Propagation::Stop;
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                let pointer = st.borrow().pick.as_ref().and(st.borrow().pointer);
                match pointer {
                    Some((x, y)) => pick_at(&st, x, y),
                    None => finish(&st, true),
                }
                return glib::Propagation::Stop;
            }
            gdk::Key::s | gdk::Key::S => {
                let again = {
                    let mut s = st.borrow_mut();
                    if s.countdown.is_none() && s.screen_missing() { s.share_again.take() } else { None }
                };
                if let Some(again) = again {
                    let windows: Vec<gtk::Window> = {
                        let mut s = st.borrow_mut();
                        s.done.take();
                        s.mons.iter().map(|m| m.window.clone()).collect()
                    };
                    for w in windows {
                        w.destroy();
                    }
                    again();
                }
                return glib::Propagation::Stop;
            }
            gdk::Key::Left => (-1.0, 0.0),
            gdk::Key::Right => (1.0, 0.0),
            gdk::Key::Up => (0.0, -1.0),
            gdk::Key::Down => (0.0, 1.0),
            _ => return glib::Propagation::Proceed,
        };
        let mut s = st.borrow_mut();
        if s.countdown.is_some() || s.purpose == Purpose::RecordScreen {
            return glib::Propagation::Stop;
        }
        s.proposed = false;
        let (sx, sy) = (dx / s.fx, dy / s.fy); // one physical pixel
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let (fx, fy) = (s.fx, s.fy);
        if let Some(r) = s.sel.as_mut() {
            if shift {
                r.w = (r.w + sx).max(1.0 / fx);
                r.h = (r.h + sy).max(1.0 / fy);
            } else {
                r.x += sx;
                r.y += sy;
            }
        }
        s.redraw();
        glib::Propagation::Stop
    });
    keys
}

fn draw(s: &State, i: usize, cr: &cairo::Context, w: i32, h: i32) {
    if let Some(shot) = &s.pick {
        return draw_pick(s, shot, i, cr, w, h);
    }
    if s.purpose == Purpose::Measure {
        return draw_ruler(s, i, cr, w, h);
    }
    let (ox, oy) = s.mons[i].origin;
    let (fw, fh) = (w as f64, h as f64);
    // Dim everything except the selection.
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
    cr.set_fill_rule(cairo::FillRule::EvenOdd);
    cr.rectangle(0.0, 0.0, fw, fh);
    if let Some(r) = s.sel {
        cr.rectangle(r.x - ox, r.y - oy, r.w, r.h);
    }
    let _ = cr.fill();

    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(15.0);
    let label = |text: &str, x: f64, y: f64| {
        if let Ok(ext) = cr.text_extents(text) {
            let x = x.clamp(4.0, (fw - ext.width() - 16.0).max(4.0));
            let y = y.clamp(22.0, fh - 8.0);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.75);
            cr.rectangle(x - 6.0, y - 18.0, ext.width() + 12.0, 26.0);
            let _ = cr.fill();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.move_to(x, y);
            let _ = cr.show_text(text);
        }
    };
    let centred = |text: &str, y: f64| {
        let width = cr.text_extents(text).map(|e| e.width()).unwrap_or(300.0);
        label(text, (fw - width) / 2.0, y);
    };

    // A screen that cannot be recorded says so.
    let m = &s.mons[i];
    let this_screen = R { x: ox, y: oy, w: m.size.0, h: m.size.1 };
    if s.shared.as_ref().is_some_and(|shared| !shared.iter().any(|r| r.intersects(&this_screen))) {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
        cr.rectangle(0.0, 0.0, fw, fh);
        let _ = cr.fill();
        centred("GNOME does not share this screen with Obtura", fh / 2.0 + 40.0);
        if s.share_again.is_some() {
            centred("Press S to choose again which screens to share", fh / 2.0 + 72.0);
        }
    }

    if let Some(left) = s.countdown {
        if let Some(r) = s.sel {
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(1.0);
            cr.rectangle(r.x - ox + 0.5, r.y - oy + 0.5, (r.w - 1.0).max(0.0), (r.h - 1.0).max(0.0));
            let _ = cr.stroke();
            let (cx, cy) = (r.x + r.w / 2.0 - ox, r.y + r.h / 2.0 - oy);
            if cx >= 0.0 && cy >= 0.0 && cx < fw && cy < fh {
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.6);
                cr.arc(cx, cy, 70.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                cr.set_font_size(96.0);
                let text = left.max(1).to_string();
                if let Ok(ext) = cr.text_extents(&text) {
                    cr.set_source_rgb(1.0, 1.0, 1.0);
                    cr.move_to(cx - ext.width() / 2.0 - ext.x_bearing(), cy - ext.height() / 2.0 - ext.y_bearing());
                    let _ = cr.show_text(&text);
                }
                cr.set_font_size(15.0);
            }
        }
        centred("Recording starts in a moment · Esc to cancel · to pause or stop, use Obtura's icon in the top bar", 40.0);
        return;
    }

    if s.purpose == Purpose::RecordScreen {
        if s.sel.is_some_and(|r| r == this_screen) {
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(4.0);
            cr.rectangle(2.0, 2.0, fw - 4.0, fh - 4.0);
            let _ = cr.stroke();
        }
        centred("Click the screen to record · Esc to cancel", fh / 2.0);
        return;
    }

    match s.sel {
        Some(r) => {
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(1.0);
            cr.rectangle(r.x - ox + 0.5, r.y - oy + 0.5, (r.w - 1.0).max(0.0), (r.h - 1.0).max(0.0));
            let _ = cr.stroke();
            if s.drag.is_none() {
                for (_, (hx, hy)) in r.handles() {
                    cr.rectangle(hx - ox - HANDLE / 2.0, hy - oy - HANDLE / 2.0, HANDLE, HANDLE);
                    cr.set_source_rgb(1.0, 1.0, 1.0);
                    let _ = cr.fill_preserve();
                    cr.set_source_rgb(0.1, 0.1, 0.1);
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                }
            }
            if let Some(p) = s.physical() {
                label(&format!("{} × {}   at {}, {}", p.w, p.h, p.x, p.y), r.x - ox, r.y - oy + r.h + 28.0);
            }
        }
        None => match s.purpose {
            Purpose::Capture => centred("Drag to select a region · Esc to cancel", fh / 2.0),
            Purpose::Scroll => {
                centred("Scrolling capture: drag over the part that scrolls (leave out the scroll bar) · Esc to cancel", fh / 2.0);
                centred("Then scroll it slowly with the mouse wheel; Obtura joins what comes into view", fh / 2.0 + 32.0);
            }
            Purpose::Record => centred("Drag to choose what to record · click a screen to record all of it · Esc to cancel", fh / 2.0),
            Purpose::RecordScreen | Purpose::Measure => {}
        },
    }
    if let (Some(r), None) = (s.sel, &s.drag) {
        let hint = match (s.purpose, s.proposed) {
            (Purpose::Capture, true) => "Enter or double-click: same region as last time · drag elsewhere for a new one · Esc to cancel",
            (Purpose::Capture, false) => "Enter or double-click to capture · Esc to cancel",
            (Purpose::Record, true) => "Enter or double-click to record the same region as last time · drag elsewhere for a new one · Esc to cancel",
            (Purpose::Record, false) => "Enter or double-click to start recording · Esc to cancel",
            (Purpose::RecordScreen, _) => "Click the screen to record · Esc to cancel",
            (Purpose::Measure, _) => "",
            (Purpose::Scroll, _) => "Enter or double-click to start, then scroll slowly · finish from Obtura's icon in the top bar · Esc to cancel",
        };
        // At the top of the screen, unless the selection is there.
        let at_top = r.y - oy > 70.0 || r.y + r.h - oy < 0.0;
        centred(hint, if at_top { 40.0 } else { fh - 24.0 });
    }
}

/// Measuring: the screen lightly dimmed, the ruler with its ends, the box it is the diagonal
/// of, and the measurement beside its end.
fn draw_ruler(s: &State, i: usize, cr: &cairo::Context, w: i32, h: i32) {
    let (ox, oy) = s.mons[i].origin;
    let (fw, fh) = (w as f64, h as f64);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.18);
    cr.paint().ok();
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(15.0);
    let label = |text: &str, x: f64, y: f64| {
        if let Ok(ext) = cr.text_extents(text) {
            let x = x.clamp(4.0, (fw - ext.width() - 16.0).max(4.0));
            let y = y.clamp(22.0, fh - 8.0);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.8);
            cr.rectangle(x - 6.0, y - 18.0, ext.width() + 12.0, 26.0);
            let _ = cr.fill();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.move_to(x, y);
            let _ = cr.show_text(text);
        }
    };
    let hint = if s.ruler.is_some() { "Drag again to measure something else · Shift: straight · Enter copies the measurement · Esc closes" } else { "Drag across what you want to measure · Shift: straight lines · Esc closes" };
    let hw = cr.text_extents(hint).map(|e| e.width()).unwrap_or(400.0);
    label(hint, (fw - hw) / 2.0, 40.0);
    let Some(r) = s.ruler else { return };
    let (ax, ay, bx, by) = (r.a.0 - ox, r.a.1 - oy, r.b.0 - ox, r.b.1 - oy);
    // The box the ruler is the diagonal of, dashed.
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.7);
    cr.set_line_width(1.0);
    cr.set_dash(&[4.0, 4.0], 0.0);
    cr.rectangle(ax.min(bx) + 0.5, ay.min(by) + 0.5, (bx - ax).abs(), (by - ay).abs());
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    // The ruler: dark under light so it shows on any picture, with a bar across each end.
    let len = (bx - ax).hypot(by - ay).max(1e-6);
    let (px, py) = (-(by - ay) / len * 7.0, (bx - ax) / len * 7.0);
    for (width, light) in [(4.0, false), (2.0, true)] {
        if light {
            cr.set_source_rgb(1.0, 0.85, 0.2);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.8);
        }
        cr.set_line_width(width);
        cr.move_to(ax, ay);
        cr.line_to(bx, by);
        for (x, y) in [(ax, ay), (bx, by)] {
            cr.move_to(x - px, y - py);
            cr.line_to(x + px, y + py);
        }
        let _ = cr.stroke();
    }
    let (dx, dy, l, deg) = r.measure(s.fx, s.fy);
    label(&format!("{} px", l.round()), bx + 14.0, by + 30.0);
    cr.set_font_size(13.0);
    label(&format!("{} × {}   {}°", dx.abs(), dy.abs(), deg.round() as i64 % 360), bx + 14.0, by + 58.0);
}

/// Colour picking: a hint at the top, and the loupe beside the pointer on its monitor.
fn draw_pick(s: &State, shot: &Bitmap, i: usize, cr: &cairo::Context, w: i32, h: i32) {
    let (ox, oy) = s.mons[i].origin;
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(15.0);
    let hint = "Click to copy a colour · Esc to cancel";
    if let Ok(ext) = cr.text_extents(hint) {
        let x = (w as f64 - ext.width()) / 2.0;
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.75);
        cr.rectangle(x - 8.0, 22.0, ext.width() + 16.0, 28.0);
        let _ = cr.fill();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(x, 41.0);
        let _ = cr.show_text(hint);
    }
    let Some((px, py)) = s.pointer else { return };
    let (lx, ly) = (px - ox, py - oy);
    if lx < 0.0 || ly < 0.0 || lx >= w as f64 || ly >= h as f64 {
        return;
    }
    // The pixels around the pointer, enlarged.
    const N: i32 = 11;
    const CELL: f64 = 11.0;
    let size = N as f64 * CELL;
    let (cx, cy) = ((px * s.fx).floor() as i32, (py * s.fy).floor() as i32);
    let inside = |x: i32, y: i32| x >= 0 && y >= 0 && (x as u32) < shot.width && (y as u32) < shot.height;
    let bx = if lx + 28.0 + size > w as f64 { lx - 28.0 - size } else { lx + 28.0 };
    let by = if ly + 28.0 + size + 44.0 > h as f64 { ly - 28.0 - size - 44.0 } else { ly + 28.0 };
    cr.set_source_rgba(0.1, 0.1, 0.1, 0.9);
    cr.rectangle(bx - 3.0, by - 3.0, size + 6.0, size + 48.0);
    let _ = cr.fill();
    for j in 0..N {
        for k in 0..N {
            let (x, y) = (cx - N / 2 + k, cy - N / 2 + j);
            if inside(x, y) {
                let c = shot.pixel(x as u32, y as u32);
                cr.set_source_rgb(c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0);
            } else {
                cr.set_source_rgb(0.4, 0.4, 0.4);
            }
            cr.rectangle(bx + k as f64 * CELL, by + j as f64 * CELL, CELL, CELL);
            let _ = cr.fill();
        }
    }
    let (mx, my) = (bx + (N / 2) as f64 * CELL, by + (N / 2) as f64 * CELL);
    cr.set_line_width(1.0);
    cr.rectangle(mx - 0.5, my - 0.5, CELL + 1.0, CELL + 1.0);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    let _ = cr.stroke();
    cr.rectangle(mx - 1.5, my - 1.5, CELL + 3.0, CELL + 3.0);
    cr.set_source_rgb(0.0, 0.0, 0.0);
    let _ = cr.stroke();
    if inside(cx, cy) {
        let c = shot.pixel(cx as u32, cy as u32);
        cr.set_source_rgb(c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0);
        cr.rectangle(bx, by + size + 8.0, 30.0, 30.0);
        let _ = cr.fill();
        cr.select_font_face("Monospace", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        cr.set_font_size(14.0);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(bx + 38.0, by + size + 22.0);
        let _ = cr.show_text(&format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]));
        cr.set_font_size(11.0);
        cr.set_source_rgb(0.75, 0.75, 0.75);
        cr.move_to(bx + 38.0, by + size + 37.0);
        let _ = cr.show_text(&format!("{cx}, {cy}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ruler_measures_in_physical_pixels() {
        let r = Ruler { a: (10.0, 100.0), b: (310.0, 14.0) };
        let (dx, dy, len, deg) = r.measure(1.0, 1.0);
        assert_eq!((dx, dy), (300.0, -86.0));
        assert_eq!(len.round(), 312.0);
        assert_eq!(deg.round(), 16.0, "up and to the right: a positive angle");
        assert_eq!(r.said(1.0, 1.0), "300 × 86 px, 312 px long at 16°");
        // At scale 2 the same ruler covers twice the pixels.
        assert_eq!(r.measure(2.0, 2.0).0, 600.0);
        assert_eq!(Ruler { a: (0.0, 0.0), b: (0.0, 50.0) }.measure(1.0, 1.0).3.round(), 270.0);
        // Shift: straight across.
        assert_eq!(Ruler::straightened((0.0, 0.0), (100.0, 9.0)), (100.0, 0.0));
        let (x, y) = Ruler::straightened((0.0, 0.0), (50.0, 55.0));
        assert!((x - y).abs() < 1e-9, "45°: {x} {y}");
    }

    #[test]
    fn rectangle_from_any_two_corners() {
        assert_eq!(R::from_points(10.0, 20.0, 4.0, 5.0), R { x: 4.0, y: 5.0, w: 6.0, h: 15.0 });
    }

    #[test]
    fn resizing_a_handle_moves_only_its_edges() {
        let r = R { x: 10.0, y: 10.0, w: 100.0, h: 50.0 };
        assert_eq!(r.resized((1, 0), 5.0, 99.0), R { x: 10.0, y: 10.0, w: 105.0, h: 50.0 });
        assert_eq!(r.resized((-1, -1), 5.0, 5.0), R { x: 15.0, y: 15.0, w: 95.0, h: 45.0 });
        // Dragging an edge past the opposite one flips the rectangle instead of going negative.
        assert_eq!(r.resized((1, 0), -120.0, 0.0), R { x: -10.0, y: 10.0, w: 20.0, h: 50.0 });
    }

    #[test]
    fn a_selection_covering_the_screen_is_told_apart() {
        let screen = R { x: 1920.0, y: 0.0, w: 1920.0, h: 1080.0 };
        assert!(R { x: 1919.0, y: 0.0, w: 1921.0, h: 1079.0 }.covers(&screen));
        assert!(!R { x: 1920.0, y: 0.0, w: 1000.0, h: 1080.0 }.covers(&screen));
    }

    #[test]
    fn handles_are_found_near_their_centre_only() {
        let r = R { x: 100.0, y: 100.0, w: 200.0, h: 100.0 };
        assert_eq!(r.handle_at(102.0, 98.0), Some((-1, -1)));
        assert_eq!(r.handle_at(200.0, 200.0), Some((0, 1)));
        assert_eq!(r.handle_at(200.0, 150.0), None);
    }

    #[test]
    fn an_area_becomes_pixels_of_the_picture() {
        let a = Area { x: 100.0, y: 50.0, w: 200.0, h: 100.0, desktop: (1000.0, 500.0) };
        assert_eq!(a.in_pixels(1000, 500), Some(Rect::new(100, 50, 200, 100)));
        // A picture twice as big (scale 2): twice the pixels.
        assert_eq!(a.in_pixels(2000, 1000), Some(Rect::new(200, 100, 400, 200)));
        // Partly outside: only what is inside.
        let b = Area { x: 900.0, y: -20.0, w: 200.0, h: 100.0, desktop: (1000.0, 500.0) };
        assert_eq!(b.in_pixels(1000, 500), Some(Rect::new(900, 0, 100, 80)));
    }

    #[test]
    fn the_last_region_is_proposed_where_it_still_fits() {
        // Physical pixels at scale 2 become logical pixels.
        assert_eq!(proposal(Rect::new(200, 100, 400, 200), 2.0, 2.0, (1000.0, 500.0)), Some(R { x: 100.0, y: 50.0, w: 200.0, h: 100.0 }));
        // Partly off the desktop (a monitor was unplugged): the part that is left.
        assert_eq!(proposal(Rect::new(900, 0, 300, 100), 1.0, 1.0, (1000.0, 500.0)), Some(R { x: 900.0, y: 0.0, w: 100.0, h: 100.0 }));
        // Wholly off the desktop: nothing is proposed.
        assert_eq!(proposal(Rect::new(1500, 0, 300, 100), 1.0, 1.0, (1000.0, 500.0)), None);
    }
}
