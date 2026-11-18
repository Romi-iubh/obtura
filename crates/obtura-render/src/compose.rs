use crate::{draw_objects, from_surface, to_surface};
use cairo::Context;
use obtura_image::{extend, redact_shaped, suggested_block, Bitmap, Rect, Redaction};
use obtura_scene::{canvas_of, Kind, Object, RectF, RedactMode, Scene, Shape};

/// The colour of the room added around the screenshot for marks that reach past its edges.
pub const MARGIN_FILL: [u8; 4] = [255, 255, 255, 255];

/// The finished picture: the base with every mark applied. Marks that reach past the edges
/// of the base make the picture bigger, white around the base (see [`canvas_of`]).
pub fn render(base: &Bitmap, scene: &Scene) -> Result<Bitmap, String> {
    render_on_canvas(base, scene.objects()).map(|(b, _)| b)
}

/// `objects` applied to `base` on the canvas they need, and that canvas in picture
/// coordinates (the base is at (0, 0); the result's top-left pixel is the canvas's corner).
pub fn render_on_canvas(base: &Bitmap, objects: &[Object]) -> Result<(Bitmap, Rect), String> {
    let c = canvas_of(objects, base.width, base.height);
    render_on(base, objects, c).map(|b| (b, c))
}

/// `objects` applied to `base` padded (white) out to `canvas`, a rectangle in picture
/// coordinates that holds the whole base.
pub fn render_on(base: &Bitmap, objects: &[Object], canvas: Rect) -> Result<Bitmap, String> {
    let c = canvas;
    if c == Rect::new(0, 0, base.width, base.height) {
        return compose(base, objects);
    }
    if c.x > 0 || c.y > 0 || (c.x as i64 + c.w as i64) < base.width as i64 || (c.y as i64 + c.h as i64) < base.height as i64 {
        return Err("the canvas must hold the whole picture".into());
    }
    let (left, top) = (c.x.unsigned_abs(), c.y.unsigned_abs());
    let padded = extend(base, left, top, c.w - base.width - left, c.h - base.height - top, MARGIN_FILL);
    let moved: Vec<Object> = objects
        .iter()
        .map(|o| {
            let mut o = o.clone();
            o.translate(left as f64, top as f64);
            o
        })
        .collect();
    compose(&padded, &moved)
}

/// Fills every redacted area of `picture` (a render of `objects` on the screenshot's area)
/// with the colour around it: what text reading gets, so hidden text is never read and the
/// reader is not thrown off by black boxes.
pub fn blank_redactions(picture: &mut Bitmap, objects: &[Object]) {
    for o in objects {
        if let Kind::Redact { rect, .. } = o.kind {
            let (x0, y0) = (rect.x.floor(), rect.y.floor());
            let (x1, y1) = ((rect.x + rect.w).ceil(), (rect.y + rect.h).ceil());
            obtura_image::blank(picture, Rect::new(x0 as i32, y0 as i32, (x1 - x0).max(0.0) as u32, (y1 - y0).max(0.0) as u32));
        }
    }
}

/// What text reading looks at: the marks drawn on the screenshot's area, redacted areas blank.
pub fn picture_to_read(base: &Bitmap, objects: &[Object]) -> Result<Bitmap, String> {
    let mut picture = compose(base, objects)?;
    blank_redactions(&mut picture, objects);
    Ok(picture)
}

/// Marks that work on the pixels below them (rather than drawing over them).
fn is_pixel_op(o: &Object) -> bool {
    matches!(o.kind, Kind::Redact { .. } | Kind::Magnify { .. })
}

/// How many of the scene's first marks end with the last redaction or magnifier (0 if there
/// is none). The live view caches `compose(base, &objects[..n])` and draws the rest on top.
pub fn underlay_len(scene: &Scene) -> usize {
    scene.objects().iter().rposition(is_pixel_op).map_or(0, |i| i + 1)
}

/// Applies marks in order. Marks between two redactions are drawn together; a redaction
/// then replaces the pixels under it, including those of marks drawn before it. A magnifier
/// shows the pixels as they are at its place in the order, except that every redaction of
/// the scene, even one added after it, is applied to what it shows: a lens never shows what
/// was hidden.
pub fn compose(base: &Bitmap, objects: &[Object]) -> Result<Bitmap, String> {
    let mut current = base.clone();
    let mut i = 0;
    while i < objects.len() {
        match &objects[i].kind {
            Kind::Redact { rect, mode } => {
                redact_object(&mut current, *rect, *mode, &objects[i]);
                i += 1;
            }
            Kind::Magnify { .. } => {
                let mut seen = current.clone();
                for later in &objects[i + 1..] {
                    if let Kind::Redact { rect, mode } = &later.kind {
                        redact_object(&mut seen, *rect, *mode, later);
                    }
                }
                current = magnify(&current, &seen, &objects[i])?;
                i += 1;
            }
            _ => {
                let end = objects[i..].iter().position(is_pixel_op).map_or(objects.len(), |p| i + p);
                current = draw_batch(&current, &objects[i..end])?;
                i = end;
            }
        }
    }
    Ok(current)
}

/// Draws the lens of a magnifier onto `base`: the area of `seen` enlarged, then the outlines.
fn magnify(base: &Bitmap, seen: &Bitmap, o: &Object) -> Result<Bitmap, String> {
    let Kind::Magnify { source, lens } = o.kind else { return Ok(base.clone()) };
    let mut surface = to_surface(base)?;
    if source.w >= 1.0 && source.h >= 1.0 {
        let picture = to_surface(seen)?;
        let cr = Context::new(&surface).map_err(|e| e.to_string())?;
        let _ = cr.save();
        crate::draw::lens_path(&cr, o);
        cr.clip();
        cr.translate(lens.x, lens.y);
        cr.scale(lens.w / source.w, lens.h / source.h);
        let _ = cr.set_source_surface(&picture, -source.x, -source.y);
        cr.source().set_filter(cairo::Filter::Good);
        // An area at the edge of the picture: the edge pixels fill the rest of the lens.
        cr.source().set_extend(cairo::Extend::Pad);
        let _ = cr.paint();
        let _ = cr.restore();
        crate::draw::magnifier_frame(&cr, o);
    }
    from_surface(&mut surface)
}

fn draw_batch(base: &Bitmap, objects: &[Object]) -> Result<Bitmap, String> {
    let mut surface = to_surface(base)?;
    {
        let cr = Context::new(&surface).map_err(|e| e.to_string())?;
        draw_objects(&cr, objects);
    }
    from_surface(&mut surface)
}

fn redact_object(bitmap: &mut Bitmap, rect: RectF, mode: RedactMode, o: &Object) {
    // Round the corners, not the size, so neighbouring rectangles meet without a gap.
    let (x0, y0) = (rect.x.round() as i32, rect.y.round() as i32);
    let (x1, y1) = ((rect.x + rect.w).round() as i32, (rect.y + rect.h).round() as i32);
    let (w, h) = ((x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32);
    // The user may pick the block size; otherwise one that suits the area.
    let block = if o.style.strength >= 2.0 { o.style.strength.round() as u32 } else { suggested_block(w, h) };
    let how = match mode {
        RedactMode::Box => Redaction::Box { rgb: [o.style.color.r, o.style.color.g, o.style.color.b] },
        RedactMode::Pixelate => Redaction::Pixelate { block },
        RedactMode::Blur => Redaction::Blur { block },
    };
    redact_shaped(bitmap, Rect::new(x0, y0, w, h), how, o.style.shape == Shape::Ellipse);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure_text;
    use obtura_image::{decode_png, encode_png};
    use obtura_scene::{Cap, Color, Dash, Point, Style};

    const RED: Color = Color::rgb(255, 0, 0);

    fn style(width: f64, filled: bool) -> Style {
        Style { color: RED, width, fill: filled.then_some(RED), font_size: 24.0, text_color: RED, ..Style::default() }
    }

    fn white(w: u32, h: u32) -> Bitmap {
        Bitmap::filled(w, h, [255, 255, 255, 255])
    }

    fn scene_with(kind: Kind, st: Style) -> Scene {
        let mut s = Scene::new();
        s.add(kind, st);
        s
    }

    #[test]
    fn an_empty_scene_changes_nothing() {
        let b = decode_png(include_bytes!("../../../tests/fixtures/sample.png")).unwrap();
        assert_eq!(render(&b, &Scene::new()).unwrap(), b);
    }

    #[test]
    fn a_filled_rectangle_colours_only_its_inside() {
        let s = scene_with(Kind::Rect { rect: RectF { x: 10.0, y: 10.0, w: 20.0, h: 10.0 } }, style(2.0, true));
        let out = render(&white(50, 40), &s).unwrap();
        assert_eq!(out.pixel(20, 15), [255, 0, 0, 255]);
        assert_eq!(out.pixel(5, 15), [255, 255, 255, 255]);
        assert_eq!(out.pixel(20, 30), [255, 255, 255, 255]);
    }

    #[test]
    fn an_outlined_rectangle_leaves_its_inside_alone() {
        let s = scene_with(Kind::Rect { rect: RectF { x: 10.0, y: 10.0, w: 30.0, h: 20.0 } }, style(4.0, false));
        let out = render(&white(60, 50), &s).unwrap();
        assert_eq!(out.pixel(10, 20), [255, 0, 0, 255]);
        assert_eq!(out.pixel(25, 20), [255, 255, 255, 255]);
    }

    #[test]
    fn a_line_inks_along_its_length_only() {
        let s = scene_with(Kind::Line { from: Point::new(5.0, 20.0), to: Point::new(55.0, 20.0), via: vec![] }, style(4.0, false));
        let out = render(&white(60, 40), &s).unwrap();
        assert_eq!(out.pixel(30, 20), [255, 0, 0, 255]);
        assert_eq!(out.pixel(30, 30), [255, 255, 255, 255]);
    }

    #[test]
    fn an_arrow_has_a_head_wider_than_its_line() {
        let s = scene_with(Kind::Line { from: Point::new(5.0, 30.0), to: Point::new(95.0, 30.0), via: vec![] }, Style { end: Cap::Arrow, ..style(4.0, false) });
        let out = render(&white(100, 60), &s).unwrap();
        let inked = |x: u32| (0..60).filter(|&y| out.pixel(x, y) != [255, 255, 255, 255]).count();
        assert!(inked(30) <= 6, "the line is thin: {}", inked(30));
        assert!(inked(85) >= 12, "the head is wide: {}", inked(85));
    }

    #[test]
    fn a_pen_stroke_draws_through_its_points() {
        let pts = vec![Point::new(5.0, 5.0), Point::new(25.0, 30.0), Point::new(45.0, 5.0), Point::new(60.0, 30.0)];
        let out = render(&white(70, 40), &scene_with(Kind::Pen { points: pts }, style(3.0, false))).unwrap();
        assert!(out.pixel(5, 5) != [255, 255, 255, 255]);
        assert!(out.pixel(60, 30) != [255, 255, 255, 255]);
        assert_eq!(out.pixel(35, 35), [255, 255, 255, 255]);
    }

    #[test]
    fn the_highlighter_lets_the_picture_show_through() {
        let base = Bitmap::filled(60, 20, [0, 0, 255, 255]);
        let pts = vec![Point::new(5.0, 10.0), Point::new(55.0, 10.0)];
        let out = render(&base, &scene_with(Kind::Highlighter { points: pts }, Style { opacity: 0.4, ..style(10.0, false) })).unwrap();
        let p = out.pixel(30, 10);
        assert!(p[0] > 60 && p[2] > 100, "mixed with the blue below, not covering it: {p:?}");
    }

    #[test]
    fn text_is_measured_and_drawn_inside_its_box() {
        let (w, h) = measure_text("Hello", &style(2.0, false));
        assert!(w > 20.0 && h > 15.0, "{w} x {h}");
        let (w2, _) = measure_text("Hello Hello", &style(2.0, false));
        assert!(w2 > w * 1.5);
        let kind = Kind::Text { pos: Point::new(10.0, 10.0), text: "Hello".into(), size: Point::new(w, h) };
        let out = render(&white(200, 80), &scene_with(kind, style(2.0, false))).unwrap();
        let inked = (10..(10 + h as u32 + 1)).flat_map(|y| (10..(10 + w as u32 + 1)).map(move |x| (x, y))).filter(|&(x, y)| out.pixel(x, y) != [255, 255, 255, 255]).count();
        assert!(inked > 30, "{inked} inked pixels");
        assert_eq!(out.pixel(150, 60), [255, 255, 255, 255]);
    }

    #[test]
    fn a_step_badge_is_a_filled_circle() {
        let badge = Style { shape: Shape::Ellipse, width: 0.0, ..style(2.0, true) };
        let out = render(&white(80, 80), &scene_with(Kind::Step { center: Point::new(40.0, 40.0), number: 3 }, badge)).unwrap();
        let r = Object::step_radius(&badge);
        assert_eq!(out.pixel((40.0 - r * 0.8) as u32, 40), [255, 0, 0, 255]);
        assert_eq!(out.pixel(2, 2), [255, 255, 255, 255]);
        assert_eq!(out.pixel(40 - r as u32 + 1, 40 - r as u32 + 1), [255, 255, 255, 255], "a circle has no corners");
    }

    #[test]
    fn a_square_badge_has_corners_and_a_diamond_has_none() {
        let draw = |shape| {
            let badge = Style { shape, width: 0.0, text_color: RED, ..style(2.0, true) };
            render(&white(80, 80), &scene_with(Kind::Step { center: Point::new(40.0, 40.0), number: 1 }, badge)).unwrap()
        };
        let corner = (40.0 - Style::default().badge + 2.0) as u32;
        assert_eq!(draw(Shape::Rect).pixel(corner, corner), [255, 0, 0, 255]);
        assert_eq!(draw(Shape::Diamond).pixel(corner, corner), [255, 255, 255, 255]);
        assert_eq!(draw(Shape::Diamond).pixel(40, 40 - 25), [255, 0, 0, 255], "a diamond reaches further than a circle on its points");
    }

    #[test]
    fn a_line_end_leaves_its_mark_at_the_end_only() {
        // The thickest column of ink within the last stretch, and in the middle.
        let thickest = |cap: Cap, xs: std::ops::Range<u32>| {
            let st = Style { end: cap, cap_filled: true, ..style(4.0, false) };
            let out = render(&white(120, 60), &scene_with(Kind::Line { from: Point::new(10.0, 30.0), to: Point::new(100.0, 30.0), via: vec![] }, st)).unwrap();
            xs.map(|x| (0..60).filter(|&y| out.pixel(x, y) != [255, 255, 255, 255]).count()).max().unwrap()
        };
        let plain = thickest(Cap::None, 75..110);
        assert!(plain <= 6, "{plain}");
        for cap in [Cap::Arrow, Cap::Chevron, Cap::Bullet, Cap::Square, Cap::Diamond, Cap::Bar] {
            assert!(thickest(cap, 75..110) >= plain + 4, "{cap:?} is wider than the plain line at the end");
            assert!(thickest(cap, 30..70) <= 6, "{cap:?}: the middle stays a thin line");
        }
    }

    #[test]
    fn a_spline_bends_through_its_middle_point_and_ends_where_it_was_drawn() {
        let kind = Kind::Line { from: Point::new(10.0, 50.0), to: Point::new(110.0, 50.0), via: vec![Point::new(60.0, 20.0)] };
        let out = render(&white(120, 70), &scene_with(kind, style(3.0, false))).unwrap();
        assert!(out.pixel(60, 20) != [255, 255, 255, 255], "the curve passes through the middle point");
        assert_eq!(out.pixel(60, 50), [255, 255, 255, 255], "not along the straight chord");
        assert!(out.pixel(10, 50) != [255, 255, 255, 255] && out.pixel(110, 50) != [255, 255, 255, 255]);
    }

    #[test]
    fn a_dashed_line_has_gaps_and_a_solid_one_does_not() {
        let gaps = |dash| {
            let out = render(&white(120, 20), &scene_with(Kind::Line { from: Point::new(5.0, 10.0), to: Point::new(115.0, 10.0), via: vec![] }, Style { dash, ..style(3.0, false) })).unwrap();
            (5..115).filter(|&x| out.pixel(x, 10) == [255, 255, 255, 255]).count()
        };
        assert_eq!(gaps(Dash::Solid), 0);
        assert!(gaps(Dash::Dashed) > 20);
        assert!(gaps(Dash::Dotted) > 20);
    }

    #[test]
    fn a_text_box_draws_its_background_and_border_around_text_that_stays_put() {
        let kind = |st: Style| {
            let (w, h) = measure_text("Hi", &st);
            let k = Kind::Text { pos: Point::new(30.0, 30.0), text: "Hi".into(), size: Point::new(w, h) };
            render(&white(120, 100), &scene_with(k, st)).unwrap()
        };
        let plain = Style { width: 0.0, ..style(0.0, false) };
        let boxed = Style { fill: Some(Color::rgb(0, 0, 255)), color: Color::rgb(0, 160, 0), width: 3.0, ..plain };
        let out = kind(boxed);
        let pad = boxed.text_pad() as u32;
        assert_eq!(out.pixel(30 - pad / 2, 32), [0, 0, 255, 255], "background just outside the text");
        assert_eq!(out.pixel(30 - pad, 40), [0, 160, 0, 255], "border at the edge of the box");
        assert_eq!(kind(plain).pixel(30 - pad / 2, 32), [255, 255, 255, 255], "no box, no background");
    }

    #[test]
    fn a_callout_has_a_box_a_line_and_an_arrow_at_the_first_point() {
        let st = Style { fill: Some(Color::WHITE), width: 3.0, ..style(3.0, false) };
        let k = Kind::Callout { tip: Point::new(15.0, 15.0), pos: Point::new(90.0, 60.0), text: "ok".into(), size: Point::new(30.0, 24.0) };
        let out = render(&Bitmap::filled(160, 110, [200, 200, 200, 255]), &scene_with(k, st)).unwrap();
        assert!(out.pixel(15, 15) != [200, 200, 200, 255], "the arrow tip is drawn at the first point");
        assert_eq!(out.pixel(92, 62), [255, 255, 255, 255], "the box is filled");
        assert!((0..160).any(|x| out.pixel(x, 60 - st.text_pad() as u32) == [255, 0, 0, 255]), "the box has a border");
        let on_line = (20..80).filter(|&i| out.pixel(i, i) == [255, 0, 0, 255]).count();
        assert!(on_line > 5, "a line joins the tip to the box: {on_line}");
    }

    #[test]
    fn a_faded_mark_lets_the_picture_show_through() {
        let base = Bitmap::filled(40, 40, [0, 0, 255, 255]);
        let st = Style { opacity: 0.5, ..style(2.0, true) };
        let out = render(&base, &scene_with(Kind::Rect { rect: RectF { x: 5.0, y: 5.0, w: 30.0, h: 30.0 } }, st)).unwrap();
        let p = out.pixel(20, 20);
        assert!(p[0] > 100 && p[0] < 160 && p[2] > 100 && p[2] < 160, "{p:?}");
    }

    // ---- Redaction, end to end: scene -> render -> PNG file -> read back. ----

    fn noise(w: u32, h: u32, seed: u32) -> Bitmap {
        let mut b = white(w, h);
        let mut s = seed.wrapping_mul(2654435761).wrapping_add(7);
        for y in 0..h {
            for x in 0..w {
                let mut p = [0, 0, 0, 255];
                for c in p.iter_mut().take(3) {
                    s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                    *c = (s >> 24) as u8;
                }
                b.set_pixel(x, y, p);
            }
        }
        b
    }

    fn exported(base: &Bitmap, scene: &Scene) -> Bitmap {
        decode_png(&encode_png(&render(base, scene).unwrap()).unwrap()).unwrap()
    }

    fn area(b: &Bitmap, r: Rect) -> Vec<u8> {
        b.crop(r).unwrap().rgba
    }

    #[test]
    fn a_blur_in_the_exported_file_holds_only_block_averages() {
        // Two pictures with identical 6x6-block averages but different pixels...
        let (w, h, block) = (60u32, 48u32, suggested_block(36, 30));
        assert_eq!(block, 6);
        let a = noise(w, h, 11);
        let mut b = a.clone();
        for by in (9..9 + 30).step_by(6) {
            for bx in (12..12 + 36).step_by(6) {
                let coords: Vec<(u32, u32)> = (by..by + 6).flat_map(|y| (bx..bx + 6).map(move |x| (x, y))).collect();
                for (i, &(x, y)) in coords.iter().enumerate() {
                    b.set_pixel(x, y, a.pixel(coords[coords.len() - 1 - i].0, coords[coords.len() - 1 - i].1));
                }
            }
        }
        let r = Rect::new(12, 9, 36, 30);
        assert!(area(&a, r) != area(&b, r));
        // ...are blurred, saved as PNG and read back: the two results are identical.
        for mode in [RedactMode::Blur, RedactMode::Pixelate] {
            let scene = scene_with(Kind::Redact { rect: RectF { x: 12.0, y: 9.0, w: 36.0, h: 30.0 }, mode }, style(1.0, false));
            let (ea, eb) = (exported(&a, &scene), exported(&b, &scene));
            let differing = area(&ea, r).iter().zip(area(&eb, r)).filter(|(x, y)| **x != *y).count();
            assert_eq!(differing, 0, "{mode:?}: the saved file still depends on the original pixels");
            assert_eq!(ea.pixel(0, 0), a.pixel(0, 0), "outside the rectangle nothing changes");
        }
    }

    #[test]
    fn a_round_blur_in_the_exported_file_still_holds_only_block_averages_and_keeps_the_corners() {
        let (w, h) = (60u32, 48u32);
        let a = noise(w, h, 21);
        let mut b = a.clone();
        for by in (9..9 + 30).step_by(6) {
            for bx in (12..12 + 36).step_by(6) {
                let coords: Vec<(u32, u32)> = (by..by + 6).flat_map(|y| (bx..bx + 6).map(move |x| (x, y))).collect();
                for (i, &(x, y)) in coords.iter().enumerate() {
                    let (sx, sy) = coords[coords.len() - 1 - i];
                    b.set_pixel(x, y, a.pixel(sx, sy));
                }
            }
        }
        let round = Style { shape: Shape::Ellipse, ..style(1.0, false) };
        for mode in [RedactMode::Blur, RedactMode::Pixelate, RedactMode::Box] {
            let scene = scene_with(Kind::Redact { rect: RectF { x: 12.0, y: 9.0, w: 36.0, h: 30.0 }, mode }, Style { strength: 6.0, ..round });
            let (ea, eb) = (exported(&a, &scene), exported(&b, &scene));
            // The centre is the same whatever the original was; the corner keeps the picture.
            assert_eq!(ea.pixel(30, 24), eb.pixel(30, 24), "{mode:?}");
            assert_eq!(area(&ea, Rect::new(22, 19, 16, 10)), area(&eb, Rect::new(22, 19, 16, 10)), "{mode:?}: the saved file depends on the original pixels");
            assert_eq!(ea.pixel(13, 10), a.pixel(13, 10), "{mode:?}: the corner of the rectangle is outside the ellipse");
        }
    }

    #[test]
    fn the_chosen_strength_sets_the_block_size() {
        let a = noise(80, 60, 31);
        let mut s = Scene::new();
        let st = Style { strength: 20.0, ..style(1.0, false) };
        s.add(Kind::Redact { rect: RectF { x: 0.0, y: 0.0, w: 80.0, h: 60.0 }, mode: RedactMode::Pixelate }, st);
        let out = render(&a, &s).unwrap();
        let mut colours: Vec<[u8; 4]> = (0..60).flat_map(|y| (0..80).map(move |x| (x, y))).map(|(x, y)| out.pixel(x, y)).collect();
        colours.sort();
        colours.dedup();
        assert!(colours.len() <= 12, "20-pixel blocks make at most 4 x 3 colours, got {}", colours.len());
    }

    #[test]
    fn a_box_in_the_exported_file_does_not_depend_on_the_original() {
        let r = Rect::new(5, 5, 30, 20);
        let scene = scene_with(Kind::Redact { rect: RectF { x: 5.0, y: 5.0, w: 30.0, h: 20.0 }, mode: RedactMode::Box }, Style { color: Color::rgb(0, 0, 0), ..style(1.0, false) });
        let (ea, eb) = (exported(&noise(50, 40, 1), &scene), exported(&noise(50, 40, 2), &scene));
        assert!(area(&ea, r) == area(&eb, r));
        assert_eq!(ea.pixel(10, 10), [0, 0, 0, 255]);
    }

    #[test]
    fn a_redaction_also_destroys_marks_under_it_but_not_those_above() {
        let mut scene = Scene::new();
        let green = Style { color: Color::rgb(0, 160, 0), width: 6.0, fill: Some(Color::rgb(0, 160, 0)), font_size: 20.0, ..Style::default() };
        // Below the redaction: a secret mark.
        scene.add(Kind::Rect { rect: RectF { x: 10.0, y: 10.0, w: 40.0, h: 20.0 } }, green);
        scene.add(Kind::Redact { rect: RectF { x: 5.0, y: 5.0, w: 50.0, h: 30.0 }, mode: RedactMode::Box }, Style { color: Color::rgb(0, 0, 0), ..style(1.0, false) });
        // Above it: stays visible.
        scene.add(Kind::Rect { rect: RectF { x: 20.0, y: 12.0, w: 10.0, h: 10.0 } }, style(2.0, true));
        let out = render(&white(80, 50), &scene).unwrap();
        assert_eq!(out.pixel(15, 15), [0, 0, 0, 255], "the green mark under the box is gone");
        assert_eq!(out.pixel(25, 17), [255, 0, 0, 255], "the red mark above it is drawn");
        assert_eq!(underlay_len(&scene), 2);
    }

    #[test]
    fn a_magnifier_shows_the_area_enlarged_in_its_lens() {
        // A picture with one red pixel block at (10..12, 10..12).
        let mut base = white(200, 100);
        for y in 10..12 {
            for x in 10..12 {
                base.set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        let source = RectF { x: 8.0, y: 8.0, w: 6.0, h: 6.0 };
        let lens = RectF { x: 100.0, y: 20.0, w: 60.0, h: 60.0 };
        let st = Style { shape: Shape::Rect, width: 0.0, ..style(2.0, false) };
        let s = scene_with(Kind::Magnify { source, lens }, st);
        let out = render(&base, &s).unwrap();
        // The red block, 10x bigger, sits at lens + (2..4) * 10.
        assert_eq!(out.pixel(130, 50), [255, 0, 0, 255]);
        assert_eq!(out.pixel(105, 25), [255, 255, 255, 255]);
        assert_eq!(underlay_len(&s), 1);
    }

    #[test]
    fn a_lens_never_shows_what_a_later_redaction_hides() {
        // Two pictures with the same 6x6-block averages in the area but different pixels.
        let a = noise(120, 60, 41);
        let mut b = a.clone();
        for by in (6..6 + 24).step_by(6) {
            for bx in (6..6 + 24).step_by(6) {
                let coords: Vec<(u32, u32)> = (by..by + 6).flat_map(|y| (bx..bx + 6).map(move |x| (x, y))).collect();
                for (i, &(x, y)) in coords.iter().enumerate() {
                    let (sx, sy) = coords[coords.len() - 1 - i];
                    b.set_pixel(x, y, a.pixel(sx, sy));
                }
            }
        }
        let area = RectF { x: 6.0, y: 6.0, w: 24.0, h: 24.0 };
        let lens = RectF { x: 60.0, y: 6.0, w: 48.0, h: 48.0 };
        for mode in [RedactMode::Blur, RedactMode::Pixelate, RedactMode::Box] {
            let mut s = Scene::new();
            // The magnifier first, the redaction of what it looks at afterwards.
            s.add(Kind::Magnify { source: area, lens }, Style { shape: Shape::Rect, width: 0.0, ..style(1.0, false) });
            s.add(Kind::Redact { rect: area, mode }, Style { strength: 6.0, color: Color::BLACK, ..style(1.0, false) });
            let (ea, eb) = (exported(&a, &s), exported(&b, &s));
            let lens_px = Rect::new(60, 6, 48, 48);
            assert_eq!(area_of(&ea, lens_px), area_of(&eb, lens_px), "{mode:?}: the lens still shows the original pixels");
        }
    }

    fn area_of(b: &Bitmap, r: Rect) -> Vec<u8> {
        b.crop(r).unwrap().rgba
    }

    #[test]
    fn a_spotlight_darkens_everything_but_its_area_and_two_do_not_darken_each_other() {
        let mut s = Scene::new();
        let st = Style { color: Color::BLACK, opacity: 0.5, ..Style::default() };
        s.add(Kind::Spotlight { rect: RectF { x: 10.0, y: 10.0, w: 20.0, h: 20.0 } }, st);
        s.add(Kind::Spotlight { rect: RectF { x: 50.0, y: 10.0, w: 20.0, h: 20.0 } }, st);
        let out = render(&white(100, 50), &s).unwrap();
        assert_eq!(out.pixel(20, 20), [255, 255, 255, 255], "first lit area");
        assert_eq!(out.pixel(60, 20), [255, 255, 255, 255], "second lit area");
        let dim = out.pixel(40, 40);
        assert!(dim[0] > 100 && dim[0] < 150, "half dark outside: {dim:?}");
    }

    #[test]
    fn a_placed_picture_is_drawn_stretched_to_its_rectangle() {
        let pixels = obtura_scene::Pixels::new(Bitmap::filled(2, 2, [0, 0, 255, 255]));
        let s = scene_with(Kind::Image { rect: RectF { x: 10.0, y: 10.0, w: 40.0, h: 20.0 }, pixels }, Style { width: 0.0, ..Style::default() });
        let out = render(&white(60, 40), &s).unwrap();
        assert_eq!(out.pixel(30, 20), [0, 0, 255, 255]);
        assert_eq!(out.pixel(55, 35), [255, 255, 255, 255]);
    }

    #[test]
    fn a_stamp_draws_its_symbol_around_its_centre() {
        let st = Style { color: RED, font_size: 40.0, ..Style::default() };
        let out = render(&white(100, 100), &scene_with(Kind::Stamp { center: Point::new(50.0, 50.0), glyph: "■".into() }, st)).unwrap();
        let inked = (30..70).flat_map(|y| (30..70).map(move |x| (x, y))).filter(|&(x, y)| out.pixel(x, y) != [255, 255, 255, 255]).count();
        assert!(inked > 200, "{inked}");
        assert_eq!(out.pixel(5, 5), [255, 255, 255, 255]);
    }

    #[test]
    fn underlay_len_is_zero_without_redactions() {
        let s = scene_with(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(5.0, 5.0), via: vec![] }, style(2.0, false));
        assert_eq!(underlay_len(&s), 0);
    }

    /// A picture of every mark, for looking at: `OBTURA_GALLERY=/path/gallery.png cargo test gallery -- --ignored`.
    #[test]
    #[ignore]
    fn gallery() {
        let Ok(path) = std::env::var("OBTURA_GALLERY") else { return };
        let mut scene = Scene::new();
        let base = Style::default();
        for (i, cap) in obtura_scene::CAPS.into_iter().enumerate() {
            let y = 30.0 + i as f64 * 34.0;
            scene.add(Kind::Line { from: Point::new(20.0, y), to: Point::new(150.0, y), via: vec![] }, Style { end: cap, start: if i == 3 { Cap::Bullet } else { Cap::None }, width: 3.0, ..base });
            scene.add(Kind::Line { from: Point::new(180.0, y + 10.0), to: Point::new(330.0, y + 10.0), via: vec![Point::new(255.0, y - 16.0)] }, Style { end: cap, cap_filled: i % 2 == 0, width: 3.0, ..base });
        }
        for (i, shape) in [Shape::Ellipse, Shape::Rect, Shape::Rounded, Shape::Diamond].into_iter().enumerate() {
            scene.add(Kind::Step { center: Point::new(380.0 + i as f64 * 60.0, 40.0), number: i as u32 + 1 }, Style { shape, fill: Some(base.color), text_color: Color::WHITE, width: 0.0, font_size: 24.0, ..base });
        }
        scene.add(Kind::Step { center: Point::new(380.0, 110.0), number: 7 }, Style { shape: Shape::Ellipse, fill: Some(Color::rgb(0x33, 0xd1, 0x7a)), color: Color::rgb(0x1c, 0x71, 0xd8), width: 4.0, text_color: Color::BLACK, font: obtura_scene::Font::Serif, italic: true, font_size: 22.0, ..base });
        let t = Style { fill: Some(Color::rgb(255, 255, 200)), color: Color::rgb(0x1c, 0x71, 0xd8), width: 2.0, text_color: Color::BLACK, corner: 6.0, font_size: 22.0, bold: false, ..base };
        let (w, h) = measure_text("Text box", &t);
        scene.add(Kind::Text { pos: Point::new(380.0, 160.0), text: "Text box".into(), size: Point::new(w, h) }, t);
        let c = Style { fill: Some(Color::WHITE), width: 3.0, text_color: Color::BLACK, corner: 6.0, font_size: 20.0, ..base };
        let (w, h) = measure_text("Look here", &c);
        scene.add(Kind::Callout { tip: Point::new(400.0, 260.0), pos: Point::new(500.0, 230.0), text: "Look here".into(), size: Point::new(w, h) }, c);
        scene.add(Kind::Rect { rect: RectF { x: 380.0, y: 290.0, w: 90.0, h: 50.0 } }, Style { fill: Some(Color { a: 90, ..Color::rgb(0x33, 0xd1, 0x7a) }), corner: 10.0, dash: Dash::Dashed, ..base });
        let noise = noise(640, 380, 5);
        scene.add(Kind::Redact { rect: RectF { x: 490.0, y: 280.0, w: 100.0, h: 80.0 }, mode: RedactMode::Blur }, Style { shape: Shape::Ellipse, ..base });
        std::fs::write(path, encode_png(&render(&noise, &scene).unwrap()).unwrap()).unwrap();
    }

    // ---- Room for marks past the edge ----

    #[test]
    fn a_mark_past_the_edge_gets_white_room_and_loses_it_when_deleted() {
        let base = Bitmap::filled(60, 40, [0, 0, 255, 255]);
        let mut s = Scene::new();
        let arrow = s.add(Kind::Line { from: Point::new(30.0, 20.0), to: Point::new(-20.0, 20.0), via: vec![] }, style(4.0, false));
        let out = render(&base, &s).unwrap();
        assert_eq!((out.width, out.height), (82, 40), "22 px more on the left: the line and half its width");
        assert_eq!(out.pixel(0, 0), MARGIN_FILL, "the room is white");
        assert_eq!(out.pixel(10, 20), [255, 0, 0, 255], "the part past the edge is drawn whole");
        assert_eq!(out.pixel(22, 0), [0, 0, 255, 255], "the screenshot starts after the room");
        assert_eq!(out.pixel(81, 39), [0, 0, 255, 255]);
        s.remove(arrow);
        assert_eq!(render(&base, &s).unwrap(), base, "no mark past the edge: no room");
    }

    #[test]
    fn a_magnifier_lens_past_the_edge_shows_the_zoomed_pixels_there() {
        let mut base = white(40, 40);
        for y in 0..10 {
            for x in 0..10 {
                base.set_pixel(x, y, [0, 160, 0, 255]);
            }
        }
        let lens = RectF { x: 50.0, y: 0.0, w: 20.0, h: 20.0 };
        let s = scene_with(Kind::Magnify { source: RectF { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, lens }, Style { shape: Shape::Rect, width: 0.0, ..style(0.0, false) });
        let out = render(&base, &s).unwrap();
        assert_eq!(out.width, 70);
        assert_eq!(out.pixel(60, 10), [0, 160, 0, 255], "the lens is outside the screenshot, in the room it made");
    }

    #[test]
    fn a_redaction_still_holds_only_block_averages_when_the_picture_grows_for_a_mark() {
        // The same pair of pictures as above: equal 6x6-block averages, different pixels.
        let (w, h) = (60u32, 48u32);
        let a = noise(w, h, 31);
        let mut b = a.clone();
        for by in (9..9 + 30).step_by(6) {
            for bx in (12..12 + 36).step_by(6) {
                let coords: Vec<(u32, u32)> = (by..by + 6).flat_map(|y| (bx..bx + 6).map(move |x| (x, y))).collect();
                for (i, &(x, y)) in coords.iter().enumerate() {
                    let (sx, sy) = coords[coords.len() - 1 - i];
                    b.set_pixel(x, y, a.pixel(sx, sy));
                }
            }
        }
        for mode in [RedactMode::Blur, RedactMode::Pixelate, RedactMode::Box] {
            let mut scene = Scene::new();
            // An arrow that sticks out 40 px above and to the left (not under the blur): the
            // picture grows.
            scene.add(Kind::Line { from: Point::new(6.0, 4.0), to: Point::new(-40.0, -40.0), via: vec![] }, style(4.0, false));
            scene.add(Kind::Redact { rect: RectF { x: 12.0, y: 9.0, w: 36.0, h: 30.0 }, mode }, style(1.0, false));
            let (ea, eb) = (exported(&a, &scene), exported(&b, &scene));
            assert_eq!((ea.width, ea.height), (w + 42, h + 42));
            // The redacted rectangle has moved with the screenshot, 42 px right and down.
            let r = Rect::new(12 + 42, 9 + 42, 36, 30);
            let differing = area(&ea, r).iter().zip(area(&eb, r)).filter(|(x, y)| **x != *y).count();
            assert_eq!(differing, 0, "{mode:?}: the saved file still depends on the original pixels");
            let original = Rect::new(12, 9, 36, 30);
            assert!(area(&ea, r) != area(&a, original), "{mode:?}: the pixels under it were replaced");
        }
    }

    /// The box of the pixels that differ from the white background, or `None`.
    fn inked(b: &Bitmap) -> Option<(u32, u32, u32, u32)> {
        let mut r: Option<(u32, u32, u32, u32)> = None;
        for y in 0..b.height {
            for x in 0..b.width {
                if b.pixel(x, y) != [255, 255, 255, 255] {
                    r = Some(match r {
                        None => (x, y, x, y),
                        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                    });
                }
            }
        }
        r
    }

    #[test]
    fn every_mark_inks_only_inside_the_room_it_asks_for() {
        // Drawn in the middle of a big picture, nothing is cut: every inked pixel must lie in
        // the mark's reach (one pixel of antialiasing allowed), or the picture would grow too
        // little and cut it.
        let c = Point::new(200.0, 200.0);
        let mut marks: Vec<(String, Kind, Style)> = Vec::new();
        let caps = [Cap::None, Cap::Arrow, Cap::Chevron, Cap::Bullet, Cap::Square, Cap::Diamond, Cap::Bar];
        for (i, cap) in caps.iter().enumerate() {
            for filled in [true, false] {
                for (j, deg) in [0.0f64, 30.0, 45.0, 90.0, 200.0, 317.0].iter().enumerate() {
                    let (dx, dy) = (deg.to_radians().cos() * 120.0, deg.to_radians().sin() * 120.0);
                    let width = [2.0, 4.0, 9.0][(i + j) % 3];
                    let st = Style { start: caps[(i + 3) % caps.len()], end: *cap, cap_filled: filled, ..style(width, false) };
                    marks.push((format!("line {cap:?} {deg}"), Kind::Line { from: c, to: c.offset(dx, dy), via: vec![] }, st));
                    let via = vec![c.offset(dx / 2.0 - dy * 0.4, dy / 2.0 + dx * 0.4)];
                    marks.push((format!("curve {cap:?} {deg}"), Kind::Line { from: c, to: c.offset(dx, dy), via }, st));
                }
            }
        }
        let pts = vec![c, c.offset(60.0, -40.0), c.offset(90.0, 50.0)];
        marks.push(("pen".into(), Kind::Pen { points: pts.clone() }, style(7.0, false)));
        marks.push(("highlighter".into(), Kind::Highlighter { points: pts }, Style { opacity: 1.0, ..style(22.0, false) }));
        let call = Style { fill: Some(Color::WHITE), width: 3.0, font_size: 22.0, ..style(3.0, false) };
        let (tw, th) = measure_text("Hi", &call);
        for tip in [c.offset(-120.0, -90.0), c.offset(150.0, 10.0), c.offset(20.0, 140.0)] {
            marks.push((format!("callout {tip:?}"), Kind::Callout { tip, pos: c, text: "Hi".into(), size: Point::new(tw, th) }, call));
        }
        for (deg, len) in [(0.0f64, 160.0), (90.0, 40.0), (33.0, 120.0)] {
            let to = c.offset(deg.to_radians().cos() * len, deg.to_radians().sin() * len);
            let st = Style { start: Cap::Bar, end: Cap::Bar, fill: Some(Color::rgb(0, 0, 200)), text_color: Color::WHITE, font_size: 15.0, ..style(2.0, false) };
            marks.push((format!("measure {deg}"), Kind::Measure { from: c, to }, st));
        }
        marks.push(("rect".into(), Kind::Rect { rect: RectF { x: 150.0, y: 160.0, w: 90.0, h: 50.0 } }, style(8.0, false)));
        marks.push(("ellipse".into(), Kind::Ellipse { rect: RectF { x: 150.0, y: 160.0, w: 90.0, h: 50.0 } }, style(8.0, true)));
        for shape in [Shape::Ellipse, Shape::Rect, Shape::Rounded, Shape::Diamond] {
            marks.push((format!("step {shape:?}"), Kind::Step { center: c, number: 7 }, Style { shape, width: 4.0, ..style(4.0, true) }));
        }
        let boxed = Style { fill: Some(Color::rgb(0, 0, 255)), width: 5.0, ..style(5.0, false) };
        let (bw, bh) = measure_text("Text", &boxed);
        marks.push(("text box".into(), Kind::Text { pos: c, text: "Text".into(), size: Point::new(bw, bh) }, boxed));
        let pic = obtura_scene::Pixels::new(Bitmap::filled(10, 10, [0, 0, 0, 255]));
        marks.push(("picture".into(), Kind::Image { rect: RectF { x: 150.0, y: 150.0, w: 60.0, h: 40.0 }, pixels: pic }, style(6.0, false)));
        marks.push(("magnifier".into(), Kind::Magnify { source: RectF { x: 190.0, y: 190.0, w: 20.0, h: 20.0 }, lens: RectF { x: 250.0, y: 180.0, w: 40.0, h: 40.0 } }, Style { shape: Shape::Ellipse, ..style(4.0, false) }));
        let mut dark_middle = white(420, 420);
        // Something for the magnifier to show.
        for y in 190..210 {
            for x in 190..210 {
                dark_middle.set_pixel(x, y, [0, 0, 0, 255]);
            }
        }
        // Curves through several bends, with every kind of end.
        for (i, cap) in caps.iter().enumerate() {
            let via = vec![c.offset(40.0, -70.0), c.offset(90.0, 60.0), c.offset(150.0, -20.0)];
            let st = Style { start: caps[(i + 2) % caps.len()], end: *cap, cap_filled: i % 2 == 0, ..style([3.0, 6.0][i % 2], false) };
            marks.push((format!("bends {cap:?}"), Kind::Line { from: c.offset(-60.0, 30.0), to: c.offset(170.0, 90.0), via }, st));
        }
        let mut angles: Vec<f64> = vec![0.0; marks.len()];
        // Turned shapes, text, badges and pictures.
        for deg in [17.0f64, 45.0, 90.0, 133.0, -70.0] {
            let a = deg.to_radians();
            let turned: Vec<(String, Kind, Style)> = vec![
                (format!("rect {deg}°"), Kind::Rect { rect: RectF { x: 150.0, y: 160.0, w: 90.0, h: 50.0 } }, style(8.0, false)),
                (format!("rounded rect {deg}°"), Kind::Rect { rect: RectF { x: 150.0, y: 160.0, w: 90.0, h: 50.0 } }, Style { corner: 12.0, ..style(6.0, true) }),
                (format!("ellipse {deg}°"), Kind::Ellipse { rect: RectF { x: 150.0, y: 160.0, w: 120.0, h: 40.0 } }, style(8.0, true)),
                (format!("text box {deg}°"), Kind::Text { pos: c, text: "Text".into(), size: Point::new(bw, bh) }, boxed),
                (format!("step {deg}°"), Kind::Step { center: c, number: 3 }, Style { shape: Shape::Diamond, ..style(4.0, true) }),
                (format!("picture {deg}°"), Kind::Image { rect: RectF { x: 150.0, y: 150.0, w: 80.0, h: 30.0 }, pixels: obtura_scene::Pixels::new(Bitmap::filled(8, 3, [0, 0, 0, 255])) }, style(6.0, false)),
            ];
            for m in turned {
                marks.push(m);
                angles.push(a);
            }
        }
        for ((name, kind, st), angle) in marks.into_iter().zip(angles) {
            let base = if matches!(kind, Kind::Magnify { .. }) { dark_middle.clone() } else { white(420, 420) };
            let mut s = Scene::new();
            let id = s.add(kind, st);
            s.get_mut(id).unwrap().angle = angle;
            let o = s.get(id).unwrap().clone();
            let reach = match o.kind {
                // The area looked at is inside the screenshot, and so is the line from it to
                // the lens up to the lens: only the lens itself needs room.
                Kind::Magnify { source, .. } => o.reach().unwrap().union(&source.inflate(st.width)),
                _ => o.reach().unwrap(),
            };
            let out = render(&base, &s).unwrap();
            let Some((x0, y0, x1, y1)) = inked(&out) else { panic!("{name}: nothing drawn") };
            let ok = x0 as f64 >= reach.x - 1.0 && y0 as f64 >= reach.y - 1.0 && (x1 + 1) as f64 <= reach.x + reach.w + 1.0 && (y1 + 1) as f64 <= reach.y + reach.h + 1.0;
            assert!(ok, "{name}: ink {x0},{y0} to {x1},{y1} is outside its reach {reach:?}");
            // A turned shape asks for no more room than its ink needs.
            if angle != 0.0 && matches!(o.kind, Kind::Rect { .. } | Kind::Ellipse { .. }) && o.style.corner == 0.0 {
                let tight = x0 as f64 <= reach.x + 1.5 && y0 as f64 <= reach.y + 1.5 && (x1 + 1) as f64 >= reach.x + reach.w - 1.5 && (y1 + 1) as f64 >= reach.y + reach.h - 1.5;
                assert!(tight, "{name}: ink {x0},{y0} to {x1},{y1} is well inside its reach {reach:?}");
            }
        }
    }

    #[test]
    fn the_picture_to_read_holds_nothing_of_what_was_hidden() {
        let a = noise(60, 40, 5);
        for mode in [RedactMode::Blur, RedactMode::Pixelate, RedactMode::Box] {
            let s = scene_with(Kind::Redact { rect: RectF { x: 10.2, y: 5.5, w: 30.0, h: 20.0 }, mode }, style(1.0, false));
            let read = picture_to_read(&a, s.objects()).unwrap();
            // The whole area, rounded outwards, is one flat colour: no detail of what was there.
            let inside = area(&read, Rect::new(10, 5, 31, 21));
            assert!(inside.chunks(4).all(|p| p == &inside[..4]), "{mode:?}: not flat");
            assert_eq!(read.pixel(0, 0), a.pixel(0, 0), "outside it, the picture");
        }
    }

    #[test]
    fn a_measure_shows_its_length_in_a_label_over_its_middle() {
        let st = Style { start: Cap::Bar, end: Cap::Bar, fill: Some(Color::rgb(0, 0, 200)), text_color: Color::WHITE, font_size: 15.0, ..style(2.0, false) };
        let (from, to) = (Point::new(20.0, 50.0), Point::new(220.0, 50.0));
        assert_eq!(Object::measure_label(from, to), "200 px");
        let out = render(&white(240, 100), &scene_with(Kind::Measure { from, to }, st)).unwrap();
        let r = Object::measure_box(from, to, &st);
        let (cx, cy) = ((r.x + r.w / 2.0) as u32, (r.y + r.h / 2.0) as u32);
        let label: Vec<[u8; 4]> = (r.x as u32 + 4..(r.x + r.w) as u32 - 4).map(|x| out.pixel(x, cy)).collect();
        assert!(label.iter().any(|p| *p == [0, 0, 200, 255]), "the pill");
        assert!(label.iter().any(|p| p[0] > 200 && p[2] > 200), "white text in it");
        assert_eq!(out.pixel(40, 50), [255, 0, 0, 255], "the line");
        assert!(out.pixel(20, 42) == [255, 0, 0, 255], "a bar at the end");
        assert_eq!(out.pixel(cx, 20), [255, 255, 255, 255]);
    }

    #[test]
    fn an_arrow_pointing_at_the_edge_does_not_grow_the_picture_and_one_crossing_it_does() {
        let base = white(200, 100);
        let to_edge = scene_with(Kind::Line { from: Point::new(20.0, 50.0), to: Point::new(197.0, 50.0), via: vec![] }, Style { end: Cap::Arrow, ..style(4.0, false) });
        assert_eq!(render(&base, &to_edge).unwrap().width, 200);
        let across = scene_with(Kind::Line { from: Point::new(20.0, 50.0), to: Point::new(230.0, 50.0), via: vec![] }, Style { end: Cap::Arrow, ..style(4.0, false) });
        assert_eq!(render(&base, &across).unwrap().width, 232);
    }
}
