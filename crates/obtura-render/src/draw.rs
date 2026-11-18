use cairo::{Context, Format, ImageSurface, LineCap, LineJoin};
use obtura_scene::{line_pieces, t_back_from_end, Cap, Color, Cubic, Dash, Kind, Object, Point, RectF, Shape, Style};
use pangocairo::pango;

fn set_color(cr: &Context, c: Color) {
    cr.set_source_rgba(c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0, c.a as f64 / 255.0);
}

/// Draws marks in order. Redactions and magnifiers are skipped: they work on pixels, see
/// [`crate::compose`]. All spotlights are drawn together, where the first one is: one shade
/// with a hole for each lit area (two separate shades would darken each other's holes).
pub fn draw_objects(cr: &Context, objects: &[Object]) {
    let mut spotlights_done = false;
    for o in objects {
        if matches!(o.kind, Kind::Spotlight { .. }) {
            if spotlights_done {
                continue;
            }
            spotlights_done = true;
            let lit: Vec<&Object> = objects.iter().filter(|o| matches!(o.kind, Kind::Spotlight { .. })).collect();
            let _ = cr.save();
            spotlight(cr, &lit);
            let _ = cr.restore();
            continue;
        }
        let _ = cr.save();
        // Several parts of one mark (outline and fill, line and head) fade together.
        let faded = o.style.opacity < 0.999;
        if faded {
            cr.push_group();
        }
        draw_one(cr, o);
        if faded {
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(o.style.opacity.clamp(0.0, 1.0));
        }
        let _ = cr.restore();
    }
}

fn draw_one(cr: &Context, o: &Object) {
    let s = &o.style;
    // A turned mark is drawn upright in a turned frame around its centre.
    let angle = o.turn();
    if angle != 0.0 {
        let c = o.center();
        cr.translate(c.x, c.y);
        cr.rotate(angle);
        cr.translate(-c.x, -c.y);
    }
    cr.set_line_cap(LineCap::Round);
    cr.set_line_join(LineJoin::Round);
    cr.set_line_width(s.width);
    set_color(cr, s.color);
    match &o.kind {
        Kind::Line { from, to, via } => line(cr, *from, *to, via, s),
        Kind::Rect { rect } => {
            shape_path(cr, Shape::Rect, *rect, s.corner);
            fill_and_outline(cr, s, s.corner > 0.0);
        }
        Kind::Ellipse { rect } => {
            if rect.w > 0.0 && rect.h > 0.0 {
                shape_path(cr, Shape::Ellipse, *rect, 0.0);
                fill_and_outline(cr, s, true);
            }
        }
        Kind::Pen { points } => {
            apply_dash(cr, s);
            stroke_path(cr, points, s.width);
        }
        Kind::Highlighter { points } => {
            cr.set_line_cap(LineCap::Butt);
            stroke_path(cr, points, s.width);
        }
        Kind::Text { pos, text, size } => {
            if s.has_box() {
                text_box(cr, Object::text_box(*pos, *size, s, false), s);
            }
            show_text(cr, *pos, text, s, s.text_color);
        }
        Kind::Callout { tip, pos, text, size } => {
            let rect = Object::text_box(*pos, *size, s, true);
            let leader = Object::leader_style(s);
            if !rect.contains(*tip) {
                let anchor = obtura_scene::nearest_side_midpoint(rect, *tip);
                cr.set_line_width(leader.width);
                line(cr, anchor, *tip, &[], &leader);
            }
            text_box(cr, rect, s);
            show_text(cr, *pos, text, s, s.text_color);
        }
        Kind::Step { center, number } => {
            let r = s.badge;
            let rect = RectF { x: center.x - r, y: center.y - r, w: 2.0 * r, h: 2.0 * r };
            shape_path(cr, s.shape, if s.shape == Shape::Diamond { rect.inflate(r * 0.3) } else { rect }, 0.0);
            fill_and_outline(cr, s, true);
            let layout = layout(cr, &number.to_string(), s);
            let (w, h) = layout.pixel_size();
            set_color(cr, s.text_color);
            cr.move_to(center.x - w as f64 / 2.0, center.y - h as f64 / 2.0);
            pangocairo::functions::show_layout(cr, &layout);
        }
        Kind::Redact { .. } | Kind::Magnify { .. } | Kind::Spotlight { .. } => {}
        Kind::Measure { from, to } => {
            line(cr, *from, *to, &[], s);
            // The length, in a pill over the middle of the line. The pill's size is fixed by
            // the scene (so the picture knows how much room it takes); the text is centred in it.
            let r = Object::measure_box(*from, *to, s);
            if let Some(bg) = s.fill {
                shape_path(cr, Shape::Rect, r, r.h / 2.0);
                set_color(cr, bg);
                let _ = cr.fill();
            }
            let label = Object::measure_label(*from, *to);
            let layout = layout(cr, &label, &Style { bold: true, ..*s });
            let (w, h) = layout.pixel_size();
            let k = (r.w * 0.92 / w.max(1) as f64).min(1.0);
            let _ = cr.save();
            cr.translate(r.x + r.w / 2.0, r.y + r.h / 2.0);
            // Never wider than its pill, whatever the font.
            cr.scale(k, k);
            set_color(cr, s.text_color);
            cr.move_to(-w as f64 / 2.0, -h as f64 / 2.0);
            pangocairo::functions::show_layout(cr, &layout);
            let _ = cr.restore();
        }
        Kind::Stamp { center, glyph } => {
            let layout = layout(cr, glyph, &Style { font: obtura_scene::Font::Sans, bold: false, italic: false, ..*s });
            if is_emoji(glyph) {
                let mut font = pango::FontDescription::new();
                font.set_family(EMOJI_FAMILY);
                font.set_absolute_size(s.font_size * pango::SCALE as f64);
                layout.set_font_description(Some(&font));
            }
            let (_, logical) = layout.pixel_extents();
            set_color(cr, s.color);
            cr.move_to(center.x - logical.width() as f64 / 2.0 - logical.x() as f64, center.y - logical.height() as f64 / 2.0 - logical.y() as f64);
            pangocairo::functions::show_layout(cr, &layout);
        }
        Kind::Image { rect, pixels } => {
            if rect.w < 0.5 || rect.h < 0.5 {
                return;
            }
            if let Ok(img) = crate::to_surface(&pixels.0) {
                let _ = cr.save();
                shape_path(cr, Shape::Rect, *rect, s.corner);
                cr.clip();
                cr.translate(rect.x, rect.y);
                cr.scale(rect.w / pixels.0.width.max(1) as f64, rect.h / pixels.0.height.max(1) as f64);
                let _ = cr.set_source_surface(&img, 0.0, 0.0);
                cr.source().set_filter(cairo::Filter::Good);
                let _ = cr.paint();
                let _ = cr.restore();
            }
            if s.width > 0.0 {
                shape_path(cr, Shape::Rect, *rect, s.corner);
                let st = Style { fill: None, ..*s };
                fill_and_outline(cr, &st, true);
            }
        }
    }
}

/// The fonts asked for colour emoji, in order.
pub const EMOJI_FAMILY: &str = "Noto Color Emoji, Apple Color Emoji, Twemoji, Sans";

/// Whether `glyph` is a colour emoji (rather than a symbol drawn in the mark's colour).
pub fn is_emoji(glyph: &str) -> bool {
    glyph.chars().any(|c| c == '\u{fe0f}' || (c as u32) >= 0x1f000 || matches!(c, '✅' | '❌' | '❓' | '❗'))
}

/// The shade around the lit areas. The style (colour, how dark) is the first spotlight's.
fn spotlight(cr: &Context, lit: &[&Object]) {
    let Some(first) = lit.first() else { return };
    let (x0, y0, x1, y1) = cr.clip_extents().unwrap_or((-1e5, -1e5, 1e5, 1e5));
    cr.set_fill_rule(cairo::FillRule::EvenOdd);
    cr.rectangle(x0, y0, x1 - x0, y1 - y0);
    for o in lit {
        if let Kind::Spotlight { rect } = o.kind {
            let shape = if o.style.shape == Shape::Ellipse { Shape::Ellipse } else { Shape::Rect };
            shape_path(cr, shape, rect, o.style.corner);
        }
    }
    let c = first.style.color;
    cr.set_source_rgba(c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0, first.style.opacity.clamp(0.0, 1.0));
    let _ = cr.fill();
}

/// The outlines of a magnifier: the area looked at, the lens and a line between them. Its
/// inside is drawn by [`crate::compose`], which has the pixels.
pub(crate) fn magnifier_frame(cr: &Context, o: &Object) {
    let Kind::Magnify { source, lens } = o.kind else { return };
    let s = &o.style;
    let shape = if s.shape == Shape::Ellipse { Shape::Ellipse } else { Shape::Rect };
    cr.set_line_cap(LineCap::Round);
    cr.set_line_join(LineJoin::Round);
    set_color(cr, s.color);
    let thin = (s.width * 0.5).max(1.0);
    // The line joins the two outlines along the line between their centres.
    let (a, b) = (center_of(source), center_of(lens));
    if let (Some(from), Some(to)) = (edge_toward(source, shape, a, b), edge_toward(lens, shape, b, a)) {
        if source.inflate(2.0).contains(to) || lens.inflate(2.0).contains(from) {
            // They overlap: no line.
        } else {
            cr.set_line_width(thin);
            cr.move_to(from.x, from.y);
            cr.line_to(to.x, to.y);
            let _ = cr.stroke();
        }
    }
    cr.set_line_width(thin);
    shape_path(cr, shape, source, 0.0);
    let _ = cr.stroke();
    if s.width > 0.0 {
        cr.set_line_width(s.width);
        shape_path(cr, shape, lens, s.corner);
        let _ = cr.stroke();
    }
}

fn center_of(r: RectF) -> Point {
    Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0)
}

/// Where the ray from `from` (the centre of `r`) toward `to` leaves the shape in `r`.
fn edge_toward(r: RectF, shape: Shape, from: Point, to: Point) -> Option<Point> {
    let d = Dir::between(from, to)?;
    let (hw, hh) = ((r.w / 2.0).max(0.5), (r.h / 2.0).max(0.5));
    let t = match shape {
        Shape::Ellipse => 1.0 / ((d.x / hw).powi(2) + (d.y / hh).powi(2)).sqrt(),
        _ => {
            let tx = if d.x.abs() > 1e-9 { hw / d.x.abs() } else { f64::INFINITY };
            let ty = if d.y.abs() > 1e-9 { hh / d.y.abs() } else { f64::INFINITY };
            tx.min(ty)
        }
    };
    Some(Point::new(from.x + d.x * t, from.y + d.y * t))
}

/// The path of the lens's shape, for clipping what it shows.
pub(crate) fn lens_path(cr: &Context, o: &Object) {
    if let Kind::Magnify { lens, .. } = o.kind {
        let shape = if o.style.shape == Shape::Ellipse { Shape::Ellipse } else { Shape::Rect };
        shape_path(cr, shape, lens, o.style.corner);
    }
}

/// Fills the current path with the style's fill, if any, then outlines it if the width is
/// above zero.
fn fill_and_outline(cr: &Context, s: &Style, round_join: bool) {
    if let Some(fill) = s.fill {
        set_color(cr, fill);
        let _ = cr.fill_preserve();
    }
    if s.width > 0.0 {
        set_color(cr, s.color);
        cr.set_line_join(if round_join { LineJoin::Round } else { LineJoin::Miter });
        apply_dash(cr, s);
        let _ = cr.stroke();
    } else {
        cr.new_path();
    }
}

fn apply_dash(cr: &Context, s: &Style) {
    let w = s.width.max(1.0);
    match s.dash {
        Dash::Solid => {}
        Dash::Dashed => {
            cr.set_line_cap(LineCap::Butt);
            cr.set_dash(&[w * 3.0, w * 2.0], 0.0);
        }
        Dash::Dotted => {
            cr.set_line_cap(LineCap::Round);
            cr.set_dash(&[0.01, w * 2.2], 0.0);
        }
    }
}

/// The path of a shape in `r`. A `Rect` with `corner` above zero has rounded corners.
fn shape_path(cr: &Context, shape: Shape, r: RectF, corner: f64) {
    match shape {
        Shape::Rect if corner <= 0.0 => cr.rectangle(r.x, r.y, r.w, r.h),
        Shape::Rect | Shape::Rounded => {
            let radius = if corner > 0.0 { corner } else { r.w.min(r.h) * 0.25 }.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
            let (x1, y1) = (r.x + r.w, r.y + r.h);
            let q = std::f64::consts::FRAC_PI_2;
            cr.new_sub_path();
            cr.arc(x1 - radius, r.y + radius, radius, -q, 0.0);
            cr.arc(x1 - radius, y1 - radius, radius, 0.0, q);
            cr.arc(r.x + radius, y1 - radius, radius, q, 2.0 * q);
            cr.arc(r.x + radius, r.y + radius, radius, 2.0 * q, 3.0 * q);
            cr.close_path();
        }
        Shape::Ellipse => {
            cr.save().ok();
            cr.translate(r.x + r.w / 2.0, r.y + r.h / 2.0);
            cr.scale((r.w / 2.0).max(0.001), (r.h / 2.0).max(0.001));
            cr.new_sub_path();
            cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
            cr.restore().ok(); // the path is kept; the line width is not squashed
        }
        Shape::Diamond => {
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            cr.move_to(cx, r.y);
            cr.line_to(r.x + r.w, cy);
            cr.line_to(cx, r.y + r.h);
            cr.line_to(r.x, cy);
            cr.close_path();
        }
    }
}

/// The background and border behind a text.
fn text_box(cr: &Context, rect: RectF, s: &Style) {
    shape_path(cr, Shape::Rect, rect, s.corner.min(rect.h / 2.0));
    let style = Style { dash: Dash::Solid, ..*s };
    fill_and_outline(cr, &style, true);
}

fn show_text(cr: &Context, pos: Point, text: &str, s: &Style, color: Color) {
    let layout = layout(cr, text, s);
    set_color(cr, color);
    cr.move_to(pos.x, pos.y);
    pangocairo::functions::show_layout(cr, &layout);
}

// ---- Lines, curves and their ends ----

#[derive(Clone, Copy)]
struct Dir {
    x: f64,
    y: f64,
}

impl Dir {
    fn between(a: Point, b: Point) -> Option<Dir> {
        let len = a.distance(b);
        (len > 1e-6).then(|| Dir { x: (b.x - a.x) / len, y: (b.y - a.y) / len })
    }
}

/// How far the line stops short of its tip so the end sits on a clean edge.
fn setback(cap: Cap, unit: f64, filled: bool) -> f64 {
    Object::cap_setback(cap, unit, filled)
}

/// Cuts `len` off the end `b` of the piece. Returns the piece left and the direction its end
/// points in.
fn trim_end(c: Cubic, len: f64) -> (Cubic, Option<Dir>) {
    if len <= 0.0 {
        let dir = Dir::between(c.c2, c.b).or_else(|| Dir::between(c.c1, c.b)).or_else(|| Dir::between(c.a, c.b));
        return (c, dir);
    }
    let t = t_back_from_end(&c, len);
    let head = c.head(t);
    (head, Dir::between(head.b, c.b))
}

fn line(cr: &Context, from: Point, to: Point, via: &[Point], s: &Style) {
    let total = match via {
        [] => from.distance(to),
        _ => line_pieces(from, via, to).iter().map(Cubic::hull_length).sum(),
    };
    if total < 0.5 {
        return;
    }
    // The same rules measure the ink, so the picture grows exactly as far as a line reaches.
    let unit = Object::cap_unit_for(from, to, via, s);
    let mut pieces = line_pieces(from, via, to);
    let last = pieces.len() - 1;

    // Shorten the line at each end that has a head, remembering where the heads go.
    let (trimmed, end_dir) = trim_end(pieces[last], if s.end == Cap::None { 0.0 } else { setback(s.end, unit, s.cap_filled) });
    pieces[last] = trimmed;
    let end_dir = end_dir.or_else(|| Dir::between(from, to));
    let (trimmed, start_dir) = trim_end(pieces[0].reversed(), if s.start == Cap::None { 0.0 } else { setback(s.start, unit, s.cap_filled) });
    pieces[0] = trimmed.reversed();
    let start_dir = start_dir.or_else(|| Dir::between(to, from));
    cr.save().ok();
    apply_dash(cr, s);
    cr.move_to(pieces[0].a.x, pieces[0].a.y);
    for p in &pieces {
        if via.is_empty() {
            cr.line_to(p.b.x, p.b.y);
        } else {
            cr.curve_to(p.c1.x, p.c1.y, p.c2.x, p.c2.y, p.b.x, p.b.y);
        }
    }
    let _ = cr.stroke();
    cr.restore().ok();

    cr.set_dash(&[], 0.0);
    cr.set_line_cap(LineCap::Round);
    if let Some(d) = end_dir {
        draw_cap(cr, s.end, to, d, unit, s);
    }
    if let Some(d) = start_dir {
        draw_cap(cr, s.start, from, d, unit, s);
    }
}

/// A line end at `tip`; `dir` is the direction the line is heading, out of the line.
fn draw_cap(cr: &Context, cap: Cap, tip: Point, dir: Dir, unit: f64, s: &Style) {
    let (ux, uy) = (dir.x, dir.y);
    let (px, py) = (-uy, ux);
    let at = |along: f64, across: f64| Point::new(tip.x + ux * along + px * across, tip.y + uy * along + py * across);
    let finish = |cr: &Context, thin: f64| {
        if s.cap_filled {
            let _ = cr.fill_preserve();
            cr.set_line_width(s.width * 0.4);
            let _ = cr.stroke();
        } else {
            cr.set_line_width(thin);
            let _ = cr.stroke();
        }
    };
    cr.set_line_join(LineJoin::Round);
    match cap {
        Cap::None => {}
        Cap::Arrow => {
            let (b1, b2) = (at(-unit, unit * 0.45), at(-unit, -unit * 0.45));
            cr.move_to(tip.x, tip.y);
            cr.line_to(b1.x, b1.y);
            cr.line_to(b2.x, b2.y);
            cr.close_path();
            finish(cr, s.width.max(1.0) * 0.7);
        }
        Cap::Chevron => {
            let (a, b) = (at(-unit * 0.8, unit * 0.55), at(-unit * 0.8, -unit * 0.55));
            cr.set_line_width(s.width);
            cr.move_to(a.x, a.y);
            cr.line_to(tip.x, tip.y);
            cr.line_to(b.x, b.y);
            let _ = cr.stroke();
        }
        Cap::Bullet => {
            cr.new_sub_path();
            cr.arc(tip.x, tip.y, (unit * 0.32).max(s.width * 0.9), 0.0, std::f64::consts::TAU);
            finish(cr, s.width.max(1.0) * 0.8);
        }
        Cap::Square => {
            let h = (unit * 0.28).max(s.width * 0.9);
            let pts = [at(h, h), at(h, -h), at(-h, -h), at(-h, h)];
            cr.move_to(pts[0].x, pts[0].y);
            pts[1..].iter().for_each(|p| cr.line_to(p.x, p.y));
            cr.close_path();
            finish(cr, s.width.max(1.0) * 0.8);
        }
        Cap::Diamond => {
            let h = (unit * 0.42).max(s.width);
            let pts = [at(h, 0.0), at(0.0, h * 0.8), at(-h, 0.0), at(0.0, -h * 0.8)];
            cr.move_to(pts[0].x, pts[0].y);
            pts[1..].iter().for_each(|p| cr.line_to(p.x, p.y));
            cr.close_path();
            finish(cr, s.width.max(1.0) * 0.8);
        }
        Cap::Bar => {
            let (a, b) = (at(0.0, unit * 0.5), at(0.0, -unit * 0.5));
            cr.set_line_width(s.width);
            cr.move_to(a.x, a.y);
            cr.line_to(b.x, b.y);
            let _ = cr.stroke();
        }
    }
}

/// A smooth stroke through the points (quadratic curves between midpoints).
fn stroke_path(cr: &Context, pts: &[Point], width: f64) {
    match pts {
        [] => {}
        [p] => {
            cr.arc(p.x, p.y, width / 2.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        [a, b] => {
            cr.move_to(a.x, a.y);
            cr.line_to(b.x, b.y);
            let _ = cr.stroke();
        }
        _ => {
            cr.move_to(pts[0].x, pts[0].y);
            let mut cur = pts[0];
            for i in 1..pts.len() - 1 {
                let (p, n) = (pts[i], pts[i + 1]);
                let mid = Point::new((p.x + n.x) / 2.0, (p.y + n.y) / 2.0);
                // Quadratic (cur, p, mid) as a cubic.
                cr.curve_to(cur.x + 2.0 / 3.0 * (p.x - cur.x), cur.y + 2.0 / 3.0 * (p.y - cur.y), mid.x + 2.0 / 3.0 * (p.x - mid.x), mid.y + 2.0 / 3.0 * (p.y - mid.y), mid.x, mid.y);
                cur = mid;
            }
            let last = pts[pts.len() - 1];
            cr.line_to(last.x, last.y);
            let _ = cr.stroke();
        }
    }
}

fn layout(cr: &Context, text: &str, s: &Style) -> pango::Layout {
    let layout = pangocairo::functions::create_layout(cr);
    let mut font = pango::FontDescription::new();
    font.set_family(s.font.family());
    font.set_weight(if s.bold { pango::Weight::Bold } else { pango::Weight::Normal });
    font.set_style(if s.italic { pango::Style::Italic } else { pango::Style::Normal });
    font.set_absolute_size(s.font_size * pango::SCALE as f64);
    layout.set_font_description(Some(&font));
    layout.set_text(text);
    layout
}

/// The size in pixels that `text` takes in the style's font, for the mark's `size` and hit testing.
pub fn measure_text(text: &str, s: &Style) -> (f64, f64) {
    let Ok(surface) = ImageSurface::create(Format::ARgb32, 1, 1) else { return (s.font_size * text.chars().count() as f64 * 0.6, s.font_size * 1.3) };
    let Ok(cr) = Context::new(&surface) else { return (0.0, 0.0) };
    let (w, h) = layout(&cr, text, s).pixel_size();
    (w as f64, h as f64)
}
