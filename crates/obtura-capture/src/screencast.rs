//! Screen sharing for recordings: the `org.freedesktop.portal.ScreenCast` portal, called
//! directly over the session bus with GIO (no `ashpd`), and a fake one for tests.
//!
//! A [`Cast`] is one sharing session: GNOME asks the user which screens Obtura may record the
//! first time (and again when the screens change); after that a "restore token" makes it
//! silent. While a session is open GNOME shows its screen-sharing icon in the top bar; a click
//! on it stops the session, which Obtura hears through [`Cast::on_closed`].
//!
//! The frames themselves come through PipeWire. [`Cast::video_source`] gives the
//! `gst-launch-1.0` source for one stream and the descriptor it reads from; the fake session
//! gives a moving test picture instead, and test tones instead of the microphone, so tests
//! never record the real screen or sound.

use crate::CaptureError;
use gio::glib::{self, variant::ObjectPath, variant::ToVariant, Variant, VariantDict};
use gio::prelude::*;
use std::{cell::RefCell, os::fd::OwnedFd, rc::Rc};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";

const SOURCE_MONITOR: u32 = 1;
const SOURCE_WINDOW: u32 = 2;
const CURSOR_HIDDEN: u32 = 1;
const CURSOR_EMBEDDED: u32 = 2;
/// Remember the choice until the user revokes it.
const PERSIST_UNTIL_REVOKED: u32 = 2;
/// Ask every time.
const PERSIST_NONE: u32 = 0;

/// What to ask for.
#[derive(Clone, Debug, Default)]
pub struct CastRequest {
    /// Draw the mouse pointer into the frames.
    pub pointer: bool,
    /// The token from the last session, so GNOME does not ask again.
    pub restore_token: Option<String>,
    /// One window instead of screens: GNOME lists the open windows and the user picks one,
    /// every time (a window's choice is not remembered).
    pub windows: bool,
}

/// One shared screen. Position and size are in desktop (logical) pixels, as the compositor
/// lays out the monitors; the frames themselves are in physical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub node: u32,
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
}

/// Where sound comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// The default microphone.
    Microphone,
    /// What the computer plays (the default output's monitor).
    Computer,
}

pub type CastDone = Box<dyn FnOnce(Result<Cast, CaptureError>) + 'static>;

/// An open screen-sharing session.
pub struct Cast {
    pub streams: Vec<Stream>,
    /// Tests only: a tall page the fake screen scrolls through (see `scroll`).
    pub fake_page: Option<std::sync::Arc<obtura_image::Bitmap>>,
    /// Give this to the next [`CastRequest`] so GNOME does not ask again.
    pub restore_token: Option<String>,
    inner: Inner,
    on_closed: Rc<RefCell<Vec<Box<dyn Fn()>>>>,
    closed_sub: RefCell<Option<gio::SignalSubscription>>,
}

enum Inner {
    Portal { conn: gio::DBusConnection, session: String },
    /// A test picture of each size (moving, for recordings) and, for still frames, a fixed
    /// picture.
    Fake { sizes: Vec<(u32, u32)>, picture: Option<Vec<u8>> },
}

impl Cast {
    /// A session over a moving test picture of `width` x `height`, at (0, 0). Its still frame
    /// ([`Cast::grab_frame`]) is `picture` (a PNG) if given.
    pub fn fake(width: u32, height: u32, picture: Option<Vec<u8>>) -> Cast {
        Cast {
            streams: vec![Stream { node: 0, position: Some((0, 0)), size: Some((width as i32, height as i32)) }],
            restore_token: None,
            fake_page: None,
            inner: Inner::Fake { sizes: vec![(width, height)], picture },
            on_closed: Rc::default(),
            closed_sub: RefCell::new(None),
        }
    }

    /// A session over several test screens: (width, height, x, y) each, at scale 1.
    pub fn fake_screens(screens: &[(u32, u32, i32, i32)]) -> Cast {
        Cast {
            streams: screens.iter().enumerate().map(|(i, (w, h, x, y))| Stream { node: i as u32, position: Some((*x, *y)), size: Some((*w as i32, *h as i32)) }).collect(),
            restore_token: None,
            fake_page: None,
            inner: Inner::Fake { sizes: screens.iter().map(|(w, h, _, _)| (*w, *h)).collect(), picture: None },
            on_closed: Rc::default(),
            closed_sub: RefCell::new(None),
        }
    }

    /// One still frame of stream `index`, as a PNG in physical pixels (no flash: this is how
    /// screenshots are taken without GNOME's flash). `done` is called once.
    pub fn grab_frame(&self, index: usize, done: impl FnOnce(Result<Vec<u8>, CaptureError>) + 'static) {
        let stream = match self.streams.get(index) {
            Some(s) => s.clone(),
            None => return done(Err(CaptureError::Failed("no such screen".into()))),
        };
        let fd = match &self.inner {
            Inner::Fake { picture: Some(png), .. } => return done(Ok(png.clone())),
            Inner::Fake { .. } => None,
            Inner::Portal { conn, session } => match open_pipewire_remote(conn, session) {
                Ok(fd) => Some(fd),
                Err(e) => return done(Err(e)),
            },
        };
        let source: Vec<String> = match (&self.inner, fd.is_some()) {
            (Inner::Fake { sizes, .. }, _) => {
                let (width, height) = sizes.get(index).copied().unwrap_or((320, 200));
                vec!["videotestsrc".into(), "num-buffers=1".into(), "!".into(), format!("video/x-raw,width={width},height={height}")]
            }
            _ => vec!["pipewiresrc".into(), "fd=3".into(), format!("path={}", stream.node), "num-buffers=1".into()],
        };
        crate::record::grab_png(source, fd, done);
    }

    /// Tests only: the fake screen shows `page` (a tall picture), scrolled to the top: its still
    /// frame is that view, and a scrolling capture scrolls down through it (see `scroll`).
    pub fn with_page(mut self, page: obtura_image::Bitmap) -> Cast {
        if let (Inner::Fake { sizes, picture }, Some((w, h))) = (&mut self.inner, self.streams.first().and_then(|s| s.size)) {
            let view = crate::scroll::fake_view(&page, w as u32, h as u32, 0);
            *sizes = vec![(w as u32, h as u32)];
            *picture = obtura_image::encode_png(&view).ok();
        }
        self.fake_page = Some(std::sync::Arc::new(page));
        self
    }

    pub fn is_fake(&self) -> bool {
        matches!(self.inner, Inner::Fake { .. })
    }

    /// The `gst-launch-1.0` source of stream `index`, ending in raw video, and the descriptor
    /// it reads (to be given to the program as descriptor 3).
    pub fn video_source(&self, index: usize) -> Result<(Vec<String>, Option<OwnedFd>), CaptureError> {
        let (mut sources, fd) = self.video_sources(&[index])?;
        Ok((sources.remove(0), fd))
    }

    /// The sources of several streams at once (to record several screens into one picture), all
    /// reading the one descriptor given (as descriptor 3).
    pub fn video_sources(&self, indices: &[usize]) -> Result<(Vec<Vec<String>>, Option<OwnedFd>), CaptureError> {
        let streams: Vec<&Stream> = indices.iter().map(|i| self.streams.get(*i).ok_or_else(|| CaptureError::Failed("no such screen".into()))).collect::<Result<_, _>>()?;
        match &self.inner {
            Inner::Fake { sizes, .. } => Ok((
                indices
                    .iter()
                    .map(|i| {
                        let (width, height) = sizes.get(*i).copied().unwrap_or((320, 200));
                        let pattern = ["ball", "smpte", "snow", "pinwheel"][i % 4];
                        vec!["videotestsrc".into(), "is-live=true".into(), format!("pattern={pattern}"), "!".into(), format!("video/x-raw,width={width},height={height},framerate=30/1")]
                    })
                    .collect(),
                None,
            )),
            Inner::Portal { conn, session } => {
                let fd = open_pipewire_remote(conn, session)?;
                Ok((
                    streams
                        .iter()
                        .map(|stream| {
                            vec![
                                "pipewiresrc".into(),
                                "fd=3".into(),
                                format!("path={}", stream.node),
                                "do-timestamp=true".into(),
                                // GNOME only sends a frame when something changes: repeat the last
                                // one every second so the recording keeps going on a still screen.
                                "keepalive-time=1000".into(),
                                "resend-last=true".into(),
                                "!".into(),
                                "video/x-raw,max-framerate=30/1".into(),
                            ]
                        })
                        .collect(),
                    Some(fd),
                ))
            }
        }
    }

    /// The `gst-launch-1.0` source of `sound`, ending in raw audio. The fake session gives a
    /// test tone: tests never open the real microphone.
    pub fn sound_source(&self, sound: Sound) -> Vec<String> {
        sound_args(sound, self.is_fake())
    }

    /// Calls `f` when the session ends from outside (the user stopped the sharing in GNOME's
    /// top bar).
    pub fn on_closed(&self, f: impl Fn() + 'static) {
        self.on_closed.borrow_mut().push(Box::new(f));
    }

    /// Ends the session (GNOME's screen-sharing icon goes away).
    pub fn close(&self) {
        self.closed_sub.borrow_mut().take();
        if let Inner::Portal { conn, session } = &self.inner {
            conn.call(Some(DEST), session, SESSION, "Close", None, None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, |_| {});
        }
    }
}

impl Drop for Cast {
    fn drop(&mut self) {
        self.close();
    }
}

/// The `gst-launch-1.0` source of `sound`, ending in raw audio: the real microphone or the
/// computer's sound, or (`fake`) test tones that never touch the real microphone.
pub fn sound_args(sound: Sound, fake: bool) -> Vec<String> {
    match (fake, sound) {
        (true, Sound::Microphone) => vec!["audiotestsrc".into(), "is-live=true".into(), "wave=sine".into(), "volume=0.2".into()],
        (true, Sound::Computer) => vec!["audiotestsrc".into(), "is-live=true".into(), "wave=ticks".into(), "volume=0.2".into()],
        (false, Sound::Microphone) => vec!["pulsesrc".into(), "client-name=Obtura".into()],
        (false, Sound::Computer) => vec!["pulsesrc".into(), "device=@DEFAULT_MONITOR@".into(), "client-name=Obtura".into()],
    }
}

fn next_token() -> u32 {
    thread_local!(static N: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });
    N.with(|n| {
        n.set(n.get() + 1);
        n.get()
    })
}

fn objpath(p: &str) -> Result<Variant, CaptureError> {
    ObjectPath::try_from(p.to_owned()).map(|p| p.to_variant()).map_err(|_| CaptureError::Failed(format!("bad session path {p}")))
}

type Answer = Box<dyn FnOnce(Result<VariantDict, CaptureError>)>;

/// Calls a portal method that answers through a Request object: subscribes to the answer
/// first (a fast answer would be missed otherwise), then calls. `args` gets the handle token
/// to put in the options.
fn request(conn: &gio::DBusConnection, method: &str, args: impl FnOnce(&str) -> Result<Variant, CaptureError>, done: Answer) {
    let sender = conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    let token = format!("obtura_cast_{}_{}", std::process::id(), next_token());
    let request_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let args = match args(&token) {
        Ok(a) => a,
        Err(e) => return done(Err(e)),
    };
    let done = Rc::new(RefCell::new(Some(done)));
    let sub: Rc<RefCell<Option<gio::SignalSubscription>>> = Rc::new(RefCell::new(None));
    let (done2, sub2) = (done.clone(), sub.clone());
    let what = method.to_string();
    let id = conn.subscribe_to_signal(Some(DEST), Some(REQUEST), Some("Response"), Some(&request_path), None, gio::DBusSignalFlags::NONE, move |signal| {
        sub2.borrow_mut().take();
        let code: u32 = signal.parameters.child_value(0).get().unwrap_or(99);
        let results = VariantDict::new(Some(&signal.parameters.child_value(1)));
        if let Some(done) = done2.borrow_mut().take() {
            done(match code {
                0 => Ok(results),
                1 => Err(CaptureError::Cancelled),
                _ => Err(CaptureError::Failed(format!("the system refused to share the screen ({what})"))),
            });
        }
    });
    *sub.borrow_mut() = Some(id);
    let (done3, sub3) = (done.clone(), sub.clone());
    conn.call(Some(DEST), PATH, SCREENCAST, method, Some(&args), None, gio::DBusCallFlags::NONE, -1, gio::Cancellable::NONE, move |reply| {
        let Err(e) = reply else { return };
        sub3.borrow_mut().take();
        if let Some(done) = done3.borrow_mut().take() {
            let denied = e.matches(gio::DBusError::AccessDenied);
            done(Err(if denied { CaptureError::Denied } else { CaptureError::Failed(e.message().to_string()) }));
        }
    });
}

fn options(pairs: Vec<(&str, Variant)>) -> Variant {
    let d = VariantDict::new(None);
    for (k, v) in pairs {
        d.insert_value(k, &v);
    }
    d.end()
}

/// Opens a sharing session: CreateSession, SelectSources (monitors, several allowed, remembered
/// until revoked), Start. Start shows GNOME's chooser unless the restore token still works.
pub(crate) fn start(request_: CastRequest, done: CastDone) {
    let conn = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(c) => c,
        Err(e) => return done(Err(CaptureError::Failed(format!("no session bus: {e}")))),
    };
    let session_token = format!("obtura_session_{}_{}", std::process::id(), next_token());
    let c2 = conn.clone();
    request(
        &conn,
        "CreateSession",
        |tok| Ok(Variant::tuple_from_iter([options(vec![("handle_token", tok.to_variant()), ("session_handle_token", session_token.to_variant())])])),
        Box::new(move |result| {
            let session = match result.map(|r| r.lookup_value("session_handle", None).and_then(|v| v.str().map(str::to_owned))) {
                Ok(Some(s)) => s,
                Ok(None) => return done(Err(CaptureError::Failed("the system opened no sharing session".into()))),
                Err(e) => return done(Err(e)),
            };
            select(c2, session, request_, done);
        }),
    );
}

fn select(conn: gio::DBusConnection, session: String, request_: CastRequest, done: CastDone) {
    let cursor = if request_.pointer { CURSOR_EMBEDDED } else { CURSOR_HIDDEN };
    let (types, multiple, persist) = if request_.windows { (SOURCE_WINDOW, false, PERSIST_NONE) } else { (SOURCE_MONITOR, true, PERSIST_UNTIL_REVOKED) };
    let (c2, s2) = (conn.clone(), session.clone());
    let path = session.clone();
    request(
        &conn,
        "SelectSources",
        move |tok| {
            let mut o = vec![
                ("handle_token", tok.to_variant()),
                ("types", types.to_variant()),
                ("multiple", multiple.to_variant()),
                ("cursor_mode", cursor.to_variant()),
                ("persist_mode", persist.to_variant()),
            ];
            if let Some(t) = request_.restore_token.as_ref().filter(|t| !t.is_empty() && !request_.windows) {
                o.push(("restore_token", t.to_variant()));
            }
            Ok(Variant::tuple_from_iter([objpath(&path)?, options(o)]))
        },
        Box::new(move |result| match result {
            Ok(_) => start_streams(c2, s2, done),
            Err(e) => {
                close_session(&c2, &s2);
                done(Err(e))
            }
        }),
    );
}

fn start_streams(conn: gio::DBusConnection, session: String, done: CastDone) {
    let (c2, s2) = (conn.clone(), session.clone());
    let path = session.clone();
    request(
        &conn,
        "Start",
        move |tok| Ok(Variant::tuple_from_iter([objpath(&path)?, "".to_variant(), options(vec![("handle_token", tok.to_variant())])])),
        Box::new(move |result| {
            let results = match result {
                Ok(r) => r,
                Err(e) => {
                    close_session(&c2, &s2);
                    return done(Err(e));
                }
            };
            let streams = results.lookup_value("streams", None).map(|v| parse_streams(&v)).unwrap_or_default();
            if streams.is_empty() {
                close_session(&c2, &s2);
                return done(Err(CaptureError::Failed("no screen was shared".into())));
            }
            let restore_token = results.lookup_value("restore_token", None).and_then(|v| v.str().map(str::to_owned));
            let cast = Cast { streams, restore_token, fake_page: None, inner: Inner::Portal { conn: c2.clone(), session: s2.clone() }, on_closed: Rc::default(), closed_sub: RefCell::new(None) };
            // The user may stop the sharing from GNOME's top bar.
            let listeners = cast.on_closed.clone();
            let sub = c2.subscribe_to_signal(Some(DEST), Some(SESSION), Some("Closed"), Some(&s2), None, gio::DBusSignalFlags::NONE, move |_| {
                for f in listeners.borrow().iter() {
                    f();
                }
            });
            *cast.closed_sub.borrow_mut() = Some(sub);
            done(Ok(cast));
        }),
    );
}

/// Streams from Start's answer: an array of (node id, properties).
fn parse_streams(v: &Variant) -> Vec<Stream> {
    (0..v.n_children())
        .map(|i| {
            let s = v.child_value(i);
            let props = VariantDict::new(Some(&s.child_value(1)));
            Stream {
                node: s.child_value(0).get().unwrap_or(0),
                position: props.lookup_value("position", None).and_then(|v| v.get()),
                size: props.lookup_value("size", None).and_then(|v| v.get()),
            }
        })
        .collect()
}

fn close_session(conn: &gio::DBusConnection, session: &str) {
    conn.call(Some(DEST), session, SESSION, "Close", None, None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE, |_| {});
}

/// The PipeWire connection of a session (a descriptor), for one recording program.
fn open_pipewire_remote(conn: &gio::DBusConnection, session: &str) -> Result<OwnedFd, CaptureError> {
    let failed = |e: String| CaptureError::Failed(format!("cannot reach the shared screen: {e}"));
    let (reply, fds) = conn
        .call_with_unix_fd_list_sync(
            Some(DEST),
            PATH,
            SCREENCAST,
            "OpenPipeWireRemote",
            Some(&Variant::tuple_from_iter([objpath(session)?, options(vec![])])),
            None,
            gio::DBusCallFlags::NONE,
            10_000,
            None::<&gio::UnixFDList>,
            gio::Cancellable::NONE,
        )
        .map_err(|e| failed(e.to_string()))?;
    let idx: glib::variant::Handle = reply.child_value(0).get().ok_or_else(|| failed("bad answer".into()))?;
    let fds = fds.ok_or_else(|| failed("no descriptor".into()))?;
    fds.get(idx.0).map_err(|e| failed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_streams_of_the_start_answer() {
        let props = options(vec![("position", (1920i32, 0i32).to_variant()), ("size", (1920i32, 1080i32).to_variant()), ("source_type", 1u32.to_variant())]);
        let one = Variant::tuple_from_iter([57u32.to_variant(), props]);
        let bare = Variant::tuple_from_iter([58u32.to_variant(), options(vec![])]);
        let arr = Variant::array_from_iter_with_type(one.type_(), [one.clone(), bare]);
        assert_eq!(
            parse_streams(&arr),
            vec![Stream { node: 57, position: Some((1920, 0)), size: Some((1920, 1080)) }, Stream { node: 58, position: None, size: None }]
        );
    }

    /// Runs `start` on a private main loop until it answers.
    fn wait_for<T: 'static>(start: impl FnOnce(Box<dyn FnOnce(T)>)) -> T {
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(|| {
            let ml = glib::MainLoop::new(Some(&ctx), false);
            let slot = Rc::new(RefCell::new(None));
            let (s, m) = (slot.clone(), ml.clone());
            start(Box::new(move |v| {
                *s.borrow_mut() = Some(v);
                m.quit();
            }));
            if slot.borrow().is_none() {
                ml.run();
            }
            slot.take().expect("an answer")
        })
        .unwrap()
    }

    #[test]
    fn a_fake_session_gives_its_still_picture() {
        let png = include_bytes!("../../../tests/fixtures/sample.png").to_vec();
        let cast = Cast::fake(800, 500, Some(png.clone()));
        let got = wait_for(|done| cast.grab_frame(0, done)).unwrap();
        assert_eq!(got, png);
    }

    #[test]
    fn a_still_frame_comes_through_gst_launch() {
        if !crate::record::tools_present() {
            return;
        }
        let cast = Cast::fake(320, 200, None);
        let got = wait_for(|done| cast.grab_frame(0, done)).unwrap();
        assert_eq!(crate::png_size(&got), Some((320, 200)));
    }

    /// On the real desktop, with the screens remembered by GNOME (no dialog; GNOME shows its
    /// sharing icon for a few seconds). Run by hand:
    /// `cargo test -p obtura-capture real_screen -- --ignored --nocapture`.
    /// The token file is `~/.local/state/obtura/screen-share-token` (or $OBTURA_TOKEN_FILE); a
    /// token works once, so the new one is written back.
    #[test]
    #[ignore]
    fn real_screen_still_frames_without_a_flash() {
        let file = std::env::var_os("OBTURA_TOKEN_FILE").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".local/state/obtura/screen-share-token"));
        let token = std::fs::read_to_string(&file).ok().map(|t| t.trim().to_string());
        let t0 = std::time::Instant::now();
        let cast = wait_for(|done| start(CastRequest { pointer: false, restore_token: token, windows: false }, done)).expect("a sharing session");
        if let Some(t) = &cast.restore_token {
            std::fs::write(&file, t).unwrap();
        }
        println!("session open after {:?}: {:?}", t0.elapsed(), cast.streams);
        for i in 0..cast.streams.len() {
            let png = wait_for(|done| cast.grab_frame(i, done)).expect("a frame");
            let bitmap = obtura_image_check(&png);
            println!("screen {i}: {:?} after {:?}, {bitmap}", crate::png_size(&png), t0.elapsed());
        }
        cast.close();
    }

    /// How much of a PNG is not black (without decoding it fully here: its size is enough to
    /// tell a real screen from a black frame).
    fn obtura_image_check(png: &[u8]) -> String {
        format!("{} kB of PNG", png.len() / 1024)
    }

    #[test]
    fn a_fake_session_never_touches_the_real_screen_or_sound() {
        let cast = Cast::fake(320, 200, None);
        let (video, fd) = cast.video_source(0).unwrap();
        assert!(fd.is_none());
        assert_eq!(video[0], "videotestsrc");
        assert!(video.iter().any(|a| a.contains("width=320,height=200")));
        for sound in [Sound::Microphone, Sound::Computer] {
            assert_eq!(cast.sound_source(sound)[0], "audiotestsrc");
        }
        assert!(cast.video_source(1).is_err());
    }
}
