//! Reading QR codes (through the pure-Rust `rqrr` crate; nothing to install).
//!
//! Screens show QR codes small (2-4 pixels per module), smoothed, sometimes light on
//! dark. So a picture is read as it is, then, if that finds nothing, twice as large and
//! with its colours inverted. The same code found twice is given once.

use crate::Bitmap;

/// One QR code found in a picture.
#[derive(Clone, Debug, PartialEq)]
pub struct QrCode {
    /// What it holds. Bytes that are not UTF-8 are shown with replacement characters.
    pub text: String,
    /// Its four corners in the picture's pixels: top-left, top-right, bottom-right,
    /// bottom-left (as the code is printed, so they turn with it).
    pub corners: [(f64, f64); 4],
}

impl QrCode {
    /// The smallest upright rectangle around the code: `(x, y, width, height)`.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let xs = self.corners.iter().map(|c| c.0);
        let ys = self.corners.iter().map(|c| c.1);
        let (x0, x1) = xs.fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
        let (y0, y1) = ys.fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
        (x0, y0, x1 - x0, y1 - y0)
    }

    /// True when the text is a web or mail link (shown with an Open button).
    pub fn is_link(&self) -> bool {
        let t = self.text.trim().to_ascii_lowercase();
        (t.starts_with("https://") || t.starts_with("http://") || t.starts_with("mailto:")) && !t.contains(char::is_whitespace)
    }
}

/// Every QR code in the picture, top to bottom then left to right.
pub fn decode_qr(picture: &Bitmap) -> Vec<QrCode> {
    let mut found = read(picture, 1, false);
    if found.is_empty() {
        found = read(picture, 1, true);
    }
    // Small codes: twice as large (each pixel doubled) helps the module grid fit.
    if found.is_empty() && (picture.width as u64 * picture.height as u64) <= 12_000_000 {
        found = read(picture, 2, false);
        if found.is_empty() {
            found = read(picture, 2, true);
        }
    }
    let mut unique: Vec<QrCode> = Vec::new();
    for code in found {
        if !unique.iter().any(|u| u.text == code.text && overlaps(u, &code)) {
            unique.push(code);
        }
    }
    unique.sort_by(|a, b| {
        let (ax, ay, _, ah) = a.bounds();
        let (bx, by, _, _) = b.bounds();
        if (ay - by).abs() < ah / 2.0 { ax.total_cmp(&bx) } else { ay.total_cmp(&by) }
    });
    unique
}

fn overlaps(a: &QrCode, b: &QrCode) -> bool {
    let (ax, ay, aw, ah) = a.bounds();
    let (bx, by, bw, bh) = b.bounds();
    ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
}

/// Grey level of one pixel, transparent parts read as white (as they are shown).
fn grey(picture: &Bitmap, x: u32, y: u32) -> u8 {
    let [r, g, b, a] = picture.pixel(x, y);
    let lum = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;
    let a = a as u32;
    ((lum * a + 255 * (255 - a)) / 255) as u8
}

fn read(picture: &Bitmap, scale: u32, invert: bool) -> Vec<QrCode> {
    let (w, h) = (picture.width as usize * scale as usize, picture.height as usize * scale as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| {
        let v = grey(picture, (x / scale as usize) as u32, (y / scale as usize) as u32);
        if invert { 255 - v } else { v }
    });
    let mut out = Vec::new();
    for grid in prepared.detect_grids() {
        let mut bytes = Vec::new();
        if grid.decode_to(&mut bytes).is_err() {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let s = scale as f64;
        let corners = grid.bounds.map(|p| (p.x as f64 / s, p.y as f64 / s));
        out.push(QrCode { text, corners });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_png, Rect};

    const ONE: &[u8] = include_bytes!("../../../tests/fixtures/qr-one.png");
    const TWO: &[u8] = include_bytes!("../../../tests/fixtures/qr-two.png");

    #[test]
    fn reads_a_small_code_on_a_page() {
        let pic = decode_png(ONE).unwrap();
        let codes = decode_qr(&pic);
        assert_eq!(codes.len(), 1, "{codes:?}");
        assert_eq!(codes[0].text, "https://example.org/obtura");
        assert!(codes[0].is_link());
        // The code was placed at (30, 40) with a 4-module quiet zone of 3 px.
        let (x, y, w, h) = codes[0].bounds();
        assert!((x - 42.0).abs() < 4.0 && (y - 52.0).abs() < 4.0, "{x} {y}");
        assert!(w > 60.0 && h > 60.0);
    }

    #[test]
    fn reads_two_codes_left_to_right() {
        let pic = decode_png(TWO).unwrap();
        let texts: Vec<String> = decode_qr(&pic).into_iter().map(|c| c.text).collect();
        assert_eq!(texts, ["first code", "WIFI:S:Demo;T:WPA;P:not-a-real-secret;;"]);
    }

    #[test]
    fn reads_light_on_dark_and_finds_nothing_in_plain_pictures() {
        let mut pic = decode_png(ONE).unwrap();
        for p in pic.rgba.chunks_mut(4) {
            p[0] = 255 - p[0];
            p[1] = 255 - p[1];
            p[2] = 255 - p[2];
        }
        assert_eq!(decode_qr(&pic).len(), 1);
        assert!(decode_qr(&Bitmap::filled(300, 200, [255, 255, 255, 255])).is_empty());
        assert!(decode_qr(&Bitmap::filled(0, 0, [0; 4])).is_empty());
    }

    #[test]
    fn a_boxed_code_cannot_be_read() {
        // What Smart Redact and the box tool rely on: a hidden code is gone.
        let mut pic = decode_png(ONE).unwrap();
        crate::redact(&mut pic, Rect::new(30, 40, 120, 120), crate::Redaction::Box { rgb: [0, 0, 0] });
        assert!(decode_qr(&pic).is_empty());
    }

    #[test]
    fn links_are_recognised() {
        let code = |t: &str| QrCode { text: t.into(), corners: [(0.0, 0.0); 4] };
        assert!(code("https://a.b/c").is_link());
        assert!(code("mailto:x@y.z").is_link());
        assert!(!code("javascript:alert(1)").is_link());
        assert!(!code("https://a b").is_link());
        assert!(!code("WIFI:S:x;;").is_link());
    }
}
