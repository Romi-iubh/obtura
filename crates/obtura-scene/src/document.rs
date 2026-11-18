use crate::frame::Frame;
use crate::geometry::{Point, RectF};
use crate::object::Id;
use crate::scene::Scene;
use obtura_image::{cut_band, extend, flip_horizontal, flip_vertical, join, resize, rotate_ccw, rotate_cw, Bitmap, Rect};
use std::rc::Rc;

/// The base picture and its marks, with unlimited undo and redo.
///
/// Call [`Document::begin_edit`] before every change. It saves the state, so one call is
/// one undo step. A gesture that ends up changing nothing calls
/// [`Document::cancel_edit`].
pub struct Document {
    pub base: Rc<Bitmap>,
    pub scene: Scene,
    /// Beautify: the frame around the picture, or `None`.
    pub frame: Option<Frame>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

#[derive(Clone)]
struct Snapshot {
    base: Rc<Bitmap>,
    scene: Scene,
    frame: Option<Frame>,
}

impl Document {
    pub fn new(base: Bitmap) -> Self {
        Self { base: Rc::new(base), scene: Scene::new(), frame: None, undo: Vec::new(), redo: Vec::new() }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { base: self.base.clone(), scene: self.scene.clone(), frame: self.frame }
    }

    fn restore(&mut self, s: Snapshot) {
        self.base = s.base;
        self.scene = s.scene;
        self.frame = s.frame;
    }

    /// Saves the current state as an undo step. Anything that could be redone is dropped.
    pub fn begin_edit(&mut self) {
        let s = self.snapshot();
        self.undo.push(s);
        self.redo.clear();
    }

    /// Drops the step saved by the last `begin_edit` (nothing changed after all).
    pub fn cancel_edit(&mut self) {
        self.undo.pop();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        let now = self.snapshot();
        self.redo.push(now);
        self.restore(prev);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        let now = self.snapshot();
        self.undo.push(now);
        self.restore(next);
        true
    }

    /// Cuts the base picture to `rect` and moves the marks with it. One undo step.
    /// Marks wholly outside the part kept go with the rest; a mark partly inside stays whole
    /// (the picture grows to hold it, see [`crate::canvas_of`]).
    /// Returns false (and changes nothing) if `rect` holds no pixel of the picture.
    pub fn crop(&mut self, rect: Rect) -> bool {
        let Some(r) = rect.clamp_to(self.base.width, self.base.height) else { return false };
        let Some(cropped) = self.base.crop(r) else { return false };
        self.begin_edit();
        self.base = Rc::new(cropped);
        let kept = RectF { x: r.x as f64, y: r.y as f64, w: r.w as f64, h: r.h as f64 };
        let gone: Vec<Id> = self.scene.objects().iter().filter(|o| !o.bounds().intersects(&kept)).map(|o| o.id).collect();
        for id in gone {
            self.scene.remove(id);
        }
        self.scene.translate_all(-(r.x as f64), -(r.y as f64));
        true
    }

    /// Replaces the picture with `new_base` and moves every mark through `map`. One undo step.
    fn transform(&mut self, new_base: Bitmap, map: &dyn Fn(Point) -> Point) {
        self.begin_edit();
        self.base = Rc::new(new_base);
        self.scene.map_all(map);
    }

    /// A quarter turn, clockwise or not. The marks turn with the picture (text stays upright).
    pub fn rotate(&mut self, clockwise: bool) {
        let (w, h) = (self.base.width as f64, self.base.height as f64);
        if clockwise {
            let b = rotate_cw(&self.base);
            self.transform(b, &|p| Point::new(h - p.y, p.x));
        } else {
            let b = rotate_ccw(&self.base);
            self.transform(b, &|p| Point::new(p.y, w - p.x));
        }
    }

    /// A mirror image, left to right (`horizontal`) or upside down.
    pub fn flip(&mut self, horizontal: bool) {
        let (w, h) = (self.base.width as f64, self.base.height as f64);
        if horizontal {
            let b = flip_horizontal(&self.base);
            self.transform(b, &|p| Point::new(w - p.x, p.y));
        } else {
            let b = flip_vertical(&self.base);
            self.transform(b, &|p| Point::new(p.x, h - p.y));
        }
    }

    /// Resizes the picture to `w` pixels wide, keeping its proportions; the marks are scaled
    /// with it (their line widths and text too). Returns false if nothing changes.
    pub fn resize(&mut self, w: u32) -> bool {
        let w = w.max(1);
        if w == self.base.width {
            return false;
        }
        let k = w as f64 / self.base.width as f64;
        let h = ((self.base.height as f64 * k).round() as u32).max(1);
        let b = resize(&self.base, w, h);
        self.begin_edit();
        self.base = Rc::new(b);
        self.scene.scale_all(k);
        true
    }

    /// Adds space around the picture, filled with `fill`. The marks stay over what they marked.
    pub fn extend(&mut self, left: u32, top: u32, right: u32, bottom: u32, fill: [u8; 4]) -> bool {
        if left + top + right + bottom == 0 {
            return false;
        }
        let b = extend(&self.base, left, top, right, bottom, fill);
        let (dx, dy) = (left as f64, top as f64);
        self.transform(b, &|p| p.offset(dx, dy));
        true
    }

    /// Cuts the band of rows (`horizontal`) or columns between `from` and `to` out of the
    /// picture and joins the rest. Marks beyond the band move in with the picture; points
    /// inside it land on the seam. Returns false if nothing was cut.
    pub fn cut(&mut self, from: u32, to: u32, horizontal: bool) -> bool {
        let Some(b) = cut_band(&self.base, from, to, horizontal) else { return false };
        let (from, to) = (from as f64, to.min(if horizontal { self.base.height } else { self.base.width }) as f64);
        let fold = move |v: f64| if v >= to { v - (to - from) } else if v > from { from } else { v };
        self.transform(b, &|p| if horizontal { Point::new(p.x, fold(p.y)) } else { Point::new(fold(p.x), p.y) });
        true
    }

    /// Puts `other` to the right of the picture (`beside`) or below it; the space around the
    /// smaller one is `fill`.
    pub fn attach(&mut self, other: &Bitmap, beside: bool, fill: [u8; 4]) {
        let (b, (ax, ay)) = join(&self.base, other, beside, 0, fill);
        let (dx, dy) = (ax as f64, ay as f64);
        self.transform(b, &|p| p.offset(dx, dy));
    }

    /// The picture and `others` put together (the picture first), see
    /// [`obtura_image::combine`]; the marks move with the picture. One undo step.
    pub fn combine(&mut self, others: &[Bitmap], arrange: obtura_image::Arrange, gap: u32, margin: u32, align: obtura_image::Align, fill: [u8; 4]) {
        let mut all: Vec<&Bitmap> = vec![&self.base];
        all.extend(others.iter());
        let (b, at) = obtura_image::combine(&all, arrange, gap, margin, align, fill);
        let (dx, dy) = (at[0].0 as f64, at[0].1 as f64);
        self.transform(b, &|p| p.offset(dx, dy));
    }

    /// Removes a mark. One undo step.
    pub fn remove(&mut self, id: Id) -> bool {
        if self.scene.get(id).is_none() {
            return false;
        }
        self.begin_edit();
        self.scene.remove(id);
        true
    }

    /// Moves a mark. One undo step per call; for a drag, move the mark between
    /// `begin_edit` and the end of the drag instead.
    pub fn move_object(&mut self, id: Id, dx: f64, dy: f64) -> bool {
        if self.scene.get(id).is_none() {
            return false;
        }
        self.begin_edit();
        if let Some(o) = self.scene.get_mut(id) {
            o.translate(dx, dy);
        }
        true
    }

    /// The point in the picture nearest to `p` (clamps to the picture's edges).
    pub fn clamp_point(&self, p: Point) -> Point {
        Point::new(p.x.clamp(0.0, self.base.width as f64), p.y.clamp(0.0, self.base.height as f64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Kind, Style};

    fn doc() -> Document {
        Document::new(Bitmap::filled(100, 80, [10, 20, 30, 255]))
    }

    fn line() -> Kind {
        Kind::Line { from: Point::new(10.0, 10.0), to: Point::new(60.0, 40.0), via: vec![] }
    }

    #[test]
    fn undo_and_redo_walk_through_the_edits() {
        let mut d = doc();
        d.begin_edit();
        let a = d.scene.add(line(), Style::default());
        d.begin_edit();
        let b = d.scene.add(line(), Style::default());
        assert_eq!(d.scene.objects().len(), 2);
        assert!(d.undo());
        assert!(d.scene.get(a).is_some() && d.scene.get(b).is_none());
        assert!(d.undo());
        assert!(d.scene.objects().is_empty());
        assert!(!d.undo());
        assert!(d.redo() && d.redo());
        assert_eq!(d.scene.objects().len(), 2);
        assert!(!d.redo());
    }

    #[test]
    fn a_new_edit_drops_the_redo_steps() {
        let mut d = doc();
        d.begin_edit();
        d.scene.add(line(), Style::default());
        d.undo();
        assert!(d.can_redo());
        d.begin_edit();
        d.scene.add(line(), Style::default());
        assert!(!d.can_redo());
    }

    #[test]
    fn a_cancelled_edit_leaves_no_step() {
        let mut d = doc();
        d.begin_edit();
        d.cancel_edit();
        assert!(!d.can_undo());
    }

    #[test]
    fn crop_moves_the_marks_and_is_undoable() {
        let mut d = doc();
        d.begin_edit();
        let id = d.scene.add(line(), Style::default());
        assert!(d.crop(Rect::new(5, 4, 50, 40)));
        assert_eq!((d.base.width, d.base.height), (50, 40));
        match &d.scene.get(id).unwrap().kind {
            Kind::Line { from, .. } => assert_eq!(*from, Point::new(5.0, 6.0)),
            _ => unreachable!(),
        }
        d.undo();
        assert_eq!((d.base.width, d.base.height), (100, 80));
        match &d.scene.get(id).unwrap().kind {
            Kind::Line { from, .. } => assert_eq!(*from, Point::new(10.0, 10.0)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn turning_moves_the_marks_with_the_picture_and_four_turns_change_nothing() {
        let mut d = doc();
        d.begin_edit();
        let id = d.scene.add(line(), Style::default());
        d.rotate(true);
        assert_eq!((d.base.width, d.base.height), (80, 100));
        match &d.scene.get(id).unwrap().kind {
            // (10, 10) in a 100 x 80 picture: a quarter turn clockwise puts it at (70, 10).
            Kind::Line { from, .. } => assert_eq!(*from, Point::new(70.0, 10.0)),
            _ => unreachable!(),
        }
        d.rotate(true);
        d.rotate(true);
        d.rotate(true);
        assert_eq!(d.scene.get(id).unwrap().kind, line());
        d.rotate(false);
        d.rotate(true);
        assert_eq!(d.scene.get(id).unwrap().kind, line());
        d.flip(true);
        d.flip(true);
        d.flip(false);
        d.flip(false);
        assert_eq!(d.scene.get(id).unwrap().kind, line());
    }

    #[test]
    fn cutting_a_strip_moves_the_marks_below_it_up() {
        let mut d = doc();
        d.begin_edit();
        let id = d.scene.add(line(), Style::default()); // from (10, 10) to (60, 40)
        assert!(d.cut(20, 30, true));
        assert_eq!(d.base.height, 70);
        match &d.scene.get(id).unwrap().kind {
            Kind::Line { from, to, .. } => assert_eq!((*from, *to), (Point::new(10.0, 10.0), Point::new(60.0, 30.0))),
            _ => unreachable!(),
        }
        assert!(d.undo());
        assert_eq!(d.base.height, 80);
    }

    #[test]
    fn resizing_and_extending_keep_the_marks_on_what_they_marked() {
        let mut d = doc();
        d.begin_edit();
        let id = d.scene.add(line(), Style::default());
        assert!(d.resize(50));
        assert_eq!((d.base.width, d.base.height), (50, 40));
        match &d.scene.get(id).unwrap().kind {
            Kind::Line { from, .. } => assert_eq!(*from, Point::new(5.0, 5.0)),
            _ => unreachable!(),
        }
        assert!(d.extend(10, 20, 0, 0, [255, 255, 255, 255]));
        assert_eq!((d.base.width, d.base.height), (60, 60));
        match &d.scene.get(id).unwrap().kind {
            Kind::Line { from, .. } => assert_eq!(*from, Point::new(15.0, 25.0)),
            _ => unreachable!(),
        }
        d.attach(&Bitmap::filled(30, 100, [0, 0, 0, 255]), true, [255, 255, 255, 255]);
        assert_eq!((d.base.width, d.base.height), (90, 100));
        match &d.scene.get(id).unwrap().kind {
            Kind::Line { from, .. } => assert_eq!(*from, Point::new(15.0, 45.0), "the first picture is centred"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn cropping_outside_the_picture_does_nothing() {
        let mut d = doc();
        assert!(!d.crop(Rect::new(200, 200, 10, 10)));
        assert!(!d.can_undo());
    }

    #[test]
    fn remove_and_move_are_single_undo_steps() {
        let mut d = doc();
        d.begin_edit();
        let id = d.scene.add(line(), Style::default());
        assert!(d.move_object(id, 5.0, 5.0));
        assert!(d.remove(id));
        assert!(d.undo());
        assert!(d.scene.get(id).is_some());
        assert!(!d.remove(999));
    }

    #[test]
    fn crop_drops_the_marks_left_outside_and_keeps_those_partly_inside_whole() {
        let mut d = doc();
        d.begin_edit();
        let outside = d.scene.add(Kind::Line { from: Point::new(80.0, 60.0), to: Point::new(95.0, 75.0), via: vec![] }, Style::default());
        let across = d.scene.add(Kind::Line { from: Point::new(20.0, 20.0), to: Point::new(90.0, 20.0), via: vec![] }, Style::default());
        assert!(d.crop(Rect::new(0, 0, 50, 40)));
        assert!(d.scene.get(outside).is_none(), "cut away with its part of the picture");
        assert!(d.scene.get(across).is_some(), "partly inside: kept whole");
        assert!(d.scene.canvas(50, 40).w > 50, "the picture grows to hold the part past the edge");
        assert!(d.undo());
        assert!(d.scene.get(outside).is_some(), "undo brings it back");
    }
}
