//! Beautify: a frame around the screenshot (background colour, gradient or picture, padding,
//! rounded corners, shadow, border, a window title bar, a device around it, a torn bottom
//! edge, an aspect ratio). It belongs to the document (one undo step per change) and is
//! applied on export, around the finished picture.

use crate::geometry::{Point, RectF};
use crate::object::Color;
use obtura_image::Bitmap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Background {
    /// Transparent: only the picture (and its shadow) in the saved file.
    None,
    Solid(Color),
    /// From the top-left corner to the bottom-right corner.
    Gradient(Color, Color),
    /// A picture, covering the whole background (cut to fit, never stretched). The frame keeps
    /// only its key; the pixels are in the picture store (see [`register_picture`]).
    Picture(PictureKey),
}

/// Names a background picture by its content: the same pixels always give the same key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PictureKey(pub [u8; 16]);

impl PictureKey {
    /// The key of these pixels (two FNV-1a hashes, size included: no two pictures in a
    /// person's own collection share one in practice).
    pub fn of(b: &Bitmap) -> PictureKey {
        let mut h1: u64 = 0xcbf2_9ce4_8422_2325;
        let mut h2: u64 = 0x8422_2325_cbf2_9ce4;
        for (i, byte) in b.width.to_le_bytes().iter().chain(b.height.to_le_bytes().iter()).chain(b.rgba.iter()).enumerate() {
            h1 = (h1 ^ *byte as u64).wrapping_mul(0x0100_0000_01b3);
            h2 = (h2 ^ (*byte as u64).wrapping_add(i as u64 & 0xff)).wrapping_mul(0x0000_0100_0000_01b3 ^ 0x2_0000);
        }
        let mut k = [0u8; 16];
        k[..8].copy_from_slice(&h1.to_le_bytes());
        k[8..].copy_from_slice(&h2.to_le_bytes());
        PictureKey(k)
    }

    /// 32 hexadecimal digits: also the picture's file name in the store.
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Option<PictureKey> {
        if s.len() != 32 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let mut k = [0u8; 16];
        for (i, byte) in k.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(PictureKey(k))
    }
}

impl std::fmt::Debug for PictureKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PictureKey({})", self.hex())
    }
}

impl Serialize for PictureKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for PictureKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        PictureKey::from_hex(&s).ok_or_else(|| serde::de::Error::custom("not a picture key"))
    }
}

type Loader = Box<dyn Fn(&PictureKey) -> Option<Bitmap> + Send + Sync>;

fn store() -> &'static Mutex<(HashMap<PictureKey, Arc<Bitmap>>, Option<Loader>)> {
    static STORE: OnceLock<Mutex<(HashMap<PictureKey, Arc<Bitmap>>, Option<Loader>)>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new((HashMap::new(), None)))
}

/// Keeps a background picture in memory and returns its key.
pub fn register_picture(b: Bitmap) -> PictureKey {
    let key = PictureKey::of(&b);
    if let Ok(mut s) = store().lock() {
        s.0.insert(key, Arc::new(b));
    }
    key
}

/// Where pictures not in memory are found (the app reads them from its picture store).
pub fn set_picture_loader(f: impl Fn(&PictureKey) -> Option<Bitmap> + Send + Sync + 'static) {
    if let Ok(mut s) = store().lock() {
        s.1 = Some(Box::new(f));
    }
}

/// The pixels of a background picture, if known.
pub fn picture(key: &PictureKey) -> Option<Arc<Bitmap>> {
    let mut s = store().lock().ok()?;
    if let Some(b) = s.0.get(key) {
        return Some(b.clone());
    }
    let b = Arc::new(s.1.as_ref()?(key)?);
    s.0.insert(*key, b.clone());
    Some(b)
}

/// A device drawn around the picture, as if it were on its screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Device {
    #[default]
    None,
    Phone,
    Tablet,
    Laptop,
    Monitor,
}

impl Device {
    pub const ALL: [Device; 5] = [Device::None, Device::Phone, Device::Tablet, Device::Laptop, Device::Monitor];

    pub fn name(self) -> &'static str {
        match self {
            Device::None => "None",
            Device::Phone => "Phone",
            Device::Tablet => "Tablet",
            Device::Laptop => "Laptop",
            Device::Monitor => "Monitor",
        }
    }

    /// The unit the device is drawn in: its bezel's width for a screen `w` x `h`.
    pub fn unit(w: f64, h: f64) -> f64 {
        (w.min(h) * 0.04).clamp(8.0, 40.0).round()
    }

    /// The bezel around the screen: left, top, right, bottom.
    pub fn bezel(self, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let u = Self::unit(w, h);
        match self {
            Device::None => (0.0, 0.0, 0.0, 0.0),
            Device::Phone => (u, (u * 2.2).round(), u, (u * 2.2).round()),
            Device::Tablet => ((u * 1.6).round(), (u * 1.6).round(), (u * 1.6).round(), (u * 1.6).round()),
            Device::Laptop => (u, (u * 1.4).round(), u, u),
            Device::Monitor => ((u * 0.8).round(), (u * 0.8).round(), (u * 0.8).round(), (u * 1.6).round()),
        }
    }

    /// Everything the device adds around the screen (bezel, and a laptop's base or a
    /// monitor's stand): left, top, right, bottom.
    pub fn margins(self, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let u = Self::unit(w, h);
        let (l, t, r, b) = self.bezel(w, h);
        match self {
            Device::Laptop => (l + u * 4.0, t, r + u * 4.0, b + (u * 1.3).round()),
            Device::Monitor => (l, t, r, b + (u * 4.9).round()),
            _ => (l, t, r, b),
        }
    }

    /// Corner radius of the device's body.
    pub fn body_corner(self, w: f64, h: f64) -> f64 {
        let u = Self::unit(w, h);
        match self {
            Device::None => 0.0,
            Device::Phone => u * 3.0,
            Device::Tablet => u * 2.0,
            Device::Laptop | Device::Monitor => u * 0.8,
        }
    }

    /// Corner radius of the screen (the picture).
    pub fn screen_corner(self, w: f64, h: f64) -> f64 {
        let u = Self::unit(w, h);
        match self {
            Device::Phone => u * 1.6,
            Device::Tablet => u * 0.6,
            _ => 0.0,
        }
    }
}

/// A window-style bar above the picture, with the three round buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bar {
    #[default]
    None,
    Light,
    Dark,
    /// A light bar with an address field, like a web browser.
    Browser,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Frame {
    pub background: Background,
    /// Space between the picture and the edge of the result, in picture pixels.
    pub padding: f64,
    /// Corner radius of the picture (with its bar).
    pub corner: f64,
    /// Size of the soft shadow under the picture; 0 for none.
    pub shadow: f64,
    /// Width of a line around the picture; 0 for none.
    pub border: f64,
    pub border_color: Color,
    pub bar: Bar,
    /// The bottom edge of the picture is torn, like a strip of paper.
    pub torn: bool,
    /// Width and height the result is made to fit (by adding background), or `None`.
    pub ratio: Option<(u32, u32)>,
    /// A device around the picture (phone, tablet, laptop, monitor).
    pub device: Device,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            background: Background::Gradient(Color::rgb(0x62, 0x5e, 0xe8), Color::rgb(0xd1, 0x5c, 0xb5)),
            padding: 56.0,
            corner: 12.0,
            shadow: 28.0,
            border: 0.0,
            border_color: Color::rgb(0x3d, 0x38, 0x46),
            bar: Bar::None,
            torn: false,
            ratio: None,
            device: Device::None,
        }
    }
}

/// Where everything goes, in pixels of the result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameLayout {
    pub width: u32,
    pub height: u32,
    /// The top-left corner of the picture in the result.
    pub pic: Point,
    /// The picture with its bar: what gets the corners, border and shadow.
    pub card: RectF,
    /// Height of the bar (0 without one).
    pub bar_h: f64,
    /// The device's body around the card (the card itself without a device).
    pub body: RectF,
}

/// Depth of the teeth of a torn edge.
pub const TORN_DEPTH: f64 = 10.0;

impl Frame {
    pub fn bar_height(&self) -> f64 {
        match self.bar {
            Bar::None => 0.0,
            Bar::Browser => 44.0,
            _ => 34.0,
        }
    }

    /// The layout for a `w` x `h` picture. The picture always lands on whole pixels.
    pub fn layout(&self, w: u32, h: u32) -> FrameLayout {
        let bar_h = self.bar_height();
        let pad = self.padding.max(0.0).round();
        let card_w = w as f64;
        let card_h = h as f64 + bar_h;
        let (ml, mt, mr, mb) = self.device.margins(card_w, card_h);
        let (outer_w, outer_h) = (card_w + ml + mr, card_h + mt + mb);
        let mut total_w = outer_w + 2.0 * pad;
        let mut total_h = outer_h + 2.0 * pad;
        if let Some((rw, rh)) = self.ratio.filter(|(a, b)| *a > 0 && *b > 0) {
            let r = rw as f64 / rh as f64;
            if total_w / total_h < r {
                total_w = (total_h * r).ceil();
            } else {
                total_h = (total_w / r).ceil();
            }
        }
        let (width, height) = (total_w.ceil() as u32, total_h.ceil() as u32);
        let cx = ((width as f64 - outer_w) / 2.0).floor() + ml;
        let cy = ((height as f64 - outer_h) / 2.0).floor() + mt;
        let card = RectF { x: cx, y: cy, w: card_w, h: card_h };
        let (bl, bt, br, bb) = self.device.bezel(card_w, card_h);
        let body = RectF { x: cx - bl, y: cy - bt, w: card_w + bl + br, h: card_h + bt + bb };
        FrameLayout { width, height, pic: Point::new(cx, cy + bar_h), card, bar_h, body }
    }

    /// Corner radius of the picture: the frame's, or a phone's or tablet's screen corners.
    pub fn picture_corner(&self, l: &FrameLayout) -> f64 {
        self.corner.max(self.device.screen_corner(l.card.w, l.card.h))
    }

    /// Whether the bottom edge is torn (never inside a device).
    pub fn is_torn(&self) -> bool {
        self.torn && self.device == Device::None
    }

    /// The points of a torn bottom edge across `card`, from right to left. The same picture
    /// size always gives the same teeth.
    pub fn torn_edge(card: RectF) -> Vec<Point> {
        let bottom = card.y + card.h;
        let mut pts = Vec::new();
        let mut seed: u32 = (card.w as u32).wrapping_mul(2654435761) ^ 0x9e37_79b9;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f64 / (1u32 << 24) as f64
        };
        let mut x = card.x + card.w;
        pts.push(Point::new(x, bottom - TORN_DEPTH * 0.5));
        let mut down = false;
        while x > card.x {
            x -= 6.0 + next() * 8.0;
            let depth = if down { 0.0 } else { TORN_DEPTH * (0.55 + 0.45 * next()) };
            pts.push(Point::new(x.max(card.x), bottom - depth));
            down = !down;
        }
        pts
    }
}

/// Picture coordinates of the whole result, for a view that shows the frame: the result's
/// rectangle with the picture at (0, 0).
pub fn content_rect(frame: Option<&Frame>, w: u32, h: u32) -> RectF {
    match frame {
        None => RectF { x: 0.0, y: 0.0, w: w as f64, h: h as f64 },
        Some(f) => {
            let l = f.layout(w, h);
            RectF { x: -l.pic.x, y: -l.pic.y, w: l.width as f64, h: l.height as f64 }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_goes_all_round_and_a_bar_goes_on_top() {
        let f = Frame { padding: 20.0, bar: Bar::None, ..Frame::default() };
        let l = f.layout(100, 50);
        assert_eq!((l.width, l.height), (140, 90));
        assert_eq!(l.pic, Point::new(20.0, 20.0));
        let f = Frame { bar: Bar::Dark, ..f };
        let l = f.layout(100, 50);
        assert_eq!(l.height, 90 + 34);
        assert_eq!(l.pic, Point::new(20.0, 54.0));
        assert_eq!(l.card, RectF { x: 20.0, y: 20.0, w: 100.0, h: 84.0 });
    }

    #[test]
    fn a_ratio_adds_background_to_reach_it_and_keeps_the_picture_centred() {
        let f = Frame { padding: 0.0, ratio: Some((16, 9)), ..Frame::default() };
        let l = f.layout(100, 100);
        assert_eq!((l.width, l.height), (178, 100));
        assert_eq!(l.pic, Point::new(39.0, 0.0));
        let l = Frame { padding: 0.0, ratio: Some((1, 1)), ..Frame::default() }.layout(300, 100);
        assert_eq!((l.width, l.height), (300, 300));
        assert_eq!(l.pic.y, 100.0);
    }

    #[test]
    fn the_torn_edge_spans_the_card_and_stays_within_its_depth() {
        let card = RectF { x: 10.0, y: 0.0, w: 200.0, h: 100.0 };
        let pts = Frame::torn_edge(card);
        assert_eq!(pts.first().unwrap().x, 210.0);
        assert_eq!(pts.last().unwrap().x, 10.0);
        assert!(pts.iter().all(|p| p.y <= 100.0 && p.y >= 100.0 - TORN_DEPTH));
        assert_eq!(pts, Frame::torn_edge(card), "the same every time");
    }

    #[test]
    fn a_device_goes_around_the_picture_and_a_laptop_has_a_base() {
        let f = Frame { padding: 10.0, device: Device::Phone, ..Frame::default() };
        let l = f.layout(400, 800);
        let u = Device::unit(400.0, 800.0);
        assert_eq!(u, 16.0);
        assert_eq!(l.body, RectF { x: l.card.x - u, y: l.card.y - (u * 2.2).round(), w: 400.0 + 2.0 * u, h: 800.0 + 2.0 * (u * 2.2).round() });
        assert_eq!((l.width, l.height), (400 + 2 * 16 + 20, 800 + 2 * 35 + 20));
        assert_eq!(l.body.x, 10.0, "the body sits in the padding");
        let f = Frame { padding: 0.0, device: Device::Laptop, ..Frame::default() };
        let l = f.layout(1000, 600);
        let u = Device::unit(1000.0, 600.0);
        assert!(l.width as f64 >= 1000.0 + 2.0 * u * 5.0, "the base is wider than the lid");
        assert!(l.height as f64 > l.body.y + l.body.h, "the base is below the lid");
        assert!(!Frame { torn: true, device: Device::Tablet, ..Frame::default() }.is_torn());
    }

    #[test]
    fn a_background_picture_is_found_by_its_key() {
        let b = Bitmap::filled(3, 2, [9, 8, 7, 255]);
        let key = register_picture(b.clone());
        assert_eq!(key, PictureKey::of(&b));
        assert_ne!(key, PictureKey::of(&Bitmap::filled(2, 3, [9, 8, 7, 255])), "the size counts");
        assert_eq!(*picture(&key).unwrap(), b);
        assert_eq!(PictureKey::from_hex(&key.hex()), Some(key));
        assert_eq!(PictureKey::from_hex("../../etc/passwd"), None);
        let f = Frame { background: Background::Picture(key), ..Frame::default() };
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<Frame>(&json).unwrap(), f);
        // A frame saved before devices existed still reads.
        let old = r#"{"padding":5.0,"bar":"Dark"}"#;
        assert_eq!(serde_json::from_str::<Frame>(old).unwrap().device, Device::None);
    }

    #[test]
    fn the_content_rect_puts_the_picture_at_the_origin() {
        let f = Frame { padding: 10.0, ..Frame::default() };
        assert_eq!(content_rect(Some(&f), 50, 30), RectF { x: -10.0, y: -10.0, w: 70.0, h: 50.0 });
        assert_eq!(content_rect(None, 50, 30), RectF { x: 0.0, y: 0.0, w: 50.0, h: 30.0 });
    }
}
