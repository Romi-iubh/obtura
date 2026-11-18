//! Icons drawn as stroke SVGs, coloured at runtime for the light or dark style.
//! (GTK's symbolic recolouring only recolours fills, which would break stroke icons.)
//! Every icon is a 16x16 drawing; `fill="C"` means "fill with the icon colour".

use adw::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;

fn body(name: &str) -> &'static str {
    match name {
        "select" => r##"<path d="M3 2l9 6-4 1 2 4.5-1.8.8-2-4.5L3 12z" fill="C"/>"##,
        "arrow" => r##"<path d="M2.5 13.5l11-11M7 2.5h6.5V9"/>"##,
        // The Line and Curve tools draw plain lines and arrows: each icon shows one of each.
        "line" => r##"<path d="M1.5 9.5l7-7M6.5 14.5l7.5-7.5M9.5 6.8h4.5v4.5"/>"##,
        "spline" => r##"<path d="M1.5 9C1.8 5 4.5 2.2 8.8 2M5.5 14.5C5.8 10.5 8.5 7.6 13.8 7.5M11.3 5.3l2.6 2.2-2.7 2.1"/>"##,
        "callout" => r##"<rect x="7" y="2.5" width="7.5" height="5.5" rx="1"/><path d="M7 5.5H4.5M2 12.5l3-3.5M2 12.5l.2-2.6M2 12.5l2.4-.9"/>"##,
        "new" => r##"<path d="M2 5V2h3M11 2h3v3M14 11v3h-3M5 14H2v-3M8 5.5v5M5.5 8h5"/>"##,
        "rect" => r##"<rect x="2" y="3.5" width="12" height="9" rx="0.5"/>"##,
        "ellipse" => r##"<ellipse cx="8" cy="8" rx="6" ry="4.5"/>"##,
        "pen" => r##"<path d="M11 2l3 3-8 8-4 1 1-4z"/><path d="M9.5 3.5l3 3"/>"##,
        "highlighter" => r##"<path d="M9.5 2.5l4 4-6 6H3.5v-4z"/><path d="M2 14.5h12" stroke="#f5c211" stroke-width="2.5"/>"##,
        "text" => r##"<path d="M3 3.5V2h10v1.5M8 2v12M6 14h4"/>"##,
        "step" => r##"<circle cx="8" cy="8" r="6"/><path d="M6.4 6.6L8.4 5.2V11"/>"##,
        "stamp" => r##"<path d="M8 1.8l1.8 3.9 4.2.5-3.1 2.9.8 4.2L8 11.2l-3.7 2.1.8-4.2L2 6.2l4.2-.5z"/>"##,
        "spotlight" => r##"<path fill="C" fill-opacity=".4" fill-rule="evenodd" stroke="none" d="M1.5 2.5h13v11h-13zM11.5 8a3.5 3.5 0 1 1-7 0a3.5 3.5 0 1 1 7 0z"/><circle cx="8" cy="8" r="3.5"/><rect x="1.5" y="2.5" width="13" height="11" rx="1"/>"##,
        "magnify" => r##"<circle cx="6.5" cy="6.5" r="4.5"/><path d="M10 10l4.5 4.5M4.5 6.5h4M6.5 4.5v4"/>"##,
        "picture" => r##"<rect x="1.5" y="2.5" width="13" height="11" rx="1"/><circle cx="5.5" cy="6" r="1.3"/><path d="M1.5 12l4-4 3 3 2-2 4 4"/>"##,
        "image" => r##"<rect x="1.5" y="2.5" width="13" height="11" rx="1"/><path d="M4 2.5v11M12 2.5v11"/>"##,
        "beautify" => r##"<path d="M8 1.5l1.3 3.2 3.2 1.3-3.2 1.3L8 10.5 6.7 7.3 3.5 6l3.2-1.3zM12.5 10l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7zM3.5 10.5l.5 1.3 1.3.5-1.3.5-.5 1.3-.5-1.3-1.3-.5 1.3-.5z"/>"##,
        "cut" => r##"<circle cx="4" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><path d="M5.4 10.6L12 2.5M10.6 10.6L4 2.5M1 7.5h2M5 7.5h2M9 7.5h2M13 7.5h2"/>"##,
        "blur" => r##"<path d="M8 2.5s-4.5 4.8-4.5 7.5a4.5 4.5 0 009 0C12.5 7.3 8 2.5 8 2.5z"/>"##,
        "pixelate" => r##"<path d="M2 2h4v4H2zM10 2h4v4h-4zM6 6h4v4H6zM2 10h4v4H2zM10 10h4v4h-4z"/>"##,
        "box" => r##"<rect x="1.5" y="5" width="13" height="6" rx="0.5" fill="C"/>"##,
        "crop" => r##"<path d="M4 1.5V12h10.5M1.5 4H12v10.5"/>"##,
        "ocr" => r##"<path d="M1.5 4.5v-3h3M11.5 1.5h3v3M14.5 11.5v3h-3M4.5 14.5h-3v-3M5 5h6M8 5v6.5"/>"##,
        // Three finder squares of a QR code and a few modules.
        // A page taller than its frame, with an arrow down.
        "scroll" => r##"<rect x="2.5" y="3.5" width="8" height="9" rx="1"/><path d="M4.5 1.5h4M4.5 6h4M4.5 8.5h3M13.5 4v9M11.8 11.3l1.7 1.7 1.7-1.7"/>"##,
        "qr" => r##"<rect x="1.5" y="1.5" width="5" height="5" rx="0.5"/><rect x="9.5" y="1.5" width="5" height="5" rx="0.5"/><rect x="1.5" y="9.5" width="5" height="5" rx="0.5"/><path d="M3.5 3.5h1v1h-1zM11.5 3.5h1v1h-1zM3.5 11.5h1v1h-1z" fill="C"/><path d="M9.5 9.5h2v2M14.5 9.5v1.5M12.5 14.5h2M9.5 13.5v1"/>"##,
        "picker" => r##"<path d="M10.2 2.3a1.8 1.8 0 012.5 0l1 1a1.8 1.8 0 010 2.5L12 7.5 8.5 4z"/><path d="M9.5 5L3 11.5l-.5 2 2-.5L11 6.5"/>"##,
        "measure" => r##"<path d="M1.5 5.5v5M14.5 5.5v5M1.5 8h13M4.5 8v2M7 8v1.5M9.5 8v2M12 8v1.5"/>"##,
        "redact-smart" => r##"<rect x="1.5" y="8" width="9" height="5" rx="0.5" fill="C"/><path d="M12 1.5l.6 1.4 1.4.6-1.4.6-.6 1.4-.6-1.4-1.4-.6 1.4-.6zM9 3.5l-4.5 3"/>"##,
        "pin" => r##"<path d="M9.5 1.8l4.7 4.7-1.6.5-2.6 2.6-.3 3-1.3 1.3-6.4-6.4 1.3-1.3 3-.3L9 3.4zM5 11l-3.5 3.5"/>"##,
        "library" => r##"<rect x="1.5" y="1.5" width="5.5" height="5.5" rx="1"/><rect x="9" y="1.5" width="5.5" height="5.5" rx="1"/><rect x="1.5" y="9" width="5.5" height="5.5" rx="1"/><rect x="9" y="9" width="5.5" height="5.5" rx="1"/>"##,
        "undo" => r##"<path d="M5 3L2.5 5.5 5 8M2.5 5.5H10a3.5 3.5 0 010 7H6"/>"##,
        "redo" => r##"<path d="M11 3l2.5 2.5L11 8M13.5 5.5H6a3.5 3.5 0 000 7h4"/>"##,
        "copy" => r##"<rect x="5.5" y="5.5" width="8" height="8" rx="1"/><path d="M3.5 10.5h-1v-8h8v1"/>"##,
        "save" => r##"<path d="M8 2v8M4.5 7L8 10.5 11.5 7M2.5 13.5h11"/>"##,
        "region" => r##"<path d="M2 5V2h3M11 2h3v3M14 11v3h-3M5 14H2v-3"/>"##,
        "screen" => r##"<rect x="1.5" y="2.5" width="13" height="8.5" rx="1"/><path d="M5.5 14h5M8 11v3"/>"##,
        "record" => r##"<circle cx="8" cy="8" r="6"/><circle cx="8" cy="8" r="3" fill="C"/>"##,
        _ => r##"<circle cx="8" cy="8" r="5"/>"##,
    }
}

fn texture(name: &str, color: &str) -> Option<gdk::Texture> {
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 16 16"><g fill="none" stroke="{color}" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round">{}</g></svg>"##,
        body(name).replace("fill=\"C\"", &format!("fill=\"{color}\""))
    );
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(svg.into_bytes())).ok()
}

thread_local! {
    static IMAGES: RefCell<Vec<(glib::WeakRef<gtk::Image>, &'static str)>> = const { RefCell::new(Vec::new()) };
}

fn fg() -> &'static str {
    if adw::StyleManager::default().is_dark() {
        "#ffffff"
    } else {
        "#2e3436"
    }
}

/// An image showing the named icon; it follows light/dark changes.
pub fn image(name: &'static str) -> gtk::Image {
    let img = gtk::Image::new();
    img.set_pixel_size(16);
    if let Some(t) = texture(name, fg()) {
        img.set_paintable(Some(&t));
    }
    IMAGES.with(|v| v.borrow_mut().push((img.downgrade(), name)));
    img
}

/// A flat button showing the named icon.
pub fn button(name: &'static str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_child(Some(&image(name)));
    b.set_tooltip_text(Some(tooltip));
    b
}

pub fn init() {
    // While developing, the app icon comes from `data/icons` (installed, from the system).
    if cfg!(debug_assertions) {
        if let Some(display) = gdk::Display::default() {
            gtk::IconTheme::for_display(&display).add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../data/icons"));
        }
    }
    adw::StyleManager::default().connect_dark_notify(|_| {
        let c = fg();
        IMAGES.with(|v| {
            v.borrow_mut().retain(|(w, name)| match w.upgrade() {
                Some(img) => {
                    if let Some(t) = texture(name, c) {
                        img.set_paintable(Some(&t));
                    }
                    true
                }
                None => false,
            })
        });
    });
}
