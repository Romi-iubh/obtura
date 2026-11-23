//! Screen capture behind one trait. Everything above this crate asks a [`CaptureBackend`]
//! for a [`Shot`]; only a backend ever talks to a portal. Tests use [`FakeBackend`].
//! No GTK here.

mod fake;
mod portal;
pub mod record;
mod screencast;
pub mod scroll;
mod shot;
#[cfg(windows)]
pub mod windows;

pub use fake::FakeBackend;
pub use portal::PortalBackend;
pub use screencast::{sound_args, Cast, CastDone, CastRequest, Sound, Stream};
pub use shot::{png_size, Shot};
#[cfg(windows)]
pub use windows::WindowsBackend;

/// Why screen recording does not start on Windows.
pub const NOT_ON_WINDOWS: &str = "Recording the screen is not available in Obtura for Windows yet.";

/// What to capture. More kinds (region, window, monitor) are added as they are built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureKind {
    /// The whole desktop, every monitor, in physical pixels.
    FullScreen,
}

#[derive(Clone, Copy, Debug)]
pub struct CaptureRequest {
    pub kind: CaptureKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// The user closed the system dialog or pressed Escape.
    Cancelled,
    /// The system refused permission (for screenshots: the user said no in GNOME's question,
    /// or turned Screenshots off for Obtura in GNOME's Settings).
    Denied,
    /// GNOME asks the user once whether Obtura may take screenshots, and it can ask only while
    /// one of Obtura's windows has the focus. Capture again from a focused window.
    NeedsPermission,
    /// Anything else: the text is meant for the user.
    Failed(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Capture cancelled"),
            Self::Denied => f.write_str("Permission to capture the screen was refused"),
            Self::NeedsPermission => f.write_str("GNOME has not yet been asked to let Obtura take screenshots"),
            Self::Failed(why) => write!(f, "Capture failed: {why}"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// A screen or a window as the system names it, and where it is in a picture from
/// [`CaptureBackend::capture`] (physical pixels from the picture's top-left corner).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// A screen: the system's name for it (Windows: `\\.\DISPLAY1`). A window: its title.
    pub name: String,
    pub rect: obtura_image::Rect,
}

/// Called once with the result. Backends may call it at once (the fake one) or later,
/// from the main loop (a portal), so it is a callback and not a return value.
pub type Done = Box<dyn FnOnce(Result<Shot, CaptureError>) + 'static>;

pub trait CaptureBackend {
    fn capture(&self, request: CaptureRequest, done: Done);

    /// Opens a screen-sharing session (see [`Cast`]): for a recording, or for still frames
    /// (screenshots without GNOME's flash). The system may ask the user which screens to share.
    fn screencast(&self, request: CastRequest, done: CastDone);

    /// Whether this backend shares the real screen (the system may ask the user first). The
    /// fake backend does not.
    fn shares_real_screen(&self) -> bool {
        true
    }

    /// Whether [`capture`](Self::capture) is silent (no flash, no question), so that the still
    /// picture behind the overlay is taken with it rather than through screen sharing (Windows).
    fn capture_is_quiet(&self) -> bool {
        false
    }

    /// The screens and their exact places in a picture from [`capture`](Self::capture), when
    /// the system tells an app (Windows). `None`: the app asks GTK (GNOME), whose layout the
    /// picture follows.
    fn screens(&self) -> Option<Vec<Place>> {
        None
    }

    /// The windows on the screen now, the front one first, with their places in a picture from
    /// [`capture`](Self::capture), when the system lets an app know them (Windows). Obtura's
    /// own windows are left out. `None`: the system lists the windows itself (GNOME's list,
    /// through screen sharing).
    fn windows(&self) -> Option<Vec<Place>> {
        None
    }

    /// Forgets the user's earlier "no" to screenshots, so that the system asks again at the
    /// next [`capture`](Self::capture) made from a focused window. Only the user's own request
    /// ("Ask Again") calls this: the system still asks, nothing is allowed here.
    fn ask_again(&self, done: Box<dyn FnOnce()>) {
        done()
    }
}
