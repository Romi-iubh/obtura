//! Recording the screen with the system's GStreamer tools (`gst-launch-1.0`, packages
//! gstreamer1.0-tools, gstreamer1.0-pipewire and the usual plugins), fed by a [`Cast`].
//!
//! A recording is made of segments: one `gst-launch-1.0` program per stretch between a start
//! (or a resume) and a pause (or the stop). Stopping a segment sends it Ctrl+C (`-e` turns that
//! into a clean end of stream, so the file is finished properly). The segments are joined at
//! the end ([`join`]). A GIF is recorded as numbered PNG frames, turned into a GIF afterwards
//! (`obtura_image::gif`). MP4 files are written in fragments, so even a recording cut short
//! stays playable.

use crate::{screencast::Sound, Cast, CaptureError};
use std::{
    cell::RefCell,
    ffi::OsStr,
    os::fd::BorrowedFd,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

/// The system's GStreamer launcher (not a copy elsewhere on the PATH).
pub const GST_LAUNCH: &str = "/usr/bin/gst-launch-1.0";
const GST_INSPECT: &str = "/usr/bin/gst-inspect-1.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mp4,
    WebM,
    Gif,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::WebM => "webm",
            Format::Gif => "gif",
        }
    }

    /// The extension of one segment's file.
    pub fn segment_extension(self) -> &'static str {
        match self {
            Format::Gif => "png",
            f => f.extension(),
        }
    }
}

/// A part of the frame to keep, in the frame's own (physical) pixels: pixels cut off each side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Crop {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Crop {
    /// Cuts a `frame` (width, height) down to `keep` (x, y, width, height). Video encoders want
    /// even sizes, so with `even` one more pixel goes from the right or bottom when needed.
    pub fn of(frame: (u32, u32), keep: (u32, u32, u32, u32), even: bool) -> Crop {
        let (fw, fh) = frame;
        let x = keep.0.min(fw.saturating_sub(2));
        let y = keep.1.min(fh.saturating_sub(2));
        let mut w = keep.2.min(fw - x).max(2);
        let mut h = keep.3.min(fh - y).max(2);
        if even {
            w -= w % 2;
            h -= h % 2;
        }
        Crop { left: x, top: y, right: fw.saturating_sub(x + w), bottom: fh.saturating_sub(y + h) }
    }

    /// The size left after cutting a `frame`.
    pub fn size(&self, frame: (u32, u32)) -> (u32, u32) {
        (frame.0.saturating_sub(self.left + self.right), frame.1.saturating_sub(self.top + self.bottom))
    }
}

/// The encoders this system has, found once with `gst-inspect-1.0`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Encoders {
    pub x264: bool,
    pub openh264: bool,
    pub aac: Option<&'static str>,
}

impl Encoders {
    pub fn detect() -> Encoders {
        let aac = ["avenc_aac", "voaacenc"].into_iter().find(|e| element_exists(e));
        Encoders { x264: element_exists("x264enc"), openh264: element_exists("openh264enc"), aac }
    }

    pub fn can_record(&self, format: Format) -> bool {
        match format {
            Format::Mp4 => self.x264 || self.openh264,
            Format::WebM | Format::Gif => true,
        }
    }
}

fn element_exists(name: &str) -> bool {
    std::process::Command::new(GST_INSPECT).arg("--exists").arg(name).status().is_ok_and(|s| s.success())
}

/// Whether videos can be trimmed: GStreamer's `avwait` (package gstreamer1.0-plugins-bad).
pub fn can_trim() -> bool {
    element_exists("avwait")
}

/// Whether the GStreamer tools and the PipeWire source are installed.
pub fn tools_present() -> bool {
    Path::new(GST_LAUNCH).exists() && element_exists("pipewiresrc")
}

/// Everything one segment needs, besides the source.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub format: Format,
    /// The frame's size in physical pixels (to work out the crop) and the part kept.
    pub crop: Option<Crop>,
    pub sounds: Vec<Sound>,
    /// GIF: frames per second and the size of each frame.
    pub gif_fps: u32,
    pub gif_size: (u32, u32),
    pub encoders: Encoders,
    /// Threads for the encoder.
    pub threads: u32,
    /// Several screens in one picture: where each source goes and how big, in physical pixels
    /// of the picture (x, y, width, height), in the order of the sources. Empty for one.
    pub layout: Vec<(i64, i64, u32, u32)>,
    /// The size of every frame, when it is fixed: the picture holding several screens, or a
    /// window (a window that changes size is fitted into its first size, with black borders).
    pub size: Option<(u32, u32)>,
}

fn words(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split_whitespace().map(str::to_string)
}

/// The arguments of `gst-launch-1.0 -e -q` for one segment: the `videos` (from
/// [`Cast::video_sources`]; several are put side by side as `plan.layout` says) and the sound
/// sources, into `out` (for a GIF: a folder pattern of numbered PNG files, the first number being
/// `first_frame`).
pub fn pipeline(videos: &[Vec<String>], sounds: &[Vec<String>], plan: &Plan, out: &Path, first_frame: u32) -> Vec<String> {
    let several = videos.len() > 1;
    let mut a: Vec<String> = if several {
        // One picture of all the screens: each scaled to its place on a black background.
        let mut head = vec!["compositor".to_string(), "name=vmix".into(), "background=black".into()];
        for (i, (x, y, w, h)) in plan.layout.iter().enumerate() {
            head.extend([format!("sink_{i}::xpos={x}"), format!("sink_{i}::ypos={y}"), format!("sink_{i}::width={w}"), format!("sink_{i}::height={h}")]);
        }
        let (w, h) = plan.size.unwrap_or((1920, 1080));
        head.extend(words(&format!("! video/x-raw,width={w},height={h},framerate=30/1")));
        head
    } else {
        let mut v = videos.first().cloned().unwrap_or_default();
        if let Some((w, h)) = plan.size {
            // A window: always this size, whatever it does.
            v.extend(words(&format!("! videoconvert ! videoscale add-borders=true ! video/x-raw,width={w},height={h},pixel-aspect-ratio=1/1")));
        }
        v
    };
    if let Some(c) = plan.crop.filter(|c| *c != Crop::default()) {
        a.push("!".into());
        a.extend(words(&format!("videocrop left={} top={} right={} bottom={}", c.left, c.top, c.right, c.bottom)));
    }
    let t = plan.threads.max(1);
    let file = |a: &mut Vec<String>| {
        a.push("!".into());
        a.push("filesink".into());
        a.push(format!("location={}", out.display()));
    };
    match plan.format {
        Format::Gif => {
            a.push("!".into());
            a.extend(words(&format!("videorate ! video/x-raw,framerate={}/1 ! videoscale ! video/x-raw,width={},height={} ! videoconvert ! video/x-raw,format=RGBA ! queue ! pngenc compression-level=1 ! multifilesink", plan.gif_fps.max(1), plan.gif_size.0, plan.gif_size.1)));
            a.push(format!("location={}", out.display()));
            a.push(format!("index={first_frame}"));
            add_screens(&mut a, videos, several);
            return a;
        }
        f => {
            a.push("!".into());
            a.extend(words(&video_chain(f, &plan.encoders, t)));
        }
    }
    file(&mut a);
    add_screens(&mut a, videos, several);
    if !sounds.is_empty() {
        let encoder = sound_encoder(plan.format, &plan.encoders);
        let mixed = sounds.len() > 1;
        if mixed {
            a.extend(words(&format!("audiomixer name=amix ! audioconvert ! {encoder} ! queue ! mux.")));
        }
        for source in sounds {
            a.extend(source.iter().cloned());
            a.extend(words("! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! queue"));
            if mixed {
                a.push("!".into());
                a.push("amix.".into());
            } else {
                a.extend(words(&format!("! audioconvert ! {encoder} ! queue ! mux.")));
            }
        }
    }
    a
}

/// Each screen's source into the picture of all of them.
fn add_screens(a: &mut Vec<String>, videos: &[Vec<String>], several: bool) {
    if !several {
        return;
    }
    for v in videos {
        a.extend(v.iter().cloned());
        a.extend(words("! videoconvert ! queue ! vmix."));
    }
}

/// The arguments of `gst-launch-1.0 -m` that report how loud `source` is, ten times a second,
/// and throw the sound away (for the level meters of the Record window).
pub fn level_args(source: Vec<String>) -> Vec<String> {
    let mut a = source;
    a.extend(words("! audioconvert ! level interval=100000000 post-messages=true ! fakesink sync=false"));
    a
}

/// How loud, from 0.0 (silent, -60 dB or less) to 1.0 (0 dB): a `level` line of
/// `gst-launch-1.0 -m` (the loudest channel's peak). `None` for other lines.
pub fn parse_level(line: &str) -> Option<f64> {
    if !line.contains("level, ") {
        return None;
    }
    let peaks = line.split("peak=(GValueArray)<").nth(1)?.split('>').next()?;
    let db = peaks.split(',').filter_map(|v| v.trim().parse::<f64>().ok()).fold(f64::NEG_INFINITY, f64::max);
    if !db.is_finite() {
        return Some(0.0);
    }
    Some(((db + 60.0) / 60.0).clamp(0.0, 1.0))
}

/// Raw video to the muxer named `mux`: the encoder of `format` (MP4: H.264 in fragments, so a file
/// cut short stays playable; WebM: VP8).
fn video_chain(format: Format, encoders: &Encoders, t: u32) -> String {
    let convert = format!("videoconvert chroma-mode=none dither=none matrix-mode=output-only n-threads={t} ! video/x-raw,format=I420 ! queue !");
    match format {
        Format::Mp4 if encoders.x264 => format!("{convert} x264enc speed-preset=veryfast tune=zerolatency pass=qual quantizer=20 threads={t} ! video/x-h264,profile=high ! queue ! h264parse ! mp4mux name=mux fragment-duration=500 fragment-mode=first-moov-then-finalise"),
        Format::Mp4 => format!("{convert} openh264enc deblocking=off background-detection=false complexity=low adaptive-quantization=false qp-max=26 qp-min=26 multi-thread={t} slice-mode=auto ! queue ! h264parse ! mp4mux name=mux fragment-duration=500 fragment-mode=first-moov-then-finalise"),
        _ => format!("{convert} vp8enc cpu-used=16 max-quantizer=17 deadline=1 keyframe-max-dist=150 threads={t} static-threshold=1000 buffer-size=20000 ! queue ! webmmux name=mux"),
    }
}

/// Raw sound to the muxer: AAC in MP4 when there is an AAC encoder, Opus otherwise.
fn sound_encoder(format: Format, encoders: &Encoders) -> String {
    match (format, encoders.aac) {
        (Format::Mp4, Some(aac)) => format!("{aac} bitrate=192000 ! aacparse"),
        _ => "opusenc bitrate=128000 ! opusparse".to_string(),
    }
}

fn demuxer(format: Format) -> &'static str {
    match format {
        Format::Mp4 => "qtdemux",
        _ => "matroskademux",
    }
}

/// A queue that never fills: the file reader feeds the picture and the sound from one thread,
/// and the cutting holds the sound back until the picture reaches the start.
const OPEN_QUEUE: &str = "queue max-size-buffers=0 max-size-bytes=0 max-size-time=0";

/// Cuts a recording (`format`, with or without sound) to the part from `start` to `end`
/// (nanoseconds), encoded again with the recording's own settings, into `out`. The cut is
/// exact to the frame (GStreamer's `avwait`, then the times are moved back to zero).
#[allow(clippy::too_many_arguments)]
pub fn trim_args(input: &Path, format: Format, with_sound: bool, start: u64, end: u64, encoders: &Encoders, threads: u32, out: &Path) -> Vec<String> {
    let mut a = vec!["filesrc".to_string(), format!("location={}", input.display())];
    a.extend(words(&format!("! {} name=d avwait name=w mode=running-time target-running-time={start} end-running-time={end}", demuxer(format))));
    a.extend(words(&format!("d.video_0 ! {OPEN_QUEUE} ! decodebin ! videoconvert ! w.vsink w.vsrc ! identity ts-offset=-{start} ! {} ! filesink", video_chain(format, encoders, threads.max(1)))));
    a.push(format!("location={}", out.display()));
    if with_sound {
        a.extend(words(&format!("d.audio_0 ! {OPEN_QUEUE} ! decodebin ! audioconvert ! audioresample ! w.asink w.asrc ! identity ts-offset=-{start} ! audioconvert ! {} ! queue ! mux.", sound_encoder(format, encoders))));
    }
    a
}

/// Small pictures of a recording for choosing where to cut: `per_second` of them for each
/// second (a fraction), `width` wide, numbered from 0 in `pattern` (like `dir/thumb-%05d.png`).
/// Picture n shows the moment n / `per_second`.
pub fn thumbnail_args(input: &Path, format: Format, per_second: f64, width: u32, pattern: &Path) -> Vec<String> {
    let rate = ((per_second * 1000.0).round() as u32).max(1);
    let mut a = vec!["filesrc".to_string(), format!("location={}", input.display())];
    a.extend(words(&format!("! {} name=d d.video_0 ! queue ! decodebin ! videoconvert ! videorate ! video/x-raw,framerate={rate}/1000 ! videoscale ! video/x-raw,width={width},pixel-aspect-ratio=1/1 ! videoconvert ! pngenc compression-level=1 ! multifilesink", demuxer(format))));
    a.push(format!("location={}", pattern.display()));
    a
}

/// One `gst-launch-1.0` program recording one segment.
pub struct Segment {
    sub: gio::Subprocess,
    stopping: Rc<RefCell<bool>>,
}

/// How a segment's program ended: normally (after [`Segment::stop`]) or by itself, with what
/// it wrote on its error output.
pub type Ended = Box<dyn FnOnce(Result<(), String>)>;

impl Segment {
    /// Starts recording stream `stream` of `cast` with `plan` into `out`. `ended` is called once,
    /// when the program ends: after [`Segment::stop`] with `Ok`, or by itself (an error) with
    /// what went wrong.
    pub fn start(cast: &Cast, streams: &[usize], plan: &Plan, out: &Path, first_frame: u32, ended: Ended) -> Result<Segment, CaptureError> {
        let (videos, fd) = cast.video_sources(streams)?;
        let sounds: Vec<Vec<String>> = if plan.format == Format::Gif { Vec::new() } else { plan.sounds.iter().map(|s| cast.sound_source(*s)).collect() };
        let args = pipeline(&videos, &sounds, plan, out, first_frame);
        let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDERR_PIPE | gio::SubprocessFlags::STDOUT_SILENCE);
        if let Some(fd) = fd {
            // SAFETY: only names the descriptor number the program will see (3).
            launcher.take_fd(fd, unsafe { BorrowedFd::borrow_raw(3) });
        }
        let mut argv: Vec<&OsStr> = vec![GST_LAUNCH.as_ref(), "-e".as_ref(), "-q".as_ref()];
        argv.extend(args.iter().map(|s| OsStr::new(s.as_str())));
        let sub = launcher.spawn(&argv).map_err(|e| CaptureError::Failed(format!("cannot start {GST_LAUNCH}: {e}")))?;
        let stopping = Rc::new(RefCell::new(false));
        let s2 = stopping.clone();
        let sub2 = sub.clone();
        sub.communicate_utf8_async(None, gio::Cancellable::NONE, move |result| {
            let errors = result.ok().and_then(|(_, err)| err).map(|e| e.to_string()).unwrap_or_default();
            let ok = sub2.is_successful() || (*s2.borrow() && sub2.has_signaled());
            ended(if ok { Ok(()) } else { Err(explain(&errors)) });
        });
        Ok(Segment { sub, stopping })
    }

    /// Asks the program to finish its file (Ctrl+C, which `-e` turns into a clean end). If it
    /// has not ended after `grace`, it is killed (an MP4 written in fragments stays playable).
    pub fn stop(&self, grace: Duration) {
        *self.stopping.borrow_mut() = true;
        self.sub.send_signal(2);
        let sub = self.sub.clone();
        gio::glib::timeout_add_local_once(grace, move || {
            if sub.identifier().is_some() {
                sub.force_exit();
            }
        });
    }
}

/// The useful line of what `gst-launch-1.0` wrote when it failed.
fn explain(errors: &str) -> String {
    let line = errors.lines().find(|l| l.starts_with("ERROR")).or_else(|| errors.lines().find(|l| !l.trim().is_empty())).unwrap_or("the recording program stopped");
    line.trim_start_matches("ERROR: ").trim().to_string()
}

/// The parser for the sound a segment of `plan` holds, if it holds sound.
pub fn sound_parser(plan: &Plan) -> Option<&'static str> {
    match (plan.format, plan.sounds.is_empty(), plan.encoders.aac) {
        (Format::Gif, _, _) | (_, true, _) => None,
        (Format::Mp4, false, Some(_)) => Some("aacparse"),
        _ => Some("opusparse"),
    }
}

/// Joins video segments (in name order, `pattern` being a file pattern like `dir/seg-*.mp4`)
/// into `out` without encoding them again. `sound` is the parser of their sound, if any
/// ([`sound_parser`]).
pub fn join_args(pattern: &Path, format: Format, sound: Option<&str>, out: &Path) -> Vec<String> {
    let mut a: Vec<String> = vec!["splitmuxsrc".into(), "name=s".into(), format!("location={}", pattern.display())];
    let (mux, video_parse) = match format {
        Format::Mp4 => ("mp4mux name=mux fragment-duration=500 fragment-mode=first-moov-then-finalise", "! h264parse"),
        _ => ("webmmux name=mux", ""),
    };
    a.extend(words(&format!("s.video_0 ! queue {video_parse} ! {mux} ! filesink")));
    a.push(format!("location={}", out.display()));
    if let Some(parse) = sound {
        a.extend(words(&format!("s.audio_0 ! queue ! {parse} ! mux.")));
    }
    a
}

/// Runs `gst-launch-1.0 -e -q args` and calls `done` when it ends.
pub fn run(args: Vec<String>, done: impl FnOnce(Result<(), String>) + 'static) {
    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDERR_PIPE | gio::SubprocessFlags::STDOUT_SILENCE);
    let mut argv: Vec<&OsStr> = vec![GST_LAUNCH.as_ref(), "-e".as_ref(), "-q".as_ref()];
    argv.extend(args.iter().map(|s| OsStr::new(s.as_str())));
    match launcher.spawn(&argv) {
        Err(e) => done(Err(format!("cannot start {GST_LAUNCH}: {e}"))),
        Ok(sub) => {
            let sub2 = sub.clone();
            sub.communicate_utf8_async(None, gio::Cancellable::NONE, move |result| {
                let errors = result.ok().and_then(|(_, err)| err).map(|e| e.to_string()).unwrap_or_default();
                done(if sub2.is_successful() { Ok(()) } else { Err(explain(&errors)) });
            });
        }
    }
}

/// Runs `gst-launch-1.0` on `source` (ending in raw video, reading `fd` as descriptor 3 if
/// given) for one frame, and gives that frame as PNG bytes. Gives up after 8 seconds.
pub fn grab_png(source: Vec<String>, fd: Option<std::os::fd::OwnedFd>, done: impl FnOnce(Result<Vec<u8>, CaptureError>) + 'static) {
    let mut args = source;
    args.extend(words("! videoconvert ! video/x-raw,format=RGBA ! pngenc compression-level=1 ! fdsink fd=1"));
    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE);
    if let Some(fd) = fd {
        // SAFETY: only names the descriptor number the program will see (3).
        launcher.take_fd(fd, unsafe { BorrowedFd::borrow_raw(3) });
    }
    let mut argv: Vec<&OsStr> = vec![GST_LAUNCH.as_ref(), "-q".as_ref()];
    argv.extend(args.iter().map(|s| OsStr::new(s.as_str())));
    let sub = match launcher.spawn(&argv) {
        Ok(s) => s,
        Err(e) => return done(Err(CaptureError::Failed(format!("cannot start {GST_LAUNCH}: {e}")))),
    };
    let watchdog = {
        let sub = sub.clone();
        gio::glib::timeout_add_local_once(Duration::from_secs(8), move || sub.force_exit())
    };
    let watchdog = RefCell::new(Some(watchdog));
    let sub2 = sub.clone();
    sub.communicate_async(None, gio::Cancellable::NONE, move |result| {
        if let Some(w) = watchdog.borrow_mut().take() {
            w.remove();
        }
        let (out, err) = match result {
            Ok((out, err)) => (out.map(|b| b.to_vec()).unwrap_or_default(), err.map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default()),
            Err(e) => return done(Err(CaptureError::Failed(e.to_string()))),
        };
        if sub2.is_successful() && crate::png_size(&out).is_some() {
            done(Ok(out))
        } else {
            done(Err(CaptureError::Failed(format!("no picture of the screen came: {}", explain(&err)))))
        }
    });
}

/// The segment files of a recording folder, in order.
pub fn segments(dir: &Path, format: Format) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().and_then(|e| e.to_str()) == Some(format.segment_extension())).collect())
        .unwrap_or_default();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(format: Format) -> Plan {
        Plan { format, crop: None, sounds: vec![], gif_fps: 10, gif_size: (400, 300), encoders: Encoders { x264: true, openh264: true, aac: Some("avenc_aac") }, threads: 4, layout: vec![], size: None }
    }

    fn joined(a: &[String]) -> String {
        a.join(" ")
    }

    const VIDEO: [&str; 3] = ["videotestsrc", "!", "video/x-raw"];

    fn video() -> Vec<Vec<String>> {
        vec![VIDEO.iter().map(|s| s.to_string()).collect()]
    }

    #[test]
    fn crops_to_the_part_kept_with_even_sizes_for_video() {
        let c = Crop::of((1920, 1080), (100, 50, 301, 201), true);
        assert_eq!(c, Crop { left: 100, top: 50, right: 1920 - 100 - 300, bottom: 1080 - 50 - 200 });
        assert_eq!(c.size((1920, 1080)), (300, 200));
        // A GIF keeps odd sizes.
        assert_eq!(Crop::of((1920, 1080), (100, 50, 301, 201), false).size((1920, 1080)), (301, 201));
        // Never past the frame.
        let c = Crop::of((800, 600), (700, 500, 400, 400), true);
        assert_eq!(c.size((800, 600)), (100, 100));
        assert_eq!((c.left, c.top), (700, 500));
    }

    #[test]
    fn mp4_uses_h264_in_fragments_and_crops_first() {
        let mut p = plan(Format::Mp4);
        p.crop = Some(Crop { left: 10, top: 20, right: 30, bottom: 40 });
        let a = joined(&pipeline(&video(), &[], &p, Path::new("/x/seg-0001.mp4"), 0));
        assert!(a.starts_with("videotestsrc ! video/x-raw ! videocrop left=10 top=20 right=30 bottom=40 ! videoconvert"), "{a}");
        assert!(a.contains("x264enc") && a.contains("format=I420") && a.contains("mp4mux name=mux fragment-duration=500"), "{a}");
        assert!(a.ends_with("filesink location=/x/seg-0001.mp4"), "{a}");
        assert!(!a.contains("audio"), "{a}");
        p.encoders.x264 = false;
        assert!(joined(&pipeline(&video(), &[], &p, Path::new("/x/a.mp4"), 0)).contains("openh264enc"));
    }

    #[test]
    fn a_path_with_spaces_stays_one_argument() {
        let a = pipeline(&video(), &[], &plan(Format::WebM), Path::new("/home/u/My Videos/seg 1.webm"), 0);
        assert!(a.contains(&"location=/home/u/My Videos/seg 1.webm".to_string()));
        assert!(a.iter().all(|w| w.starts_with("location=") || !w.contains(' ')));
    }

    #[test]
    fn sound_goes_into_the_file_mixed_when_there_are_two() {
        let mic = vec!["pulsesrc".to_string()];
        let both = vec![mic.clone(), vec!["pulsesrc".to_string(), "device=@DEFAULT_MONITOR@".to_string()]];
        let a = joined(&pipeline(&video(), &both, &plan(Format::WebM), Path::new("/x/a.webm"), 0));
        assert!(a.contains("audiomixer name=amix ! audioconvert ! opusenc bitrate=128000 ! opusparse ! queue ! mux."), "{a}");
        assert_eq!(a.matches("! amix.").count(), 2, "{a}");
        let a = joined(&pipeline(&video(), &[mic.clone()], &plan(Format::Mp4), Path::new("/x/a.mp4"), 0));
        assert!(a.contains("pulsesrc ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! queue ! audioconvert ! avenc_aac bitrate=192000 ! aacparse ! queue ! mux."), "{a}");
        assert!(!a.contains("amix"));
        // No AAC encoder: Opus, which MP4 holds too.
        let mut p = plan(Format::Mp4);
        p.encoders.aac = None;
        assert!(joined(&pipeline(&video(), &[mic], &p, Path::new("/x/a.mp4"), 0)).contains("opusenc"));
    }

    #[test]
    fn a_gif_is_recorded_as_numbered_frames_without_sound() {
        let a = joined(&pipeline(&video(), &[], &plan(Format::Gif), Path::new("/x/frame-%06d.png"), 37));
        assert!(a.contains("videorate ! video/x-raw,framerate=10/1 ! videoscale ! video/x-raw,width=400,height=300"), "{a}");
        assert!(a.ends_with("multifilesink location=/x/frame-%06d.png index=37"), "{a}");
    }

    #[test]
    fn joining_copies_the_streams_into_one_file() {
        let mut p = plan(Format::WebM);
        p.sounds = vec![Sound::Microphone];
        let a = joined(&join_args(Path::new("/x/seg-*.webm"), Format::WebM, sound_parser(&p), Path::new("/x/joined.webm")));
        assert_eq!(a, "splitmuxsrc name=s location=/x/seg-*.webm s.video_0 ! queue ! webmmux name=mux ! filesink location=/x/joined.webm s.audio_0 ! queue ! opusparse ! mux.");
        let a = joined(&join_args(Path::new("/x/seg-*.mp4"), Format::Mp4, sound_parser(&plan(Format::Mp4)), Path::new("/x/j.mp4")));
        assert!(a.contains("s.video_0 ! queue ! h264parse ! mp4mux") && !a.contains("audio"), "{a}");
        let mut p = plan(Format::Mp4);
        p.sounds = vec![Sound::Computer];
        assert_eq!(sound_parser(&p), Some("aacparse"));
        p.encoders.aac = None;
        assert_eq!(sound_parser(&p), Some("opusparse"));
    }

    /// Runs one segment with the system's gst-launch-1.0 for `secs`, then stops it with Ctrl+C.
    fn record_segment(args: Vec<String>, secs: f64) {
        let mut child = std::process::Command::new(GST_LAUNCH).arg("-e").arg("-q").args(&args).spawn().unwrap();
        std::thread::sleep(Duration::from_secs_f64(secs));
        std::process::Command::new("/bin/kill").args(["-INT", &child.id().to_string()]).status().unwrap();
        assert!(child.wait().unwrap().success(), "segment failed: {}", args.join(" "));
    }

    fn discover(file: &Path) -> String {
        let out = std::process::Command::new("/usr/bin/gst-discoverer-1.0").arg("-v").arg(file).output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn seconds(info: &str) -> f64 {
        let d = info.lines().find_map(|l| l.trim().strip_prefix("Duration: ")).expect("a duration");
        let parts: Vec<f64> = d.split(':').map(|p| p.parse().unwrap()).collect();
        parts[0] * 3600.0 + parts[1] * 60.0 + parts[2]
    }

    /// The real pipelines, with the fake session's test picture and test tones: two segments
    /// of each video format with two sounds mixed, joined into one file of the right length and
    /// size; and GIF frames numbered on across segments.
    #[test]
    fn the_pipelines_really_record_and_join() {
        if !tools_present() {
            eprintln!("skipped: the GStreamer tools are not installed");
            return;
        }
        let dir = std::env::temp_dir().join(format!("obtura-record-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cast = Cast::fake(320, 240, None);
        let encoders = Encoders::detect();
        for format in [Format::Mp4, Format::WebM] {
            if !encoders.can_record(format) {
                continue;
            }
            let sub = dir.join(format.extension());
            std::fs::create_dir_all(&sub).unwrap();
            let p = Plan { format, crop: Some(Crop::of((320, 240), (10, 10, 201, 151), true)), sounds: vec![Sound::Microphone, Sound::Computer], gif_fps: 10, gif_size: (1, 1), encoders, threads: 2, layout: vec![], size: None };
            let sounds: Vec<Vec<String>> = p.sounds.iter().map(|s| cast.sound_source(*s)).collect();
            for n in 1..=2 {
                let (video, _) = cast.video_source(0).unwrap();
                record_segment(pipeline(&[video], &sounds, &p, &sub.join(format!("seg-{n:04}.{}", format.extension())), 0), 1.5);
            }
            assert_eq!(segments(&sub, format).len(), 2);
            let out = sub.join(format!("joined.{}", format.extension()));
            let ok = std::process::Command::new(GST_LAUNCH).arg("-e").arg("-q").args(join_args(&sub.join(format!("seg-*.{}", format.extension())), format, sound_parser(&p), &out)).status().unwrap();
            assert!(ok.success(), "joining {format:?}");
            let info = discover(&out);
            let length = seconds(&info);
            assert!((2.4..=3.6).contains(&length), "{format:?} lasts {length} s\n{info}");
            assert!(info.contains("Width: 200") && info.contains("Height: 150"), "{format:?} size\n{info}");
            assert!(info.contains("audio codec"), "{format:?} has no sound\n{info}");
            // Cut from 0.5 s to 1.5 s: one second, sound kept, same size.
            let cut = sub.join(format!("cut.{}", format.extension()));
            let ok = std::process::Command::new(GST_LAUNCH).arg("-e").arg("-q").args(trim_args(&out, format, true, 500_000_000, 1_500_000_000, &encoders, 2, &cut)).status().unwrap();
            assert!(ok.success(), "trimming {format:?}");
            let info = discover(&cut);
            let length = seconds(&info);
            assert!((0.9..=1.15).contains(&length), "{format:?} cut lasts {length} s\n{info}");
            assert!(info.contains("Width: 200") && info.contains("audio codec"), "{format:?} cut\n{info}");
            // Pictures to choose where to cut: 4 a second over about 3 s.
            let thumbs = sub.join("thumbs");
            std::fs::create_dir_all(&thumbs).unwrap();
            let ok = std::process::Command::new(GST_LAUNCH).arg("-q").args(thumbnail_args(&out, format, 4.0, 80, &thumbs.join("t-%05d.png"))).status().unwrap();
            assert!(ok.success());
            let n = std::fs::read_dir(&thumbs).unwrap().count();
            assert!((9..=16).contains(&n), "{n} pictures for a 3 s recording at 4 a second");
            let first = std::fs::read(thumbs.join("t-00000.png")).unwrap();
            assert_eq!(crate::png_size(&first), Some((80, 60)));
        }
        // GIF: frames numbered on from one segment to the next.
        let frames = dir.join("gif");
        std::fs::create_dir_all(&frames).unwrap();
        let p = Plan { format: Format::Gif, crop: None, sounds: vec![], gif_fps: 10, gif_size: (160, 120), encoders, threads: 2, layout: vec![], size: None };
        let pattern = frames.join("frame-%06d.png");
        let (video, _) = cast.video_source(0).unwrap();
        let video = vec![video];
        record_segment(pipeline(&video, &[], &p, &pattern, 0), 1.2);
        let first = segments(&frames, Format::Gif).len() as u32;
        assert!(first >= 8, "{first} frames in 1.2 s at 10 a second");
        record_segment(pipeline(&video, &[], &p, &pattern, first), 1.2);
        let all = segments(&frames, Format::Gif);
        assert!(all.len() as u32 >= first + 8);
        assert!(all.last().unwrap().file_name().unwrap().to_str().unwrap() == format!("frame-{:06}.png", all.len() - 1), "numbered without gaps");
        let png = std::fs::read(&all[0]).unwrap();
        assert_eq!(crate::png_size(&png), Some((160, 120)));
        // Two screens side by side in one picture, the second one smaller and lower.
        let screens = Cast::fake_screens(&[(320, 240, 0, 0), (160, 120, 320, 60)]);
        let (videos, _) = screens.video_sources(&[0, 1]).unwrap();
        let both = dir.join("both");
        std::fs::create_dir_all(&both).unwrap();
        let format = if encoders.can_record(Format::Mp4) { Format::Mp4 } else { Format::WebM };
        let p = Plan { format, crop: None, sounds: vec![Sound::Microphone], gif_fps: 10, gif_size: (1, 1), encoders, threads: 2, layout: vec![(0, 0, 320, 240), (320, 60, 160, 120)], size: Some((480, 240)) };
        let out = both.join(format!("both.{}", format.extension()));
        record_segment(pipeline(&videos, &[screens.sound_source(Sound::Microphone)], &p, &out, 0), 1.5);
        let info = discover(&out);
        assert!(info.contains("Width: 480") && info.contains("Height: 240"), "two screens in one picture\n{info}");
        assert!(seconds(&info) >= 1.0, "{info}");
        // A window that changes size keeps the recording's size.
        let window = Cast::fake(300, 200, None);
        let (video, _) = window.video_source(0).unwrap();
        let p = Plan { format, crop: None, sounds: vec![], gif_fps: 10, gif_size: (1, 1), encoders, threads: 2, layout: vec![], size: Some((400, 200)) };
        let out = both.join(format!("window.{}", format.extension()));
        record_segment(pipeline(&[video], &[], &p, &out, 0), 1.2);
        let info = discover(&out);
        assert!(info.contains("Width: 400") && info.contains("Height: 200"), "the window's size kept\n{info}");
        // Level meters: a test tone reads as some sound.
        let mut child = std::process::Command::new(GST_LAUNCH).arg("-m").args(level_args(screens.sound_source(Sound::Microphone))).stdout(std::process::Stdio::piped()).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(700));
        let _ = child.kill();
        let out = child.wait_with_output().unwrap();
        let levels: Vec<f64> = String::from_utf8_lossy(&out.stdout).lines().filter_map(parse_level).collect();
        assert!(!levels.is_empty() && levels.iter().all(|l| (0.3..0.95).contains(l)), "{levels:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn several_screens_are_put_together_and_levels_are_read() {
        let mut p = plan(Format::WebM);
        p.layout = vec![(0, 0, 1920, 1080), (1920, 0, 1280, 720)];
        p.size = Some((3200, 1080));
        let two = vec![vec!["pipewiresrc".to_string(), "path=5".into()], vec!["pipewiresrc".to_string(), "path=6".into()]];
        let a = joined(&pipeline(&two, &[], &p, Path::new("/x/a.webm"), 0));
        assert!(a.starts_with("compositor name=vmix background=black sink_0::xpos=0 sink_0::ypos=0 sink_0::width=1920 sink_0::height=1080 sink_1::xpos=1920"), "{a}");
        assert!(a.contains("! video/x-raw,width=3200,height=1080,framerate=30/1 ! videoconvert"), "{a}");
        assert!(a.ends_with("pipewiresrc path=5 ! videoconvert ! queue ! vmix. pipewiresrc path=6 ! videoconvert ! queue ! vmix."), "{a}");
        let line = r#"Got message #40 from element "level0" (element): level, endtime=(guint64)2, rms=(GValueArray)< -16.9 >, peak=(GValueArray)< -13.98, -30.0 >, decay=(GValueArray)< -13.9 >;"#;
        let l = parse_level(line).unwrap();
        assert!((l - (60.0 - 13.98) / 60.0).abs() < 1e-9);
        assert_eq!(parse_level("Got message #8 (state-changed)"), None);
        assert_eq!(parse_level("level, peak=(GValueArray)< -inf >;"), Some(0.0));
    }

    #[test]
    fn trimming_cuts_by_time_and_encodes_like_the_recording() {
        let e = Encoders { x264: true, openh264: true, aac: Some("avenc_aac") };
        let a = joined(&trim_args(Path::new("/v/My Rec.mp4"), Format::Mp4, true, 2_000_000_000, 9_500_000_000, &e, 4, Path::new("/v/cut.mp4")));
        assert!(a.starts_with("filesrc location=/v/My Rec.mp4 ! qtdemux name=d avwait name=w mode=running-time target-running-time=2000000000 end-running-time=9500000000"), "{a}");
        assert!(a.contains("w.vsrc ! identity ts-offset=-2000000000 ! videoconvert") && a.contains("x264enc"), "{a}");
        assert!(a.contains("w.asrc ! identity ts-offset=-2000000000 ! audioconvert ! avenc_aac bitrate=192000 ! aacparse ! queue ! mux."), "{a}");
        let a = joined(&trim_args(Path::new("/v/r.webm"), Format::WebM, false, 0, u64::MAX, &e, 4, Path::new("/v/c.webm")));
        assert!(a.contains("matroskademux") && a.contains("vp8enc") && !a.contains("audio"), "{a}");
    }

    #[test]
    fn explains_a_failure_by_its_error_line() {
        assert_eq!(explain("Setting pipeline...\nERROR: from element /x: Could not open resource.\nmore"), "from element /x: Could not open resource.");
        assert_eq!(explain(""), "the recording program stopped");
    }
}
