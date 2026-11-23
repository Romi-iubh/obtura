//! The capture flow: wait for the delay, hide Obtura's own windows, take a still picture of the
//! screen, let the user choose a region on it if asked, then either open the editor or copy/save
//! the picture straight away (command line).
//!
//! GNOME flashes the whole screen whenever an app takes a screenshot through its screenshot
//! service, and a flash before the region is even chosen looks like a full-screen capture. So
//! the still picture behind the overlay is taken through screen sharing (one frame of each
//! shared screen: no flash; GNOME shows its sharing icon for a few seconds instead, and asks
//! once which screens Obtura may see). Captures without the overlay (full screen, same region
//! again) use the screenshot service: there the flash comes when the picture is taken.
//!
//! The screenshot service needs the user's permission once, and GNOME asks for it only while an
//! Obtura window has the focus; Obtura's windows are out of the way while it captures, so the
//! first refusal is answered by asking from a window (see [`permission`]), then capturing again.
//!
//! Obtura's own windows must not be in the picture. They are made see-through rather than
//! hidden: a hidden window comes back wherever GNOME puts it (often on another screen), a
//! see-through one never moves.

use crate::{overlay, window::Window};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use obtura_capture::{CaptureBackend, CaptureError, CaptureKind, CaptureRequest, CastRequest};
use obtura_core::{
    naming::{format_name, unique_path, LocalTime},
    settings::Settings,
};
use obtura_image::{decode_png, encode_png, Bitmap};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Region,
    Screen,
    /// The region chosen last time, without the overlay (the overlay if there is none yet).
    LastRegion,
    /// A part of the screen chosen on the overlay, then joined while the user scrolls it.
    Scrolling,
    /// One window, picked in GNOME's list of windows (on Windows: clicked on the overlay), asked every time.
    Window,
}

/// What happens to the picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Open it in the editor.
    Editor,
    /// No editor: copy to the clipboard and/or save to a file (`Some("")` means the
    /// save folder under the naming template).
    Direct { copy: bool, save: Option<PathBuf> },
    /// Read the text in the picture and copy it (the command line also prints it).
    Text,
    /// Read the QR codes in the picture and copy what they hold (printed too).
    Qr,
    /// Pin the picture on the screen.
    Pin,
    /// Pick one pixel's colour on the frozen screen and copy it as `#rrggbb`.
    Color,
    /// Measure on the frozen screen; the measurement is copied (and printed).
    Measure,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub mode: Mode,
    /// Seconds to wait before capturing, 0 to 30.
    pub delay: u32,
    pub output: Output,
}

impl Request {
    pub fn interactive(mode: Mode) -> Self {
        Self { mode, delay: 0, output: Output::Editor }
    }
}

/// How a capture ended. Becomes the exit status on the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Done, with something for the command line to print (text read, a colour).
    Printed(String),
    Cancelled,
    Failed(String),
}

impl Outcome {
    pub fn exit_code(&self) -> i32 {
        match self {
            Outcome::Done | Outcome::Printed(_) => 0,
            Outcome::Failed(_) => 1,
            Outcome::Cancelled => 3,
        }
    }
}

/// Time for the compositor to really remove our windows before the screen is captured.
const HIDE_DELAY: Duration = Duration::from_millis(250);
/// After the fullscreen overlay: GNOME hid its top bar and the dock while it was up, and the
/// dock slides back in.
pub const AFTER_OVERLAY: Duration = Duration::from_millis(400);

thread_local! {
    static HELD: Cell<bool> = const { Cell::new(false) };
    /// The capture under way, if any: alive as long as its `done` is (so a capture whose
    /// `done` was dropped without being called, with a closed dialog, is not under way either).
    static UNDER_WAY: RefCell<std::rc::Weak<()>> = RefCell::new(std::rc::Weak::new());
}

/// Whether a capture is under way, from its delay to its end (one at a time: a second overlay
/// over the first, or a region capture in the middle of a scrolling one, would mix them up).
pub fn is_busy() -> bool {
    UNDER_WAY.with(|u| u.borrow().strong_count() > 0)
}

/// Keeps Obtura running after its windows close, so the next capture is instant.
pub(crate) fn keep_running(app: &adw::Application) {
    if !HELD.replace(true) {
        std::mem::forget(app.hold());
    }
}

pub type Done = Box<dyn FnOnce(Outcome)>;

pub fn start(app: &adw::Application, backend: Rc<dyn CaptureBackend>, request: Request, done: Done) {
    // A second scrolling capture while one runs finishes it (one key starts and finishes).
    if request.mode == Mode::Scrolling && crate::scroll::is_running() {
        crate::scroll::finish();
        return done(Outcome::Done);
    }
    if is_busy() {
        obtura_core::log::info("a capture is already under way: no other one is started");
        return done(Outcome::Failed("another capture is under way".into()));
    }
    let under_way = Rc::new(());
    UNDER_WAY.with(|u| *u.borrow_mut() = Rc::downgrade(&under_way));
    let settings = Settings::load();
    // Stay running after an editor capture so the next one is instant, and after --copy,
    // because the clipboard only lasts while Obtura runs. A script that only saves a file
    // (--save) must get its prompt back, so it does not stay.
    let stays = match request.output {
        Output::Editor => settings.keep_running,
        Output::Direct { copy, .. } => copy,
        // The pinned window lives as long as Obtura.
        Output::Pin => true,
        // The clipboard too, but only once something was copied (see `deliver`, `pick`): if
        // nothing was found, Obtura ends and the command line gets its answer.
        Output::Text | Output::Qr | Output::Color | Output::Measure => false,
    };
    if stays {
        keep_running(app);
    }
    // The application must not quit while a capture is under way (no window is open
    // during the delay or while the overlay is up); it is released when the capture ends.
    let guard = app.hold();
    let app2 = app.clone();
    let done: Done = Box::new(move |outcome| {
        drop(under_way);
        if !matches!(outcome, Outcome::Done | Outcome::Printed(_)) {
            // Nothing replaces the editor that asked for this capture (its New button): it
            // stays, also through the next capture.
            for w in app2.windows() {
                crate::window::keep(&w);
            }
        }
        done(outcome);
        drop(guard);
    });
    // The first time a screenshot is taken through screen sharing, say what GNOME will ask.
    // A scrolling capture always goes through screen sharing.
    let quiet = (settings.quiet_capture && needs_still(&request)) || request.mode == Mode::Scrolling;
    if quiet && backend.shares_real_screen() && !backend.capture_is_quiet() && obtura_core::screen_share::load().is_none() {
        let app2 = app.clone();
        return explain_sharing(app, move |go| match go {
            true => begin(&app2, backend, request, settings, done),
            false => done(Outcome::Cancelled),
        });
    }
    begin(app, backend, request, settings, done)
}

/// Whether the capture shows the overlay (so needs a still picture of the screen first).
fn needs_still(request: &Request) -> bool {
    matches!(request.output, Output::Color | Output::Measure) || request.mode == Mode::Region || (request.mode == Mode::LastRegion && obtura_core::last_region::load().is_none())
}

/// Says, before GNOME's question, what to answer to it. `then(true)`: go on; `then(false)`: cancel.
fn explain_sharing(app: &adw::Application, then: impl FnOnce(bool) + 'static) {
    let parent: gtk::Window = app.active_window().filter(|w| w.is_visible()).unwrap_or_else(|| crate::home::open(app).upcast());
    let dialog = adw::AlertDialog::new(
        Some("Let Obtura see your screens"),
        Some("GNOME will now ask which screens Obtura may see. Select all of them, tick “Remember this selection”, then click Share. GNOME will not ask again.\n\nNothing leaves this computer."),
    );
    ask(&dialog, &parent, "Continue", then);
}

/// Shows `dialog` with Cancel and `go`; `then(true)` once it has gone from the screen if `go`
/// was chosen, `then(false)` otherwise.
fn ask(dialog: &adw::AlertDialog, parent: &impl IsA<gtk::Widget>, go: &str, then: impl FnOnce(bool) + 'static) {
    dialog.add_responses(&[("cancel", "Cancel"), ("go", go)]);
    dialog.set_response_appearance("go", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("go"));
    dialog.set_close_response("cancel");
    let then = RefCell::new(Some(then));
    dialog.connect_response(None, move |_, response| {
        if let Some(then) = then.borrow_mut().take() {
            let go = response == "go";
            glib::timeout_add_local_once(Duration::from_millis(150), move || then(go));
        }
    });
    dialog.present(Some(parent));
}

/// GNOME refused the screenshot because it has not asked the user yet (it asks only while an
/// Obtura window has the focus), or because the user said no. Asks from a visible window: on
/// Continue, the screenshot service is called from there, so GNOME shows its question; once
/// allowed, the capture starts again as it was asked (that first picture shows Obtura's own
/// window, so it is not used).
fn permission(app: &adw::Application, backend: Rc<dyn CaptureBackend>, request: Request, hidden: Vec<gtk::Window>, error: CaptureError, done: Done) {
    // Visible again, without giving up a "replace this editor" choice: the capture may still go on.
    for w in &hidden {
        reveal(w);
        w.present();
    }
    // The question needs a window; from the command line the start window opens for it, and
    // goes again once answered (so that `obtura --save` still ends).
    let (parent, opened): (gtk::Window, Option<gtk::Window>) = match hidden.iter().find(|w| w.is_visible()).cloned().or_else(|| app.active_window().filter(|w| w.is_visible())) {
        Some(w) => (w, None),
        None => {
            let home: gtk::Window = crate::home::open(app).upcast();
            (home.clone(), Some(home))
        }
    };
    let close_opened = move || {
        if let Some(w) = &opened {
            w.close();
        }
    };
    let denied = error == CaptureError::Denied;
    let dialog = if denied {
        adw::AlertDialog::new(Some("Screenshots are not allowed"), Some("GNOME was told not to let Obtura take screenshots. Click Ask Again, then Allow in GNOME's question."))
    } else {
        adw::AlertDialog::new(Some("Allow Obtura to take screenshots"), Some("GNOME asks this only once. Click Continue, then Allow in GNOME's question."))
    };
    let app = app.clone();
    ask(&dialog, &parent, if denied { "Ask Again" } else { "Continue" }, move |go| {
        if !go {
            close_opened();
            for w in &hidden {
                crate::window::keep(w);
            }
            return done(Outcome::Cancelled);
        }
        let b2 = backend.clone();
        let take = move || {
            b2.clone().capture(
                CaptureRequest { kind: CaptureKind::FullScreen },
                Box::new(move |result| match result {
                    // Allowed: now the real capture, Obtura's windows out of the way again.
                    Ok(_) => {
                        close_opened();
                        begin(&app, b2, Request { delay: 0, ..request }, Settings::load(), done)
                    }
                    Err(e) => {
                        close_opened();
                        let e = match e {
                            CaptureError::Denied => CaptureError::Failed("Obtura was not allowed to take screenshots. The next time, Obtura offers to ask again.".into()),
                            CaptureError::NeedsPermission => CaptureError::Failed("GNOME did not ask whether Obtura may take screenshots. Click in an Obtura window and try again.".into()),
                            e => e,
                        };
                        fail(&app, &request, &hidden, e, done)
                    }
                }),
            )
        };
        if denied {
            backend.ask_again(Box::new(take));
        } else {
            take();
        }
    });
}

/// Whether a refusal can be answered by asking the user (see [`permission`]).
fn asks_permission(e: &CaptureError) -> bool {
    matches!(e, CaptureError::NeedsPermission | CaptureError::Denied)
}

/// Waits `seconds`, counting down in Obtura's top-bar icon (its menu: take it now, cancel), then
/// `then(true)`; `then(false)` if cancelled. Without a top bar that shows status icons (or while
/// recording, which has the icon), a quiet notification says when the screenshot comes.
pub(crate) fn count_down(app: &adw::Application, seconds: u32, then: impl FnOnce(bool) + 'static) {
    if seconds == 0 {
        return then(true);
    }
    let then: Rc<RefCell<Option<Box<dyn FnOnce(bool)>>>> = Rc::new(RefCell::new(Some(Box::new(then))));
    let left = Rc::new(Cell::new(seconds));
    let finish = {
        let then = then.clone();
        move |go: bool| {
            if let Some(f) = then.borrow_mut().take() {
                crate::tray::hide();
                f(go);
            }
        }
    };
    let finish = Rc::new(finish);
    let label = |n: u32| format!("{n} s");
    let shown = !crate::recorder::is_recording() && {
        let f = finish.clone();
        crate::tray::show_as(crate::tray::Mode::Countdown, &label(seconds), move |a| match a {
            crate::tray::Action::CaptureNow => f(true),
            crate::tray::Action::Cancel => f(false),
            _ => {}
        })
    };
    if !shown {
        let n = gio::Notification::new(&format!("Screenshot in {seconds} seconds"));
        n.set_body(Some("Get the screen ready: Obtura takes the picture when the time is up."));
        n.set_priority(gio::NotificationPriority::Low);
        app.send_notification(Some("obtura-countdown"), &n);
    }
    let app = app.clone();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if then.borrow().is_none() {
            return glib::ControlFlow::Break;
        }
        let n = left.get().saturating_sub(1);
        left.set(n);
        if n == 0 {
            app.withdraw_notification("obtura-countdown");
            finish(true);
            return glib::ControlFlow::Break;
        }
        if shown {
            crate::tray::update(&label(n), false);
        }
        glib::ControlFlow::Continue
    });
}

fn begin(app: &adw::Application, backend: Rc<dyn CaptureBackend>, request: Request, settings: Settings, done: Done) {
    let app = app.clone();
    let delay = request.delay.min(30);
    count_down(&app.clone(), delay, move |go| {
        if !go {
            return done(Outcome::Cancelled);
        }
        // Our own windows must not be in the picture.
        let hidden: Vec<gtk::Window> = app.windows().into_iter().filter(|w| w.is_visible() && w.opacity() > 0.0).collect();
        for w in &hidden {
            conceal(w);
        }
        let wait = if hidden.is_empty() { Duration::ZERO } else { HIDE_DELAY };
        glib::timeout_add_local_once(wait, move || {
            if request.mode == Mode::Scrolling {
                crate::scroll::start(&app, backend, request, hidden, settings, done)
            } else if request.mode == Mode::Window {
                window(&app, backend, request, hidden, settings, done)
            } else if needs_still(&request) {
                choose(&app, backend, request, hidden, settings, done)
            } else {
                grab(&app, backend, request, hidden, settings, done)
            }
        });
    });
}

/// A window picked in GNOME's list: one frame of it, through screen sharing (no flash). Where the
/// system lists the windows itself (Windows), it is picked on the overlay instead.
fn window(app: &adw::Application, backend: Rc<dyn CaptureBackend>, request: Request, hidden: Vec<gtk::Window>, settings: Settings, done: Done) {
    if let Some(windows) = backend.windows() {
        return pick_window(app, backend, windows, request, hidden, settings, done);
    }
    let app = app.clone();
    backend.screencast(
        CastRequest { pointer: false, restore_token: None, windows: true },
        Box::new(move |result| match result {
            Ok(cast) => {
                let cast = Rc::new(cast);
                let c2 = cast.clone();
                cast.grab_frame(0, move |frame| {
                    c2.close();
                    match frame.and_then(|png| decode_png(&png).map_err(|e| CaptureError::Failed(e.to_string()))) {
                        Ok(picture) => deliver(&app, picture, &request, hidden, &settings, done),
                        Err(e) => fail(&app, &request, &hidden, e, done),
                    }
                });
            }
            Err(e) => fail(&app, &request, &hidden, e, done),
        }),
    );
}

/// A window picked on the overlay (Windows): a still picture of the screen, the windows on it lit
/// up under the pointer, and the one clicked cut from the picture as it is on the screen (as the
/// Snipping Tool does).
fn pick_window(app: &adw::Application, backend: Rc<dyn CaptureBackend>, windows: Vec<obtura_capture::Place>, request: Request, hidden: Vec<gtk::Window>, settings: Settings, done: Done) {
    let app = app.clone();
    backend.capture(
        CaptureRequest { kind: CaptureKind::FullScreen },
        Box::new(move |result| {
            let picture = match result.and_then(|shot| decode_png(&shot.png).map_err(|e| CaptureError::Failed(e.to_string()))) {
                Ok(p) => Rc::new(p),
                Err(e) => return fail(&app, &request, &hidden, e, done),
            };
            let windows: Vec<obtura_capture::Place> = windows.into_iter().filter_map(|p| p.rect.clamp_to(picture.width, picture.height).map(|rect| obtura_capture::Place { rect, ..p })).collect();
            if windows.is_empty() {
                return fail(&app, &request, &hidden, CaptureError::Failed("There is no window to capture: open the window first, then choose A Window again.".into()), done);
            }
            let said = windows.iter().map(|p| (p.rect, window_said(&p.name, p.rect))).collect();
            let slot = Rc::new(RefCell::new(Some((request, hidden, settings, done))));
            let (app2, picture2, slot2) = (app.clone(), picture.clone(), slot.clone());
            let shown = overlay::pick_window(
                &picture,
                said,
                Box::new(move |chosen| {
                    let Some((request, hidden, settings, done)) = slot2.borrow_mut().take() else { return };
                    match chosen.and_then(|j| windows.get(j)).and_then(|p| picture2.crop(p.rect)) {
                        Some(cropped) => deliver(&app2, cropped, &request, hidden, &settings, done),
                        None => fail(&app2, &request, &hidden, CaptureError::Cancelled, done),
                    }
                }),
            );
            if let Err(why) = shown {
                if let Some((request, hidden, _, done)) = slot.borrow_mut().take() {
                    fail(&app, &request, &hidden, CaptureError::Failed(why), done);
                }
            }
        }),
    );
}

/// What the overlay says about a window: its title (shortened) and size.
fn window_said(title: &str, r: obtura_image::Rect) -> String {
    let title = title.trim();
    let short: String = if title.chars().count() > 60 { format!("{}…", title.chars().take(59).collect::<String>()) } else { title.to_string() };
    format!("{short}   {} × {}", r.w, r.h)
}

/// The region chosen last time, in pixels of the screen.
fn last_region() -> Option<obtura_image::Rect> {
    obtura_core::last_region::load().map(|r| obtura_image::Rect::new(r.x, r.y, r.w, r.h))
}

fn remember(r: obtura_image::Rect) {
    let _ = obtura_core::last_region::save(obtura_core::last_region::Region { x: r.x, y: r.y, w: r.w, h: r.h });
}

/// A still picture of the whole desktop (physical pixels) and, when it was taken through screen
/// sharing, the parts of the desktop it holds (desktop logical pixels).
pub struct Still {
    pub picture: Bitmap,
    pub shared: Option<Vec<(f64, f64, f64, f64)>>,
}

/// Takes the still picture, then shows the overlay (region, or the colour picker) over it.
fn choose(app: &adw::Application, backend: Rc<dyn CaptureBackend>, request: Request, hidden: Vec<gtk::Window>, settings: Settings, done: Done) {
    let app = app.clone();
    still_picture(
        backend.clone(),
        settings.quiet_capture,
        Box::new(move |result| match result {
            Ok(still) if request.output == Output::Color => pick(&app, still.picture, hidden, done),
            Ok(still) if request.output == Output::Measure => measure(&app, still.picture, hidden, done),
            Ok(still) => region(&app, backend, still, request, hidden, settings, done),
            Err(e) if asks_permission(&e) => permission(&app, backend, request, hidden, e, done),
            Err(e) => fail(&app, &request, &hidden, e, done),
        }),
    );
}

pub type StillDone = Box<dyn FnOnce(Result<Still, CaptureError>)>;

/// A still picture of the screen. `quiet`: through screen sharing (no flash; GNOME may ask which
/// screens the first time). If sharing is not possible, through GNOME's screenshot service.
pub fn still_picture(backend: Rc<dyn CaptureBackend>, quiet: bool, done: StillDone) {
    // Without a flash anyway (Windows): no screen sharing needed.
    if !quiet || backend.capture_is_quiet() {
        return flash_picture(backend, done);
    }
    let token = obtura_core::screen_share::load();
    let b2 = backend.clone();
    backend.screencast(
        CastRequest { pointer: false, restore_token: token, windows: false },
        Box::new(move |result| match result {
            Ok(cast) => {
                if let Some(t) = &cast.restore_token {
                    let _ = obtura_core::screen_share::save(t);
                }
                let cast = Rc::new(cast);
                let c2 = cast.clone();
                grab_frames(cast, move |frames| {
                    c2.close();
                    match frames {
                        Ok(still) => done(Ok(still)),
                        Err(e) => {
                            obtura_core::log::warn(&format!("{e}; taking the screenshot through GNOME's screenshot service"));
                            flash_picture(b2, done)
                        }
                    }
                });
            }
            Err(CaptureError::Cancelled) => done(Err(CaptureError::Cancelled)),
            Err(e) => {
                obtura_core::log::warn(&format!("screen sharing: {e}; taking the screenshot through GNOME's screenshot service"));
                flash_picture(b2, done)
            }
        }),
    );
}

/// GNOME's screenshot service: the whole desktop (GNOME flashes).
fn flash_picture(backend: Rc<dyn CaptureBackend>, done: StillDone) {
    backend.capture(
        CaptureRequest { kind: CaptureKind::FullScreen },
        Box::new(move |result| done(result.and_then(|shot| decode_png(&shot.png).map_err(|e| CaptureError::Failed(e.to_string()))).map(|picture| Still { picture, shared: None }))),
    );
}

/// One still frame of every shared screen, put together as the desktop.
pub fn grab_frames(cast: Rc<obtura_capture::Cast>, done: impl FnOnce(Result<Still, CaptureError>) + 'static) {
    let n = cast.streams.len();
    let frames: Rc<RefCell<Vec<Option<Result<Vec<u8>, CaptureError>>>>> = Rc::new(RefCell::new(vec![None; n]));
    let done: Rc<RefCell<Option<Box<dyn FnOnce(Result<Still, CaptureError>)>>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    for i in 0..n {
        let (frames, done, cast2) = (frames.clone(), done.clone(), cast.clone());
        cast.grab_frame(i, move |result| {
            frames.borrow_mut()[i] = Some(result);
            if frames.borrow().iter().any(|f| f.is_none()) {
                return;
            }
            let Some(done) = done.borrow_mut().take() else { return };
            let mut pictures = Vec::new();
            for (stream, f) in cast2.streams.iter().zip(frames.borrow_mut().drain(..)) {
                match f.expect("every frame came").and_then(|png| decode_png(&png).map_err(|e| CaptureError::Failed(e.to_string()))) {
                    Ok(b) => pictures.push((stream.clone(), b)),
                    Err(e) => return done(Err(e)),
                }
            }
            match overlay::desktop() {
                Ok(desktop) => done(Ok(compose(&pictures, &desktop))),
                Err(e) => done(Err(CaptureError::Failed(e))),
            }
        });
    }
}

/// The desktop as one picture: each shared screen's frame at its place (physical pixels, at the
/// largest screen scale), screens not shared left dark.
pub fn compose(frames: &[(obtura_capture::Stream, Bitmap)], desktop: &overlay::Desktop) -> Still {
    let scale = desktop.screens.iter().map(|s| s.scale).fold(1.0, f64::max);
    let place = |s: &obtura_capture::Stream, b: &Bitmap| {
        let (x, y) = s.position.map(|(x, y)| ((x - desktop.origin.0) as f64, (y - desktop.origin.1) as f64)).unwrap_or((0.0, 0.0));
        let (w, h) = s.size.map(|(w, h)| (w as f64, h as f64)).unwrap_or((b.width as f64 / scale, b.height as f64 / scale));
        (x, y, w, h)
    };
    let rects: Vec<(f64, f64, f64, f64)> = frames.iter().map(|(s, b)| place(s, b)).collect();
    let (mut dw, mut dh) = desktop.screens.iter().fold((0.0f64, 0.0f64), |(w, h), s| (w.max(s.x + s.w), h.max(s.y + s.h)));
    for r in &rects {
        dw = dw.max(r.0 + r.2);
        dh = dh.max(r.1 + r.3);
    }
    let mut picture = Bitmap::filled((dw * scale).round().max(1.0) as u32, (dh * scale).round().max(1.0) as u32, [32, 32, 32, 255]);
    for ((_, b), r) in frames.iter().zip(&rects) {
        obtura_image::paste(&mut picture, b, (r.0 * scale).round() as i64, (r.1 * scale).round() as i64);
    }
    Still { picture, shared: Some(rects) }
}

fn grab(app: &adw::Application, backend: Rc<dyn CaptureBackend>, request: Request, hidden: Vec<gtk::Window>, settings: Settings, done: Done) {
    let app = app.clone();
    let b2 = backend.clone();
    backend.capture(
        CaptureRequest { kind: CaptureKind::FullScreen },
        Box::new(move |result| {
            let decoded = result.and_then(|shot| decode_png(&shot.png).map_err(|e| CaptureError::Failed(e.to_string())));
            match decoded {
                // Several screens: the one remembered, or the user says which (or all).
                Ok(full) if request.mode == Mode::Screen => crate::screens::take(&app, b2, full, request, hidden, settings, done),
                Ok(full) if request.mode == Mode::LastRegion => match last_region().and_then(|r| full.crop(r)) {
                    Some(cropped) => deliver(&app, cropped, &request, hidden, &settings, done),
                    // The last region is no longer on the screen: choose one on this picture.
                    None => region(&app, b2, Still { picture: full, shared: None }, request, hidden, settings, done),
                },
                Ok(full) => deliver(&app, full, &request, hidden, &settings, done),
                Err(e) if asks_permission(&e) => permission(&app, b2, request, hidden, e, done),
                Err(e) => fail(&app, &request, &hidden, e, done),
            }
        }),
    );
}

/// The colour picker on the frozen screen: the colour clicked goes to the clipboard.
fn pick(app: &adw::Application, full: Bitmap, hidden: Vec<gtk::Window>, done: Done) {
    let shot = Rc::new(full);
    let done = Rc::new(RefCell::new(Some(done)));
    let (app2, shot2, hidden2, done2) = (app.clone(), shot.clone(), hidden.clone(), done.clone());
    let shown = overlay::pick_color(
        shot,
        Box::new(move |area| {
            let Some(done) = done2.borrow_mut().take() else { return };
            restore(&hidden2);
            match area.and_then(|a| a.in_pixels(shot2.width, shot2.height)) {
                Some(r) => {
                    let c = shot2.pixel(r.x as u32, r.y as u32);
                    let hex = obtura_core::color::format([c[0], c[1], c[2]], Settings::load().color_format);
                    if let Some(display) = gdk::Display::default() {
                        display.clipboard().set_text(&hex);
                    }
                    keep_running(&app2);
                    notify(&app2, "Colour copied", &format!("{hex} is on the clipboard"));
                    done(Outcome::Printed(hex));
                }
                None => done(Outcome::Cancelled),
            }
        }),
    );
    if let Err(why) = shown {
        if let Some(done) = done.borrow_mut().take() {
            restore(&hidden);
            notify(app, "Could not pick a colour", &why);
            done(Outcome::Failed(why));
        }
    }
}

/// Measuring on the frozen screen: Enter copies the last measurement.
fn measure(app: &adw::Application, full: Bitmap, hidden: Vec<gtk::Window>, done: Done) {
    let (w, h) = (full.width, full.height);
    let done = Rc::new(RefCell::new(Some(done)));
    let (app2, hidden2, done2) = (app.clone(), hidden.clone(), done.clone());
    let shown = overlay::measure(
        &full,
        Box::new(move |area| {
            let Some(done) = done2.borrow_mut().take() else { return };
            restore(&hidden2);
            match area {
                Some(a) => {
                    let ruler = overlay::Ruler { a: (a.x, a.y), b: (a.x + a.w, a.y + a.h) };
                    let said = ruler.said(w as f64 / a.desktop.0, h as f64 / a.desktop.1);
                    if let Some(display) = gdk::Display::default() {
                        display.clipboard().set_text(&said);
                    }
                    keep_running(&app2);
                    notify(&app2, "Measurement copied", &said);
                    done(Outcome::Printed(said));
                }
                None => done(Outcome::Cancelled),
            }
        }),
    );
    if let Err(why) = shown {
        if let Some(done) = done.borrow_mut().take() {
            restore(&hidden);
            notify(app, "Could not measure", &why);
            done(Outcome::Failed(why));
        }
    }
}

fn region(app: &adw::Application, backend: Rc<dyn CaptureBackend>, still: Still, request: Request, hidden: Vec<gtk::Window>, settings: Settings, done: Done) {
    let shot = Rc::new(still.picture);
    // The overlay's callback, "share other screens" and "could not show it" share `done`; one runs.
    let done = Rc::new(RefCell::new(Some(done)));
    let (app2, shot2, hidden2, request2, done2, settings2) = (app.clone(), shot.clone(), hidden.clone(), request.clone(), done.clone(), settings.clone());
    let share_again: Option<Box<dyn FnOnce()>> = still.shared.is_some().then(|| {
        let (app, request, hidden, done, settings) = (app.clone(), request.clone(), hidden.clone(), done.clone(), settings.clone());
        Box::new(move || {
            // GNOME asks again which screens to share; then the same capture goes on.
            let _ = obtura_core::screen_share::forget();
            if let Some(done) = done.borrow_mut().take() {
                choose(&app, backend, request, hidden, settings, done);
            }
        }) as Box<dyn FnOnce()>
    });
    let shown = overlay::select_region(
        &shot,
        overlay::Purpose::Capture,
        last_region(),
        0,
        still.shared,
        share_again,
        Box::new(move |area| {
            let Some(done) = done2.borrow_mut().take() else { return };
            match area.and_then(|a| a.in_pixels(shot2.width, shot2.height)).and_then(|r| shot2.crop(r).map(|c| (r, c))) {
                Some((r, cropped)) => {
                    // Remembered for "Same Region Again", and proposed next time.
                    remember(r);
                    deliver(&app2, cropped, &request2, hidden2, &settings2, done)
                }
                None => fail(&app2, &request2, &hidden2, CaptureError::Cancelled, done), // back to where the user was
            }
        }),
    );
    if let Err(why) = shown {
        if let Some(done) = done.borrow_mut().take() {
            fail(app, &request, &hidden, CaptureError::Failed(why), done);
        }
    }
}

pub(crate) fn deliver(app: &adw::Application, image: Bitmap, request: &Request, hidden: Vec<gtk::Window>, settings: &Settings, done: Done) {
    match &request.output {
        Output::Editor => {
            // The editor's New takes this kind of capture again (remembered before the editor
            // opens, so its New shows it).
            if let Some(kind) = new_kind(request) {
                let mut s = Settings::load();
                if s.new_capture != kind {
                    s.new_capture = kind.to_string();
                    let _ = s.save();
                }
            }
            // On the clipboard before anything else: ready to paste while the editor opens.
            if settings.copy_after_capture {
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_texture(&crate::modules::texture_of(&image));
                }
            }
            // The start window was only the launcher: it goes. An editor that this capture
            // replaces goes too, once the new one is up. Any other window comes back.
            let mut replaced = Vec::new();
            for w in hidden {
                if w.widget_name() == crate::home::NAME {
                    w.close();
                } else if crate::window::is_replaced(&w) {
                    replaced.push(w);
                } else {
                    reveal(&w);
                    w.present();
                }
            }
            let editor = Window::open(app, image.clone());
            editor.present();
            let weak = Rc::downgrade(&editor);
            crate::library::record(&image, settings, move |id| {
                if let Some(w) = weak.upgrade() {
                    w.library_id.replace(Some(id));
                }
            });
            for w in replaced {
                crate::window::retire(&w);
            }
            done(Outcome::Done);
        }
        Output::Direct { copy, save } => {
            restore(&hidden);
            // `--save` alone ends Obtura once the file is written; the capture must be in the
            // library by then (it is written in the background). Its text is read when the
            // library window next opens.
            let hold = app.hold();
            crate::library::record(&image, settings, move |_| drop(hold));
            let outcome = output_directly(app, &image, *copy, save.as_deref(), settings);
            done(outcome);
        }
        Output::Pin => {
            restore(&hidden);
            crate::library::record(&image, settings, |_| {});
            crate::pin::open(app, &image);
            done(Outcome::Done);
        }
        Output::Text => {
            restore(&hidden);
            let app = app.clone();
            crate::inspect::read_text(image, settings.ocr_languages.clone(), move |result| match result {
                Ok(words) => {
                    let text = obtura_core::ocr::text(&words);
                    if text.is_empty() {
                        notify(&app, "No text found", "There is no text Obtura can read in that part of the screen.");
                        done(Outcome::Failed("no text was found".into()));
                    } else {
                        if let Some(display) = gdk::Display::default() {
                            display.clipboard().set_text(&text);
                        }
                        keep_running(&app);
                        notify(&app, &format!("Copied {} of text", crate::inspect::lines_said(&text)), &crate::inspect::preview(&text));
                        done(Outcome::Printed(text));
                    }
                }
                Err(e) => {
                    notify(&app, "Could not read the text", &e.to_string());
                    done(Outcome::Failed(e.to_string()));
                }
            });
        }
        Output::Qr => {
            restore(&hidden);
            let app = app.clone();
            crate::inspect::read_qr(image, move |codes| {
                if codes.is_empty() {
                    notify(&app, "No QR code found", "There is no QR code Obtura can read in that part of the screen.");
                    return done(Outcome::Failed("no QR code was found".into()));
                }
                let text = codes.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n");
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_text(&text);
                }
                keep_running(&app);
                let title = if codes.len() == 1 { "QR code copied".to_string() } else { format!("{} QR codes copied", codes.len()) };
                let n = gio::Notification::new(&title);
                n.set_body(Some(&crate::inspect::preview(&text)));
                // A link can be opened from the notification (after reading it there).
                if let Some(link) = codes.iter().find(|c| c.is_link()) {
                    n.add_button_with_target_value("Open Link", "app.open-link", Some(&link.text.trim().to_variant()));
                }
                app.send_notification(Some("obtura-capture"), &n);
                done(Outcome::Printed(text));
            });
        }
        Output::Color | Output::Measure => done(Outcome::Failed("this works on the whole screen".into())),
    }
}

/// The kind of capture the editor's New takes again after `request` (one of
/// `modules::NEW_KINDS`), for captures that open the editor.
pub fn new_kind(request: &Request) -> Option<&'static str> {
    if request.output != Output::Editor {
        return None;
    }
    Some(match (request.mode, request.delay) {
        (Mode::Region, 3) => "region-3",
        (Mode::Region, 5) => "region-5",
        (Mode::Region, 10) => "region-10",
        (Mode::Region, _) => "region",
        (Mode::LastRegion, _) => "last-region",
        (Mode::Screen, 5) => "screen-5",
        (Mode::Screen, _) => "screen",
        (Mode::Window, _) => "window",
        (Mode::Scrolling, _) => "scrolling",
    })
}

fn output_directly(app: &adw::Application, image: &Bitmap, copy: bool, save: Option<&std::path::Path>, settings: &Settings) -> Outcome {
    let mut said = Vec::new();
    if let Some(path) = save {
        let path = if path.as_os_str().is_empty() {
            unique_path(&settings.save_dir(), &format_name(&settings.name_template, &now()), "png")
        } else {
            path.to_path_buf()
        };
        let written = encode_png(image).map_err(|e| e.to_string()).and_then(|bytes| {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))
        });
        match written {
            Ok(()) => said.push(format!("Saved {}", path.display())),
            Err(e) => {
                notify(app, "Could not save the picture", &e);
                return Outcome::Failed(e);
            }
        }
    }
    if copy {
        match gdk::Display::default() {
            Some(display) => {
                display.clipboard().set_texture(&crate::modules::texture_of(image));
                said.push("Copied to the clipboard".into());
            }
            None => return Outcome::Failed("no display to copy to".into()),
        }
    }
    notify(app, "Obtura", &said.join(". "));
    Outcome::Done
}

pub(crate) fn notify(app: &adw::Application, title: &str, body: &str) {
    let n = gio::Notification::new(title);
    n.set_body(Some(body));
    app.send_notification(Some("obtura-capture"), &n);
}

pub fn now() -> LocalTime {
    let t = glib::DateTime::now_local().unwrap_or_else(|_| glib::DateTime::now_utc().expect("clock"));
    LocalTime { year: t.year(), month: t.month() as u32, day: t.day_of_month() as u32, hour: t.hour() as u32, minute: t.minute() as u32, second: t.second() as u32 }
}

pub(crate) fn restore(hidden: &[gtk::Window]) {
    for w in hidden {
        crate::window::keep(w);
        reveal(w);
        w.present();
    }
}

thread_local! {
    /// Windows [`conceal`]ed that had a see-through look of their own (a faded pin), and it.
    static OWN_OPACITY: RefCell<Vec<(glib::WeakRef<gtk::Window>, f64)>> = const { RefCell::new(Vec::new()) };
}

/// Takes one of Obtura's windows out of the picture without moving it: it stays where it is,
/// see-through, and takes no clicks. (A hidden window would come back wherever GNOME puts it.)
pub fn conceal(w: &gtk::Window) {
    let own = w.opacity();
    if own > 0.0 && own < 1.0 {
        OWN_OPACITY.with(|v| {
            let mut v = v.borrow_mut();
            v.retain(|(x, _)| x.upgrade().is_some_and(|x| &x != w));
            v.push((w.downgrade(), own));
        });
    }
    w.set_opacity(0.0);
    w.set_can_target(false);
}

/// Shows a window [`conceal`]ed (or hidden) again, as it looked before.
pub fn reveal(w: &gtk::Window) {
    let own = OWN_OPACITY.with(|v| {
        let mut v = v.borrow_mut();
        let own = v.iter().find(|(x, _)| x.upgrade().as_ref() == Some(w)).map(|(_, o)| *o);
        v.retain(|(x, _)| x.upgrade().is_some_and(|x| &x != w));
        own
    });
    w.set_opacity(own.unwrap_or(1.0));
    w.set_can_target(true);
    w.set_visible(true);
}

/// Cancelling is not an error. Anything else is explained to the user: in a dialog when
/// the capture was started from a window, as a notification from the command line.
pub(crate) fn fail(app: &adw::Application, request: &Request, hidden: &[gtk::Window], error: CaptureError, done: Done) {
    restore(hidden);
    if error == CaptureError::Cancelled {
        return done(Outcome::Cancelled);
    }
    let text = error.to_string();
    obtura_core::log::error(&format!("capture failed: {text}"));
    match request.output {
        Output::Editor => {
            let parent = hidden.first().cloned().or_else(|| app.active_window()).unwrap_or_else(|| crate::home::open(app).upcast());
            let dialog = adw::AlertDialog::new(Some("Could not capture the screen"), Some(&text));
            dialog.add_response("ok", "OK");
            dialog.present(Some(&parent));
        }
        _ => notify(app, "Could not capture the screen", &text),
    }
    done(Outcome::Failed(text));
}
