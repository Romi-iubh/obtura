//! Trimming a recording: two sliders choose where it starts and where it ends, with the picture
//! at each point; then the recording is cut there and encoded again (exact to the frame). The
//! untrimmed recording goes to the wastebasket, from where it can be restored.

use adw::prelude::*;
use gtk::{gio, glib};
use obtura_capture::record::{self, Format};
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

/// The recording to trim.
#[derive(Clone, Debug)]
pub struct Source {
    pub path: PathBuf,
    pub format: Format,
    pub with_sound: bool,
    pub length: Duration,
}

/// "0:03.5", "12:40.0".
pub fn time_text(seconds: f64) -> String {
    let s = (seconds.max(0.0) * 10.0).round() / 10.0;
    let m = (s / 60.0).floor();
    format!("{}:{:04.1}", m as u64, s - m * 60.0)
}

/// How many preview pictures per second: about 150 in all, at most 10 a second.
fn pictures_per_second(length: f64) -> f64 {
    (150.0 / length.max(0.1)).clamp(0.2, 10.0)
}

/// The end to give the cutting: none when the end is the end of the recording.
fn end_ns(end: f64, length: f64) -> u64 {
    if end >= length - 0.05 {
        u64::MAX
    } else {
        (end * 1e9).round() as u64
    }
}

fn scratch_dir() -> PathBuf {
    thread_local!(static N: Cell<u32> = const { Cell::new(0) });
    let n = N.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    obtura_core::paths::cache_dir().join(format!("trim-{}-{n}", std::process::id()))
}

/// The preview pictures of a GIF, made here (GStreamer does not read GIFs everywhere).
fn gif_pictures(path: &Path, per_second: f64, dir: &Path) -> Result<f64, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let (mut t, mut next, mut n) = (0.0f64, 0.0f64, 0u32);
    obtura_image::gif::for_each_frame(&bytes, |picture, delay| {
        let end = t + delay as f64 / 100.0;
        while next < end {
            let w = 320.min(picture.width);
            let h = ((picture.height as f64 * w as f64 / picture.width as f64).round() as u32).max(1);
            let small = obtura_image::resize(picture, w, h);
            let png = obtura_image::encode_png(&small).map_err(|e| e.to_string())?;
            std::fs::write(dir.join(format!("thumb-{n:05}.png")), png).map_err(|e| e.to_string())?;
            n += 1;
            next = n as f64 / per_second;
        }
        t = end;
        Ok(true)
    })?;
    Ok(t)
}

/// Opens the Trim window over `parent`. `done(path, length)` is called after a trim, with the
/// trimmed recording (at the same place, unless the original could not go to the wastebasket).
pub fn open(parent: &impl IsA<gtk::Widget>, source: Source, done: impl Fn(PathBuf, Duration) + 'static) {
    let dir = scratch_dir();
    let _ = std::fs::create_dir_all(&dir);
    let length = Rc::new(Cell::new(source.length.as_secs_f64().max(0.2)));
    let per_second = pictures_per_second(length.get());

    let picture = |label: &str| {
        let p = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).width_request(250).height_request(150).css_classes(["card"]).build();
        let caption = gtk::Label::builder().label(label).css_classes(["dim-label"]).build();
        let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
        b.append(&p);
        b.append(&caption);
        (b, p, caption)
    };
    let (start_box, start_pic, _) = picture("Starts here");
    let (end_box, end_pic, _) = picture("Ends here");
    let previews = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    previews.set_halign(gtk::Align::Center);
    previews.append(&start_box);
    previews.append(&end_box);

    let len = length.get();
    let scale = |value: f64| {
        let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, len, 0.1);
        s.set_value(value);
        s.set_hexpand(true);
        s.set_draw_value(false);
        s.adjustment().set_page_increment(1.0);
        s
    };
    let start = scale(0.0);
    let end = scale(len);
    let start_time = gtk::Label::builder().label(time_text(0.0)).width_chars(8).xalign(1.0).css_classes(["numeric"]).build();
    let end_time = gtk::Label::builder().label(time_text(len)).width_chars(8).xalign(1.0).css_classes(["numeric"]).build();
    let grid = gtk::Grid::builder().row_spacing(6).column_spacing(12).build();
    grid.attach(&gtk::Label::builder().label("Start").xalign(0.0).build(), 0, 0, 1, 1);
    grid.attach(&start, 1, 0, 1, 1);
    grid.attach(&start_time, 2, 0, 1, 1);
    grid.attach(&gtk::Label::builder().label("End").xalign(0.0).build(), 0, 1, 1, 1);
    grid.attach(&end, 1, 1, 1, 1);
    grid.attach(&end_time, 2, 1, 1, 1);
    let summary = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).build();
    let busy = gtk::Spinner::builder().spinning(false).visible(false).build();

    let cancel = gtk::Button::builder().label("Cancel").css_classes(["pill"]).build();
    let trim = gtk::Button::builder().label("Trim").css_classes(["pill", "suggested-action"]).tooltip_text("Cut the recording here. The untrimmed recording goes to the wastebasket.").sensitive(false).build();
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    buttons.append(&cancel);
    buttons.append(&trim);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.append(&previews);
    body.append(&grid);
    body.append(&summary);
    body.append(&busy);
    body.append(&buttons);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&body));
    let dialog = adw::Dialog::builder().title("Trim the Recording").content_width(600).child(&view).build();

    // Cutting a video needs GStreamer's avwait (a GIF is cut here).
    let trimmable = source.format == Format::Gif || record::can_trim();
    // The pictures, once made: (count, pictures per second).
    let pictures: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let show = {
        let (start, end, start_pic, end_pic, start_time, end_time, summary, trim, pictures, dir, length) = (start.clone(), end.clone(), start_pic.clone(), end_pic.clone(), start_time.clone(), end_time.clone(), summary.clone(), trim.clone(), pictures.clone(), dir.clone(), length.clone());
        Rc::new(move || {
            let (a, b, len) = (start.value(), end.value(), length.get());
            start_time.set_label(&time_text(a));
            end_time.set_label(&time_text(b));
            let kept = (b - a).max(0.0);
            if trimmable {
                summary.remove_css_class("error");
                summary.set_label(&format!("Keeps {} to {}  ({:.1} s of {:.1} s)", time_text(a), time_text(b), kept, len));
            }
            trim.set_sensitive(trimmable && kept >= 0.2 && (a > 0.05 || b < len - 0.05));
            let n = pictures.get();
            if n > 0 {
                let at = |t: f64| dir.join(format!("thumb-{:05}.png", ((t * per_second).floor() as u32).min(n - 1)));
                start_pic.set_filename(Some(at(a)));
                end_pic.set_filename(Some(at((b - 0.05).max(0.0))));
            }
        })
    };
    show();
    {
        let (show, end) = (show.clone(), end.clone());
        start.connect_value_changed(move |s| {
            // The start stays before the end.
            if s.value() > end.value() - 0.2 {
                end.set_value((s.value() + 0.2).min(end.adjustment().upper()));
            }
            show();
        });
    }
    {
        let (show, start) = (show.clone(), start.clone());
        end.connect_value_changed(move |e| {
            if e.value() < start.value() + 0.2 {
                start.set_value((e.value() - 0.2).max(0.0));
            }
            show();
        });
    }

    // Make the preview pictures.
    {
        let (show, pictures, dir2, length2, end2, src) = (show.clone(), pictures.clone(), dir.clone(), length.clone(), end.clone(), source.clone());
        let count = move |dir: &Path| std::fs::read_dir(dir).map(|r| r.filter(|e| e.as_ref().is_ok_and(|e| e.file_name().to_string_lossy().starts_with("thumb-"))).count() as u32).unwrap_or(0);
        if source.format == Format::Gif {
            glib::spawn_future_local(async move {
                let d = dir2.clone();
                let path = src.path.clone();
                if let Ok(Ok(total)) = gio::spawn_blocking(move || gif_pictures(&path, per_second, &d)).await {
                    // The GIF knows its exact length.
                    length2.set(total);
                    end2.adjustment().set_upper(total);
                    end2.set_value(total);
                }
                pictures.set(count(&dir2));
                show();
            });
        } else {
            let args = record::thumbnail_args(&source.path, source.format, per_second, 320, &dir.join("thumb-%05d.png"));
            record::run(args, move |_| {
                pictures.set(count(&dir2));
                show();
            });
        }
    }

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let dir = dir.clone();
        dialog.connect_closed(move |_| {
            let _ = std::fs::remove_dir_all(&dir);
        });
    }
    if !trimmable {
        for c in [start.upcast_ref::<gtk::Widget>(), end.upcast_ref(), trim.upcast_ref()] {
            c.set_sensitive(false);
        }
        summary.add_css_class("error");
        summary.set_label("Trimming a video needs one more GStreamer package. Install it with:\nsudo apt install gstreamer1.0-plugins-bad");
        summary.set_selectable(true);
    }
    let done = Rc::new(done);
    let controls: Vec<gtk::Widget> = vec![start.clone().upcast(), end.clone().upcast(), trim.clone().upcast(), cancel.clone().upcast()];
    trim.connect_clicked({
        let (dialog, summary, busy, length, dir) = (dialog.clone(), summary.clone(), busy.clone(), length.clone(), dir.clone());
        move |_| {
            let (a, b, len) = (start.value(), end.value(), length.get());
            for c in &controls {
                c.set_sensitive(false);
            }
            busy.set_visible(true);
            busy.set_spinning(true);
            summary.set_label("Trimming…");
            let out = dir.join(format!("trimmed.{}", source.format.extension()));
            let original = source.path.clone();
            let finished = {
                let (dialog, summary, busy, controls, done, original, out) = (dialog.clone(), summary.clone(), busy.clone(), controls.clone(), done.clone(), original.clone(), out.clone());
                move |result: Result<(), String>| {
                    busy.set_spinning(false);
                    busy.set_visible(false);
                    match result.and_then(|_| replace(&original, &out)) {
                        Ok(path) => {
                            done(path, Duration::from_secs_f64((b - a).max(0.0)));
                            dialog.close();
                        }
                        Err(e) => {
                            for c in &controls {
                                c.set_sensitive(true);
                            }
                            summary.add_css_class("error");
                            summary.set_label(&format!("Could not trim it: {e}"));
                        }
                    }
                }
            };
            if source.format == Format::Gif {
                let out2 = out.clone();
                glib::spawn_future_local(async move {
                    let made = gio::spawn_blocking(move || -> Result<(), String> {
                        let bytes = std::fs::read(&original).map_err(|e| e.to_string())?;
                        let file = std::fs::File::create(&out2).map_err(|e| e.to_string())?;
                        let (mut w, _) = obtura_image::gif::trim(&bytes, a * 100.0, b * 100.0, std::io::BufWriter::new(file))?;
                        std::io::Write::flush(&mut w).map_err(|e| e.to_string())
                    })
                    .await;
                    finished(made.unwrap_or_else(|_| Err("trimming stopped".into())));
                });
            } else {
                let threads = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4).clamp(1, 8);
                let args = record::trim_args(&original, source.format, source.with_sound, (a * 1e9).round() as u64, end_ns(b, len), &crate::recorder::encoders(), threads, &out);
                record::run(args, finished);
            }
        }
    });
    dialog.present(Some(parent));
}

/// Puts the trimmed recording in place of the original, which goes to the wastebasket. If it
/// cannot, the trimmed one is saved beside it as "… (trimmed)".
fn replace(original: &Path, trimmed: &Path) -> Result<PathBuf, String> {
    if !std::fs::metadata(trimmed).is_ok_and(|m| m.len() > 0) {
        return Err("nothing was made".into());
    }
    let target = match gio::File::for_path(original).trash(gio::Cancellable::NONE) {
        Ok(()) => original.to_path_buf(),
        Err(_) => {
            let stem = original.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let ext = original.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            obtura_core::naming::unique_path(original.parent().unwrap_or(Path::new(".")), &format!("{stem} (trimmed)"), &ext)
        }
    };
    crate::recorder::move_file(trimmed, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_well() {
        assert_eq!(time_text(0.0), "0:00.0");
        assert_eq!(time_text(3.46), "0:03.5");
        assert_eq!(time_text(759.95), "12:40.0");
    }

    #[test]
    fn about_150_pictures_at_most_10_a_second() {
        assert_eq!(pictures_per_second(5.0), 10.0);
        assert_eq!(pictures_per_second(60.0), 2.5);
        assert_eq!(pictures_per_second(3600.0), 0.2);
    }

    #[test]
    fn the_end_of_the_recording_is_no_cut() {
        assert_eq!(end_ns(10.0, 10.0), u64::MAX);
        assert_eq!(end_ns(9.5, 10.0), 9_500_000_000);
    }
}
