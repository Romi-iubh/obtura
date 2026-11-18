//! Reading the picture: copy its text (OCR with tesseract), read its QR codes, copy a colour, and Smart Redact
//! (find what should not be shared and hide it in one click). Text reading and the colour
//! picker look at the picture with its redactions applied, so they never reveal what was hidden.

use crate::{editor::Request, icons, tools::Tool, window::Window};
use adw::prelude::*;
use gtk::{gio, glib};
use obtura_core::{
    ocr::{self, OcrError, Word},
    sensitive::{self, Finding},
};
use obtura_image::{Bitmap, Rect};
use obtura_scene::{Kind, RectF, RedactMode};
use std::{cell::RefCell, rc::Rc};

/// Reads the text of `picture` on another thread, then calls `then` back on this one.
pub fn read_text(picture: Bitmap, langs: String, then: impl FnOnce(Result<Vec<Word>, OcrError>) + 'static) {
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || ocr::recognize(&picture, &langs)).await;
        then(result.unwrap_or_else(|_| Err(OcrError::Failed("the text reader stopped".into()))));
    });
}

/// Reads the QR codes of `picture` on another thread, then calls `then` back on this one.
pub fn read_qr(picture: Bitmap, then: impl FnOnce(Vec<obtura_image::QrCode>) + 'static) {
    glib::spawn_future_local(async move {
        let codes = gio::spawn_blocking(move || obtura_image::decode_qr(&picture)).await;
        then(codes.unwrap_or_default());
    });
}

/// Read QR Codes (editor): the picture as seen, with what was hidden blanked, so a code
/// under a blur or a box is never read. One code is copied at once; several are listed.
pub fn grab_qr(w: &Rc<Window>) {
    let reading = adw::Toast::new("Looking for QR codes…");
    reading.set_timeout(0);
    w.toasts.add_toast(reading.clone());
    let wk = Rc::downgrade(w);
    read_qr(w.editor.picture_to_read(), move |codes| {
        reading.dismiss();
        let Some(w) = wk.upgrade() else { return };
        match codes.len() {
            0 => w.toast("No QR code was found in the picture"),
            1 => {
                copy_text(&w, &codes[0].text);
                let toast = adw::Toast::new(&format!("Copied the QR code: “{}”", preview(&codes[0].text)));
                if codes[0].is_link() {
                    toast.set_button_label(Some("Open Link"));
                    let (link, win) = (codes[0].text.trim().to_string(), w.win.clone());
                    toast.connect_button_clicked(move |_| open_link(&win, &link));
                }
                w.toasts.add_toast(toast);
            }
            _ => {
                let wk = Rc::downgrade(&w);
                show_codes(&w.win, &codes, Rc::new(move |text| {
                    if let Some(w) = wk.upgrade() {
                        copy_text(&w, text);
                    }
                }));
            }
        }
    });
}

/// Opens a link read from a QR code in the web browser (the user clicked Open Link after
/// reading the whole address). Obtura itself never goes online.
pub fn open_link(parent: &impl IsA<gtk::Window>, link: &str) {
    gtk::UriLauncher::new(link).launch(Some(parent), gio::Cancellable::NONE, |_| {});
}

/// The codes found, each with Copy (and Open Link for a link), in a dialog.
pub fn show_codes(parent: &impl IsA<gtk::Widget>, codes: &[obtura_image::QrCode], copy: Rc<dyn Fn(&str)>) {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let window = parent.as_ref().root().and_downcast::<gtk::Window>();
    for code in codes {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let text = gtk::Label::builder().label(&code.text).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).selectable(true).xalign(0.0).hexpand(true).max_width_chars(40).build();
        row.append(&text);
        if code.is_link() {
            let open = gtk::Button::builder().label("Open Link").valign(gtk::Align::Center).build();
            let (link, window) = (code.text.trim().to_string(), window.clone());
            open.connect_clicked(move |_| {
                if let Some(win) = &window {
                    open_link(win, &link);
                }
            });
            row.append(&open);
        }
        let copy_button = gtk::Button::builder().label("Copy").valign(gtk::Align::Center).build();
        let (text, copy) = (code.text.clone(), copy.clone());
        copy_button.connect_clicked(move |b| {
            copy(&text);
            b.set_label("Copied");
        });
        row.append(&copy_button);
        list.append(&row);
    }
    let dialog = adw::AlertDialog::new(Some(&format!("{} QR codes found", codes.len())), None);
    dialog.set_extra_child(Some(&list));
    dialog.add_response("close", "Close");
    dialog.present(Some(parent));
}

/// The first words of `text`, for a message.
pub fn preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 48 {
        format!("{}…", flat.chars().take(47).collect::<String>())
    } else {
        flat
    }
}

/// How many lines, in words.
pub fn lines_said(text: &str) -> String {
    match text.lines().filter(|l| !l.trim().is_empty()).count() {
        1 => "1 line".into(),
        n => format!("{n} lines"),
    }
}

/// Explains why text could not be read: tesseract missing gets the command to install it.
fn explain(w: &Rc<Window>, e: &OcrError) {
    match e {
        OcrError::Missing => {
            let dialog = adw::AlertDialog::new(Some("Text reading is not installed"), Some("Obtura reads text with the tesseract program, which is not on this computer. Install it from a terminal with:\n\nsudo apt install tesseract-ocr"));
            dialog.add_responses(&[("copy", "Copy the Command"), ("ok", "OK")]);
            dialog.set_default_response(Some("ok"));
            let win = w.win.clone();
            dialog.connect_response(Some("copy"), move |_, _| win.clipboard().set_text("sudo apt install tesseract-ocr"));
            dialog.present(Some(&w.win));
        }
        OcrError::Failed(_) => w.toast(&e.to_string()),
    }
}

/// Puts text on the clipboard. The window then stops keeping its picture there.
fn copy_text(w: &Rc<Window>, text: &str) {
    w.win.clipboard().set_text(text);
    w.clip_ours.set(false);
}

/// Copy Text: reads the text in `area` (picture pixels) and copies it.
pub fn grab_text(w: &Rc<Window>, area: Rect) {
    let picture = w.editor.picture_to_read();
    let Some(part) = picture.crop(area) else { return };
    let reading = adw::Toast::new("Reading the text…");
    reading.set_timeout(0);
    w.toasts.add_toast(reading.clone());
    let wk = Rc::downgrade(w);
    read_text(part, obtura_core::settings::Settings::load().ocr_languages, move |result| {
        reading.dismiss();
        let Some(w) = wk.upgrade() else { return };
        match result {
            Ok(words) => {
                let text = ocr::text(&words);
                if text.is_empty() {
                    w.toast("No text was found there");
                } else {
                    copy_text(&w, &text);
                    w.toast(&format!("Copied {}: “{}”", lines_said(&text), preview(&text)));
                }
            }
            Err(e) => explain(&w, &e),
        }
    });
}

pub fn install(w: &Rc<Window>) {
    let wk = Rc::downgrade(w);
    w.editor.connect_request(move |r| {
        let Some(w) = wk.upgrade() else { return };
        match r {
            Request::GrabText(area) => grab_text(&w, *area),
            Request::PickColor(c) => {
                let said = obtura_core::color::format([c[0], c[1], c[2]], obtura_core::settings::Settings::load().color_format);
                copy_text(&w, &said);
                w.toast(&format!("Copied the colour {said}"));
            }
        }
    });
}

// ---- Smart Redact ----

/// The panel's state: what was found, in which picture, and which are ticked.
#[derive(Default)]
struct Found {
    /// The picture the findings were made in (the editor's picture as seen).
    picture: Option<Rc<Bitmap>>,
    findings: Vec<Finding>,
    ticked: Vec<bool>,
    mode: Option<RedactMode>,
}

/// The Smart Redact button and its panel.
pub fn smart_redact_button(w: &Rc<Window>) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .child(&icons::image("redact-smart"))
        .tooltip_text("Smart Redact (Ctrl+Shift+X): finds email addresses, phone numbers, card numbers, IBANs, IP addresses, keys, passwords and QR codes, and hides them")
        .css_classes(["flat"])
        .build();
    let popover = gtk::Popover::new();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(10);
    body.set_margin_end(10);
    body.set_width_request(340);
    popover.set_child(Some(&body));
    button.set_popover(Some(&popover));
    let state = Rc::new(RefCell::new(Found::default()));

    let wk = Rc::downgrade(w);
    {
        let (body, state, popover) = (body.clone(), state.clone(), popover.clone());
        popover.clone().connect_show(move |_| {
            let Some(w) = wk.upgrade() else { return };
            // Looked for again only when the picture changed since last time.
            let picture = w.editor.seen_picture();
            let fresh = state.borrow().picture.as_ref().is_some_and(|p| Rc::ptr_eq(p, &picture));
            if fresh {
                show_findings(&w, &body, &state, &popover);
                return;
            }
            clear(&body);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let spinner = gtk::Spinner::new();
            spinner.start();
            row.append(&spinner);
            row.append(&gtk::Label::new(Some("Looking for things to hide…")));
            body.append(&row);
            let (wk, body, state, popover) = (Rc::downgrade(&w), body.clone(), state.clone(), popover.clone());
            let (to_read, langs) = (w.editor.picture_to_read(), obtura_core::settings::Settings::load().ocr_languages);
            glib::spawn_future_local(async move {
                // The text and the QR codes, read on one other thread.
                let result = gio::spawn_blocking(move || ocr::recognize(&to_read, &langs).map(|words| (words, obtura_image::decode_qr(&to_read))))
                    .await
                    .unwrap_or_else(|_| Err(OcrError::Failed("the text reader stopped".into())));
                let Some(w) = wk.upgrade() else { return };
                match result {
                    Ok((words, codes)) => {
                        let mut findings = sensitive::find(&words);
                        findings.extend(sensitive::qr_findings(&codes));
                        let mut st = state.borrow_mut();
                        st.ticked = vec![true; findings.len()];
                        st.findings = findings;
                        st.picture = Some(picture);
                        drop(st);
                        if popover.is_visible() {
                            show_findings(&w, &body, &state, &popover);
                        }
                    }
                    Err(e) => {
                        popover.popdown();
                        explain(&w, &e);
                    }
                }
            });
        });
    }
    let wk = Rc::downgrade(w);
    popover.connect_closed(move |_| {
        if let Some(w) = wk.upgrade() {
            w.editor.show_proposals(Vec::new());
        }
    });
    button
}

fn clear(body: &gtk::Box) {
    while let Some(c) = body.first_child() {
        body.remove(&c);
    }
}

/// A finding's area with a margin, so no edge of a letter shows, inside the picture.
fn padded(f: &Finding, (pw, ph): (u32, u32)) -> RectF {
    let (x, y, w, h) = f.rect;
    let m = (h as f64 / 6.0).max(2.0);
    let x0 = (x as f64 - m).max(0.0);
    let y0 = (y as f64 - m).max(0.0);
    let x1 = (x as f64 + w as f64 + m).min(pw as f64);
    let y1 = (y as f64 + h as f64 + m).min(ph as f64);
    // Whole pixels that hold all of it: the left and top edges go down, the right and bottom up.
    RectF { x: x0.floor(), y: y0.floor(), w: x1.ceil() - x0.floor(), h: y1.ceil() - y0.floor() }
}

fn show_findings(w: &Rc<Window>, body: &gtk::Box, state: &Rc<RefCell<Found>>, popover: &gtk::Popover) {
    clear(body);
    let size = w.editor.size();
    let (findings, ticked) = {
        let st = state.borrow();
        (st.findings.clone(), st.ticked.clone())
    };
    let outline = {
        let (w, state) = (Rc::downgrade(w), state.clone());
        move || {
            let Some(w) = w.upgrade() else { return };
            let st = state.borrow();
            let areas = st.findings.iter().zip(&st.ticked).filter(|(_, t)| **t).map(|(f, _)| padded(f, size)).collect();
            w.editor.show_proposals(areas);
        }
    };
    if findings.is_empty() {
        let title = gtk::Label::builder().label("Nothing to hide was found").halign(gtk::Align::Start).css_classes(["heading"]).build();
        let what = gtk::Label::builder()
            .label("Obtura looked for email addresses, phone numbers, card numbers, IBANs, IP addresses, keys and passwords in the text of the picture, and for QR codes. Check it yourself too: hide anything else with Blur (B), Pixelate (X) or Box (O).")
            .wrap(true)
            .max_width_chars(40)
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        body.append(&title);
        body.append(&what);
        outline();
        return;
    }
    let title = gtk::Label::builder().label(format!("Found {} to hide", count_said(findings.len()))).halign(gtk::Align::Start).css_classes(["heading"]).build();
    body.append(&title);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let hide = gtk::Button::builder().css_classes(["suggested-action", "pill"]).build();
    let set_label = {
        let (hide, state) = (hide.clone(), state.clone());
        move || {
            let n = state.borrow().ticked.iter().filter(|t| **t).count();
            hide.set_label(&if n == 0 { "Nothing Ticked".to_string() } else { format!("Hide {}", count_said(n)) });
            hide.set_sensitive(n > 0);
        }
    };
    for (i, f) in findings.iter().enumerate() {
        let check = gtk::CheckButton::new();
        check.set_active(ticked[i]);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&gtk::Label::builder().label(capitalised(f.kind.name(1))).css_classes(["dim-label"]).width_chars(16).xalign(0.0).build());
        row.append(&gtk::Label::builder().label(preview(&f.text)).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(26).xalign(0.0).build());
        check.set_child(Some(&row));
        let (state, outline, set_label) = (state.clone(), outline.clone(), set_label.clone());
        check.connect_toggled(move |c| {
            if let Some(t) = state.borrow_mut().ticked.get_mut(i) {
                *t = c.is_active();
            }
            outline();
            set_label();
        });
        list.append(&check);
    }
    let scroller = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).max_content_height(260).build();
    body.append(&scroller);

    // How to hide: a box (default), blur or pixelate.
    let how = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    how.add_css_class("linked");
    let mut first: Option<gtk::ToggleButton> = None;
    let current = state.borrow().mode.unwrap_or(RedactMode::Box);
    for (mode, label) in [(RedactMode::Box, "Box"), (RedactMode::Blur, "Blur"), (RedactMode::Pixelate, "Pixelate")] {
        let b = gtk::ToggleButton::with_label(label);
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        b.set_active(mode == current);
        let state = state.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                state.borrow_mut().mode = Some(mode);
            }
        });
        how.append(&b);
    }
    let how_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    how_row.append(&gtk::Label::builder().label("Hide with").hexpand(true).halign(gtk::Align::Start).build());
    how_row.append(&how);
    body.append(&how_row);
    set_label();
    body.append(&hide);
    outline();
    // Enter hides everything ticked.
    hide.grab_focus();

    let (wk, state, popover) = (Rc::downgrade(w), state.clone(), popover.clone());
    hide.connect_clicked(move |_| {
        let Some(w) = wk.upgrade() else { return };
        let (marks, n) = {
            let st = state.borrow();
            let mode = st.mode.unwrap_or(RedactMode::Box);
            let tool = match mode {
                RedactMode::Box => Tool::Box,
                RedactMode::Blur => Tool::Blur,
                RedactMode::Pixelate => Tool::Pixelate,
            };
            let style = w.editor.style_of(tool);
            let marks: Vec<_> = st.findings.iter().zip(&st.ticked).filter(|(_, t)| **t).map(|(f, _)| (Kind::Redact { rect: padded(f, size), mode }, style)).collect();
            let n = marks.len();
            (marks, n)
        };
        popover.popdown();
        w.editor.add_marks(marks);
        w.toast(&format!("Hid {}. Undo with Ctrl+Z", count_said(n)));
    });
}

fn count_said(n: usize) -> String {
    if n == 1 {
        "1 item".into()
    } else {
        format!("{n} items")
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_text_is_shortened_for_a_message() {
        assert_eq!(preview("short\ntext"), "short text");
        let long = "word ".repeat(30);
        assert!(preview(&long).ends_with('…') && preview(&long).chars().count() == 48);
        assert_eq!(lines_said("a\n\nb\nc"), "3 lines");
    }

    #[test]
    fn a_hidden_area_has_a_margin_and_stays_in_the_picture() {
        let f = Finding { kind: sensitive::Kind::Email, text: "a@b.co".into(), rect: (1, 10, 50, 18) };
        let r = padded(&f, (200, 100));
        assert_eq!(r, RectF { x: 0.0, y: 7.0, w: 54.0, h: 24.0 });
    }
}
