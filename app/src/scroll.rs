//! Scrolling capture: a page longer than the screen in one picture.
//!
//! The user chooses the part of the screen that scrolls (on the still picture, like a region),
//! then scrolls it with the mouse wheel. Obtura takes frames of that part through screen sharing
//! (`obtura_capture::scroll`) and joins them as they come (`obtura_image::stitch`). No other
//! permission is needed: Obtura never moves the pointer or scrolls by itself.
//!
//! While it runs, Obtura's icon in the top bar shows how tall the picture is, with "Finish
//! Scrolling Capture" and "Cancel"; opening Obtura or running `obtura --scroll` again finishes
//! too. Scrolling too fast (no overlap between two frames) is said beside the icon: scrolling
//! back up a little lets it go on. The picture then goes where the capture asked (the editor,
//! the clipboard, a file...).

use crate::{
    capture::{self, Done, Outcome, Request},
    overlay, recorder,
};
use adw::prelude::*;
use gtk::{gio, glib};
use obtura_capture::{scroll::Frames, CaptureBackend, CaptureError, CastRequest};
use obtura_core::{screen_share, settings::Settings};
use obtura_image::{
    decode_png,
    stitch::{Pushed, Stitcher},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

/// A scrolling capture stops by itself after this long.
const LONGEST: Duration = Duration::from_secs(300);

struct Run {
    app: adw::Application,
    cast: Rc<obtura_capture::Cast>,
    frames: RefCell<Option<Frames>>,
    dir: PathBuf,
    stitcher: RefCell<Stitcher>,
    /// The next frame number to read.
    next: Cell<u32>,
    started: Instant,
    /// Finishing (`Some(true)`) or cancelling (`Some(false)`) once the frames have stopped.
    ending: Cell<Option<bool>>,
    stopped: Cell<bool>,
    /// Frames in a row that did not overlap the last one joined.
    lost: Cell<u32>,
    /// What happened to the frames (for the log): joined, unchanged, not overlapping.
    counts: Cell<(u32, u32, u32)>,
    request: Request,
    settings: Settings,
    hidden: Vec<gtk::Window>,
    done: RefCell<Option<Done>>,
    tray: bool,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Run>>> = const { RefCell::new(None) };
}

fn current() -> Option<Rc<Run>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Whether a scrolling capture is under way.
pub fn is_running() -> bool {
    current().is_some()
}

/// Finishes the scrolling capture under way (its picture goes where it was asked).
pub fn finish() {
    if let Some(r) = current() {
        r.end(true);
    }
}

/// Starts a scrolling capture (Obtura's windows are already out of the way): screen sharing,
/// the overlay to choose what scrolls, then the frames.
pub fn start(app: &adw::Application, backend: Rc<dyn CaptureBackend>, request: Request, hidden: Vec<gtk::Window>, settings: Settings, done: Done) {
    if is_running() {
        finish();
        capture::restore(&hidden);
        return done(Outcome::Done);
    }
    // The user scrolls other windows for minutes: a see-through Obtura window over them would
    // catch the wheel, the clicks and the keys (the editor's keys draw and delete). Hidden,
    // they come back at the end (an editor this capture replaces closes instead).
    for w in &hidden {
        w.set_visible(false);
    }
    // (On Windows Obtura reads the frames from the screen itself.)
    if !obtura_capture::record::tools_present() && backend.shares_real_screen() && !backend.capture_is_quiet() {
        return capture::fail(app, &request, &hidden, CaptureError::Failed("A scrolling capture needs GStreamer's tools. Install them with:\nsudo apt install gstreamer1.0-tools gstreamer1.0-pipewire gstreamer1.0-plugins-good".into()), done);
    }
    let app = app.clone();
    backend.screencast(
        CastRequest { pointer: false, restore_token: screen_share::load(), windows: false },
        Box::new(move |result| match result {
            Ok(cast) => {
                if let Some(t) = &cast.restore_token {
                    let _ = screen_share::save(t);
                }
                let cast = Rc::new(cast);
                let c2 = cast.clone();
                capture::grab_frames(cast, move |still| match still {
                    Ok(still) => choose(&app, c2, still, request, hidden, settings, done),
                    Err(e) => {
                        c2.close();
                        capture::fail(&app, &request, &hidden, e, done)
                    }
                });
            }
            Err(e) => capture::fail(&app, &request, &hidden, e, done),
        }),
    );
}

/// The overlay: the part that scrolls.
fn choose(app: &adw::Application, cast: Rc<obtura_capture::Cast>, still: capture::Still, request: Request, hidden: Vec<gtk::Window>, settings: Settings, done: Done) {
    let desktop = match overlay::desktop() {
        Ok(d) => d,
        Err(e) => {
            cast.close();
            return capture::fail(app, &request, &hidden, CaptureError::Failed(e), done);
        }
    };
    let streams = recorder::stream_rects(&cast, &desktop);
    // Windows: the one shared "screen" is the whole desktop, read from the screen like the
    // still picture, so the part chosen is cut by the picture's own pixels (exact at any
    // screen scale, whatever scale GTK reports).
    #[cfg(windows)]
    let exact = cast.screen_area(0).map(|_| (still.picture.width, still.picture.height));
    #[cfg(not(windows))]
    let exact: Option<(u32, u32)> = None;
    let slot = Rc::new(RefCell::new(Some((cast, request, hidden, settings, done))));
    let (app2, slot2) = (app.clone(), slot.clone());
    let shown = overlay::select_region(
        &still.picture,
        overlay::Purpose::Scroll,
        None,
        0,
        still.shared,
        None,
        Box::new(move |area| {
            let Some((cast, request, hidden, settings, done)) = slot2.borrow_mut().take() else { return };
            let target = area.as_ref().and_then(|a| match exact {
                Some(frame) => a.in_pixels(frame.0, frame.1).map(|r| {
                    let crop = obtura_capture::record::Crop::of(frame, (r.x.max(0) as u32, r.y.max(0) as u32, r.w, r.h), false);
                    recorder::Target { stream: 0, crop, size: crop.size(frame) }
                }),
                None => recorder::target(&streams, (a.x, a.y, a.w, a.h), false),
            });
            match (area, target) {
                (None, _) => {
                    cast.close();
                    capture::fail(&app2, &request, &hidden, CaptureError::Cancelled, done)
                }
                (Some(_), None) => {
                    cast.close();
                    capture::fail(&app2, &request, &hidden, CaptureError::Failed("That part of the screen is not shared with Obtura.".into()), done)
                }
                // Once the overlay is off the screen.
                (Some(_), Some(t)) => {
                    let app = app2.clone();
                    glib::timeout_add_local_once(capture::AFTER_OVERLAY, move || run(&app, cast, t, request, hidden, settings, done));
                }
            }
        }),
    );
    if let Err(e) = shown {
        if let Some((cast, request, hidden, _, done)) = slot.borrow_mut().take() {
            cast.close();
            capture::fail(app, &request, &hidden, CaptureError::Failed(e), done);
        }
    }
}

fn scratch_dir() -> PathBuf {
    thread_local!(static N: Cell<u32> = const { Cell::new(0) });
    let n = N.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    obtura_core::paths::cache_dir().join(format!("scrolling-{}-{n}", std::process::id()))
}

/// Takes frames and joins them as they come, until finished.
fn run(app: &adw::Application, cast: Rc<obtura_capture::Cast>, t: recorder::Target, request: Request, hidden: Vec<gtk::Window>, settings: Settings, done: Done) {
    let dir = scratch_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        cast.close();
        return capture::fail(app, &request, &hidden, CaptureError::Failed(format!("{}: {e}", dir.display())), done);
    }
    let tray = !recorder::is_recording()
        && crate::tray::show_as(crate::tray::Mode::Scrolling, "Scroll now", |a| match a {
            crate::tray::Action::Finish => finish(),
            crate::tray::Action::Cancel => {
                if let Some(r) = current() {
                    r.end(false);
                }
            }
            _ => {}
        });
    if !tray {
        // No status icons in this top bar: say how to finish.
        let n = gio::Notification::new("Scroll slowly through what you selected");
        n.set_body(Some("To finish, open Obtura again (or run obtura --scroll again)."));
        n.set_priority(gio::NotificationPriority::Low);
        n.add_button("Finish", "app.scroll-finish");
        app.send_notification(Some("obtura-scrolling"), &n);
    }
    let r = Rc::new(Run {
        app: app.clone(),
        cast,
        frames: RefCell::new(None),
        dir: dir.clone(),
        stitcher: RefCell::new(Stitcher::new()),
        next: Cell::new(0),
        started: Instant::now(),
        ending: Cell::new(None),
        stopped: Cell::new(false),
        lost: Cell::new(0),
        counts: Cell::new((0, 0, 0)),
        request,
        settings,
        hidden,
        done: RefCell::new(Some(done)),
        tray,
    });
    CURRENT.with(|c| *c.borrow_mut() = Some(r.clone()));
    // GNOME's sharing icon stops the session: finish with what was taken.
    r.cast.on_closed(finish);
    let weak = Rc::downgrade(&r);
    let started = Frames::start(&r.cast, t.stream, t.crop, t.size, &dir, Box::new(move |result| {
        if let Some(r) = weak.upgrade() {
            if let Err(e) = result {
                obtura_core::log::error(&format!("scrolling capture: the frames stopped: {e}"));
            }
            r.stopped.set(true);
            // Stopped by itself (the sharing ended): finish with what there is.
            if r.ending.get().is_none() {
                r.ending.set(Some(true));
            }
            r.read_frames();
            r.wrap_up();
        }
    }));
    match started {
        Ok(f) => *r.frames.borrow_mut() = Some(f),
        Err(e) => {
            obtura_core::log::error(&format!("scrolling capture: {e}"));
            r.stopped.set(true);
            r.ending.set(Some(false));
            let done = r.done.borrow_mut().take();
            r.clean_up();
            if let Some(done) = done {
                capture::fail(&r.app, &r.request, &r.hidden, e, done);
            }
            return;
        }
    }
    // Join the frames as they come, and show how tall the picture is.
    let weak = Rc::downgrade(&r);
    glib::timeout_add_local(Duration::from_millis(250), move || {
        let Some(r) = weak.upgrade() else { return glib::ControlFlow::Break };
        if r.stopped.get() {
            return glib::ControlFlow::Break;
        }
        let refused = r.read_frames();
        if refused || r.started.elapsed() > LONGEST {
            r.end(true);
        }
        if r.tray && r.ending.get().is_none() {
            let h = r.stitcher.borrow().height();
            // Said once it lasts (one frame caught in the middle of a change is not enough).
            let label = if r.lost.get() >= 2 { "Too fast: scroll back up a little".to_string() } else if h == 0 { "Scroll now".into() } else { format!("{h} px") };
            crate::tray::update(&label, false);
        }
        glib::ControlFlow::Continue
    });
}

impl Run {
    /// Joins the frames waiting in the folder (and removes them): a few at a time while the
    /// frames come, so the top-bar menu stays quick; all of them once they have stopped. True
    /// when the picture is as tall as it may be.
    fn read_frames(&self) -> bool {
        let mut full = false;
        let mut budget = if self.stopped.get() { u32::MAX } else { 6 };
        while budget > 0 {
            budget -= 1;
            let path = obtura_capture::scroll::frame_path(&self.dir, self.next.get());
            if !path.exists() {
                // A frame may be written while the next one is already there (the GIF pipeline
                // numbers on): skip a missing one only if a later one exists.
                let later = obtura_capture::scroll::waiting(&self.dir).into_iter().next();
                match later.and_then(|p| p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.strip_prefix("frame-")).and_then(|n| n.parse::<u32>().ok())) {
                    Some(n) if n > self.next.get() => {
                        self.next.set(n);
                        continue;
                    }
                    _ => break,
                }
            }
            // GStreamer writes a frame under its final name: it is complete once the next one
            // has begun (or the frames have stopped).
            if !self.stopped.get() && !obtura_capture::scroll::frame_path(&self.dir, self.next.get() + 1).exists() {
                break;
            }
            let frame = std::fs::read(&path).ok().and_then(|png| decode_png(&png).ok());
            let _ = std::fs::remove_file(&path);
            self.next.set(self.next.get() + 1);
            let Some(frame) = frame else { continue };
            let (mut moved, mut same, mut lost) = self.counts.get();
            match self.stitcher.borrow_mut().push(&frame) {
                Pushed::Lost => {
                    lost += 1;
                    self.lost.set(self.lost.get() + 1);
                }
                Pushed::Refused => full = true,
                Pushed::Moved { .. } => {
                    moved += 1;
                    self.lost.set(0);
                }
                Pushed::Same => {
                    same += 1;
                    self.lost.set(0);
                }
                Pushed::First => {}
            }
            self.counts.set((moved, same, lost));
        }
        full
    }

    /// Finishing (`keep`) or cancelling: the frames stop first; then [`Run::wrap_up`].
    fn end(&self, keep: bool) {
        if self.ending.get().is_some() {
            return;
        }
        self.ending.set(Some(keep));
        if self.tray {
            crate::tray::update(if keep { "Joining…" } else { "Cancelling…" }, false);
        }
        match self.frames.borrow().as_ref() {
            Some(f) => f.stop(),
            None => self.wrap_up(),
        }
    }

    fn clean_up(&self) {
        CURRENT.with(|c| c.borrow_mut().take());
        if self.tray {
            crate::tray::hide();
        }
        self.app.withdraw_notification("obtura-scrolling");
        self.cast.close();
        // Only Obtura's own scratch folder of this capture.
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// Once the frames have stopped: the picture goes where it was asked.
    fn wrap_up(&self) {
        let Some(done) = self.done.borrow_mut().take() else { return };
        let keep = self.ending.get().unwrap_or(true);
        let picture = if keep { self.stitcher.borrow().picture() } else { None };
        self.clean_up();
        match picture {
            Some(p) => {
                let (moved, same, lost) = self.counts.get();
                obtura_core::log::info(&format!("scrolling capture: {} x {} ({moved} frames joined, {same} unchanged, {lost} not overlapping)", p.width, p.height));
                capture::deliver(&self.app, p, &self.request, self.hidden.clone(), &self.settings, done)
            }
            None if keep => capture::fail(&self.app, &self.request, &self.hidden, CaptureError::Failed("No picture came from the screen.".into()), done),
            None => {
                capture::restore(&self.hidden);
                done(Outcome::Cancelled)
            }
        }
    }
}

/// Removes what a crashed Obtura left: `scrolling-<pid>-<n>` folders of processes that are gone.
pub fn clean_up_leftovers() {
    let Ok(entries) = std::fs::read_dir(obtura_core::paths::cache_dir()) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(pid) = name.strip_prefix("scrolling-").and_then(|r| r.split('-').next()).and_then(|p| p.parse::<u32>().ok()) else { continue };
        if pid != std::process::id() && !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}
