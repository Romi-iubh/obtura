//! Scrolling capture, the screen side: frames of one part of a shared screen, taken while the
//! user scrolls, written as numbered PNG files (`frame-000000.png`...) into a folder, for the
//! app to join (`obtura_image::stitch`). The frames come from the system's `gst-launch-1.0`
//! (the GIF recording pipeline, at full size); the fake session scrolls through a tall test page
//! instead, so tests never see the real screen.

use crate::{
    record::{Crop, Encoders, Format, Plan, Segment},
    Cast, CaptureError,
};
use obtura_image::{encode_png, Bitmap, Rect};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

/// Frames a second: enough to follow a wheel scroll, few enough to join as they come.
pub const FPS: u32 = 8;

/// How the fake screen scrolls: rows per frame.
const FAKE_STEP: u32 = 40;

/// Rows at the top of the fake page that stay put, like a web page's sticky header.
pub const FAKE_HEADER: u32 = 50;

pub type Ended = Box<dyn FnOnce(Result<(), String>)>;

/// The frames being taken.
pub enum Frames {
    Program(Segment),
    /// Taken by Obtura itself (the fake screen; the screen on Windows).
    Fake(Arc<AtomicBool>),
}

/// The name of frame `n` in `dir`.
pub fn frame_path(dir: &Path, n: u32) -> PathBuf {
    dir.join(format!("frame-{n:06}.png"))
}

impl Frames {
    /// Starts taking frames of `stream` of `cast`, the part `crop` of it (`size` once cut) into
    /// `dir`. `ended` is called once the taking stops (after [`Frames::stop`], or by itself).
    pub fn start(cast: &Cast, stream: usize, crop: Crop, size: (u32, u32), dir: &Path, ended: Ended) -> Result<Frames, CaptureError> {
        if let Some(page) = cast.fake_page.clone() {
            let (w, h) = cast.streams.get(stream).and_then(|s| s.size).map(|(w, h)| (w as u32, h as u32)).unwrap_or((page.width, 400));
            return Ok(Frames::Fake(fake_frames(page, (w, h), crop, dir.to_path_buf(), ended)));
        }
        #[cfg(windows)]
        if let Some(area) = cast.screen_area(stream) {
            return Ok(Frames::Fake(screen_frames(area, crop, dir.to_path_buf(), ended)));
        }
        let plan = Plan { format: Format::Gif, crop: Some(crop), sounds: Vec::new(), gif_fps: FPS, gif_size: size, encoders: Encoders::default(), threads: 2, layout: Vec::new(), size: None };
        Segment::start(cast, &[stream], &plan, &dir.join("frame-%06d.png"), 0, ended).map(Frames::Program)
    }

    pub fn stop(&self) {
        match self {
            Frames::Program(seg) => seg.stop(Duration::from_secs(3)),
            Frames::Fake(stop) => stop.store(true, Ordering::SeqCst),
        }
    }
}

/// What the fake screen (`w` x `h`) shows with `page` scrolled down by `at` rows: the page's
/// header (its first rows, fixed) and below it the page from `FAKE_HEADER + at`.
pub fn fake_view(page: &Bitmap, w: u32, h: u32, at: u32) -> Bitmap {
    let mut view = Bitmap::filled(w, h, [255, 255, 255, 255]);
    if let Some(header) = page.crop(Rect::new(0, 0, w, FAKE_HEADER.min(h))) {
        obtura_image::paste(&mut view, &header, 0, 0);
    }
    if let Some(rest) = page.crop(Rect::new(0, (FAKE_HEADER + at) as i32, w, h.saturating_sub(FAKE_HEADER))) {
        obtura_image::paste(&mut view, &rest, 0, FAKE_HEADER as i64);
    }
    view
}

/// The fake screen scrolling down through `page`, FPS frames a second, until stopped (it stays
/// at the bottom of the page once there).
fn fake_frames(page: Arc<Bitmap>, (w, h): (u32, u32), crop: Crop, dir: PathBuf, ended: Ended) -> Arc<AtomicBool> {
    let keep = Rect::new(crop.left as i32, crop.top as i32, w.saturating_sub(crop.left + crop.right), h.saturating_sub(crop.top + crop.bottom));
    let last = page.height.saturating_sub(h);
    frames_from(move |n| Ok(fake_view(&page, w, h, (n * FAKE_STEP).min(last)).crop(keep).unwrap_or_else(|| Bitmap::filled(1, 1, [0; 4]))), dir, ended)
}

/// Windows: the part `crop` of `area` of the screen, read FPS times a second.
#[cfg(windows)]
fn screen_frames(area: crate::windows::Area, crop: Crop, dir: PathBuf, ended: Ended) -> Arc<AtomicBool> {
    let part = crate::windows::Area {
        x: area.x + crop.left as i32,
        y: area.y + crop.top as i32,
        width: area.width.saturating_sub(crop.left + crop.right),
        height: area.height.saturating_sub(crop.top + crop.bottom),
    };
    frames_from(move |_| crate::windows::grab(part), dir, ended)
}

/// Writes the frames `take` gives (frame number in, picture out), FPS a second, into `dir`
/// until stopped, on a thread of its own.
fn frames_from(mut take: impl FnMut(u32) -> Result<Bitmap, String> + Send + 'static, dir: PathBuf, ended: Ended) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || {
        let mut n = 0u32;
        let pace = Duration::from_millis(1000 / FPS as u64);
        let result = loop {
            if s2.load(Ordering::SeqCst) {
                break Ok(());
            }
            let t0 = std::time::Instant::now();
            let written = take(n).and_then(|frame| encode_png(&frame).map_err(|e| e.to_string())).and_then(|png| {
                let tmp = dir.join(format!("frame-{n:06}.tmp"));
                std::fs::write(&tmp, png).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, frame_path(&dir, n)).map_err(|e| e.to_string())
            });
            if let Err(e) = written {
                break Err(e);
            }
            n += 1;
            std::thread::sleep(pace.saturating_sub(t0.elapsed()));
        };
        let _ = tx.send(result);
    });
    // The answer comes back on the main loop.
    let ended = std::cell::RefCell::new(Some(ended));
    gio::glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
        Ok(r) => {
            if let Some(f) = ended.borrow_mut().take() {
                f(r);
            }
            gio::glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => gio::glib::ControlFlow::Continue,
        Err(_) => gio::glib::ControlFlow::Break,
    });
    stop
}

/// The frames waiting in `dir`, oldest first (only finished ones: `.png`).
pub fn waiting(dir: &Path) -> Vec<PathBuf> {
    crate::record::segments(dir, Format::Gif)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_screen_keeps_its_header_and_scrolls_the_rest() {
        let mut page = Bitmap::filled(40, 300, [255, 255, 255, 255]);
        for y in 0..300 {
            page.set_pixel(0, y, [y as u8, 0, 0, 255]);
        }
        let v = fake_view(&page, 40, 100, 30);
        assert_eq!(v.pixel(0, 10), [10, 0, 0, 255], "header");
        assert_eq!(v.pixel(0, FAKE_HEADER), [(FAKE_HEADER + 30) as u8, 0, 0, 255], "the page, scrolled");
    }

    /// The test page, seen through the fake screen as it scrolls (part of it cut as the user
    /// chose), joins back into the page itself.
    #[test]
    fn the_scrolled_test_page_joins_back_whole() {
        let page = obtura_image::decode_png(include_bytes!("../../../tests/fixtures/long-page.png")).unwrap();
        let (w, h) = (800, 500);
        let keep = Rect::new(100, 0, 600, 480);
        let mut s = obtura_image::stitch::Stitcher::new();
        let last = page.height - h;
        let mut n = 0;
        loop {
            let at = (n * FAKE_STEP).min(last);
            let r = s.push(&fake_view(&page, w, h, at).crop(keep).unwrap());
            assert!(!matches!(r, obtura_image::stitch::Pushed::Lost | obtura_image::stitch::Pushed::Refused), "frame {n}: {r:?}");
            if at == last {
                break;
            }
            n += 1;
        }
        let pic = s.picture().unwrap();
        // The header once, then the page down to the bottom of the last view.
        let expected = page.crop(Rect::new(100, 0, 600, page.height - 20)).unwrap();
        assert_eq!((pic.width, pic.height), (expected.width, expected.height));
        assert!(pic == expected, "the joined picture differs from the page");
    }
}
