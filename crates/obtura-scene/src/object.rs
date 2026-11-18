use crate::geometry::{flatten_pieces, line_pieces, nearest_side_midpoint, rotate_point, rotated_bounds, Cubic, Point, RectF};
use obtura_image::{decode_png, encode_png, Bitmap};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::rc::Rc;

pub type Id = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const BLACK: Color = Color::rgb(0, 0, 0);

    /// Parses `#rrggbb`.
    pub fn from_hex(s: &str) -> Option<Color> {
        let s = s.strip_prefix('#')?;
        if s.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Black or white, whichever reads better on top of this colour.
    pub fn contrast(self) -> Color {
        let light = 0.299 * self.r as f64 + 0.587 * self.g as f64 + 0.114 * self.b as f64 > 160.0;
        if light {
            Color::BLACK
        } else {
            Color::WHITE
        }
    }
}

/// The outline of a step badge, or of a redaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shape {
    /// A rectangle (a "square" for a step badge).
    #[default]
    Rect,
    Rounded,
    /// An ellipse (a "circle" for a step badge).
    Ellipse,
    Diamond,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dash {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

/// What a line or curve ends with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cap {
    #[default]
    None,
    /// A triangle.
    Arrow,
    /// An open arrow head, two strokes.
    Chevron,
    Bullet,
    Square,
    Diamond,
    /// A short stroke across the line.
    Bar,
}

pub const CAPS: [Cap; 7] = [Cap::None, Cap::Arrow, Cap::Chevron, Cap::Bullet, Cap::Square, Cap::Diamond, Cap::Bar];

impl Cap {
    pub fn name(self) -> &'static str {
        match self {
            Cap::None => "Plain",
            Cap::Arrow => "Arrow",
            Cap::Chevron => "Chevron",
            Cap::Bullet => "Dot",
            Cap::Square => "Square",
            Cap::Diamond => "Diamond",
            Cap::Bar => "Bar",
        }
    }
}

/// Font families every system maps to something sensible, or any installed family by name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Font {
    #[default]
    Sans,
    Serif,
    Mono,
    Handwriting,
    /// An installed font family chosen by name ("Ubuntu", "Noto Serif"...). A family that is
    /// not installed falls back to the system's sans.
    Family(FamilyName),
}

pub const FONTS: [Font; 4] = [Font::Sans, Font::Serif, Font::Mono, Font::Handwriting];

impl Font {
    pub fn name(&self) -> &str {
        match self {
            Font::Sans => "Sans",
            Font::Serif => "Serif",
            Font::Mono => "Monospace",
            Font::Handwriting => "Handwriting",
            Font::Family(f) => f.as_str(),
        }
    }

    /// The family name given to the text engine.
    pub fn family(&self) -> &str {
        match self {
            Font::Sans => "Sans",
            Font::Serif => "Serif",
            Font::Mono => "Monospace",
            Font::Handwriting => "Cursive",
            Font::Family(f) => f.as_str(),
        }
    }
}

/// A font family's name, kept inside the style itself (the style is copied around freely):
/// at most 62 bytes, which every real family name fits in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FamilyName {
    len: u8,
    bytes: [u8; 62],
}

impl FamilyName {
    /// The name, cut at 62 bytes (on a character boundary). Quotes and backslashes, which no
    /// family name has, are dropped.
    pub fn new(name: &str) -> FamilyName {
        let mut bytes = [0u8; 62];
        let mut len = 0;
        for c in name.trim().chars().filter(|c| !matches!(c, '"' | '\\' | '\'') && !c.is_control()) {
            let n = c.len_utf8();
            if len + n > bytes.len() {
                break;
            }
            c.encode_utf8(&mut bytes[len..len + n]);
            len += n;
        }
        FamilyName { len: len as u8, bytes }
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("Sans")
    }
}

impl std::fmt::Debug for FamilyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl Serialize for FamilyName {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FamilyName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(FamilyName::new(&String::deserialize(d)?))
    }
}

/// How a mark looks. Which fields matter depends on the kind of mark; the Formatting panel
/// shows only those.
///
/// Three colours: `color` draws lines and outlines (and a callout's border and line),
/// `fill` is the inside (a shape, a text background, a step badge) and `text_color` is the
/// lettering (text, callout, step number).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub color: Color,
    /// Line width in picture pixels; for text, callouts and steps the width of the border.
    pub width: f64,
    /// The inside of a shape, the background of text, the badge of a step. `None`: nothing.
    pub fill: Option<Color>,
    pub text_color: Color,
    /// Text height in picture pixels (the number's height, for a step).
    pub font_size: f64,
    pub font: Font,
    pub bold: bool,
    pub italic: bool,
    /// Radius of a step badge.
    pub badge: f64,
    pub shape: Shape,
    /// Corner radius of a rectangle or a text box.
    pub corner: f64,
    pub dash: Dash,
    pub start: Cap,
    pub end: Cap,
    /// Size of the line ends, 1.0 being the normal size.
    pub cap_size: f64,
    /// Line ends are solid; off means outlined.
    pub cap_filled: bool,
    /// 0.0 (invisible) to 1.0.
    pub opacity: f64,
    /// Blur and pixelate: the block size in pixels. 0 picks one that suits the area.
    pub strength: f64,
}

impl Default for Style {
    fn default() -> Self {
        let red = Color::rgb(0xe0, 0x1b, 0x24);
        Self {
            color: red,
            width: 4.0,
            fill: None,
            text_color: red,
            font_size: 28.0,
            font: Font::Sans,
            bold: true,
            italic: false,
            badge: 21.0,
            shape: Shape::Rect,
            corner: 0.0,
            dash: Dash::Solid,
            start: Cap::None,
            end: Cap::None,
            cap_size: 1.0,
            cap_filled: true,
            opacity: 1.0,
            strength: 0.0,
        }
    }
}

impl Style {
    /// Size of a line end: the head of an arrow, the width of a dot.
    pub fn cap_unit(&self) -> f64 {
        (10.0 + self.width * 3.0) * self.cap_size
    }

    /// Whether text with this style has a visible box (a background or a border).
    pub fn has_box(&self) -> bool {
        self.fill.is_some() || self.width > 0.0
    }

    /// Space between a text and its box.
    pub fn text_pad(&self) -> f64 {
        self.font_size * 0.3
    }

    /// Width of a callout's line: never thinner than it can be seen.
    pub fn leader_width(&self) -> f64 {
        self.width.max(2.0)
    }

    /// Half the width of a step badge's box (a diamond reaches further than a circle).
    pub fn badge_extent(&self) -> f64 {
        match self.shape {
            Shape::Diamond => self.badge * 1.3,
            _ => self.badge,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedactMode {
    Blur,
    Pixelate,
    Box,
}

/// The pixels of a picture placed on the screenshot. Shared, so undo steps cost nothing;
/// saved as PNG.
#[derive(Clone)]
pub struct Pixels(pub Rc<Bitmap>);

impl Pixels {
    pub fn new(bitmap: Bitmap) -> Self {
        Self(Rc::new(bitmap))
    }
}

impl PartialEq for Pixels {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}

impl std::fmt::Debug for Pixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pixels({} x {})", self.0.width, self.0.height)
    }
}

impl Serialize for Pixels {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let png = encode_png(&self.0).map_err(serde::ser::Error::custom)?;
        s.serialize_bytes(&png)
    }
}

impl<'de> Deserialize<'de> for Pixels {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let bytes: Vec<u8> = Vec::deserialize(d)?;
        decode_png(&bytes).map(Pixels::new).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    /// A line; with bends in `via` it is a smooth curve through them, in order (one bend: the
    /// curve passes through it at its middle). What it ends with comes from the style.
    Line {
        from: Point,
        to: Point,
        #[serde(default, deserialize_with = "bends")]
        via: Vec<Point>,
    },
    Rect { rect: RectF },
    Ellipse { rect: RectF },
    Pen { points: Vec<Point> },
    Highlighter { points: Vec<Point> },
    /// `pos` is the top-left corner of the text; `size` is the measured size of the text, kept
    /// up to date by whoever draws it (this crate has no fonts), and used for hit testing.
    /// A background or border goes around the text, so the text never moves when they change.
    Text { pos: Point, text: String, size: Point },
    /// A text box with a line to what it explains, ending in an arrow at `tip`.
    Callout { tip: Point, pos: Point, text: String, size: Point },
    /// A numbered badge centred on `center`.
    Step { center: Point, number: u32 },
    /// Hides what is under `rect` in the exported picture (the style's shape picks a
    /// rectangle or an ellipse).
    Redact { rect: RectF, mode: RedactMode },
    /// Dims the whole picture except `rect` (the style's shape: rectangle or ellipse). The
    /// style's colour is the shade, its opacity how dark.
    Spotlight { rect: RectF },
    /// Shows what is in `source` enlarged in `lens` (same proportions), with a line between
    /// them. The style's shape: ellipse or rectangle.
    Magnify { source: RectF, lens: RectF },
    /// An emoji or a symbol centred on `center`, as tall as the style's font size.
    Stamp { center: Point, glyph: String },
    /// A picture placed over the screenshot, stretched to `rect`.
    Image { rect: RectF, pixels: Pixels },
    /// A dimension line from `from` to `to`, labelled with its length in pixels.
    Measure { from: Point, to: Point },
}

/// The bends of a line: a list now; before, at most one point (or nothing).
fn bends<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Point>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Saved {
        Many(Vec<Point>),
        One(Point),
    }
    Ok(match Option::<Saved>::deserialize(d)? {
        None => Vec::new(),
        Some(Saved::Many(v)) => v,
        Some(Saved::One(p)) => vec![p],
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub id: Id,
    pub kind: Kind,
    pub style: Style,
    /// How far the mark is turned around its centre, in radians, clockwise on the screen.
    /// Only shapes, text, stamps, steps and pictures keep an angle (see [`Object::turns`]);
    /// strokes and lines are turned by moving their points.
    #[serde(default)]
    pub angle: f64,
}

fn points_bounds(points: impl IntoIterator<Item = Point>) -> RectF {
    let mut it = points.into_iter();
    let Some(first) = it.next() else { return RectF::default() };
    it.fold(RectF { x: first.x, y: first.y, w: 0.0, h: 0.0 }, |r, p| r.union(&RectF { x: p.x, y: p.y, w: 0.0, h: 0.0 }))
}

/// How far back from its end `b` a piece is `len` away from it (as `t`), found the way the
/// renderer finds where to cut a line short for its end.
pub fn t_back_from_end(c: &Cubic, len: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if c.at(mid).distance(c.b) > len {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// The direction (a unit vector, out of the line) of the end `b` of the piece, seen from `len`
/// back along it: how the renderer turns a line end.
fn end_direction(c: &Cubic, len: f64) -> Option<(f64, f64)> {
    let unit = |p: Point, q: Point| {
        let d = p.distance(q);
        (d > 1e-6).then(|| ((q.x - p.x) / d, (q.y - p.y) / d))
    };
    if len <= 0.0 {
        return unit(c.c2, c.b).or_else(|| unit(c.c1, c.b)).or_else(|| unit(c.a, c.b));
    }
    unit(c.at(t_back_from_end(c, len)), c.b)
}

/// Where the ink of a line end goes: `tip` is the end point, `dir` points out of the line.
/// Mirrors the shapes the renderer draws (see `draw_cap` in obtura-render).
fn cap_ink(cap: Cap, tip: Point, (ux, uy): (f64, f64), unit: f64, s: &Style) -> Option<RectF> {
    let (px, py) = (-uy, ux);
    let at = |along: f64, across: f64| Point::new(tip.x + ux * along + px * across, tip.y + uy * along + py * across);
    // Half the outline of a head: 0.7-0.8 of the width when outlined, 0.4 when solid.
    let outline = s.width.max(1.0) * 0.4;
    let (pts, pad) = match cap {
        Cap::None => return None,
        Cap::Arrow => (vec![tip, at(-unit, unit * 0.45), at(-unit, -unit * 0.45)], outline),
        Cap::Chevron => (vec![at(-unit * 0.8, unit * 0.55), tip, at(-unit * 0.8, -unit * 0.55)], s.width / 2.0),
        Cap::Bullet => {
            let r = (unit * 0.32).max(s.width * 0.9);
            (vec![tip.offset(-r, -r), tip.offset(r, r)], outline)
        }
        Cap::Square => {
            let h = (unit * 0.28).max(s.width * 0.9);
            (vec![at(h, h), at(h, -h), at(-h, -h), at(-h, h)], outline)
        }
        Cap::Diamond => {
            let h = (unit * 0.42).max(s.width);
            (vec![at(h, 0.0), at(0.0, h * 0.8), at(-h, 0.0), at(0.0, -h * 0.8)], outline)
        }
        Cap::Bar => (vec![at(0.0, unit * 0.5), at(0.0, -unit * 0.5)], s.width / 2.0),
    };
    Some(points_bounds(pts).inflate(pad))
}

/// Where the ink of a line or curve goes, with its ends.
fn line_ink(from: Point, to: Point, via: &[Point], s: &Style) -> RectF {
    let pieces = line_pieces(from, via, to);
    let body = pieces.iter().map(Cubic::bounds).reduce(|a, b| a.union(&b)).unwrap_or_default();
    let mut ink = body.inflate(s.width / 2.0);
    if from.distance(to) < 0.5 && via.is_empty() {
        return ink;
    }
    let unit = Object::cap_unit_for(from, to, via, s);
    let (first, last) = (pieces[0].reversed(), pieces[pieces.len() - 1]);
    for (cap, piece, tip) in [(s.end, last, to), (s.start, first, from)] {
        let len = Object::cap_setback(cap, unit, s.cap_filled);
        if let Some(r) = end_direction(&piece, len).and_then(|d| cap_ink(cap, tip, d, unit, s)) {
            ink = ink.union(&r);
        }
    }
    ink
}

impl Object {
    /// Radius of a step badge.
    pub fn step_radius(style: &Style) -> f64 {
        style.badge
    }

    /// The rectangle behind the text of a text mark.
    pub fn text_box(pos: Point, size: Point, style: &Style, always: bool) -> RectF {
        let text = RectF { x: pos.x, y: pos.y, w: size.x, h: size.y };
        if always || style.has_box() {
            text.inflate(style.text_pad())
        } else {
            text
        }
    }

    /// The points a line or curve is drawn through, for hit testing.
    pub fn line_points(from: Point, to: Point, via: &[Point]) -> Vec<Point> {
        if via.is_empty() {
            return vec![from, to];
        }
        flatten_pieces(&line_pieces(from, via, to))
    }

    /// Whether this kind of mark keeps an angle (shapes, text, stamps, steps, pictures).
    pub fn turns(&self) -> bool {
        matches!(self.kind, Kind::Rect { .. } | Kind::Ellipse { .. } | Kind::Text { .. } | Kind::Stamp { .. } | Kind::Step { .. } | Kind::Image { .. })
    }

    /// Whether the mark can be turned with its round handle: those that keep an angle, and
    /// freehand strokes (whose points turn).
    pub fn can_rotate(&self) -> bool {
        self.turns() || matches!(self.kind, Kind::Pen { .. } | Kind::Highlighter { .. })
    }

    /// What the mark turns around: the middle of its shape, text, badge or picture.
    pub fn center(&self) -> Point {
        let mid = |r: RectF| Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
        match &self.kind {
            Kind::Rect { rect } | Kind::Ellipse { rect } | Kind::Image { rect, .. } | Kind::Redact { rect, .. } | Kind::Spotlight { rect } => mid(*rect),
            Kind::Text { pos, size, .. } => Point::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0),
            Kind::Stamp { center, .. } | Kind::Step { center, .. } => *center,
            _ => mid(self.upright_bounds()),
        }
    }

    /// The angle actually used (0 for kinds that do not keep one).
    pub fn turn(&self) -> f64 {
        if self.turns() { self.angle } else { 0.0 }
    }

    /// The mark turned by `delta` radians more around its centre.
    pub fn rotated(&self, delta: f64) -> Object {
        let mut o = self.clone();
        if self.turns() {
            o.angle = normal_angle(self.angle + delta);
        } else if let Kind::Pen { points } | Kind::Highlighter { points } = &mut o.kind {
            let c = self.center();
            points.iter_mut().for_each(|p| *p = rotate_point(*p, c, delta));
        }
        o
    }

    /// Where the round handle that turns the mark goes: `gap` above the middle of its top edge
    /// (turned with it). `None` for marks that do not turn.
    pub fn rotate_handle(&self, gap: f64) -> Option<Point> {
        if !self.can_rotate() {
            return None;
        }
        let b = self.upright_bounds();
        Some(rotate_point(Point::new(b.x + b.w / 2.0, b.y - gap), self.center(), self.turn()))
    }

    /// A point of the picture in the mark's own upright frame (its angle undone).
    pub fn to_upright(&self, p: Point) -> Point {
        rotate_point(p, self.center(), -self.turn())
    }

    /// Where a callout's line leaves its text box.
    pub fn callout_anchor(&self) -> Option<Point> {
        match &self.kind {
            Kind::Callout { tip, pos, size, .. } => Some(nearest_side_midpoint(Self::text_box(*pos, *size, &self.style, true), *tip)),
            _ => None,
        }
    }

    /// The box around the mark, for selection and hit testing.
    pub fn bounds(&self) -> RectF {
        rotated_bounds(self.upright_bounds(), self.center(), self.turn())
    }

    /// The box around the mark as if it were not turned.
    pub fn upright_bounds(&self) -> RectF {
        let s = &self.style;
        let pad = s.width / 2.0;
        match &self.kind {
            Kind::Line { from, to, via } => {
                let reach = if s.start != Cap::None || s.end != Cap::None { s.cap_unit().max(pad) } else { pad };
                points_bounds(Self::line_points(*from, *to, via)).inflate(reach)
            }
            Kind::Rect { rect } | Kind::Ellipse { rect } => rect.inflate(pad),
            Kind::Redact { rect, .. } => *rect,
            Kind::Pen { points } | Kind::Highlighter { points } => {
                let pad = if matches!(self.kind, Kind::Highlighter { .. }) { s.width * 2.0 } else { pad };
                points_bounds(points.iter().copied()).inflate(pad)
            }
            Kind::Text { pos, size, .. } => Self::text_box(*pos, *size, s, false).inflate(if s.has_box() { pad } else { 0.0 }),
            Kind::Callout { tip, pos, size, .. } => {
                let text = Self::text_box(*pos, *size, s, true).inflate(pad);
                text.union(&RectF { x: tip.x, y: tip.y, w: 0.0, h: 0.0 }.inflate(Style { width: s.leader_width(), cap_size: 1.0, ..*s }.cap_unit()))
            }
            Kind::Step { center, .. } => {
                let r = s.badge_extent() + pad;
                RectF { x: center.x - r, y: center.y - r, w: 2.0 * r, h: 2.0 * r }
            }
            Kind::Spotlight { rect } => *rect,
            Kind::Magnify { source, lens } => source.union(lens).inflate(pad),
            Kind::Stamp { center, .. } => Self::stamp_box(*center, s),
            Kind::Image { rect, .. } => rect.inflate(pad),
            Kind::Measure { from, to } => line_ink(*from, *to, &[], s).union(&Self::measure_box(*from, *to, s)),
        }
    }

    /// The label of a dimension line: its length in pixels.
    pub fn measure_label(from: Point, to: Point) -> String {
        format!("{} px", from.distance(to).round())
    }

    /// The box of a dimension line's label, over the middle of the line. Its size comes from
    /// the number of characters (not from a font), so it is known without drawing.
    pub fn measure_box(from: Point, to: Point, s: &Style) -> RectF {
        let chars = Self::measure_label(from, to).chars().count() as f64;
        let (w, h) = (s.font_size * (0.7 * chars + 0.8), s.font_size * 1.5);
        let (cx, cy) = ((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
        RectF { x: cx - w / 2.0, y: cy - h / 2.0, w, h }
    }

    /// The area the mark needs in the finished picture. A mark that reaches past the edge of
    /// the screenshot makes the picture grow (white space) to hold all of it. `None` for the
    /// marks that only work on the screenshot's pixels (redactions, spotlights): they never
    /// make it grow. A magnifier needs room for its lens only.
    ///
    /// Unlike [`Object::bounds`] (generous, for clicking) this is where the ink really goes: an
    /// arrow pointing at the edge does not make the picture grow, one that crosses it does.
    pub fn reach(&self) -> Option<RectF> {
        let s = &self.style;
        match &self.kind {
            Kind::Redact { .. } | Kind::Spotlight { .. } => None,
            Kind::Magnify { lens, .. } => Some(lens.inflate(s.width / 2.0)),
            Kind::Line { from, to, via } => Some(line_ink(*from, *to, via, s)),
            // A turned ellipse: the box of the turned outline, exactly (its half-widths are
            // its support along x and y, plus half the line).
            Kind::Ellipse { rect } if self.angle != 0.0 => {
                let (a, b) = (rect.w / 2.0, rect.h / 2.0);
                let (sn, cs) = self.angle.sin_cos();
                let hx = ((a * cs).powi(2) + (b * sn).powi(2)).sqrt() + s.width / 2.0;
                let hy = ((a * sn).powi(2) + (b * cs).powi(2)).sqrt() + s.width / 2.0;
                let c = self.center();
                Some(RectF { x: c.x - hx, y: c.y - hy, w: 2.0 * hx, h: 2.0 * hy })
            }
            Kind::Pen { points } | Kind::Highlighter { points } => Some(points_bounds(points.iter().copied()).inflate(s.width / 2.0)),
            Kind::Callout { tip, pos, size, .. } => {
                let rect = Self::text_box(*pos, *size, s, true);
                let mut ink = rect.inflate(s.width / 2.0);
                if !rect.contains(*tip) {
                    let anchor = nearest_side_midpoint(rect, *tip);
                    ink = ink.union(&line_ink(anchor, *tip, &[], &Self::leader_style(s)));
                }
                Some(ink)
            }
            _ => Some(self.bounds()),
        }
    }

    /// The style of a callout's line: its own width, an arrow at the tip.
    pub fn leader_style(s: &Style) -> Style {
        Style { width: s.leader_width(), cap_size: 1.0, start: Cap::None, end: Cap::Arrow, cap_filled: true, dash: Dash::Solid, ..*s }
    }

    /// The size the ends of a line from `from` to `to` (through `via`) are drawn: the style's,
    /// smaller on a short line so the heads never cover it.
    pub fn cap_unit_for(from: Point, to: Point, via: &[Point], s: &Style) -> f64 {
        let total = match via {
            [] => from.distance(to),
            [v] => {
                let c = crate::geometry::quad_control(from, *v, to);
                from.distance(c) + c.distance(to)
            }
            _ => line_pieces(from, via, to).iter().map(Cubic::hull_length).sum(),
        };
        let both = s.start != Cap::None && s.end != Cap::None;
        s.cap_unit().min(total * if both { 0.45 } else { 0.9 })
    }

    /// How far a line stops short of its tip so the end sits on a clean edge.
    pub fn cap_setback(cap: Cap, unit: f64, filled: bool) -> f64 {
        match cap {
            Cap::Arrow => unit,
            Cap::Bullet if !filled => unit * 0.32,
            Cap::Square if !filled => unit * 0.28,
            Cap::Diamond if !filled => unit * 0.42,
            _ => 0.0,
        }
    }

    /// The square a stamp takes.
    pub fn stamp_box(center: Point, s: &Style) -> RectF {
        let r = s.font_size * 0.65;
        RectF { x: center.x - r, y: center.y - r, w: 2.0 * r, h: 2.0 * r }
    }

    /// The lens of a magnifier for `source`, `zoom` times bigger, placed beside it on the side
    /// with the most room in a `w` x `h` picture.
    pub fn lens_for(source: RectF, zoom: f64, w: f64, h: f64) -> RectF {
        let (lw, lh) = (source.w * zoom, source.h * zoom);
        let gap = 24.0;
        let (cx, cy) = (source.x + source.w / 2.0, source.y + source.h / 2.0);
        let right = w - (source.x + source.w);
        let below = h - (source.y + source.h);
        let candidates = [
            (right, Point::new(source.x + source.w + gap, cy - lh / 2.0)),
            (source.x, Point::new(source.x - gap - lw, cy - lh / 2.0)),
            (below, Point::new(cx - lw / 2.0, source.y + source.h + gap)),
            (source.y, Point::new(cx - lw / 2.0, source.y - gap - lh)),
        ];
        let (_, at) = candidates.into_iter().max_by(|a, b| a.0.total_cmp(&b.0)).expect("four sides");
        let x = at.x.clamp(0.0, (w - lw).max(0.0));
        let y = at.y.clamp(0.0, (h - lh).max(0.0));
        RectF { x, y, w: lw, h: lh }
    }

    /// Moves the mark by (dx, dy).
    pub fn translate(&mut self, dx: f64, dy: f64) {
        let shift = |p: &mut Point| *p = p.offset(dx, dy);
        match &mut self.kind {
            Kind::Line { from, to, via } => {
                shift(from);
                shift(to);
                via.iter_mut().for_each(shift);
            }
            Kind::Rect { rect } | Kind::Ellipse { rect } | Kind::Redact { rect, .. } => {
                rect.x += dx;
                rect.y += dy;
            }
            Kind::Pen { points } | Kind::Highlighter { points } => points.iter_mut().for_each(shift),
            Kind::Text { pos, .. } => shift(pos),
            Kind::Callout { tip, pos, .. } => {
                shift(tip);
                shift(pos);
            }
            Kind::Step { center, .. } | Kind::Stamp { center, .. } => shift(center),
            Kind::Spotlight { rect } | Kind::Image { rect, .. } => {
                rect.x += dx;
                rect.y += dy;
            }
            Kind::Magnify { source, lens } => {
                for r in [source, lens] {
                    r.x += dx;
                    r.y += dy;
                }
            }
            Kind::Measure { from, to } => {
                shift(from);
                shift(to);
            }
        }
    }

    /// Moves every point of the mark through `f` (a rotation, a flip, a cut). Rectangles and
    /// lines follow their corners; text, badges, stamps and pictures stay upright and the same
    /// size: only their centre moves.
    pub fn map_points(&mut self, f: &dyn Fn(Point) -> Point) {
        // A mirror turns the other way: a turned mark's angle changes sign.
        let (o, x, y) = (f(Point::new(0.0, 0.0)), f(Point::new(1.0, 0.0)), f(Point::new(0.0, 1.0)));
        if (x.x - o.x) * (y.y - o.y) - (x.y - o.y) * (y.x - o.x) < 0.0 {
            self.angle = -self.angle;
        }
        let rect_through = |r: &mut RectF| *r = RectF::from_points(f(Point::new(r.x, r.y)), f(Point::new(r.x + r.w, r.y + r.h)));
        let upright = |r: RectF| {
            let c = f(Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0));
            RectF { x: c.x - r.w / 2.0, y: c.y - r.h / 2.0, ..r }
        };
        match &mut self.kind {
            Kind::Line { from, to, via } => {
                *from = f(*from);
                *to = f(*to);
                via.iter_mut().for_each(|v| *v = f(*v));
            }
            Kind::Rect { rect } | Kind::Ellipse { rect } | Kind::Redact { rect, .. } | Kind::Spotlight { rect } => rect_through(rect),
            Kind::Magnify { source, lens } => {
                rect_through(source);
                *lens = upright(*lens);
            }
            Kind::Pen { points } | Kind::Highlighter { points } => points.iter_mut().for_each(|p| *p = f(*p)),
            Kind::Text { pos, size, .. } => {
                let r = upright(RectF { x: pos.x, y: pos.y, w: size.x, h: size.y });
                *pos = Point::new(r.x, r.y);
            }
            Kind::Callout { tip, pos, size, .. } => {
                *tip = f(*tip);
                let r = upright(RectF { x: pos.x, y: pos.y, w: size.x, h: size.y });
                *pos = Point::new(r.x, r.y);
            }
            Kind::Step { center, .. } | Kind::Stamp { center, .. } => *center = f(*center),
            Kind::Image { rect, .. } => *rect = upright(*rect),
            Kind::Measure { from, to } => {
                *from = f(*from);
                *to = f(*to);
            }
        }
    }

    /// Makes the mark `k` times bigger around the picture's origin, its line widths and
    /// text sizes included (the picture was resized by `k`).
    pub fn scale(&mut self, k: f64) {
        let sc = |r: &mut RectF| *r = RectF { x: r.x * k, y: r.y * k, w: r.w * k, h: r.h * k };
        let sp = |p: &mut Point| *p = Point::new(p.x * k, p.y * k);
        match &mut self.kind {
            Kind::Line { from, to, via } => {
                sp(from);
                sp(to);
                via.iter_mut().for_each(sp);
            }
            Kind::Rect { rect } | Kind::Ellipse { rect } | Kind::Redact { rect, .. } | Kind::Spotlight { rect } | Kind::Image { rect, .. } => sc(rect),
            Kind::Magnify { source, lens } => {
                sc(source);
                sc(lens);
            }
            Kind::Pen { points } | Kind::Highlighter { points } => points.iter_mut().for_each(sp),
            Kind::Text { pos, size, .. } | Kind::Callout { pos, size, .. } => {
                sp(pos);
                sp(size);
            }
            Kind::Step { center, .. } | Kind::Stamp { center, .. } => sp(center),
            Kind::Measure { from, to } => {
                sp(from);
                sp(to);
            }
        }
        if let Kind::Callout { tip, .. } = &mut self.kind {
            sp(tip);
        }
        let s = &mut self.style;
        s.width *= k;
        s.font_size *= k;
        s.badge *= k;
        s.corner *= k;
        if s.strength >= 2.0 {
            s.strength = (s.strength * k).round().max(2.0);
        }
    }

    /// The handles the user can drag to reshape the selected mark, in a fixed order (turned
    /// with the mark). Rectangles: the four corners and four sides, clockwise from the
    /// top-left. Lines: the two ends, then each bend.
    pub fn handles(&self) -> Vec<Point> {
        let (c, a) = (self.center(), self.turn());
        self.upright_handles().into_iter().map(|p| rotate_point(p, c, a)).collect()
    }

    /// The handles as if the mark were not turned.
    fn upright_handles(&self) -> Vec<Point> {
        let corners = |r: RectF| vec![Point::new(r.x, r.y), Point::new(r.x + r.w, r.y), Point::new(r.x + r.w, r.y + r.h), Point::new(r.x, r.y + r.h)];
        match &self.kind {
            Kind::Line { from, to, via } => {
                let mut h = vec![*from, *to];
                h.extend(via.iter().copied());
                h
            }
            Kind::Rect { rect } | Kind::Ellipse { rect } | Kind::Redact { rect, .. } | Kind::Spotlight { rect } | Kind::Image { rect, .. } => rect_handles(*rect),
            Kind::Callout { tip, pos, size, .. } => {
                let b = Self::text_box(*pos, *size, &self.style, true);
                vec![*tip, Point::new(b.x, b.y), Point::new(b.x + b.w, b.y + b.h)]
            }
            Kind::Text { pos, size, .. } => corners(Self::text_box(*pos, *size, &self.style, true)),
            Kind::Step { center, .. } => vec![Point::new(center.x + self.style.badge_extent(), center.y)],
            Kind::Stamp { center, .. } => corners(Self::stamp_box(*center, &self.style)),
            Kind::Pen { points } | Kind::Highlighter { points } => corners(points_bounds(points.iter().copied())),
            Kind::Magnify { source, lens } => {
                let mut h = corners(*source);
                h.extend(corners(*lens));
                h.push(Point::new(lens.x + lens.w / 2.0, lens.y + lens.h / 2.0));
                h
            }
            Kind::Measure { from, to } => vec![*from, *to],
        }
    }

    /// The mark as it is when handle `i` is dragged to `p`, computed from the mark as it was
    /// when the drag began (`self`), so a drag past the opposite side is reversible. A turned
    /// mark is reshaped in its own upright frame; the handle opposite stays where it was.
    pub fn with_handle(&self, i: usize, p: Point) -> Object {
        let a = self.turn();
        if a == 0.0 {
            return self.with_upright_handle(i, p);
        }
        let mut o = self.with_upright_handle(i, self.to_upright(p));
        let anchor = match &self.kind {
            Kind::Rect { .. } | Kind::Ellipse { .. } | Kind::Image { .. } => Some((i + 4) % 8),
            Kind::Text { .. } | Kind::Stamp { .. } => Some((i + 2) % 4),
            _ => None,
        };
        if let Some(k) = anchor {
            let (before, after) = (self.upright_handles(), o.upright_handles());
            if let (Some(b), Some(n)) = (before.get(k), after.get(k)) {
                let was = rotate_point(*b, self.center(), a);
                let now = rotate_point(*n, o.center(), a);
                o.translate(was.x - now.x, was.y - now.y);
            }
        }
        o
    }

    /// Adds a bend to a line where it passes nearest `p` (a straight line becomes a curve).
    /// Returns the new bend's handle index.
    pub fn add_bend(&mut self, p: Point) -> Option<usize> {
        let Kind::Line { from, to, via } = &mut self.kind else { return None };
        // Which stretch between two of its points `p` is nearest: the bend goes in there.
        let mut pts = vec![*from];
        pts.extend(via.iter().copied());
        pts.push(*to);
        let pieces = line_pieces(*from, via, *to);
        let mut best = (f64::MAX, 0usize);
        for (k, piece) in pieces.iter().enumerate() {
            for j in 0..=32 {
                let d = piece.at(j as f64 / 32.0).distance(p);
                if d < best.0 {
                    best = (d, k);
                }
            }
        }
        via.insert(best.1, p);
        Some(2 + best.1)
    }

    /// Removes the bend of handle `i` (2 and up); the line stays where its other points are.
    pub fn remove_bend(&mut self, i: usize) -> bool {
        match &mut self.kind {
            Kind::Line { via, .. } if i >= 2 && i - 2 < via.len() => {
                via.remove(i - 2);
                true
            }
            _ => false,
        }
    }

    fn with_upright_handle(&self, i: usize, p: Point) -> Object {
        let mut o = self.clone();
        // A corner (0-3, clockwise from the top-left) of `r` dragged to `p`, the opposite corner
        // staying put: the factor the size changes by, and that corner.
        let corner_scale = |r: RectF, i: usize| {
            let fixed = match i % 4 {
                0 => Point::new(r.x + r.w, r.y + r.h),
                1 => Point::new(r.x, r.y + r.h),
                2 => Point::new(r.x, r.y),
                _ => Point::new(r.x + r.w, r.y),
            };
            let k = ((p.x - fixed.x).abs() / r.w.max(1.0)).max((p.y - fixed.y).abs() / r.h.max(1.0)).max(0.05);
            (k, fixed)
        };
        // Where a box of size (w, h) goes so that its corner opposite `i` is at `fixed`.
        let anchored = |fixed: Point, w: f64, h: f64, i: usize| match i % 4 {
            0 => Point::new(fixed.x - w, fixed.y - h),
            1 => Point::new(fixed.x, fixed.y - h),
            2 => fixed,
            _ => Point::new(fixed.x - w, fixed.y),
        };
        match &mut o.kind {
            Kind::Line { from, to, via } => match i {
                0 => *from = p,
                1 => *to = p,
                k => {
                    if let Some(v) = via.get_mut(k - 2) {
                        *v = p;
                    }
                }
            },
            Kind::Rect { rect } | Kind::Ellipse { rect } | Kind::Redact { rect, .. } | Kind::Spotlight { rect } | Kind::Image { rect, .. } => *rect = drag_rect(*rect, i, p),
            Kind::Callout { tip, pos, size, .. } => {
                let pad = self.style.text_pad();
                match i {
                    0 => *tip = p,
                    1 => *pos = Point::new(p.x + pad, p.y + pad),
                    _ => {
                        // The far corner of the box: the text grows or shrinks.
                        let b = Self::text_box(*pos, *size, &self.style, true);
                        let k = ((p.y - b.y) / b.h.max(1.0)).max((p.x - b.x) / b.w.max(1.0)).max(0.1);
                        o.style.font_size = (self.style.font_size * k).clamp(6.0, 400.0);
                        *size = Point::new(size.x * k, size.y * k);
                    }
                }
            }
            Kind::Text { pos, size, .. } => {
                let b = Self::text_box(*pos, *size, &self.style, true);
                let (k, fixed) = corner_scale(b, i);
                let font = (self.style.font_size * k).clamp(6.0, 400.0);
                let k = font / self.style.font_size;
                o.style.font_size = font;
                *size = Point::new(size.x * k, size.y * k);
                let pad = o.style.text_pad();
                let nb = anchored(fixed, size.x + 2.0 * pad, size.y + 2.0 * pad, i);
                *pos = Point::new(nb.x + pad, nb.y + pad);
            }
            Kind::Step { center, .. } => {
                let extent = (p.x - center.x).abs().max((p.y - center.y).abs()).max(4.0);
                let k = extent / self.style.badge_extent().max(1.0);
                o.style.badge = (self.style.badge * k).clamp(4.0, 300.0);
                o.style.font_size = (self.style.font_size * k).clamp(4.0, 400.0);
            }
            Kind::Stamp { center, .. } => {
                let (k, fixed) = corner_scale(Self::stamp_box(*center, &self.style), i);
                o.style.font_size = (self.style.font_size * k).clamp(8.0, 1000.0);
                let side = Self::stamp_box(Point::new(0.0, 0.0), &o.style).w;
                let nb = anchored(fixed, side, side, i);
                *center = Point::new(nb.x + side / 2.0, nb.y + side / 2.0);
            }
            Kind::Pen { points } | Kind::Highlighter { points } => {
                let b = points_bounds(points.iter().copied());
                let (fixed, moving) = match i {
                    0 => (Point::new(b.x + b.w, b.y + b.h), Point::new(b.x, b.y)),
                    1 => (Point::new(b.x, b.y + b.h), Point::new(b.x + b.w, b.y)),
                    2 => (Point::new(b.x, b.y), Point::new(b.x + b.w, b.y + b.h)),
                    _ => (Point::new(b.x + b.w, b.y), Point::new(b.x, b.y + b.h)),
                };
                let kx = if (moving.x - fixed.x).abs() < 1e-9 { 1.0 } else { (p.x - fixed.x) / (moving.x - fixed.x) };
                let ky = if (moving.y - fixed.y).abs() < 1e-9 { 1.0 } else { (p.y - fixed.y) / (moving.y - fixed.y) };
                for q in points.iter_mut() {
                    *q = Point::new(fixed.x + (q.x - fixed.x) * kx, fixed.y + (q.y - fixed.y) * ky);
                }
            }
            Kind::Magnify { source, lens } => {
                let zoom = lens.w / source.w.max(1.0);
                match i {
                    0..=3 => {
                        // The area looked at: the lens keeps its centre and its zoom.
                        *source = drag_rect(*source, [0, 2, 4, 6][i], p);
                        let (cx, cy) = (lens.x + lens.w / 2.0, lens.y + lens.h / 2.0);
                        let (w, h) = (source.w * zoom, source.h * zoom);
                        *lens = RectF { x: cx - w / 2.0, y: cy - h / 2.0, w, h };
                    }
                    4..=7 => {
                        // The lens: bigger or smaller, same proportions as the area.
                        let (k, fixed) = corner_scale(*lens, i - 4);
                        let (w, h) = (lens.w * k, lens.h * k);
                        let at = anchored(fixed, w, h, i - 4);
                        *lens = RectF { x: at.x, y: at.y, w, h };
                    }
                    _ => {
                        lens.x = p.x - lens.w / 2.0;
                        lens.y = p.y - lens.h / 2.0;
                    }
                }
            }
            Kind::Measure { from, to } => match i {
                0 => *from = p,
                _ => *to = p,
            },
        }
        o
    }
}

/// An angle brought into -π..π.
pub fn normal_angle(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    let a = a.rem_euclid(t);
    if a > std::f64::consts::PI { a - t } else { a }
}

fn rect_handles(rect: RectF) -> Vec<Point> {
    let (x1, y1, mx, my) = (rect.x + rect.w, rect.y + rect.h, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    vec![Point::new(rect.x, rect.y), Point::new(mx, rect.y), Point::new(x1, rect.y), Point::new(x1, my), Point::new(x1, y1), Point::new(mx, y1), Point::new(rect.x, y1), Point::new(rect.x, my)]
}

/// The rectangle after dragging handle `i` (see [`Object::handles`]) to `p`.
fn drag_rect(r: RectF, i: usize, p: Point) -> RectF {
    let (mut x0, mut y0, mut x1, mut y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    match i {
        0 => (x0, y0) = (p.x, p.y),
        1 => y0 = p.y,
        2 => (x1, y0) = (p.x, p.y),
        3 => x1 = p.x,
        4 => (x1, y1) = (p.x, p.y),
        5 => y1 = p.y,
        6 => (x0, y1) = (p.x, p.y),
        _ => x0 = p.x,
    }
    RectF::from_points(Point::new(x0, y0), Point::new(x1, y1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(kind: Kind) -> Object {
        Object { id: 1, kind, style: Style::default(), angle: 0.0 }
    }

    #[test]
    fn colour_hex_round_trip() {
        assert_eq!(Color::from_hex("#3584e4"), Some(Color::rgb(0x35, 0x84, 0xe4)));
        assert_eq!(Color::rgb(1, 2, 255).to_hex(), "#0102ff");
        assert_eq!(Color::from_hex("3584e4"), None);
        assert_eq!(Color::from_hex("#35"), None);
    }

    #[test]
    fn text_reads_black_on_light_and_white_on_dark() {
        assert_eq!(Color::rgb(0xf5, 0xc2, 0x11).contrast(), Color::BLACK);
        assert_eq!(Color::rgb(0xe0, 0x1b, 0x24).contrast(), Color::WHITE);
    }

    #[test]
    fn translate_moves_the_bounds() {
        let mut o = obj(Kind::Rect { rect: RectF { x: 10.0, y: 10.0, w: 20.0, h: 5.0 } });
        let before = o.bounds();
        o.translate(7.0, -3.0);
        let after = o.bounds();
        assert_eq!((after.x - before.x, after.y - before.y), (7.0, -3.0));
    }

    #[test]
    fn translate_moves_a_curve_and_a_callout_as_one() {
        let mut c = obj(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(10.0, 0.0), via: vec![Point::new(5.0, 4.0)] });
        c.translate(1.0, 2.0);
        assert_eq!(c.kind, Kind::Line { from: Point::new(1.0, 2.0), to: Point::new(11.0, 2.0), via: vec![Point::new(6.0, 6.0)] });
        let mut k = obj(Kind::Callout { tip: Point::new(0.0, 0.0), pos: Point::new(50.0, 50.0), text: "x".into(), size: Point::new(10.0, 10.0) });
        k.translate(5.0, 5.0);
        assert!(matches!(k.kind, Kind::Callout { tip, pos, .. } if tip == Point::new(5.0, 5.0) && pos == Point::new(55.0, 55.0)));
    }

    #[test]
    fn the_bounds_of_a_curve_hold_the_bulge_and_the_line_ends() {
        let mut o = obj(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(100.0, 0.0), via: vec![Point::new(50.0, 40.0)] });
        o.style.end = Cap::Arrow;
        let b = o.bounds();
        assert!(b.y + b.h >= 40.0 && b.x <= 0.0 && b.x + b.w >= 100.0, "{b:?}");
    }

    #[test]
    fn dragging_a_rectangle_handle_keeps_the_opposite_side_even_when_crossed() {
        let o = obj(Kind::Rect { rect: RectF { x: 10.0, y: 10.0, w: 40.0, h: 20.0 } });
        // The left side handle (7), dragged to the right of the right side: the rectangle flips.
        let Kind::Rect { rect } = o.with_handle(7, Point::new(80.0, 0.0)).kind else { unreachable!() };
        assert_eq!(rect, RectF { x: 50.0, y: 10.0, w: 30.0, h: 20.0 });
        // The corner handles move two sides at once.
        let Kind::Rect { rect } = o.with_handle(4, Point::new(70.0, 60.0)).kind else { unreachable!() };
        assert_eq!(rect, RectF { x: 10.0, y: 10.0, w: 60.0, h: 50.0 });
        assert_eq!(o.handles().len(), 8);
        assert_eq!(o.handles()[0], Point::new(10.0, 10.0));
    }

    #[test]
    fn a_curve_has_a_bend_handle_and_a_straight_line_does_not() {
        let straight = obj(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(10.0, 0.0), via: vec![] });
        assert_eq!(straight.handles().len(), 2);
        let curved = obj(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(10.0, 0.0), via: vec![Point::new(5.0, 3.0)] });
        assert_eq!(curved.handles()[2], Point::new(5.0, 3.0));
        let Kind::Line { via, .. } = curved.with_handle(2, Point::new(5.0, 9.0)).kind else { unreachable!() };
        assert_eq!(via, vec![Point::new(5.0, 9.0)]);
    }

    #[test]
    fn dragging_a_text_corner_resizes_the_text_and_keeps_the_opposite_corner() {
        let o = obj(Kind::Text { pos: Point::new(100.0, 100.0), text: "hi".into(), size: Point::new(40.0, 20.0) });
        let before = Object::text_box(Point::new(100.0, 100.0), Point::new(40.0, 20.0), &o.style, true);
        // The bottom-right corner (2) pulled out to twice the height.
        let bigger = o.with_handle(2, Point::new(before.x + before.w * 2.0, before.y + before.h * 2.0));
        assert!((bigger.style.font_size - o.style.font_size * 2.0).abs() < 1e-6);
        let Kind::Text { pos, size, .. } = bigger.kind else { unreachable!() };
        let after = Object::text_box(pos, size, &bigger.style, true);
        assert!((after.x - before.x).abs() < 1e-6 && (after.y - before.y).abs() < 1e-6, "the top-left corner stays");
    }

    #[test]
    fn a_step_badge_and_a_pen_stroke_resize_by_their_handles() {
        let step = obj(Kind::Step { center: Point::new(50.0, 50.0), number: 1 });
        let h = step.handles();
        assert_eq!(h.len(), 1);
        let big = step.with_handle(0, Point::new(50.0 + step.style.badge_extent() * 2.0, 50.0));
        assert!((big.style.badge - step.style.badge * 2.0).abs() < 1e-6);
        let pen = obj(Kind::Pen { points: vec![Point::new(0.0, 0.0), Point::new(10.0, 10.0)] });
        let wide = pen.with_handle(2, Point::new(20.0, 10.0));
        assert_eq!(wide.kind, Kind::Pen { points: vec![Point::new(0.0, 0.0), Point::new(20.0, 10.0)] });
    }

    #[test]
    fn a_magnifier_keeps_its_proportions() {
        let source = RectF { x: 10.0, y: 10.0, w: 40.0, h: 20.0 };
        let lens = Object::lens_for(source, 2.0, 400.0, 300.0);
        assert_eq!((lens.w, lens.h), (80.0, 40.0));
        assert!(lens.x >= source.x + source.w, "beside the area, on the side with room: {lens:?}");
        let m = obj(Kind::Magnify { source, lens });
        // A bigger area: the lens grows with it, about its own centre.
        let Kind::Magnify { source: s2, lens: l2 } = m.with_handle(2, Point::new(70.0, 50.0)).kind else { unreachable!() };
        assert!((l2.w / l2.h - s2.w / s2.h).abs() < 1e-9);
        assert!((l2.x + l2.w / 2.0 - (lens.x + lens.w / 2.0)).abs() < 1e-9);
        // The centre handle moves only the lens.
        let Kind::Magnify { source: s3, lens: l3 } = m.with_handle(8, Point::new(300.0, 200.0)).kind else { unreachable!() };
        assert_eq!(s3, source);
        assert_eq!((l3.x + l3.w / 2.0, l3.y + l3.h / 2.0), (300.0, 200.0));
    }

    #[test]
    fn mapping_points_turns_rectangles_but_keeps_text_upright() {
        // A quarter turn clockwise in a 100 x 50 picture: (x, y) -> (50 - y, x).
        let turn = |p: Point| Point::new(50.0 - p.y, p.x);
        let mut r = obj(Kind::Rect { rect: RectF { x: 10.0, y: 5.0, w: 30.0, h: 10.0 } });
        r.map_points(&turn);
        assert_eq!(r.kind, Kind::Rect { rect: RectF { x: 35.0, y: 10.0, w: 10.0, h: 30.0 } });
        let mut t = obj(Kind::Text { pos: Point::new(10.0, 10.0), text: "a".into(), size: Point::new(20.0, 10.0) });
        t.map_points(&turn);
        let Kind::Text { pos, size, .. } = t.kind else { unreachable!() };
        assert_eq!(size, Point::new(20.0, 10.0));
        assert_eq!((pos.x + 10.0, pos.y + 5.0), (35.0, 20.0), "its centre turned");
    }

    #[test]
    fn scaling_a_mark_scales_its_widths_and_text() {
        let mut o = obj(Kind::Line { from: Point::new(10.0, 10.0), to: Point::new(20.0, 10.0), via: vec![] });
        o.scale(0.5);
        assert_eq!(o.kind, Kind::Line { from: Point::new(5.0, 5.0), to: Point::new(10.0, 5.0), via: vec![] });
        assert_eq!(o.style.width, Style::default().width * 0.5);
        assert_eq!(o.style.font_size, Style::default().font_size * 0.5);
    }

    #[test]
    fn a_placed_picture_saves_and_loads_its_pixels() {
        let pixels = Pixels::new(Bitmap::filled(3, 2, [1, 2, 3, 255]));
        let o = obj(Kind::Image { rect: RectF { x: 0.0, y: 0.0, w: 3.0, h: 2.0 }, pixels });
        let json = serde_json::to_string(&o).unwrap();
        let back: Object = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }

    #[test]
    fn lines_saved_before_bends_were_a_list_still_open() {
        let none = r#"{"Line":{"from":{"x":0.0,"y":0.0},"to":{"x":9.0,"y":0.0},"via":null}}"#;
        let one = r#"{"Line":{"from":{"x":0.0,"y":0.0},"to":{"x":9.0,"y":0.0},"via":{"x":4.0,"y":3.0}}}"#;
        let many = r#"{"Line":{"from":{"x":0.0,"y":0.0},"to":{"x":9.0,"y":0.0},"via":[{"x":2.0,"y":3.0},{"x":6.0,"y":-3.0}]}}"#;
        let via = |j: &str| match serde_json::from_str::<Kind>(j).unwrap() {
            Kind::Line { via, .. } => via,
            _ => unreachable!(),
        };
        assert_eq!(via(none), Vec::<Point>::new());
        assert_eq!(via(one), vec![Point::new(4.0, 3.0)]);
        assert_eq!(via(many).len(), 2);
        // A mark saved before marks could turn reads as upright.
        let old = r#"{"id":3,"kind":{"Rect":{"rect":{"x":0.0,"y":0.0,"w":5.0,"h":5.0}}},"style":{}}"#;
        assert_eq!(serde_json::from_str::<Object>(old).unwrap().angle, 0.0);
    }

    #[test]
    fn bends_are_added_where_the_line_passes_and_removed_again() {
        let mut o = obj(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(100.0, 0.0), via: vec![] });
        // A straight line becomes a curve through the new point.
        assert_eq!(o.add_bend(Point::new(50.0, 20.0)), Some(2));
        // Near the first half of the curve: the new bend goes before the first one.
        assert_eq!(o.add_bend(Point::new(20.0, 12.0)), Some(2));
        let Kind::Line { via, .. } = &o.kind else { unreachable!() };
        assert_eq!(via, &vec![Point::new(20.0, 12.0), Point::new(50.0, 20.0)]);
        assert_eq!(o.handles().len(), 4);
        let moved = o.with_handle(3, Point::new(60.0, 30.0));
        let Kind::Line { via, .. } = &moved.kind else { unreachable!() };
        assert_eq!(via[1], Point::new(60.0, 30.0));
        assert!(o.remove_bend(2));
        assert!(!o.remove_bend(0), "the ends are not bends");
        let Kind::Line { via, .. } = &o.kind else { unreachable!() };
        assert_eq!(via, &vec![Point::new(50.0, 20.0)]);
        assert_eq!(obj(Kind::Rect { rect: RectF::default() }).add_bend(Point::new(1.0, 1.0)), None);
    }

    #[test]
    fn a_turned_mark_is_reshaped_in_its_own_frame_and_its_opposite_corner_stays() {
        let mut o = obj(Kind::Rect { rect: RectF { x: 100.0, y: 100.0, w: 80.0, h: 40.0 } });
        o.angle = 0.6;
        let before = o.handles();
        // The bottom-right corner (4) pulled outwards along the mark's own diagonal.
        let pulled = Point::new(before[4].x + 30.0, before[4].y + 25.0);
        let after = o.with_handle(4, pulled);
        let h = after.handles();
        assert!(h[0].distance(before[0]) < 1e-6, "the top-left corner stays: {:?} {:?}", h[0], before[0]);
        assert!(h[4].distance(pulled) < 1e-6, "the corner follows the pointer");
        assert_eq!(after.angle, 0.6);
        // Turning: the box grows to hold the turned shape; a full turn comes back.
        let upright = obj(Kind::Rect { rect: RectF { x: 0.0, y: 0.0, w: 100.0, h: 10.0 } });
        let quarter = upright.rotated(std::f64::consts::FRAC_PI_2);
        let b = quarter.bounds();
        assert!((b.w - 14.0).abs() < 1e-6 && (b.h - 104.0).abs() < 1e-6, "{b:?}");
        assert!(upright.rotated(std::f64::consts::TAU).angle.abs() < 1e-9);
        // A pen stroke turns its points instead.
        let pen = obj(Kind::Pen { points: vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)] });
        let turned = pen.rotated(std::f64::consts::FRAC_PI_2);
        assert_eq!(turned.angle, 0.0);
        let Kind::Pen { points } = turned.kind else { unreachable!() };
        assert!(points[0].distance(Point::new(5.0, -5.0)) < 1e-9 && points[1].distance(Point::new(5.0, 5.0)) < 1e-9);
        // Lines and redactions have no turning handle.
        assert!(obj(Kind::Redact { rect: RectF::default(), mode: RedactMode::Box }).rotate_handle(10.0).is_none());
        assert!(o.rotate_handle(10.0).is_some());
    }

    #[test]
    fn a_mirror_turns_a_turned_mark_the_other_way() {
        let mut o = obj(Kind::Ellipse { rect: RectF { x: 10.0, y: 10.0, w: 40.0, h: 20.0 } });
        o.angle = 0.3;
        o.map_points(&|p| Point::new(100.0 - p.x, p.y));
        assert_eq!(o.angle, -0.3);
        o.map_points(&|p| Point::new(50.0 - p.y, p.x));
        assert_eq!(o.angle, -0.3, "a quarter turn of the picture keeps it");
    }

    #[test]
    fn any_installed_font_can_be_named_and_saved() {
        let st = Style { font: Font::Family(FamilyName::new("  Noto \"Serif\" Display  ")), ..Style::default() };
        assert_eq!(st.font.family(), "Noto Serif Display");
        let json = serde_json::to_string(&st).unwrap();
        assert!(json.contains(r#""font":{"Family":"Noto Serif Display"}"#), "{json}");
        assert_eq!(serde_json::from_str::<Style>(&json).unwrap(), st);
        // Old styles still read; a very long name is cut on a character boundary.
        assert_eq!(serde_json::from_str::<Style>(r#"{"font":"Serif"}"#).unwrap().font, Font::Serif);
        let long = FamilyName::new(&"é".repeat(40));
        assert_eq!(long.as_str().chars().count(), 31);
    }

    #[test]
    fn a_box_around_text_does_not_move_the_text() {
        let size = Point::new(60.0, 30.0);
        let plain = Style { width: 0.0, ..Style::default() };
        let boxed = Style { fill: Some(Color::WHITE), width: 0.0, ..Style::default() };
        let a = Object::text_box(Point::new(10.0, 10.0), size, &plain, false);
        let b = Object::text_box(Point::new(10.0, 10.0), size, &boxed, false);
        assert_eq!((a.x, a.y, a.w, a.h), (10.0, 10.0, 60.0, 30.0));
        assert!(b.x < a.x && b.y < a.y && b.w > a.w && b.h > a.h);
    }
}
