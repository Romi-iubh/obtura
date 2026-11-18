use crate::geometry::{dist_to_segment, Point, RectF};
use crate::object::{Cap, Id, Kind, Object, Shape, Style};
use obtura_image::Rect;

/// The marks of one picture. The order is the stacking order: the last is on top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    objects: Vec<Object>,
    next_id: Id,
}

/// How close, in picture pixels, a click must be to a thin mark to hit it.
const SLOP: f64 = 4.0;

impl Scene {
    pub fn new() -> Self {
        Self { objects: Vec::new(), next_id: 1 }
    }

    pub fn objects(&self) -> &[Object] {
        &self.objects
    }

    pub fn get(&self, id: Id) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }

    pub fn get_mut(&mut self, id: Id) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| o.id == id)
    }

    /// Adds a mark on top and returns its id.
    pub fn add(&mut self, kind: Kind, style: Style) -> Id {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        self.objects.push(Object { id, kind, style, angle: 0.0 });
        id
    }

    /// Adds a mark as it was kept (a saved file, the library) on top, turn and all, under a new
    /// id; returns that id.
    pub fn add_object(&mut self, mut o: Object) -> Id {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        o.id = id;
        self.objects.push(o);
        id
    }

    pub fn remove(&mut self, id: Id) -> bool {
        let before = self.objects.len();
        self.objects.retain(|o| o.id != id);
        self.objects.len() != before
    }

    /// The number the next step badge gets: one more than the highest now.
    pub fn next_step_number(&self) -> u32 {
        self.objects.iter().filter_map(|o| if let Kind::Step { number, .. } = o.kind { Some(number) } else { None }).max().unwrap_or(0) + 1
    }

    /// Numbers the step badges 1, 2, 3... in the order they were placed.
    pub fn renumber_steps(&mut self) {
        let mut n = 0;
        for o in &mut self.objects {
            if let Kind::Step { number, .. } = &mut o.kind {
                n += 1;
                *number = n;
            }
        }
    }

    /// The topmost mark at `p`, if any. Thin marks (lines, outlines) must be hit near
    /// their ink; filled shapes anywhere inside; redactions anywhere inside.
    pub fn hit_test(&self, p: Point) -> Option<Id> {
        self.objects.iter().rev().find(|o| hit(o, p)).map(|o| o.id)
    }

    pub fn translate_all(&mut self, dx: f64, dy: f64) {
        self.objects.iter_mut().for_each(|o| o.translate(dx, dy));
    }

    /// Moves every point of every mark through `f`, see [`Object::map_points`].
    pub fn map_all(&mut self, f: &dyn Fn(Point) -> Point) {
        self.objects.iter_mut().for_each(|o| o.map_points(f));
    }

    /// Scales every mark, see [`Object::scale`].
    pub fn scale_all(&mut self, k: f64) {
        self.objects.iter_mut().for_each(|o| o.scale(k));
    }

    /// Everything's box, or `None` for an empty scene.
    pub fn bounds(&self) -> Option<RectF> {
        self.objects.iter().map(|o| o.bounds()).reduce(|a, b| a.union(&b))
    }

    /// The finished picture's area for a `w` x `h` screenshot, see [`canvas_of`].
    pub fn canvas(&self, w: u32, h: u32) -> Rect {
        canvas_of(&self.objects, w, h)
    }
}

/// The most room marks can add on one side of the screenshot, in pixels (a mark dragged far
/// away must not ask for a picture too big to make).
pub const MAX_GROWTH: f64 = 8000.0;

/// The finished picture's area, in picture coordinates: the `w` x `h` screenshot at (0, 0),
/// grown on whole pixels to hold every mark that reaches past its edges (see
/// [`Object::reach`]). Nothing is stored: delete or move the mark back and the room it
/// needed is gone.
pub fn canvas_of(objects: &[Object], w: u32, h: u32) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (0.0f64, 0.0f64, w as f64, h as f64);
    for r in objects.iter().filter_map(Object::reach) {
        if [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()) {
            x0 = x0.min(r.x);
            y0 = y0.min(r.y);
            x1 = x1.max(r.x + r.w);
            y1 = y1.max(r.y + r.h);
        }
    }
    // A hair past the edge (rounding) is not worth a row of white.
    const HAIR: f64 = 0.01;
    let left = (-(x0 + HAIR)).ceil().clamp(0.0, MAX_GROWTH);
    let top = (-(y0 + HAIR)).ceil().clamp(0.0, MAX_GROWTH);
    let right = (x1 - w as f64 - HAIR).ceil().clamp(0.0, MAX_GROWTH);
    let bottom = (y1 - h as f64 - HAIR).ceil().clamp(0.0, MAX_GROWTH);
    Rect::new(-(left as i32), -(top as i32), w + left as u32 + right as u32, h + top as u32 + bottom as u32)
}

fn hit(o: &Object, p: Point) -> bool {
    // A turned mark is tested in its own upright frame.
    let p = o.to_upright(p);
    let reach = o.style.width / 2.0 + SLOP;
    match &o.kind {
        Kind::Line { from, to, via } => {
            let pts = Object::line_points(*from, *to, via);
            let near_line = pts.windows(2).any(|w| dist_to_segment(p, w[0], w[1]) <= reach);
            // An arrow head or a dot is bigger than the line it sits on.
            let cap = o.style.cap_unit() * 0.9;
            let near_end = (o.style.start != Cap::None && p.distance(*from) <= cap) || (o.style.end != Cap::None && p.distance(*to) <= cap);
            near_line || near_end
        }
        Kind::Pen { points } | Kind::Highlighter { points } => {
            let reach = if matches!(o.kind, Kind::Highlighter { .. }) { o.style.width } else { reach };
            match points.as_slice() {
                [] => false,
                [only] => p.distance(*only) <= reach,
                _ => points.windows(2).any(|w| dist_to_segment(p, w[0], w[1]) <= reach),
            }
        }
        Kind::Rect { rect } => {
            if o.style.fill.is_some() {
                rect.inflate(reach).contains(p)
            } else {
                rect.inflate(reach).contains(p) && !rect.inflate(-reach).contains(p)
            }
        }
        Kind::Ellipse { rect } => in_ellipse(rect, p, reach, o.style.fill.is_some()),
        Kind::Redact { rect, .. } => {
            if o.style.shape == Shape::Ellipse {
                in_ellipse(rect, p, 0.0, true)
            } else {
                rect.contains(p)
            }
        }
        Kind::Text { .. } | Kind::Step { .. } | Kind::Stamp { .. } | Kind::Image { .. } => o.upright_bounds().contains(p),
        // Only the edge of the lit area: clicks inside it reach the marks it shows.
        Kind::Spotlight { rect } => {
            if o.style.shape == Shape::Ellipse {
                in_ellipse(rect, p, SLOP + 2.0, false)
            } else {
                rect.inflate(SLOP + 2.0).contains(p) && !rect.inflate(-(SLOP + 2.0)).contains(p)
            }
        }
        Kind::Magnify { source, lens } => {
            let on_lens = if o.style.shape == Shape::Ellipse { in_ellipse(lens, p, reach, true) } else { lens.inflate(reach).contains(p) };
            let on_source = source.inflate(reach).contains(p) && !source.inflate(-reach).contains(p);
            on_lens || on_source
        }
        Kind::Measure { from, to } => dist_to_segment(p, *from, *to) <= reach || Object::measure_box(*from, *to, &o.style).contains(p),
        Kind::Callout { tip, pos, size, .. } => {
            let on_box = Object::text_box(*pos, *size, &o.style, true).inflate(o.style.width / 2.0).contains(p);
            let on_line = o.callout_anchor().is_some_and(|a| dist_to_segment(p, a, *tip) <= o.style.leader_width() / 2.0 + SLOP);
            on_box || on_line || p.distance(*tip) <= 10.0 + o.style.leader_width() * 1.5
        }
    }
}

/// Whether `p` is inside the ellipse in `rect` (`inside`), or near its outline (within `reach`).
fn in_ellipse(rect: &RectF, p: Point, reach: f64, inside: bool) -> bool {
    let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let (rx, ry) = ((rect.w / 2.0).max(0.5), (rect.h / 2.0).max(0.5));
    // 1.0 on the outline, below it inside, above it outside.
    let d = (((p.x - cx) / rx).powi(2) + ((p.y - cy) / ry).powi(2)).sqrt();
    let slop = reach / rx.min(ry);
    if inside {
        d <= 1.0 + slop
    } else {
        (d - 1.0).abs() <= slop
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Color, RedactMode};

    fn style() -> Style {
        Style { color: Color::rgb(255, 0, 0), width: 4.0, font_size: 20.0, ..Style::default() }
    }

    #[test]
    fn a_turned_mark_is_hit_where_it_is_drawn() {
        let mut s = Scene::new();
        let id = s.add(Kind::Rect { rect: RectF { x: 0.0, y: 45.0, w: 100.0, h: 10.0 } }, Style { fill: Some(Color::WHITE), ..style() });
        assert_eq!(s.hit_test(Point::new(90.0, 50.0)), Some(id));
        assert_eq!(s.hit_test(Point::new(50.0, 90.0)), None);
        s.get_mut(id).unwrap().angle = std::f64::consts::FRAC_PI_2;
        // Standing up now: hit along the vertical, not at the old ends.
        assert_eq!(s.hit_test(Point::new(50.0, 90.0)), Some(id));
        assert_eq!(s.hit_test(Point::new(90.0, 50.0)), None);
    }

    /// Marks kept in the library come back as they were: turned marks stay turned.
    #[test]
    fn a_kept_mark_comes_back_turned_under_a_new_id() {
        let mut kept = Scene::new();
        let id = kept.add(Kind::Rect { rect: RectF { x: 0.0, y: 45.0, w: 100.0, h: 10.0 } }, style());
        kept.get_mut(id).unwrap().angle = 0.7;
        let json = serde_json::to_string(kept.objects()).unwrap();
        let mut s = Scene::new();
        let first = s.add(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(1.0, 1.0), via: vec![] }, style());
        for o in serde_json::from_str::<Vec<Object>>(&json).unwrap() {
            let again = s.add_object(o);
            assert_ne!(again, first);
            assert_eq!(s.get(again).unwrap().angle, 0.7);
            assert_eq!(s.get(again).unwrap().kind, kept.objects()[0].kind);
        }
        assert_ne!(s.add(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(1.0, 1.0), via: vec![] }, style()), first);
    }

    #[test]
    fn ids_are_unique_and_never_reused() {
        let mut s = Scene::new();
        let a = s.add(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(1.0, 1.0), via: vec![] }, style());
        let b = s.add(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(1.0, 1.0), via: vec![] }, style());
        assert_ne!(a, b);
        s.remove(b);
        let c = s.add(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(1.0, 1.0), via: vec![] }, style());
        assert!(c != a && c != b);
    }

    #[test]
    fn steps_count_up_and_renumber() {
        let mut s = Scene::new();
        for i in 0..3 {
            let n = s.next_step_number();
            s.add(Kind::Step { center: Point::new(i as f64 * 50.0, 0.0), number: n }, style());
        }
        assert_eq!(s.next_step_number(), 4);
        let second = s.objects()[1].id;
        s.remove(second);
        s.renumber_steps();
        let numbers: Vec<u32> = s.objects().iter().filter_map(|o| if let Kind::Step { number, .. } = o.kind { Some(number) } else { None }).collect();
        assert_eq!(numbers, vec![1, 2]);
    }

    #[test]
    fn the_topmost_mark_wins_a_hit() {
        let mut s = Scene::new();
        let rect = RectF { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let filled = Style { fill: Some(Color::WHITE), ..style() };
        let below = s.add(Kind::Rect { rect }, filled);
        let above = s.add(Kind::Rect { rect }, filled);
        assert_eq!(s.hit_test(Point::new(50.0, 50.0)), Some(above));
        s.remove(above);
        assert_eq!(s.hit_test(Point::new(50.0, 50.0)), Some(below));
        assert_eq!(s.hit_test(Point::new(150.0, 50.0)), None);
    }

    #[test]
    fn an_outline_is_hit_on_its_edge_not_inside() {
        let mut s = Scene::new();
        let id = s.add(Kind::Rect { rect: RectF { x: 0.0, y: 0.0, w: 100.0, h: 100.0 } }, style());
        assert_eq!(s.hit_test(Point::new(0.0, 50.0)), Some(id));
        assert_eq!(s.hit_test(Point::new(50.0, 50.0)), None);
        let e = s.add(Kind::Ellipse { rect: RectF { x: 200.0, y: 0.0, w: 100.0, h: 50.0 } }, style());
        assert_eq!(s.hit_test(Point::new(300.0, 25.0)), Some(e));
        assert_eq!(s.hit_test(Point::new(250.0, 25.0)), None);
    }

    #[test]
    fn lines_and_pens_are_hit_near_the_ink() {
        let mut s = Scene::new();
        let l = s.add(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(100.0, 0.0), via: vec![] }, style());
        assert_eq!(s.hit_test(Point::new(50.0, 5.0)), Some(l));
        assert_eq!(s.hit_test(Point::new(50.0, 20.0)), None);
        let p = s.add(Kind::Pen { points: vec![Point::new(0.0, 50.0), Point::new(50.0, 80.0), Point::new(100.0, 50.0)] }, style());
        assert_eq!(s.hit_test(Point::new(25.0, 65.0)), Some(p));
    }

    #[test]
    fn redactions_are_hit_anywhere_inside() {
        let mut s = Scene::new();
        let r = s.add(Kind::Redact { rect: RectF { x: 10.0, y: 10.0, w: 50.0, h: 20.0 }, mode: RedactMode::Box }, style());
        assert_eq!(s.hit_test(Point::new(30.0, 20.0)), Some(r));
    }

    #[test]
    fn a_curve_is_hit_on_its_bulge_not_on_the_chord() {
        let mut s = Scene::new();
        let id = s.add(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(100.0, 0.0), via: vec![Point::new(50.0, 40.0)] }, style());
        assert_eq!(s.hit_test(Point::new(50.0, 40.0)), Some(id));
        assert_eq!(s.hit_test(Point::new(50.0, 0.0)), None, "the straight chord is empty");
    }

    #[test]
    fn an_arrow_head_is_easier_to_hit_than_the_thin_line() {
        let mut s = Scene::new();
        let st = Style { end: Cap::Arrow, width: 2.0, ..style() };
        let id = s.add(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(100.0, 0.0), via: vec![] }, st);
        assert_eq!(s.hit_test(Point::new(90.0, 7.0)), Some(id));
        assert_eq!(s.hit_test(Point::new(50.0, 9.0)), None);
    }

    #[test]
    fn a_callout_is_hit_on_its_box_its_line_and_its_tip() {
        let mut s = Scene::new();
        let kind = Kind::Callout { tip: Point::new(0.0, 0.0), pos: Point::new(100.0, 100.0), text: "hi".into(), size: Point::new(40.0, 20.0) };
        let id = s.add(kind, style());
        assert_eq!(s.hit_test(Point::new(120.0, 110.0)), Some(id), "box");
        assert_eq!(s.hit_test(Point::new(1.0, 1.0)), Some(id), "tip");
        assert_eq!(s.hit_test(Point::new(300.0, 300.0)), None);
    }

    #[test]
    fn a_spotlight_is_hit_on_its_edge_so_marks_inside_stay_reachable() {
        let mut s = Scene::new();
        let inside = s.add(Kind::Rect { rect: RectF { x: 40.0, y: 40.0, w: 20.0, h: 20.0 } }, Style { fill: Some(Color::WHITE), ..style() });
        let spot = s.add(Kind::Spotlight { rect: RectF { x: 0.0, y: 0.0, w: 100.0, h: 100.0 } }, style());
        assert_eq!(s.hit_test(Point::new(50.0, 50.0)), Some(inside));
        assert_eq!(s.hit_test(Point::new(0.0, 50.0)), Some(spot));
    }

    #[test]
    fn a_round_redaction_is_not_hit_in_its_corners() {
        let mut s = Scene::new();
        let round = Style { shape: Shape::Ellipse, ..style() };
        let id = s.add(Kind::Redact { rect: RectF { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, mode: RedactMode::Blur }, round);
        assert_eq!(s.hit_test(Point::new(50.0, 50.0)), Some(id));
        assert_eq!(s.hit_test(Point::new(3.0, 3.0)), None);
    }

    #[test]
    fn the_canvas_is_the_screenshot_until_a_mark_reaches_past_its_edge() {
        let mut s = Scene::new();
        assert_eq!(s.canvas(100, 80), Rect::new(0, 0, 100, 80));
        let inside = s.add(Kind::Rect { rect: RectF { x: 10.0, y: 10.0, w: 20.0, h: 20.0 } }, style());
        assert_eq!(s.canvas(100, 80), Rect::new(0, 0, 100, 80));
        // A line from inside to 30 px past the left edge (4 px thick: 2 px more).
        let out = s.add(Kind::Line { from: Point::new(50.0, 40.0), to: Point::new(-30.0, 40.0), via: vec![] }, style());
        assert_eq!(s.canvas(100, 80), Rect::new(-32, 0, 132, 80));
        s.remove(out);
        assert_eq!(s.canvas(100, 80), Rect::new(0, 0, 100, 80), "deleting the mark takes the room away");
        s.get_mut(inside).unwrap().translate(90.0, 65.0);
        assert_eq!(s.canvas(100, 80), Rect::new(0, 0, 122, 97), "moving a mark out grows the picture on that side");
    }

    #[test]
    fn redactions_and_spotlights_never_grow_the_canvas_and_a_magnifier_grows_it_for_its_lens() {
        let mut s = Scene::new();
        let far = RectF { x: 90.0, y: 70.0, w: 50.0, h: 50.0 };
        s.add(Kind::Redact { rect: far, mode: RedactMode::Blur }, style());
        s.add(Kind::Spotlight { rect: far }, style());
        assert_eq!(s.canvas(100, 80), Rect::new(0, 0, 100, 80));
        let lens = RectF { x: 60.0, y: -40.0, w: 40.0, h: 30.0 };
        s.add(Kind::Magnify { source: RectF { x: 10.0, y: 10.0, w: 20.0, h: 15.0 }, lens }, Style { width: 4.0, ..style() });
        assert_eq!(s.canvas(100, 80), Rect::new(0, -42, 102, 122), "the lens and its 4 px border");
    }

    #[test]
    fn a_mark_sent_very_far_away_asks_for_a_bounded_canvas() {
        let mut s = Scene::new();
        s.add(Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(1e9, 0.0), via: vec![] }, style());
        let c = s.canvas(100, 80);
        assert_eq!((c.x, c.w as f64), (-2, 102.0 + MAX_GROWTH), "2 px for the line's own width on the left, the most allowed on the right");
    }
}
