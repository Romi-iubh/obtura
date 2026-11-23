//! Recording the screen: the Record window (format, sound with level meters, pointer,
//! countdown), choosing a screen or an area on a still picture of the screen (or a window in
//! GNOME's list, or all the screens at once), the recording itself, and the window that shows
//! the saved file.
//!
//! While recording Obtura shows no window (it would be in the recording). To pause or stop:
//! * Obtura's icon in the top bar (`tray.rs`: a record symbol with the time): its menu has
//!   Pause / Resume, Stop and Save, Discard;
//! * open Obtura again (app grid, `obtura`): the recording pauses at once and a small window
//!   offers Resume, Stop and Save, Discard;
//! * GNOME's screen-sharing icon in the top bar: stops and saves;
//! * `obtura --record` again (a keyboard shortcut can run it): stops and saves;
//! * without status icons in the top bar, a notification in GNOME's message list has Stop.
//!
//! A recording is made of segments: a pause ends one, Resume starts the next; they are joined
//! at the end (see `obtura_capture::record`). A GIF is recorded as frames and made at the end
//! by `obtura_image::gif`.

use crate::{
    capture::{now, Outcome},
    overlay,
};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use obtura_capture::{
    record::{self, Crop, Encoders, Format, Plan, Segment},
    CaptureBackend, CaptureError, CastRequest, Sound,
};
use obtura_core::{
    naming::{format_name, unique_path},
    screen_share,
    settings::{RecordFormat, RecordSettings, Settings},
};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

pub type Done = Box<dyn FnOnce(Outcome)>;

/// A screen-sharing session (named so it does not hide GTK's `Cast` trait).
type Sharing = obtura_capture::Cast;

/// What to record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    /// An area chosen on the screen (or a whole screen, by a click).
    Area,
    /// A whole screen: the one Obtura's window is on (the Record window's), else the only one
    /// shared, else the one clicked.
    Screen,
    /// Every shared screen, side by side in one picture, as they are arranged.
    AllScreens,
    /// One window, picked in GNOME's list (asked every time). It may move: it is followed.
    Window,
}

/// A screen in desktop logical pixels from the desktop's top-left corner: (x, y, width, height).
type ScreenRect = (f64, f64, f64, f64);

/// The screen an Obtura window is on, so "Record a Screen" records the screen the Record window
/// was on without asking.
fn screen_of(w: &gtk::Window) -> Option<ScreenRect> {
    let monitor = WidgetExt::display(w).monitor_at_surface(&w.surface()?)?;
    let g = monitor.geometry();
    let desktop = overlay::desktop().ok()?;
    Some(((g.x() - desktop.origin.0) as f64, (g.y() - desktop.origin.1) as f64, g.width() as f64, g.height() as f64))
}

/// Time for the compositor to really remove a window before recording goes on.
const HIDE_DELAY: Duration = Duration::from_millis(250);
/// How long a segment may take to finish its file before it is stopped hard.
const GRACE: Duration = Duration::from_secs(6);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Recording,
    Pausing,
    Paused,
    Stopping,
    Finished,
}

struct Recording {
    app: adw::Application,
    cast: Rc<Sharing>,
    /// The streams recorded: one, or every screen (put together as the plan says).
    streams: Vec<usize>,
    plan: Plan,
    /// The segments, in a scratch folder removed at the end.
    dir: PathBuf,
    output: PathBuf,
    size: (u32, u32),
    segment: RefCell<Option<Segment>>,
    state: Cell<State>,
    segments: Cell<u32>,
    /// Time recorded in finished segments, and when the current one started.
    recorded: Cell<Duration>,
    since: Cell<Option<Instant>>,
    discard: Cell<bool>,
    /// The paused window should show once the segment has ended, and ask about discarding.
    wants_window: Cell<bool>,
    discard_asked: Cell<bool>,
    error: RefCell<Option<String>>,
    hidden: RefCell<Vec<gtk::Window>>,
    /// The window shown while paused, and its text.
    controls: RefCell<Option<(adw::ApplicationWindow, adw::StatusPage)>>,
    done: RefCell<Option<Done>>,
    /// Keeps Obtura running while recording (no window is open).
    _hold: gio::ApplicationHoldGuard,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Recording>>> = const { RefCell::new(None) };
    static ENCODERS: Cell<Option<Encoders>> = const { Cell::new(None) };
    static TOOLS: Cell<Option<bool>> = const { Cell::new(None) };
}

fn current() -> Option<Rc<Recording>> {
    CURRENT.with(|c| c.borrow().clone())
}

pub fn is_recording() -> bool {
    current().is_some()
}

/// The encoders this system has (looked up once).
pub(crate) fn encoders() -> Encoders {
    ENCODERS.with(|e| {
        e.get().unwrap_or_else(|| {
            let found = Encoders::detect();
            e.set(Some(found));
            found
        })
    })
}

fn tools_present() -> bool {
    TOOLS.with(|t| {
        t.get().unwrap_or_else(|| {
            let found = record::tools_present();
            t.set(Some(found));
            found
        })
    })
}

fn format_of(f: RecordFormat) -> Format {
    match f {
        RecordFormat::Mp4 => Format::Mp4,
        RecordFormat::WebM => Format::WebM,
        RecordFormat::Gif => Format::Gif,
    }
}

/// Stops the recording (and saves it). Used by `obtura --record` while recording, by the
/// notification and by GNOME's sharing icon.
pub fn stop() {
    if let Some(r) = current() {
        r.stop();
    }
}

/// Clicking Obtura's icon while recording: pause, and show what can be done.
pub fn pause_and_show() {
    if let Some(r) = current() {
        r.pause(true);
    }
}

/// Starts recording, or stops the recording under way (so one shortcut does both). `done` is
/// called when the recording is saved (with its path), cancelled or failed; when this call
/// stops a recording, at once.
pub fn toggle(app: &adw::Application, backend: Rc<dyn CaptureBackend>, what: What, done: Done) {
    if cfg!(windows) {
        not_on_windows(app);
        return done(Outcome::Failed(obtura_capture::NOT_ON_WINDOWS.into()));
    }
    if is_recording() {
        stop();
        return done(Outcome::Done);
    }
    start(app, backend, what, done);
}

fn start(app: &adw::Application, backend: Rc<dyn CaptureBackend>, what: What, done: Done) {
    let settings = Settings::load().record;
    if !tools_present() {
        return fail(app, &[], "Recording needs GStreamer's tools. Install them with:\nsudo apt install gstreamer1.0-tools gstreamer1.0-pipewire gstreamer1.0-plugins-good", done);
    }
    let format = format_of(settings.format);
    if !encoders().can_record(format) {
        return fail(app, &[], "MP4 needs an H.264 encoder, which is not installed. Choose WebM in the Record window, or install one with:\nsudo apt install gstreamer1.0-plugins-ugly", done);
    }
    let hold = app.hold();
    let prefer = if what == What::Screen { app.active_window().filter(|w| w.is_visible()).and_then(|w| screen_of(&w)) } else { None };
    // Obtura's own windows must not be in the recording, nor cover the GNOME chooser.
    let hidden: Vec<gtk::Window> = app.windows().into_iter().filter(|w| w.is_visible()).collect();
    for w in &hidden {
        w.set_visible(false);
    }
    let wait = if hidden.is_empty() { Duration::ZERO } else { HIDE_DELAY };
    let app = app.clone();
    glib::timeout_add_local_once(wait, move || share(&app, backend, what, prefer, settings, hidden, hold, done));
}

/// Opens the screen-sharing session (GNOME asks which screens the first time).
#[allow(clippy::too_many_arguments)]
fn share(app: &adw::Application, backend: Rc<dyn CaptureBackend>, what: What, prefer: Option<ScreenRect>, settings: RecordSettings, hidden: Vec<gtk::Window>, hold: gio::ApplicationHoldGuard, done: Done) {
    let windows = what == What::Window;
    let request = CastRequest { pointer: settings.pointer, restore_token: if windows { None } else { screen_share::load() }, windows };
    let app2 = app.clone();
    let b2 = backend.clone();
    backend.screencast(
        request,
        Box::new(move |result| match result {
            Ok(cast) if windows => record_window(&app2, Rc::new(cast), settings, hidden, hold, done),
            Ok(cast) => {
                if let Some(t) = &cast.restore_token {
                    let _ = screen_share::save(t);
                }
                choose(&app2, b2, Rc::new(cast), what, prefer, settings, hidden, hold, done);
            }
            Err(CaptureError::Cancelled) => {
                restore(&hidden, false);
                done(Outcome::Cancelled);
            }
            Err(e) => fail(&app2, &hidden, &format!("The screen could not be shared with Obtura: {e}"), done),
        }),
    );
}

/// A shared screen's place on the desktop: (x, y, width, height) in desktop logical pixels
/// from the desktop's top-left corner, and its scale.
pub(crate) fn stream_rects(cast: &Rc<Sharing>, desktop: &overlay::Desktop) -> Vec<((f64, f64, f64, f64), f64)> {
    let max_scale = desktop.screens.iter().map(|s| s.scale).fold(1.0, f64::max);
    let first = desktop.screens.first().map(|s| (s.w, s.h)).unwrap_or((1920.0, 1080.0));
    cast.streams
        .iter()
        .map(|s| {
            let (x, y) = s.position.map(|(x, y)| ((x - desktop.origin.0) as f64, (y - desktop.origin.1) as f64)).unwrap_or((0.0, 0.0));
            let (w, h) = s.size.map(|(w, h)| (w as f64, h as f64)).unwrap_or(first);
            let scale = desktop.screens.iter().find(|m| (m.x - x).abs() < 1.0 && (m.y - y).abs() < 1.0).map(|m| m.scale).unwrap_or(max_scale);
            ((x, y, w, h), scale)
        })
        .collect()
}

/// The whole screen "Record a Screen" records without asking: the only one shared, or the shared
/// one where Obtura's window was (`prefer`). `None`: the user clicks one.
fn screen_to_record(streams: &[(ScreenRect, f64)], prefer: Option<ScreenRect>) -> Option<ScreenRect> {
    match streams {
        [(only, _)] => Some(*only),
        _ => prefer.and_then(|p| target(streams, p, false)).map(|t| streams[t.stream].0),
    }
}

/// A window picked in GNOME's list: its first frame gives the size the recording keeps (a
/// window made bigger later is fitted in, with black borders). The countdown runs in the top bar.
fn record_window(app: &adw::Application, cast: Rc<Sharing>, settings: RecordSettings, hidden: Vec<gtk::Window>, hold: gio::ApplicationHoldGuard, done: Done) {
    let app = app.clone();
    let c2 = cast.clone();
    cast.grab_frame(0, move |frame| {
        let size = match frame.map(|png| obtura_capture::png_size(&png)) {
            Ok(Some((w, h))) if w >= 2 && h >= 2 => (w, h),
            Ok(_) => {
                c2.close();
                return fail(&app, &hidden, "No picture of the window came from GNOME.", done);
            }
            Err(e) => {
                c2.close();
                return fail(&app, &hidden, &format!("No picture of the window came from GNOME: {e}"), done);
            }
        };
        let gif = settings.format == RecordFormat::Gif;
        let size = if gif { size } else { (size.0 & !1, size.1 & !1) };
        let app2 = app.clone();
        crate::capture::count_down(&app, settings.countdown, move |go| {
            if go {
                begin(&app2, c2, Shape::Window { size }, settings, hidden, hold, done);
            } else {
                c2.close();
                restore(&hidden, false);
                done(Outcome::Cancelled);
            }
        });
    });
}

/// Every shared screen in one picture: where each goes (physical pixels, all at the largest
/// scale) and the picture's size, from the screens' places on the desktop (logical pixels).
fn all_screens(streams: &[(ScreenRect, f64)], even: bool) -> (Vec<(i64, i64, u32, u32)>, (u32, u32), ScreenRect) {
    let scale = streams.iter().map(|(_, s)| *s).fold(1.0, f64::max);
    let (x0, y0) = streams.iter().fold((f64::MAX, f64::MAX), |(x, y), (r, _)| (x.min(r.0), y.min(r.1)));
    let (x1, y1) = streams.iter().fold((f64::MIN, f64::MIN), |(x, y), (r, _)| (x.max(r.0 + r.2), y.max(r.1 + r.3)));
    let px = |v: f64| (v * scale).round();
    let layout = streams.iter().map(|((x, y, w, h), _)| (px(x - x0) as i64, px(y - y0) as i64, px(*w).max(1.0) as u32, px(*h).max(1.0) as u32)).collect();
    let (mut w, mut h) = (px(x1 - x0).max(2.0) as u32, px(y1 - y0).max(2.0) as u32);
    if even {
        w &= !1;
        h &= !1;
    }
    (layout, (w, h), (x0, y0, x1 - x0, y1 - y0))
}

/// What is recorded.
enum Shape {
    /// A part of one screen (or all of it).
    Part(Target),
    /// Every shared screen in one picture.
    All { layout: Vec<(i64, i64, u32, u32)>, size: (u32, u32) },
    /// A window, kept at this size.
    Window { size: (u32, u32) },
}

/// Which shared screen records `area` (the one holding most of it), the part of its frames to
/// keep, and the size recorded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Target {
    pub stream: usize,
    pub crop: Crop,
    pub size: (u32, u32),
}

pub(crate) fn target(streams: &[((f64, f64, f64, f64), f64)], area: (f64, f64, f64, f64), even: bool) -> Option<Target> {
    let (ax, ay, aw, ah) = area;
    let overlap = |r: &(f64, f64, f64, f64)| {
        let (x0, y0) = (ax.max(r.0), ay.max(r.1));
        let (x1, y1) = ((ax + aw).min(r.0 + r.2), (ay + ah).min(r.1 + r.3));
        (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
    };
    let (stream, (x0, y0, x1, y1)) = streams
        .iter()
        .enumerate()
        .filter_map(|(i, (r, _))| overlap(r).map(|o| (i, o)))
        .max_by(|a, b| ((a.1 .2 - a.1 .0) * (a.1 .3 - a.1 .1)).total_cmp(&((b.1 .2 - b.1 .0) * (b.1 .3 - b.1 .1))))?;
    let ((sx, sy, sw, sh), scale) = streams[stream];
    let frame = ((sw * scale).round() as u32, (sh * scale).round() as u32);
    let px = |v: f64| (v * scale).round().max(0.0) as u32;
    let crop = Crop::of(frame, (px(x0 - sx), px(y0 - sy), px(x1 - x0), px(y1 - y0)), even);
    Some(Target { stream, crop, size: crop.size(frame) })
}

/// Shows a still picture of the shared screens (taken from the session itself) to choose what
/// to record on, with the countdown.
#[allow(clippy::too_many_arguments)]
fn choose(app: &adw::Application, backend: Rc<dyn CaptureBackend>, cast: Rc<Sharing>, what: What, prefer: Option<ScreenRect>, settings: RecordSettings, hidden: Vec<gtk::Window>, hold: gio::ApplicationHoldGuard, done: Done) {
    let app = app.clone();
    let c2 = cast.clone();
    crate::capture::grab_frames(cast, move |still| match still {
        Ok(still) => choose_on(&app, backend, c2, still, what, prefer, settings, hidden, hold, done),
        Err(e) => {
            c2.close();
            fail(&app, &hidden, &format!("No picture of the screen came from GNOME: {e}"), done)
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn choose_on(app: &adw::Application, backend: Rc<dyn CaptureBackend>, cast: Rc<Sharing>, still: crate::capture::Still, what: What, prefer: Option<ScreenRect>, settings: RecordSettings, hidden: Vec<gtk::Window>, hold: gio::ApplicationHoldGuard, done: Done) {
    let desktop = match overlay::desktop() {
        Ok(d) => d,
        Err(e) => {
            cast.close();
            return fail(app, &hidden, &e, done);
        }
    };
    let streams = stream_rects(&cast, &desktop);
    let scale = desktop.screens.iter().map(|s| s.scale).fold(1.0, f64::max);
    let countdown = settings.countdown;
    let slot = Rc::new(RefCell::new(Some((cast, settings, hidden, hold, done))));
    let (app2, slot2, streams2) = (app.clone(), slot.clone(), streams.clone());
    let chosen: overlay::Done = Box::new(move |area| {
        let Some((cast, settings, hidden, hold, done)) = slot2.borrow_mut().take() else { return };
        let Some(area) = area else {
            cast.close();
            restore(&hidden, false);
            return done(Outcome::Cancelled);
        };
        let rect = (area.x, area.y, area.w, area.h);
        // Only an area chosen by hand is remembered for next time (not a whole screen).
        if what == What::Area {
            let px = |v: f64| (v * scale).round();
            let _ = obtura_core::last_region::save(obtura_core::last_region::Region { x: px(rect.0) as i32, y: px(rect.1) as i32, w: px(rect.2).max(1.0) as u32, h: px(rect.3).max(1.0) as u32 });
        }
        if what == What::AllScreens && streams2.len() > 1 {
            let (layout, size, _) = all_screens(&streams2, settings.format != RecordFormat::Gif);
            return begin(&app2, cast, Shape::All { layout, size }, settings, hidden, hold, done);
        }
        match target(&streams2, rect, settings.format != RecordFormat::Gif) {
            Some(t) => begin(&app2, cast, Shape::Part(t), settings, hidden, hold, done),
            None => {
                cast.close();
                fail(&app2, &hidden, "That part of the screen is not shared with Obtura, so it cannot be recorded. Use “Choose Again” in the Record window to share it.", done)
            }
        }
    });
    let whole_screen = match what {
        What::Screen => screen_to_record(&streams, prefer),
        // All the shared screens (one only: that one).
        What::AllScreens => Some(all_screens(&streams, true).2),
        What::Area | What::Window => None,
    };
    let shown = if let Some(r) = whole_screen {
        if countdown == 0 {
            chosen(Some(overlay::Area { x: r.0, y: r.1, w: r.2, h: r.3, desktop: (1.0, 1.0) }));
            Ok(())
        } else {
            overlay::count_down(&still.picture, r, countdown, chosen)
        }
    } else {
        let last = if what == What::Area { obtura_core::last_region::load().map(|r| obtura_image::Rect::new(r.x, r.y, r.w, r.h)) } else { None };
        // S on a screen GNOME does not share: choose the screens again, then go on.
        let (app3, slot3) = (app.clone(), slot.clone());
        let share_again: Box<dyn FnOnce()> = Box::new(move || {
            if let Some((cast, settings, hidden, hold, done)) = slot3.borrow_mut().take() {
                cast.close();
                let _ = screen_share::forget();
                share(&app3, backend, what, prefer, settings, hidden, hold, done);
            }
        });
        let purpose = if what == What::Screen { overlay::Purpose::RecordScreen } else { overlay::Purpose::Record };
        overlay::select_region(&still.picture, purpose, last, countdown, still.shared, Some(share_again), chosen)
    };
    if let Err(e) = shown {
        if let Some((cast, _, hidden, _hold, done)) = slot.borrow_mut().take() {
            cast.close();
            fail(app, &hidden, &e, done);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn begin(app: &adw::Application, cast: Rc<Sharing>, shape: Shape, settings: RecordSettings, hidden: Vec<gtk::Window>, hold: gio::ApplicationHoldGuard, done: Done) {
    let format = format_of(settings.format);
    let (streams, crop, layout, fixed, size) = match shape {
        Shape::Part(t) => (vec![t.stream], Some(t.crop), Vec::new(), None, t.size),
        Shape::All { layout, size } => ((0..cast.streams.len()).collect(), None, layout, Some(size), size),
        Shape::Window { size } => (vec![0], None, Vec::new(), Some(size), size),
    };
    let percent = settings.gif_scale.clamp(10, 100);
    let gif_size = (((size.0 * percent) / 100).max(2), ((size.1 * percent) / 100).max(2));
    let mut sounds = Vec::new();
    if settings.microphone {
        sounds.push(Sound::Microphone);
    }
    if settings.computer_sound {
        sounds.push(Sound::Computer);
    }
    let plan = Plan {
        format,
        crop,
        sounds: if format == Format::Gif { Vec::new() } else { sounds },
        gif_fps: settings.gif_fps.clamp(1, 30),
        gif_size,
        encoders: encoders(),
        threads: std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4).clamp(1, 8),
        layout,
        size: fixed,
    };
    let dir = scratch_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        cast.close();
        return fail(app, &hidden, &format!("{}: {e}", dir.display()), done);
    }
    let output = unique_path(&settings.dir(), &format_name(&settings.template, &now()), format.extension());
    let size = if format == Format::Gif { gif_size } else { size };
    let rec = Rc::new(Recording {
        app: app.clone(),
        cast,
        streams,
        plan,
        dir,
        output,
        size,
        segment: RefCell::new(None),
        state: Cell::new(State::Recording),
        segments: Cell::new(0),
        recorded: Cell::new(Duration::ZERO),
        since: Cell::new(None),
        discard: Cell::new(false),
        wants_window: Cell::new(false),
        discard_asked: Cell::new(false),
        error: RefCell::new(None),
        hidden: RefCell::new(hidden),
        controls: RefCell::new(None),
        done: RefCell::new(Some(done)),
        _hold: hold,
    });
    CURRENT.with(|c| *c.borrow_mut() = Some(rec.clone()));
    // GNOME's sharing icon in the top bar stops the session: save what was recorded.
    rec.cast.on_closed(stop);
    // Obtura's own icon in the top bar: the time, and Pause / Resume, Stop and Save, Discard.
    let shown = crate::tray::show(&clock_text(Duration::ZERO, false), |action| {
        let Some(r) = current() else { return };
        match action {
            crate::tray::Action::Pause => r.pause(false),
            crate::tray::Action::Resume => r.resume(),
            crate::tray::Action::Stop => r.stop(),
            crate::tray::Action::Discard => r.ask_discard(),
            _ => {}
        }
    });
    if !shown {
        // No status icons in this top bar: a notification (in the message list) offers Stop.
        let n = gio::Notification::new("Obtura is recording the screen");
        n.set_body(Some("To pause, open Obtura again. To stop, click the screen-sharing icon in the top bar."));
        n.set_priority(gio::NotificationPriority::Low);
        n.add_button("Stop and Save", "app.record-stop");
        n.set_default_action("app.record-controls");
        app.send_notification(Some(NOTIFICATION), &n);
    }
    // The time beside the icon, every second.
    let weak = Rc::downgrade(&rec);
    glib::timeout_add_local(Duration::from_millis(500), move || match weak.upgrade() {
        Some(r) if r.state.get() == State::Stopping => {
            crate::tray::update("Saving…", true);
            glib::ControlFlow::Continue
        }
        Some(r) if r.state.get() != State::Finished => {
            let paused = matches!(r.state.get(), State::Paused | State::Pausing);
            crate::tray::update(&clock_text(r.elapsed(), paused), paused);
            glib::ControlFlow::Continue
        }
        _ => glib::ControlFlow::Break,
    });
    // The overlay must be off the screen first (and GNOME's top bar and dock back).
    let r2 = rec.clone();
    glib::timeout_add_local_once(crate::capture::AFTER_OVERLAY, move || r2.next_segment());
}

const NOTIFICATION: &str = "obtura-recording";

/// A private folder for one recording's parts, in Obtura's cache folder.
fn scratch_dir() -> PathBuf {
    thread_local!(static N: Cell<u32> = const { Cell::new(0) });
    let n = N.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    obtura_core::paths::cache_dir().join(format!("recording-{}-{n}", std::process::id()))
}

/// Removes what a crashed Obtura left in its cache folder: recording folders of processes
/// that are gone. Only Obtura's own `recording-<pid>-<n>` folders are touched.
pub fn clean_up_leftovers() {
    let Ok(entries) = std::fs::read_dir(obtura_core::paths::cache_dir()) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(pid) = name.strip_prefix("recording-").and_then(|r| r.split('-').next()).and_then(|p| p.parse::<u32>().ok()) else { continue };
        if pid != std::process::id() && !Path::new(&format!("/proc/{pid}")).exists() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

impl Recording {
    fn next_segment(self: &Rc<Self>) {
        let n = self.segments.get() + 1;
        self.segments.set(n);
        let (out, first) = match self.plan.format {
            Format::Gif => (self.dir.join("frame-%06d.png"), record::segments(&self.dir, Format::Gif).len() as u32),
            f => (self.dir.join(format!("seg-{n:04}.{}", f.extension())), 0),
        };
        let weak = Rc::downgrade(self);
        let started = Segment::start(&self.cast, &self.streams, &self.plan, &out, first, Box::new(move |result| {
            if let Some(r) = weak.upgrade() {
                r.segment_ended(result);
            }
        }));
        match started {
            Ok(seg) => {
                *self.segment.borrow_mut() = Some(seg);
                self.state.set(State::Recording);
                self.since.set(Some(Instant::now()));
            }
            Err(e) => {
                obtura_core::log::error(&format!("the recording program could not start: {e}"));
                *self.error.borrow_mut() = Some(e.to_string());
                self.finish();
            }
        }
    }

    fn segment_ended(self: &Rc<Self>, result: Result<(), String>) {
        self.segment.borrow_mut().take();
        if let Some(t) = self.since.take() {
            self.recorded.set(self.recorded.get() + t.elapsed());
        }
        if let Err(e) = result {
            obtura_core::log::error(&format!("the recording program stopped with an error: {e}"));
            self.error.borrow_mut().get_or_insert(e);
        }
        match self.state.get() {
            State::Pausing => {
                self.state.set(State::Paused);
                crate::tray::update(&clock_text(self.elapsed(), true), true);
                if self.wants_window.get() {
                    self.show_controls();
                    if self.discard_asked.get() {
                        self.confirm_discard();
                    }
                }
            }
            // Ended by itself (an error, or the screen is no longer shared), or asked to stop.
            State::Recording | State::Stopping => self.finish(),
            State::Paused | State::Finished => {}
        }
    }

    /// Ends the current segment. With `window`, the paused window shows too (Obtura was opened
    /// while recording); from the top-bar menu, the icon there shows the pause.
    fn pause(self: &Rc<Self>, window: bool) {
        match self.state.get() {
            State::Recording => {
                self.state.set(State::Pausing);
                self.stop_clock();
                self.wants_window.set(window);
                match self.segment.borrow().as_ref() {
                    Some(seg) => seg.stop(GRACE),
                    None => self.state.set(State::Paused),
                }
                if window {
                    self.show_controls();
                }
            }
            State::Paused | State::Pausing if window => self.show_controls(),
            _ => {}
        }
        crate::tray::update(&clock_text(self.elapsed(), true), true);
    }

    /// Discard from the top-bar menu: pause, then ask in the paused window.
    fn ask_discard(self: &Rc<Self>) {
        self.pause(true);
        self.discard_asked.set(true);
        if self.state.get() == State::Paused {
            self.confirm_discard();
        }
    }

    fn confirm_discard(self: &Rc<Self>) {
        self.discard_asked.set(false);
        let Some((win, _)) = self.controls.borrow().clone() else { return };
        let dialog = adw::AlertDialog::new(Some("Discard this recording?"), Some("What was recorded is deleted."));
        dialog.add_responses(&[("cancel", "Cancel"), ("discard", "Discard")]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_close_response("cancel");
        let me = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if let (Some(r), "discard") = (me.upgrade(), response) {
                r.discard.set(true);
                r.stop();
            }
        });
        dialog.present(Some(&win));
    }

    fn resume(self: &Rc<Self>) {
        if self.state.get() != State::Paused {
            return;
        }
        if let Some((w, _)) = self.controls.borrow_mut().take() {
            w.destroy();
        }
        self.state.set(State::Recording);
        crate::tray::update(&clock_text(self.elapsed(), false), false);
        let me = self.clone();
        glib::timeout_add_local_once(HIDE_DELAY, move || {
            if me.state.get() == State::Recording && me.segment.borrow().is_none() {
                me.next_segment();
            }
        });
    }

    fn stop(self: &Rc<Self>) {
        match self.state.get() {
            State::Recording | State::Pausing => {
                self.state.set(State::Stopping);
                self.stop_clock();
                crate::tray::update("Saving…", true);
                let running = self.segment.borrow().as_ref().map(|seg| seg.stop(GRACE)).is_some();
                if !running {
                    self.finish();
                }
            }
            State::Paused => {
                self.state.set(State::Stopping);
                self.finish();
            }
            State::Stopping | State::Finished => {}
        }
    }

    fn elapsed(&self) -> Duration {
        self.recorded.get() + self.since.get().map(|t| t.elapsed()).unwrap_or_default()
    }

    /// The time stops when Pause or Stop is chosen, not when the recording program has finished
    /// its file (that can take a few seconds more).
    fn stop_clock(&self) {
        if let Some(t) = self.since.take() {
            self.recorded.set(self.recorded.get() + t.elapsed());
        }
    }

    /// The small window shown while paused. It is put on the screen at once, so it gets the
    /// focus the click on Obtura's icon gives, but stays see-through until the recording has
    /// really paused: it must not be in the recording.
    fn show_controls(self: &Rc<Self>) {
        let paused = self.state.get() == State::Paused;
        let text = || format!("{} recorded so far.\nClosing this window stops and saves the recording.", duration_text(self.elapsed()));
        if let Some((w, page)) = self.controls.borrow().as_ref() {
            if paused {
                page.set_description(Some(&text()));
                w.set_opacity(1.0);
            }
            w.present();
            return;
        }
        let page = adw::StatusPage::builder().title("Recording paused").description(text()).build();
        page.set_child(Some(&big_icon()));
        let resume = gtk::Button::builder().label("Resume").css_classes(["pill", "suggested-action"]).build();
        let save = gtk::Button::builder().label("Stop and Save").css_classes(["pill"]).build();
        let discard = gtk::Button::builder().label("Discard").css_classes(["pill", "destructive-action"]).build();
        let buttons = gtk::Box::new(gtk::Orientation::Vertical, 10);
        buttons.set_halign(gtk::Align::Center);
        buttons.set_margin_bottom(24);
        buttons.append(&resume);
        buttons.append(&save);
        buttons.append(&discard);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&page);
        body.append(&buttons);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&body));
        let win = adw::ApplicationWindow::builder().application(&self.app).title("Obtura: recording paused").default_width(380).default_height(460).content(&view).build();
        win.set_opacity(if paused { 1.0 } else { 0.0 });
        win.set_default_widget(Some(&resume));
        resume.grab_focus();
        let me = Rc::downgrade(self);
        resume.connect_clicked(move |_| {
            if let Some(r) = me.upgrade() {
                r.resume();
            }
        });
        let me = Rc::downgrade(self);
        save.connect_clicked(move |_| {
            if let Some(r) = me.upgrade() {
                r.stop();
            }
        });
        let me = Rc::downgrade(self);
        discard.connect_clicked(move |b| {
            let dialog = adw::AlertDialog::new(Some("Discard this recording?"), Some("What was recorded is deleted."));
            dialog.add_responses(&[("cancel", "Cancel"), ("discard", "Discard")]);
            dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            dialog.set_close_response("cancel");
            let me = me.clone();
            dialog.connect_response(None, move |_, response| {
                if let (Some(r), "discard") = (me.upgrade(), response) {
                    r.discard.set(true);
                    r.stop();
                }
            });
            dialog.present(Some(b));
        });
        let me = Rc::downgrade(self);
        win.connect_close_request(move |_| {
            if let Some(r) = me.upgrade() {
                // This window is closing by itself: the end must not close it again.
                r.controls.borrow_mut().take();
                if matches!(r.state.get(), State::Paused | State::Pausing) {
                    r.stop();
                }
            }
            glib::Propagation::Proceed
        });
        *self.controls.borrow_mut() = Some((win.clone(), page));
        win.present();
    }

    /// The end: the session is closed, the segments become one file in the recordings folder.
    fn finish(self: &Rc<Self>) {
        if self.state.replace(State::Finished) == State::Finished {
            return;
        }
        CURRENT.with(|c| c.borrow_mut().take());
        self.cast.close();
        crate::tray::hide();
        self.app.withdraw_notification(NOTIFICATION);
        if let Some((w, _)) = self.controls.borrow_mut().take() {
            w.destroy();
        }
        let shown = restore(&self.hidden.borrow(), true);
        let done = self.done.borrow_mut().take();
        if self.discard.get() {
            let _ = std::fs::remove_dir_all(&self.dir);
            if let Some(done) = done {
                done(Outcome::Cancelled);
            }
            return;
        }
        let window = Saved::open(&self.app, &self.output, self.elapsed(), self.size, self.plan.format, record::sound_parser(&self.plan).is_some());
        window.bring_forward(shown);
        let dir = self.dir.clone();
        let error = self.error.borrow().clone();
        let me = self.clone();
        self.save(move |result| {
            let _ = std::fs::remove_dir_all(&dir);
            match result {
                Ok(path) => {
                    window.ready(&path);
                    if let Some(done) = done {
                        done(Outcome::Printed(path.display().to_string()));
                    }
                }
                Err(e) => {
                    // When nothing could be saved, what stopped the recording is the better reason.
                    let why = match error {
                        Some(first) if e.starts_with("nothing was recorded") => first,
                        _ => e,
                    };
                    window.failed(&why);
                    if let Some(done) = done {
                        done(Outcome::Failed(why));
                    }
                }
            }
            drop(me);
        });
    }

    /// Makes the final file from the segments.
    fn save(self: &Rc<Self>, then: impl FnOnce(Result<PathBuf, String>) + 'static) {
        let output = self.output.clone();
        let format = self.plan.format;
        let parts = record::segments(&self.dir, format);
        let parts: Vec<PathBuf> = parts.into_iter().filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0)).collect();
        if parts.is_empty() {
            return then(Err("nothing was recorded".into()));
        }
        match format {
            Format::Gif => {
                let (size, fps) = (self.plan.gif_size, self.plan.gif_fps);
                let tmp = self.dir.join("made.gif");
                glib::spawn_future_local(async move {
                    let made = gio::spawn_blocking(move || make_gif(&parts, size, fps, &tmp).map(|_| tmp)).await;
                    then(made.unwrap_or_else(|_| Err("making the GIF stopped".into())).and_then(|tmp| move_file(&tmp, &output).map(|_| output)));
                });
            }
            _ if parts.len() == 1 => then(move_file(&parts[0], &output).map(|_| output)),
            _ => {
                let joined = self.dir.join(format!("joined.{}", format.extension()));
                let pattern = self.dir.join(format!("seg-*.{}", format.extension()));
                let args = record::join_args(&pattern, format, record::sound_parser(&self.plan), &joined);
                record::run(args, move |result| then(result.map_err(|e| format!("joining the parts failed: {e}")).and_then(|_| move_file(&joined, &output).map(|_| output))));
            }
        }
    }
}

fn make_gif(frames: &[PathBuf], size: (u32, u32), fps: u32, out: &Path) -> Result<(), String> {
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut gif = obtura_image::gif::GifWriter::new(std::io::BufWriter::new(file), size.0, size.1, fps as f64)?;
    for f in frames {
        let png = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let frame = obtura_image::decode_png(&png).map_err(|e| e.to_string())?;
        if (frame.width, frame.height) == size {
            gif.add(&frame)?;
        }
    }
    let (mut w, _) = gif.finish()?;
    std::io::Write::flush(&mut w).map_err(|e| e.to_string())
}

/// Moves a finished file to its place (copying when it is on another disk).
pub(crate) fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("{}: {e}", to.display());
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(failed)?;
    }
    if std::fs::rename(from, to).is_err() {
        std::fs::copy(from, to).map_err(failed)?;
        let _ = std::fs::remove_file(from);
    }
    Ok(())
}

/// The time beside the top-bar icon: "0:42", "Paused 1:02:05".
fn clock_text(d: Duration, paused: bool) -> String {
    let s = d.as_secs();
    let t = if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) };
    if paused {
        format!("Paused {t}")
    } else {
        t
    }
}

fn big_icon() -> gtk::Image {
    let icon = crate::icons::image("record");
    icon.set_pixel_size(48);
    icon
}

pub fn duration_text(d: Duration) -> String {
    let s = d.as_secs_f64().round() as u64;
    match (s / 3600, (s / 60) % 60, s % 60) {
        (0, 0, s) => format!("{s} s"),
        (0, m, s) => format!("{m} min {s:02} s"),
        (h, m, s) => format!("{h} h {m:02} min {s:02} s"),
    }
}

fn size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b => format!("{} kB", (b + 1023) / 1024),
    }
}

/// Shows Obtura's windows again after a recording (the start window was only the launcher),
/// or after a cancelled one (everything comes back). After a recording they come back without
/// asking for the focus: the "Recording saved" window must end up in front of them. Returns
/// the windows shown again.
fn restore(hidden: &[gtk::Window], finished: bool) -> Vec<gtk::Window> {
    let mut shown = Vec::new();
    for w in hidden {
        if finished && w.widget_name() == crate::home::NAME {
            w.close();
        } else {
            w.set_visible(true);
            if !finished {
                w.present();
            }
            shown.push(w.clone());
        }
    }
    shown
}

fn fail(app: &adw::Application, hidden: &[gtk::Window], why: &str, done: Done) {
    obtura_core::log::error(&format!("recording failed: {why}"));
    restore(hidden, false);
    let parent = app.active_window().unwrap_or_else(|| crate::home::open(app).upcast());
    let dialog = adw::AlertDialog::new(Some("Could not record the screen"), Some(why));
    dialog.add_response("ok", "OK");
    dialog.present(Some(&parent));
    done(Outcome::Failed(why.lines().next().unwrap_or(why).to_string()));
}

/// The window that shows a recording being saved, then saved.
struct Saved {
    win: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    page: adw::StatusPage,
    buttons: gtk::Box,
    /// The file (a trim may move it) and its length.
    path: Rc<RefCell<PathBuf>>,
    recorded: Cell<Duration>,
    size: (u32, u32),
    format: Format,
    with_sound: bool,
}

impl Saved {
    fn open(app: &adw::Application, path: &Path, recorded: Duration, size: (u32, u32), format: Format, with_sound: bool) -> Rc<Saved> {
        let page = adw::StatusPage::builder().title("Saving the recording…").description(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).build();
        let spinner = gtk::Spinner::builder().spinning(true).width_request(48).height_request(48).build();
        page.set_child(Some(&spinner));
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::Center);
        buttons.set_margin_bottom(24);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&page);
        body.append(&buttons);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&body));
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&toasts));
        let win = adw::ApplicationWindow::builder().application(app).title("Obtura: recording").default_width(680).default_height(440).content(&view).build();
        Rc::new(Saved { win, toasts, page, buttons, path: Rc::new(RefCell::new(path.to_path_buf())), recorded: Cell::new(recorded), size, format, with_sound })
    }

    /// Shows the window in front of everything, also of Obtura's windows that came back with it
    /// (`shown`): once they are on the screen (a window that appears later would cover it), and
    /// kept above the last editor among them.
    fn bring_forward(&self, shown: Vec<gtk::Window>) {
        let win = self.win.clone();
        if let Some(editor) = shown.iter().rev().find(|w| w.is_visible() && w.widget_name() != crate::home::NAME) {
            win.set_transient_for(Some(editor));
        }
        let started = Instant::now();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let ready = shown.iter().all(|w| !w.is_visible() || w.is_mapped());
            if (ready && started.elapsed() >= Duration::from_millis(150)) || started.elapsed() >= Duration::from_millis(1500) {
                win.present();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    /// Name, length, size and folder of the file.
    fn describe(&self) {
        let path = self.path.borrow().clone();
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let folder = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
        self.page.set_description(Some(&format!("{name}\n{} · {} × {} · {}\nin {folder}", duration_text(self.recorded.get()), self.size.0, self.size.1, size_text(bytes))));
    }

    fn ready(self: &Rc<Self>, path: &Path) {
        *self.path.borrow_mut() = path.to_path_buf();
        self.win.set_title(Some("Obtura: recording saved"));
        self.page.set_title("Recording saved");
        self.describe();
        self.page.set_child(Some(&big_icon()));
        let file_of = {
            let p = self.path.clone();
            move || gio::File::for_path(&*p.borrow())
        };
        let button = |label: &str, tip: &str| gtk::Button::builder().label(label).tooltip_text(tip).css_classes(["pill"]).build();
        let open = button("Open", "Play it in the usual app");
        open.add_css_class("suggested-action");
        let show = button("Show in Files", "Open its folder");
        let copy = button("Copy", "Copy the file, to paste it in a chat, a mail or a folder");
        let delete = button("Delete", "Move it to the wastebasket");
        let win = self.win.clone();
        let f = file_of.clone();
        open.connect_clicked(move |_| gtk::FileLauncher::new(Some(&f())).launch(Some(&win), gio::Cancellable::NONE, |_| {}));
        let win = self.win.clone();
        let f = file_of.clone();
        show.connect_clicked(move |_| gtk::FileLauncher::new(Some(&f())).open_containing_folder(Some(&win), gio::Cancellable::NONE, |_| {}));
        let (f, toasts) = (file_of.clone(), self.toasts.clone());
        copy.connect_clicked(move |b| {
            let list = gdk::FileList::from_array(&[f()]);
            let copied = b.clipboard().set_content(Some(&gdk::ContentProvider::for_value(&list.to_value())));
            toasts.add_toast(adw::Toast::new(if copied.is_ok() { "Copied: paste it where it should go" } else { "Could not copy it" }));
        });
        let (win, toasts, file_of) = (self.win.clone(), self.toasts.clone(), file_of.clone());
        delete.connect_clicked(move |b| {
            let dialog = adw::AlertDialog::new(Some("Delete this recording?"), Some("It goes to the wastebasket, from where it can still be restored."));
            dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            dialog.set_close_response("cancel");
            let (f, win, toasts) = (file_of(), win.clone(), toasts.clone());
            dialog.connect_response(None, move |_, response| {
                if response != "delete" {
                    return;
                }
                match f.trash(gio::Cancellable::NONE) {
                    Ok(()) => win.close(),
                    Err(e) => toasts.add_toast(adw::Toast::new(&format!("Could not delete it: {e}"))),
                }
            });
            dialog.present(Some(b));
        });
        let trim = button("Trim…", "Cut off the start or the end");
        // The button keeps this window's state alive (until the window goes).
        let me = self.clone();
        trim.connect_clicked(move |_| {
            let s = me.clone();
            let source = crate::trim::Source { path: s.path.borrow().clone(), format: s.format, with_sound: s.with_sound, length: s.recorded.get() };
            let me = Rc::downgrade(&s);
            crate::trim::open(&s.win, source, move |path, length| {
                if let Some(s) = me.upgrade() {
                    *s.path.borrow_mut() = path;
                    s.recorded.set(length);
                    s.describe();
                    s.toasts.add_toast(adw::Toast::new("Trimmed. The untrimmed recording is in the wastebasket."));
                }
            });
        });
        for b in [&open, &trim, &show, &copy, &delete] {
            self.buttons.append(b);
        }
        open.grab_focus();
    }

    fn failed(&self, why: &str) {
        obtura_core::log::error(&format!("recording not saved: {why}"));
        self.win.set_title(Some("Obtura: recording not saved"));
        self.page.set_title("Could not save the recording");
        self.page.set_description(Some(why));
        self.page.set_child(None::<&gtk::Widget>);
    }
}

/// Windows: recording is not there yet; says what Windows itself offers.
#[cfg_attr(not(windows), allow(dead_code))]
fn not_on_windows(app: &adw::Application) {
    let parent = app.active_window().unwrap_or_else(|| crate::home::open(app).upcast());
    let dialog = adw::AlertDialog::new(
        Some("Recording is not available on Windows yet"),
        Some("Obtura for Windows takes screenshots, reads text and QR codes, and annotates; recording the screen comes later.\n\nMeanwhile, Windows' own Snipping Tool records the screen: press Win+Shift+R."),
    );
    dialog.add_response("ok", "OK");
    dialog.present(Some(&parent));
}

/// The Record window: what to record and how, then Record a Screen / Record an Area.
pub fn setup(app: &adw::Application, backend: Rc<dyn CaptureBackend>) {
    if cfg!(windows) {
        return not_on_windows(app);
    }
    if is_recording() {
        return pause_and_show();
    }
    let s = Settings::load().record;
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    let format = adw::ComboRow::builder().title("Format").model(&gtk::StringList::new(&["MP4 video", "WebM video", "Animated GIF"])).selected(match s.format {
        RecordFormat::Mp4 => 0,
        RecordFormat::WebM => 1,
        RecordFormat::Gif => 2,
    }).build();
    let mic = adw::SwitchRow::builder().title("Microphone").active(s.microphone).build();
    let sound = adw::SwitchRow::builder().title("Computer sound").subtitle("What the computer plays").active(s.computer_sound).build();
    // How loud each one is, live, while it is on (so a muted microphone shows before recording).
    let fake = !backend.shares_real_screen();
    let meters: Vec<Rc<Meter>> = [(&mic, Sound::Microphone), (&sound, Sound::Computer)].into_iter().map(|(row, which)| Meter::add(row, which, fake)).collect();
    let pointer = adw::SwitchRow::builder().title("Show the mouse pointer").active(s.pointer).build();
    let counts = [0u32, 3, 5, 10];
    let countdown = adw::ComboRow::builder().title("Countdown before recording").model(&gtk::StringList::new(&["None", "3 seconds", "5 seconds", "10 seconds"])).selected(counts.iter().position(|c| *c == s.countdown).unwrap_or(1) as u32).build();
    let fps_values = [10u32, 15, 20];
    let gif_fps = adw::ComboRow::builder().title("GIF frames per second").subtitle("Fewer make a smaller file").model(&gtk::StringList::new(&["10", "15", "20"])).selected(fps_values.iter().position(|f| *f == s.gif_fps).unwrap_or(0) as u32).build();
    let scales = [100u32, 75, 50];
    let gif_size = adw::ComboRow::builder().title("GIF size").model(&gtk::StringList::new(&["Full size", "75 %", "50 %"])).selected(scales.iter().position(|v| *v == s.gif_scale).unwrap_or(0) as u32).build();
    for row in [format.upcast_ref::<gtk::Widget>(), mic.upcast_ref(), sound.upcast_ref(), pointer.upcast_ref(), countdown.upcast_ref(), gif_fps.upcast_ref(), gif_size.upcast_ref()] {
        group.add(row);
    }
    page.add(&group);
    let sync = {
        let (format, mic, sound, gif_fps, gif_size, meters) = (format.clone(), mic.clone(), sound.clone(), gif_fps.clone(), gif_size.clone(), meters.clone());
        move || {
            let gif = format.selected() == 2;
            mic.set_sensitive(!gif);
            sound.set_sensitive(!gif);
            mic.set_subtitle(if gif { "A GIF has no sound" } else { "" });
            gif_fps.set_visible(gif);
            gif_size.set_visible(gif);
            meters[0].run(!gif && mic.is_active());
            meters[1].run(!gif && sound.is_active());
        }
    };
    sync();
    let store = {
        let (format, mic, sound, pointer, countdown, gif_fps, gif_size) = (format.clone(), mic.clone(), sound.clone(), pointer.clone(), countdown.clone(), gif_fps.clone(), gif_size.clone());
        move || {
            let mut all = Settings::load();
            let r = &mut all.record;
            r.format = [RecordFormat::Mp4, RecordFormat::WebM, RecordFormat::Gif][format.selected().min(2) as usize];
            r.microphone = mic.is_active();
            r.computer_sound = sound.is_active();
            r.pointer = pointer.is_active();
            r.countdown = counts[countdown.selected().min(3) as usize];
            r.gif_fps = fps_values[gif_fps.selected().min(2) as usize];
            r.gif_scale = scales[gif_size.selected().min(2) as usize];
            let _ = all.save();
        }
    };
    let store = Rc::new(store);
    {
        let (store, sync) = (store.clone(), sync.clone());
        format.connect_selected_notify(move |_| {
            sync();
            store();
        });
    }
    for row in [&countdown, &gif_fps, &gif_size] {
        let store = store.clone();
        row.connect_selected_notify(move |_| store());
    }
    for row in [&mic, &sound, &pointer] {
        let (store, sync) = (store.clone(), sync.clone());
        row.connect_active_notify(move |_| {
            sync();
            store();
        });
    }

    // How to stop, and the screens GNOME was told Obtura may record.
    let help = adw::PreferencesGroup::builder()
        .title("Pausing and stopping")
        .description("While recording, Obtura's icon in the top bar (a red dot with the time) pauses, resumes and stops: click it. GNOME's screen-sharing icon there stops and saves too.")
        .build();
    let share = adw::ActionRow::builder().title("Screens Obtura may record").build();
    let remembered = screen_share::load().is_some();
    share.set_subtitle(if remembered {
        "Chosen and remembered by GNOME. To share other screens, choose them again."
    } else {
        "The first time, GNOME asks which screens Obtura may record: select all of them and tick “Remember this selection”."
    });
    let again = gtk::Button::builder().label("Choose Again").valign(gtk::Align::Center).visible(remembered).build();
    {
        let share = share.clone();
        again.connect_clicked(move |b| {
            let _ = screen_share::forget();
            b.set_visible(false);
            share.set_subtitle("GNOME will ask which screens Obtura may record: select all of them and tick “Remember this selection”.");
        });
    }
    share.add_suffix(&again);
    help.add(&share);
    page.add(&help);

    let screen = gtk::Button::builder().label("Record a Screen").tooltip_text("The whole screen this window is on").css_classes(["pill"]).build();
    let area = gtk::Button::builder().label("Record an Area").tooltip_text("Drag over what to record; the area chosen last time is proposed").css_classes(["pill", "suggested-action"]).build();
    let window = gtk::Button::builder().label("A Window…").tooltip_text("Pick a window in GNOME's list; the recording follows it").css_classes(["flat"]).build();
    let all = gtk::Button::builder().label("All Screens").tooltip_text("Every screen shared with Obtura, side by side in one video").css_classes(["flat"]).build();
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    buttons.set_margin_top(6);
    buttons.append(&screen);
    buttons.append(&area);
    let more = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    more.set_halign(gtk::Align::Center);
    more.set_margin_bottom(14);
    more.append(&window);
    more.append(&all);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.append(&page);
    body.append(&buttons);
    body.append(&more);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&body));
    let dialog = adw::Dialog::builder().title("Record the Screen").content_width(480).content_height(640).child(&view).build();
    dialog.set_default_widget(Some(&area));
    {
        // The meters stop with the window (and the microphone is let go).
        let meters = meters.clone();
        dialog.connect_closed(move |_| meters.iter().for_each(|m| m.run(false)));
    }
    for (button, what) in [(&screen, What::Screen), (&area, What::Area), (&window, What::Window), (&all, What::AllScreens)] {
        let (dialog, app, backend) = (dialog.clone(), app.clone(), backend.clone());
        button.connect_clicked(move |_| {
            dialog.close();
            let (app, backend) = (app.clone(), backend.clone());
            // Once the dialog has gone.
            glib::idle_add_local_once(move || start(&app, backend, what, Box::new(|_| {})));
        });
    }
    match app.active_window() {
        Some(w) => dialog.present(Some(&w)),
        None => dialog.present(Some(&crate::home::open(app))),
    }
}

/// A level meter beside a sound switch of the Record window: while on, a `gst-launch-1.0` reads
/// that sound and the bar shows how loud it is. Nothing is recorded or kept.
struct Meter {
    bar: gtk::LevelBar,
    which: Sound,
    fake: bool,
    program: RefCell<Option<gio::Subprocess>>,
}

impl Meter {
    fn add(row: &adw::SwitchRow, which: Sound, fake: bool) -> Rc<Meter> {
        let bar = gtk::LevelBar::builder().min_value(0.0).max_value(1.0).width_request(90).valign(gtk::Align::Center).visible(false).build();
        bar.set_tooltip_text(Some("How loud it is now"));
        // No colour changes at the usual marks: one plain bar.
        for name in ["low", "high", "full"] {
            bar.remove_offset_value(Some(name));
        }
        row.add_suffix(&bar);
        Rc::new(Meter { bar, which, fake, program: RefCell::new(None) })
    }

    /// Starts (`on`) or stops reading the sound.
    fn run(self: &Rc<Self>, on: bool) {
        if !on {
            if let Some(p) = self.program.borrow_mut().take() {
                p.force_exit();
            }
            self.bar.set_visible(false);
            return;
        }
        if self.program.borrow().is_some() || !record::tools_present() {
            return;
        }
        let mut argv: Vec<String> = vec!["/usr/bin/gst-launch-1.0".into(), "-m".into()];
        argv.extend(record::level_args(obtura_capture::sound_args(self.which, self.fake)));
        let args: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
        let program = match gio::Subprocess::newv(&args, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE) {
            Ok(p) => p,
            Err(e) => {
                obtura_core::log::warn(&format!("sound level: {e}"));
                return;
            }
        };
        self.bar.set_value(0.0);
        self.bar.set_visible(true);
        let Some(out) = program.stdout_pipe() else { return };
        *self.program.borrow_mut() = Some(program);
        let lines = gio::DataInputStream::new(&out);
        let weak = Rc::downgrade(self);
        read_levels(lines, weak);
    }
}

/// Reads the level lines one by one, until the program ends.
fn read_levels(lines: gio::DataInputStream, meter: std::rc::Weak<Meter>) {
    let lines2 = lines.clone();
    lines.read_line_utf8_async(glib::Priority::DEFAULT, gio::Cancellable::NONE, move |r| {
        let Some(m) = meter.upgrade() else { return };
        match r {
            Ok(Some(line)) => {
                if let Some(level) = record::parse_level(&line) {
                    m.bar.set_value(level);
                }
                read_levels(lines2, meter);
            }
            _ => {
                m.bar.set_visible(false);
                m.program.borrow_mut().take();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_holding_most_of_the_area_records_it() {
        // Two screens side by side, the right one at scale 2.
        let streams = [((0.0, 0.0, 1920.0, 1080.0), 1.0), ((1920.0, 0.0, 1280.0, 800.0), 2.0)];
        // Mostly on the right screen: cut to it, in its physical pixels.
        let t = target(&streams, (1900.0, 100.0, 300.0, 101.0), true).unwrap();
        assert_eq!(t.stream, 1);
        assert_eq!((t.crop.left, t.crop.top), (0, 200));
        assert_eq!(t.size, (560, 202));
        // On the left screen only.
        let t = target(&streams, (10.0, 20.0, 301.0, 201.0), true).unwrap();
        assert_eq!((t.stream, t.crop.left, t.crop.top, t.size), (0, 10, 20, (300, 200)));
        // A GIF keeps odd sizes.
        assert_eq!(target(&streams, (10.0, 20.0, 301.0, 201.0), false).unwrap().size, (301, 201));
        // A whole screen.
        let t = target(&streams, (1920.0, 0.0, 1280.0, 800.0), true).unwrap();
        assert_eq!((t.stream, t.crop, t.size), (1, Crop::default(), (2560, 1600)));
        // Nowhere on a shared screen.
        assert_eq!(target(&streams[..1], (2000.0, 0.0, 100.0, 100.0), true), None);
    }

    #[test]
    fn record_a_screen_takes_the_screen_of_the_window() {
        let left = (0.0, 0.0, 1920.0, 1080.0);
        let right = (1920.0, 0.0, 1920.0, 1080.0);
        let both = [(left, 1.0), (right, 1.0)];
        // One shared screen: that one, wherever the window was.
        assert_eq!(screen_to_record(&both[..1], Some(right)), Some(left));
        assert_eq!(screen_to_record(&both[..1], None), Some(left));
        // Two: the one the window is on; none known (command line): the user clicks.
        assert_eq!(screen_to_record(&both, Some(right)), Some(right));
        assert_eq!(screen_to_record(&both, Some(left)), Some(left));
        assert_eq!(screen_to_record(&both, None), None);
        // The window's screen is not shared: the user clicks.
        assert_eq!(screen_to_record(&both[..1], Some((3840.0, 0.0, 1920.0, 1080.0))), Some(left));
        assert_eq!(screen_to_record(&[(left, 1.0), ((0.0, 1080.0, 1920.0, 1080.0), 1.0)], Some(right)), None);
    }

    #[test]
    fn all_screens_go_side_by_side_at_the_largest_scale() {
        let left = ((0.0, 0.0, 1920.0, 1080.0), 1.0);
        let right = ((1920.0, 200.0, 1280.0, 800.0), 1.0);
        let (layout, size, bounds) = all_screens(&[left, right], true);
        assert_eq!(layout, vec![(0, 0, 1920, 1080), (1920, 200, 1280, 800)]);
        assert_eq!(size, (3200, 1080));
        assert_eq!(bounds, (0.0, 0.0, 3200.0, 1080.0));
        // One screen at scale 2: everything at scale 2.
        let (layout, size, _) = all_screens(&[left, ((1920.0, 0.0, 1281.0, 800.0), 2.0)], true);
        assert_eq!(layout[1], (3840, 0, 2562, 1600));
        assert_eq!(size, (6402, 2160));
        // Odd sizes are made even for video, not for a GIF.
        let odd = ((0.0, 0.0, 801.0, 601.0), 1.0);
        assert_eq!(all_screens(&[odd], true).1, (800, 600));
        assert_eq!(all_screens(&[odd], false).1, (801, 601));
    }

    #[test]
    fn the_top_bar_clock_reads_well() {
        assert_eq!(clock_text(Duration::from_secs(42), false), "0:42");
        assert_eq!(clock_text(Duration::from_secs(605), true), "Paused 10:05");
        assert_eq!(clock_text(Duration::from_secs(3725), false), "1:02:05");
    }

    #[test]
    fn durations_and_sizes_read_well() {
        assert_eq!(duration_text(Duration::from_secs(42)), "42 s");
        assert_eq!(duration_text(Duration::from_secs_f64(1.99)), "2 s");
        assert_eq!(duration_text(Duration::from_secs(72)), "1 min 12 s");
        assert_eq!(duration_text(Duration::from_secs(3725)), "1 h 02 min 05 s");
        assert_eq!(size_text(1500), "2 kB");
        assert_eq!(size_text(5 << 20), "5.0 MB");
    }
}
