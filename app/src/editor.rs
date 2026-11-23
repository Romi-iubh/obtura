//! The editor canvas: shows the picture with its marks, and turns mouse and keyboard
//! input into edits of the [`Document`]. All marks stay objects until export.

use crate::tools::{self, Fields, Tool};
use gtk::{cairo, gdk, glib, prelude::*};
use obtura_image::{Bitmap, Rect};
use obtura_render::{apply_frame, backdrop, draw_border, draw_objects, measure_text, picture_path, render, render_on, to_surface, underlay_len};
use obtura_scene::{canvas_of, content_rect, Document, Frame, Id, Kind, Object, Point, RectF, RedactMode, Style};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// What changed, so the window can refresh the right widgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The picture or its marks (title, undo buttons, "unsaved").
    Document,
    Tool,
    Selection,
    /// The style of the current tool or selection (Formatting panel).
    Style,
    /// The zoom level or the scroll position.
    View,
}

/// Work the editor asks the window to do (it has the clipboard and the toasts).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Read the text in this part of the picture (in picture pixels) and copy it.
    GrabText(Rect),
    /// This colour was clicked with the colour picker.
    PickColor([u8; 4]),
}

/// Space around the picture in the view, in pixels.
const MARGIN: f64 = 16.0;
/// A drag shorter than this (in picture pixels) draws nothing.
const MIN_DRAG: f64 = 3.0;
/// Zoom limits and the step of one wheel notch or one key press.
const MIN_ZOOM: f64 = 0.05;
const MAX_ZOOM: f64 = 16.0;
const ZOOM_STEP: f64 = 1.25;

#[derive(Clone, Copy)]
struct View {
    scale: f64,
    ox: f64,
    oy: f64,
}

impl View {
    /// The whole picture, centred, shrunk to fit if needed but never enlarged.
    #[cfg(test)]
    fn fit(img_w: u32, img_h: u32, w: i32, h: i32) -> View {
        View::fit_rect(RectF { x: 0.0, y: 0.0, w: img_w as f64, h: img_h as f64 }, w, h)
    }

    /// `content` (in picture coordinates: the picture and anything drawn around it), centred,
    /// shrunk to fit if needed but never enlarged.
    fn fit_rect(content: RectF, w: i32, h: i32) -> View {
        let scale = ((w as f64 - 2.0 * MARGIN) / content.w.max(1.0)).min((h as f64 - 2.0 * MARGIN) / content.h.max(1.0)).clamp(0.01, 1.0);
        View::placed(content, scale, (0.0, 0.0), w, h, true)
    }

    /// `content` at `scale`, scrolled to `origin` (where picture point 0,0 lands). Along an
    /// axis where it fits, it is centred; where it does not, the scroll is kept within it.
    fn placed(content: RectF, scale: f64, origin: (f64, f64), w: i32, h: i32, center_all: bool) -> View {
        let axis = |start: f64, len: f64, o: f64, size: f64| {
            let shown = len * scale;
            if center_all || shown + 2.0 * MARGIN <= size {
                ((size - shown) / 2.0 - start * scale).round()
            } else {
                // The content's edges may not come further in than the margin.
                o.clamp(size - MARGIN - (start + len) * scale, MARGIN - start * scale).round()
            }
        };
        View { scale, ox: axis(content.x, content.w, origin.0, w as f64), oy: axis(content.y, content.h, origin.1, h as f64) }
    }

    fn to_image(&self, x: f64, y: f64) -> Point {
        Point::new((x - self.ox) / self.scale, (y - self.oy) / self.scale)
    }

    fn to_widget(&self, p: Point) -> (f64, f64) {
        (p.x * self.scale + self.ox, p.y * self.scale + self.oy)
    }
}

enum Drag {
    Draw { start: Point },
    /// Moving a mark: where the drag began and the mark as it was then.
    Move { id: Id, start: Point, orig: Box<Object>, edited: bool },
    /// Reshaping the selected mark by one of its handles; `orig` is the mark as it was.
    Handle { id: Id, index: usize, orig: Box<Object>, edited: bool },
    /// Turning the selected mark by its round handle: `start` is the pointer's angle around
    /// the mark's centre when the drag began.
    Rotate { id: Id, orig: Box<Object>, start: f64, edited: bool },
    Crop { start: Point, previous: Option<RectF> },
    /// Choosing a band to cut out.
    Cut { start: Point },
    /// Choosing an area to read the text of.
    Grab { start: Point },
}

/// A band of the picture about to be cut out: from, to (rows or columns), across the picture.
#[derive(Clone, Copy)]
struct Band {
    from: f64,
    to: f64,
    horizontal: bool,
}

impl Band {
    /// The band between two points: rows if the drag went more down than across.
    fn between(a: Point, b: Point) -> Band {
        let horizontal = (b.y - a.y).abs() >= (b.x - a.x).abs();
        let (u, v) = if horizontal { (a.y, b.y) } else { (a.x, b.x) };
        Band { from: u.min(v).round(), to: u.max(v).round(), horizontal }
    }
}

struct TextEditing {
    /// The mark being edited, or `None` for a new one.
    id: Option<Id>,
    at: Point,
    style: Style,
    /// For a callout: where its arrow points.
    tip: Option<Point>,
}

/// How near (in widget pixels) the pointer must be to grab a handle.
const HANDLE_REACH: f64 = 9.0;
/// How far (in widget pixels) above a mark its round turning handle sits.
const ROTATE_GAP: f64 = 26.0;

/// The picture with the marks up to the last redaction or magnifier applied, drawn once and
/// reused while the marks above it change.
struct Underlay {
    n: usize,
    /// Where it goes, in picture coordinates (bigger than the screenshot when marks need room).
    rect: Rect,
    /// The screenshot and the marks it was drawn from: a mark below the last redaction that is
    /// reshaped, turned or restyled makes it stale (cheap to compare: pictures are shared).
    base: Rc<Bitmap>,
    marks: Vec<Object>,
    surface: cairo::ImageSurface,
}

struct State {
    doc: Document,
    tool: Tool,
    styles: HashMap<Tool, Style>,
    selected: Option<Id>,
    /// The mark being drawn, not yet in the scene.
    draft: Option<Object>,
    drag: Option<Drag>,
    underlay: Option<Underlay>,
    view: View,
    crop: Option<RectF>,
    editing: Option<TextEditing>,
    /// The mark whose style was last changed and when: a slider drag is one undo step.
    style_edit: Option<(Id, std::time::Instant)>,
    /// `None`: the picture fits the window. Otherwise the scale, kept until "Fit" is chosen.
    zoom: Option<f64>,
    /// Where picture point (0, 0) is in the widget when zoomed (the scroll position).
    origin: (f64, f64),
    /// Where the pointer is over the canvas, for zooming around it.
    pointer: Option<(f64, f64)>,
    /// Size of the canvas at the last draw.
    canvas: (i32, i32),
    /// A middle-button drag scrolling the view: the origin when it began.
    panning: Option<(f64, f64)>,
    /// The emoji or symbol the Stamp tool places.
    stamp: String,
    /// Alignment guides shown while a mark snaps: (vertical, position in the picture).
    guides: Vec<(bool, f64)>,
    /// The band the Cut Out tool is choosing.
    band: Option<Band>,
    /// The shape the crop rectangle keeps (width, height), or `None` for any.
    crop_ratio: Option<(u32, u32)>,
    /// When the frame was last changed: a slider drag is one undo step.
    frame_edit: Option<std::time::Instant>,
    /// The drawn background, shadow and bar of the frame, for the frame and size it was made for.
    backdrop: Option<(Frame, (u32, u32), cairo::ImageSurface)>,
    /// What the view showed at the last draw (picture coordinates).
    shown: RectF,
    /// While a drag or a typing goes on, the view stays put even if the picture grows (it would
    /// otherwise move under the pointer); it fits the new size afterwards.
    frozen: Option<RectF>,
    /// The area the Copy Text tool is choosing.
    grab: Option<RectF>,
    /// The picture as it is seen, every mark applied on the screenshot's area (what the colour
    /// picker and text reading look at, so they never reveal what was hidden), with the
    /// screenshot and the marks it was made from.
    seen: Option<(Rc<Bitmap>, Vec<Object>, Rc<Bitmap>)>,
    /// Areas to show outlined (smart redact proposals), in picture pixels.
    proposals: Vec<RectF>,
}

impl State {
    fn new(image: Bitmap) -> State {
        State {
            doc: Document::new(image),
            tool: Tool::Line,
            styles: HashMap::new(),
            selected: None,
            draft: None,
            drag: None,
            underlay: None,
            view: View { scale: 1.0, ox: 0.0, oy: 0.0 },
            crop: None,
            editing: None,
            style_edit: None,
            zoom: None,
            origin: (0.0, 0.0),
            pointer: None,
            canvas: (1, 1),
            panning: None,
            stamp: crate::format::STAMPS[0].to_string(),
            frame_edit: None,
            backdrop: None,
            band: None,
            crop_ratio: None,
            guides: Vec::new(),
            shown: RectF::default(),
            frozen: None,
            grab: None,
            seen: None,
            proposals: Vec::new(),
        }
    }
}

pub struct Editor {
    pub widget: gtk::Overlay,
    area: gtk::DrawingArea,
    text: gtk::TextView,
    text_css: gtk::CssProvider,
    st: RefCell<State>,
    listeners: RefCell<Vec<Rc<dyn Fn(Change)>>>,
    requests: RefCell<Vec<Rc<dyn Fn(&Request)>>>,
}

impl Editor {
    pub fn new(image: Bitmap) -> Rc<Self> {
        let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).focusable(true).build();
        let text = gtk::TextView::builder().visible(false).halign(gtk::Align::Start).valign(gtk::Align::Start).wrap_mode(gtk::WrapMode::None).accepts_tab(false).build();
        text.add_css_class("obtura-text");
        let widget = gtk::Overlay::new();
        widget.set_child(Some(&area));
        widget.add_overlay(&text);
        widget.set_overflow(gtk::Overflow::Hidden);

        let text_css = gtk::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &text_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        let st = State::new(image);
        let ed = Rc::new(Self { widget, area, text, text_css, st: RefCell::new(st), listeners: RefCell::new(Vec::new()), requests: RefCell::new(Vec::new()) });
        ed.connect();
        ed.update_cursor();
        ed
    }

    /// Calls `f` after every change. Several listeners may be added; they run in order.
    pub fn connect_change(&self, f: impl Fn(Change) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    fn notify(&self, c: Change) {
        let listeners: Vec<_> = self.listeners.borrow().clone();
        for f in listeners {
            f(c);
        }
    }

    fn redraw(&self) {
        self.area.queue_draw();
    }

    /// Calls `f` when the editor needs the window to do something (see [`Request`]).
    pub fn connect_request(&self, f: impl Fn(&Request) + 'static) {
        self.requests.borrow_mut().push(Rc::new(f));
    }

    fn request(&self, r: Request) {
        let fs: Vec<_> = self.requests.borrow().clone();
        for f in fs {
            f(&r);
        }
    }

    /// The picture as it is seen (every mark applied, on the screenshot's area).
    pub fn seen_picture(&self) -> Rc<Bitmap> {
        seen(&mut self.st.borrow_mut())
    }

    /// What text reading looks at: the picture as it is seen, with every redacted area
    /// filled with the colour around it. Nothing hidden can be read, and the reader is not
    /// thrown off by black boxes.
    pub fn picture_to_read(&self) -> Bitmap {
        let mut st = self.st.borrow_mut();
        let mut picture = seen(&mut st).as_ref().clone();
        obtura_render::blank_redactions(&mut picture, st.doc.scene.objects());
        picture
    }

    /// Outlines these areas (picture pixels) until they are cleared with an empty list.
    pub fn show_proposals(&self, areas: Vec<RectF>) {
        self.st.borrow_mut().proposals = areas;
        self.redraw();
    }

    /// Adds marks in one undo step (smart redact).
    pub fn add_marks(&self, marks: Vec<(Kind, Style)>) {
        if marks.is_empty() {
            return;
        }
        self.commit_text();
        {
            let mut st = self.st.borrow_mut();
            st.doc.begin_edit();
            for (kind, style) in marks {
                st.doc.scene.add(kind, style);
            }
            st.underlay = None;
            st.selected = None;
        }
        self.redraw();
        self.notify(Change::Document);
        self.notify(Change::Selection);
    }

    /// The style a new mark of `tool` gets.
    pub fn style_of(&self, tool: Tool) -> Style {
        style_for(&self.st.borrow(), tool)
    }

    // ---- Public state ----

    pub fn tool(&self) -> Tool {
        self.st.borrow().tool
    }

    pub fn size(&self) -> (u32, u32) {
        let st = self.st.borrow();
        (st.doc.base.width, st.doc.base.height)
    }

    /// The size of the finished picture before any frame: the screenshot, grown when marks
    /// reach past its edges.
    pub fn canvas_size(&self) -> (u32, u32) {
        let st = self.st.borrow();
        let c = st.doc.scene.canvas(st.doc.base.width, st.doc.base.height);
        (c.w, c.h)
    }

    pub fn can_undo(&self) -> bool {
        self.st.borrow().doc.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.st.borrow().doc.can_redo()
    }

    pub fn has_crop(&self) -> bool {
        self.st.borrow().crop.is_some()
    }

    /// The finished picture with every mark applied (redactions destroy their pixels), in its
    /// frame if it has one.
    pub fn export(&self) -> Result<Bitmap, String> {
        let st = self.st.borrow();
        let picture = render(&st.doc.base, &st.doc.scene)?;
        match &st.doc.frame {
            Some(f) => apply_frame(&picture, f),
            None => Ok(picture),
        }
    }

    /// The styles new marks start with, per tool (the user's saved defaults).
    pub fn set_tool_styles(&self, styles: impl IntoIterator<Item = (Tool, Style)>) {
        self.st.borrow_mut().styles.extend(styles);
        self.notify(Change::Style);
    }

    /// The tool whose style the Formatting panel shows: that of the selected mark, or the
    /// current one.
    pub fn style_tool(&self) -> Tool {
        let st = self.st.borrow();
        selected_object(&st).map(|o| tools::tool_for(&o.kind)).unwrap_or(st.tool)
    }

    /// Puts the built-in style back for `tool` (for its next marks).
    pub fn reset_tool_style(&self, tool: Tool) {
        self.st.borrow_mut().styles.remove(&tool);
        self.redraw();
        self.notify(Change::Style);
    }

    /// The frame a new capture starts with: not an undo step, and not a change to save.
    pub fn init_frame(&self, frame: Option<Frame>) {
        self.st.borrow_mut().doc.frame = frame;
        self.redraw();
    }

    /// The picture as it is now, without marks.
    pub fn picture(&self) -> Rc<Bitmap> {
        self.st.borrow().doc.base.clone()
    }

    /// The marks, bottom to top.
    pub fn marks(&self) -> Vec<Object> {
        self.st.borrow().doc.scene.objects().to_vec()
    }

    /// The marks and frame a picture reopened from the library starts with (not an undo step).
    pub fn init_marks(&self, marks: Vec<Object>, frame: Option<Frame>) {
        {
            let mut st = self.st.borrow_mut();
            for o in marks {
                st.doc.scene.add_object(o);
            }
            st.doc.frame = frame;
            st.underlay = None;
        }
        self.redraw();
        self.notify(Change::Document);
    }

    /// Changes the picture itself (turn, flip, resize, add space, attach...): `f` returns
    /// whether it changed anything. The marks were moved by `f`.
    pub fn edit_picture(&self, f: impl FnOnce(&mut Document) -> bool) -> bool {
        self.commit_text();
        let done = {
            let mut st = self.st.borrow_mut();
            let ok = f(&mut st.doc);
            if ok {
                st.underlay = None;
                st.selected = None;
                st.crop = None;
                st.draft = None;
                st.drag = None;
            }
            ok
        };
        if done {
            self.redraw();
            self.notify(Change::Document);
            self.notify(Change::Selection);
        }
        done
    }

    /// Places `text` as a text mark (the Text tool's style): where the pointer is over the
    /// picture, else in the middle. Long lines are kept as they are; it can be resized by its
    /// corners. Selected at once, so it can be moved.
    pub fn paste_text(&self, text: &str) {
        let text = text.replace("\r\n", "\n").trim_end().to_string();
        if text.is_empty() {
            return;
        }
        self.commit_text();
        self.set_tool(Tool::Select);
        {
            let mut st = self.st.borrow_mut();
            let style = style_for(&st, Tool::Text);
            let (w, h) = measure_text(&text, &style);
            let (bw, bh) = (st.doc.base.width as f64, st.doc.base.height as f64);
            let at = st.pointer.map(|(x, y)| st.view.to_image(x, y)).filter(|p| p.x >= 0.0 && p.y >= 0.0 && p.x < bw && p.y < bh);
            let pos = match at {
                Some(p) => p,
                None => Point::new(((bw - w) / 2.0).max(0.0).round(), ((bh - h) / 2.0).max(0.0).round()),
            };
            st.doc.begin_edit();
            let id = st.doc.scene.add(Kind::Text { pos, text, size: Point::new(w, h) }, style);
            st.selected = Some(id);
        }
        self.redraw();
        self.notify(Change::Document);
        self.notify(Change::Selection);
    }

    /// Places `picture` over the screenshot, in the middle, shrunk to fit if it is big, and
    /// selects it so it can be moved and resized at once.
    pub fn insert_picture(&self, picture: Bitmap) {
        self.set_tool(Tool::Select);
        {
            let mut st = self.st.borrow_mut();
            let (w, h) = (st.doc.base.width as f64, st.doc.base.height as f64);
            let (pw, ph) = (picture.width as f64, picture.height as f64);
            let k = (w * 0.6 / pw).min(h * 0.6 / ph).min(1.0);
            let rect = RectF { x: ((w - pw * k) / 2.0).round(), y: ((h - ph * k) / 2.0).round(), w: (pw * k).round(), h: (ph * k).round() };
            let style = style_for(&st, Tool::Picture);
            st.doc.begin_edit();
            let id = st.doc.scene.add(Kind::Image { rect, pixels: obtura_scene::Pixels::new(picture) }, style);
            st.selected = Some(id);
        }
        self.redraw();
        self.notify(Change::Document);
        self.notify(Change::Selection);
    }

    /// The shape the crop rectangle keeps.
    pub fn crop_ratio(&self) -> Option<(u32, u32)> {
        self.st.borrow().crop_ratio
    }

    pub fn set_crop_ratio(&self, ratio: Option<(u32, u32)>) {
        {
            let mut st = self.st.borrow_mut();
            st.crop_ratio = ratio;
            // A crop already drawn takes the new shape, about its top-left corner.
            if let (Some(c), Some((rw, rh))) = (st.crop, ratio) {
                let r = rw as f64 / rh as f64;
                let (w, h) = if c.w / c.h.max(1.0) > r { (c.h * r, c.h) } else { (c.w, c.w / r) };
                st.crop = Some(RectF { w, h, ..c });
            }
        }
        self.redraw();
        self.notify(Change::Style);
    }

    /// Beautify: the frame around the picture.
    pub fn frame(&self) -> Option<Frame> {
        self.st.borrow().doc.frame
    }

    /// Sets the frame. Changes in quick succession (a slider being dragged) are one undo step.
    pub fn set_frame(&self, frame: Option<Frame>) {
        {
            let mut st = self.st.borrow_mut();
            if st.doc.frame == frame {
                return;
            }
            let continuing = st.frame_edit.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(700));
            if !continuing {
                st.doc.begin_edit();
            }
            st.frame_edit = Some(std::time::Instant::now());
            st.doc.frame = frame;
        }
        self.redraw();
        self.notify(Change::Document);
    }

    /// The style a new mark of the current tool gets, or that of the selected mark.
    pub fn style(&self) -> Style {
        let st = self.st.borrow();
        selected_object(&st).map(|o| o.style).unwrap_or_else(|| style_for(&st, st.tool))
    }

    /// Which Formatting controls matter now.
    pub fn fields(&self) -> Fields {
        let st = self.st.borrow();
        selected_object(&st).map(|o| tools::fields_of(&o.kind)).unwrap_or_else(|| st.tool.fields())
    }

    /// Changes the style of the selected mark (one undo step), or of the current tool.
    pub fn apply_style(&self, change: impl FnOnce(&mut Style)) {
        let changed_mark = {
            let mut st = self.st.borrow_mut();
            match st.selected {
                Some(id) if st.doc.scene.get(id).is_some() => {
                    let continuing = st.style_edit.is_some_and(|(i, t)| i == id && t.elapsed() < std::time::Duration::from_millis(700));
                    if !continuing {
                        st.doc.begin_edit();
                    }
                    st.style_edit = Some((id, std::time::Instant::now()));
                    let (kind_tool, new_style) = {
                        let o = st.doc.scene.get_mut(id).expect("checked");
                        change(&mut o.style);
                        (tools::tool_for(&o.kind), o.style)
                    };
                    refresh_text_size(&mut st.doc, id, new_style);
                    // What was just chosen is also what the next mark of this tool gets.
                    if kind_tool == st.tool {
                        st.styles.insert(kind_tool, new_style);
                    }
                    st.underlay = None;
                    true
                }
                _ => {
                    let tool = st.tool;
                    let mut s = style_for(&st, tool);
                    change(&mut s);
                    st.styles.insert(tool, s);
                    false
                }
            }
        };
        self.redraw();
        self.notify(if changed_mark { Change::Document } else { Change::Style });
    }

    /// The emoji or symbol of the selected stamp, or the one the Stamp tool places.
    pub fn stamp(&self) -> String {
        let st = self.st.borrow();
        match selected_object(&st).map(|o| &o.kind) {
            Some(Kind::Stamp { glyph, .. }) => glyph.clone(),
            _ => st.stamp.clone(),
        }
    }

    /// Picks the emoji or symbol: for the selected stamp (one undo step) and for the next ones.
    pub fn set_stamp(&self, glyph: &str) {
        let changed = {
            let mut st = self.st.borrow_mut();
            st.stamp = glyph.to_string();
            let id = st.selected.filter(|id| matches!(st.doc.scene.get(*id).map(|o| &o.kind), Some(Kind::Stamp { .. })));
            match id {
                Some(id) => {
                    st.doc.begin_edit();
                    if let Some(Object { kind: Kind::Stamp { glyph: g, .. }, .. }) = st.doc.scene.get_mut(id) {
                        *g = glyph.to_string();
                    }
                    true
                }
                None => false,
            }
        };
        self.redraw();
        self.notify(if changed { Change::Document } else { Change::Style });
    }

    // ---- Actions ----

    pub fn set_tool(&self, tool: Tool) {
        self.commit_text();
        {
            let mut st = self.st.borrow_mut();
            if st.tool == tool {
                return;
            }
            st.tool = tool;
            st.selected = None;
            st.draft = None;
            st.drag = None;
            st.crop = None;
            st.grab = None;
            st.frozen = None;
        }
        self.update_cursor();
        self.redraw();
        self.notify(Change::Tool);
    }

    fn update_cursor(&self) {
        let name = if self.tool() == Tool::Select { "default" } else { "crosshair" };
        self.area.set_cursor_from_name(Some(name));
    }

    pub fn undo(&self) {
        self.commit_text();
        let done = self.st.borrow_mut().doc.undo();
        if done {
            self.after_history();
        }
    }

    pub fn redo(&self) {
        self.commit_text();
        let done = self.st.borrow_mut().doc.redo();
        if done {
            self.after_history();
        }
    }

    fn after_history(&self) {
        {
            let mut st = self.st.borrow_mut();
            st.underlay = None;
            st.crop = None;
            if let Some(id) = st.selected {
                if st.doc.scene.get(id).is_none() {
                    st.selected = None;
                }
            }
        }
        self.redraw();
        self.notify(Change::Document);
        self.notify(Change::Selection);
    }

    pub fn delete_selected(&self) {
        let removed = {
            let mut st = self.st.borrow_mut();
            match st.selected.take() {
                Some(id) => {
                    let ok = st.doc.remove(id);
                    // Steps renumber so the badges stay 1, 2, 3...
                    st.underlay = None;
                    ok
                }
                None => false,
            }
        };
        if removed {
            self.redraw();
            self.notify(Change::Document);
            self.notify(Change::Selection);
        }
    }

    /// Escape: backs out one level. Returns false if there was nothing to back out of.
    pub fn escape(&self) -> bool {
        if self.st.borrow().editing.is_some() {
            self.commit_text();
            return true;
        }
        let level = {
            let mut st = self.st.borrow_mut();
            if st.draft.is_some() || st.drag.is_some() {
                st.draft = None;
                st.drag = None;
                st.band = None;
                st.grab = None;
                st.frozen = None;
                1
            } else if st.crop.is_some() {
                st.crop = None;
                1
            } else if st.selected.is_some() {
                st.selected = None;
                2
            } else {
                0
            }
        };
        match level {
            0 if self.tool() != Tool::Select => {
                self.set_tool(Tool::Select);
                true
            }
            0 => false,
            l => {
                self.redraw();
                if l == 2 {
                    self.notify(Change::Selection);
                }
                true
            }
        }
    }

    pub fn apply_crop(&self) {
        let done = {
            let mut st = self.st.borrow_mut();
            let Some(r) = st.crop.take() else { return };
            let rect = Rect::new(r.x.round() as i32, r.y.round() as i32, r.w.round().max(1.0) as u32, r.h.round().max(1.0) as u32);
            let ok = st.doc.crop(rect);
            st.underlay = None;
            st.selected = None;
            ok
        };
        self.redraw();
        if done {
            self.notify(Change::Document);
            self.notify(Change::Selection);
        }
    }

    // ---- Zoom and scroll ----

    /// The current scale, 1.0 being one picture pixel per screen pixel.
    pub fn scale(&self) -> f64 {
        self.st.borrow().view.scale
    }

    /// Whether the picture is fitted to the window (rather than at a chosen zoom).
    pub fn is_fit(&self) -> bool {
        self.st.borrow().zoom.is_none()
    }

    /// Zooms to `scale`, keeping the picture point under `anchor` (widget coordinates; the
    /// middle of the canvas if `None`) where it is.
    pub fn zoom_to(&self, scale: f64, anchor: Option<(f64, f64)>) {
        self.commit_text();
        {
            let mut st = self.st.borrow_mut();
            let scale = scale.clamp(MIN_ZOOM, MAX_ZOOM);
            let (cw, ch) = st.canvas;
            let (ax, ay) = anchor.unwrap_or((cw as f64 / 2.0, ch as f64 / 2.0));
            let p = st.view.to_image(ax, ay);
            st.origin = (ax - p.x * scale, ay - p.y * scale);
            st.zoom = Some(scale);
        }
        self.redraw();
        self.notify(Change::View);
    }

    pub fn zoom_in(&self) {
        let (s, at) = (self.scale(), self.st.borrow().pointer);
        self.zoom_to(nice_zoom(s * ZOOM_STEP, s), at);
    }

    pub fn zoom_out(&self) {
        let (s, at) = (self.scale(), self.st.borrow().pointer);
        self.zoom_to(nice_zoom(s / ZOOM_STEP, s), at);
    }

    /// Back to "the whole picture fits the window".
    pub fn zoom_fit(&self) {
        self.commit_text();
        self.st.borrow_mut().zoom = None;
        self.redraw();
        self.notify(Change::View);
    }

    /// Scrolls the view by (dx, dy) widget pixels. Does nothing when everything fits.
    fn scroll_by(&self, dx: f64, dy: f64) {
        self.commit_text();
        {
            let mut st = self.st.borrow_mut();
            if st.zoom.is_none() {
                // Scrolling a fitted picture starts "zoomed" at the same scale.
                let s = st.view.scale;
                st.zoom = Some(s);
            }
            st.origin = (st.view.ox - dx, st.view.oy - dy);
        }
        self.redraw();
    }

    // ---- Input ----

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.area.set_draw_func({
            let weak = weak.clone();
            let shown = std::cell::Cell::new(f64::NAN);
            move |_, cr, w, h| {
                if let Some(ed) = weak.upgrade() {
                    ed.draw(cr, w, h);
                    // The fitted scale follows the window size: tell the zoom label, later
                    // (listeners may not run inside a draw).
                    let now = ed.st.borrow().view.scale;
                    if shown.replace(now) != now {
                        let weak = weak.clone();
                        glib::idle_add_local_once(move || {
                            if let Some(ed) = weak.upgrade() {
                                ed.notify(Change::View);
                            }
                        });
                    }
                }
            }
        });

        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        {
            let weak = weak.clone();
            drag.connect_drag_begin(move |g, x, y| {
                if let Some(ed) = weak.upgrade() {
                    ed.area.grab_focus();
                    ed.press(x, y, g.current_event_state());
                }
            });
        }
        {
            let weak = weak.clone();
            drag.connect_drag_update(move |g, dx, dy| {
                if let (Some(ed), Some((sx, sy))) = (weak.upgrade(), g.start_point()) {
                    ed.motion(sx + dx, sy + dy, g.current_event_state());
                }
            });
        }
        {
            let weak = weak.clone();
            drag.connect_drag_end(move |_, dx, dy| {
                if let Some(ed) = weak.upgrade() {
                    ed.release(dx.hypot(dy));
                }
            });
        }
        self.area.add_controller(drag);

        // Ctrl+wheel zooms around the pointer; the wheel scrolls (Shift: sideways).
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        {
            let weak = weak.clone();
            scroll.connect_scroll(move |c, dx, dy| {
                let Some(ed) = weak.upgrade() else { return glib::Propagation::Proceed };
                let mods = c.current_event_state();
                if mods.contains(gdk::ModifierType::CONTROL_MASK) {
                    if dy < 0.0 {
                        ed.zoom_in();
                    } else if dy > 0.0 {
                        ed.zoom_out();
                    }
                } else {
                    let unit = if c.unit() == gdk::ScrollUnit::Wheel { 48.0 } else { 1.0 };
                    let (dx, dy) = if mods.contains(gdk::ModifierType::SHIFT_MASK) && dx == 0.0 { (dy, 0.0) } else { (dx, dy) };
                    ed.scroll_by(dx * unit, dy * unit);
                }
                glib::Propagation::Stop
            });
        }
        self.area.add_controller(scroll);
        let motion = gtk::EventControllerMotion::new();
        {
            let w1 = weak.clone();
            motion.connect_motion(move |_, x, y| {
                if let Some(ed) = w1.upgrade() {
                    ed.st.borrow_mut().pointer = Some((x, y));
                    // The colour picker's loupe follows the pointer.
                    if ed.tool() == Tool::Picker {
                        ed.redraw();
                    }
                }
            });
            let weak = weak.clone();
            motion.connect_leave(move |_| {
                if let Some(ed) = weak.upgrade() {
                    ed.st.borrow_mut().pointer = None;
                    if ed.tool() == Tool::Picker {
                        ed.redraw();
                    }
                }
            });
        }
        self.area.add_controller(motion);
        // The middle button drags the view.
        let pan = gtk::GestureDrag::new();
        pan.set_button(2);
        {
            let w1 = weak.clone();
            pan.connect_drag_begin(move |_, _, _| {
                if let Some(ed) = w1.upgrade() {
                    let mut st = ed.st.borrow_mut();
                    st.panning = Some((st.view.ox, st.view.oy));
                }
            });
            let w2 = weak.clone();
            pan.connect_drag_update(move |_, dx, dy| {
                let Some(ed) = w2.upgrade() else { return };
                let Some((ox, oy)) = ed.st.borrow().panning else { return };
                let (vx, vy) = (ed.st.borrow().view.ox, ed.st.borrow().view.oy);
                ed.scroll_by(vx - (ox + dx), vy - (oy + dy));
            });
            let weak = weak.clone();
            pan.connect_drag_end(move |_, _, _| {
                if let Some(ed) = weak.upgrade() {
                    ed.st.borrow_mut().panning = None;
                }
            });
        }
        self.area.add_controller(pan);

        let click = gtk::GestureClick::new();
        click.set_button(1);
        {
            let weak = weak.clone();
            click.connect_pressed(move |_, n, x, y| {
                if let (Some(ed), true) = (weak.upgrade(), n == 2) {
                    ed.double_click(x, y);
                }
            });
        }
        self.area.add_controller(click);

        // Text typed on the picture.
        let keys = gtk::EventControllerKey::new();
        {
            let weak = weak.clone();
            keys.connect_key_pressed(move |_, key, _, mods| {
                let Some(ed) = weak.upgrade() else { return glib::Propagation::Proceed };
                match key {
                    gdk::Key::Return | gdk::Key::KP_Enter if !mods.contains(gdk::ModifierType::SHIFT_MASK) => {
                        ed.commit_text();
                        glib::Propagation::Stop
                    }
                    gdk::Key::Escape => {
                        ed.commit_text();
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            });
        }
        self.text.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        {
            let weak = weak.clone();
            focus.connect_leave(move |_| {
                if let Some(ed) = weak.upgrade() {
                    ed.commit_text();
                }
            });
        }
        self.text.add_controller(focus);
        self.text.buffer().connect_changed(move |_| {
            if let Some(ed) = weak.upgrade() {
                ed.fit_text_view();
                ed.redraw(); // the box around the text grows with it
            }
        });
    }

    /// Makes the typing field as big as what is typed, so the text is always visible.
    fn fit_text_view(&self) {
        let (style, scale) = {
            let st = self.st.borrow();
            (st.editing.as_ref().map_or(Style::default(), |e| e.style), st.view.scale)
        };
        let typed = self.typed();
        let (w, h) = measure_text(if typed.is_empty() { " " } else { &typed }, &style);
        self.text.set_width_request(((w * scale).ceil() as i32 + 24).max(48));
        self.text.set_height_request((h * scale).ceil() as i32 + 4);
    }

    /// What is typed in the text field now.
    fn typed(&self) -> String {
        let buffer = self.text.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string()
    }

    fn press(&self, wx: f64, wy: f64, _mods: gdk::ModifierType) {
        self.commit_text();
        let mut changes = Vec::new();
        {
            let mut st = self.st.borrow_mut();
            st.frozen = Some(st.shown);
            let p = st.view.to_image(wx, wy);
            // A handle of the selected mark wins over everything else.
            if let Some(o) = rotate_handle_at(&st, wx, wy) {
                let c = o.center();
                let start = (p.y - c.y).atan2(p.x - c.x);
                st.drag = Some(Drag::Rotate { id: o.id, orig: Box::new(o), start, edited: false });
                drop(st);
                self.redraw();
                return;
            }
            if let Some((id, index)) = handle_at(&st, wx, wy) {
                let orig = st.doc.scene.get(id).cloned().expect("handles belong to a mark");
                st.drag = Some(Drag::Handle { id, index, orig: Box::new(orig), edited: false });
                drop(st);
                self.redraw();
                return;
            }
            let tool = st.tool;
            if tool != Tool::Select && st.selected.take().is_some() {
                changes.push(Change::Selection);
            }
            match tool {
                Tool::Select => {
                    let hit = st.doc.scene.hit_test(p);
                    if hit != st.selected {
                        changes.push(Change::Selection);
                    }
                    st.selected = hit;
                    st.drag = hit.and_then(|id| st.doc.scene.get(id).cloned()).map(|o| Drag::Move { id: o.id, start: p, orig: Box::new(o), edited: false });
                }
                Tool::Text => {
                    drop(st);
                    self.open_text_at(p);
                    return;
                }
                Tool::Step => {
                    let number = st.doc.scene.next_step_number();
                    let style = style_for(&st, tool);
                    st.doc.begin_edit();
                    let id = st.doc.scene.add(Kind::Step { center: p, number }, style);
                    st.selected = Some(id);
                    changes.push(Change::Document);
                    changes.push(Change::Selection);
                }
                Tool::Stamp => {
                    let style = style_for(&st, tool);
                    let glyph = st.stamp.clone();
                    st.doc.begin_edit();
                    let id = st.doc.scene.add(Kind::Stamp { center: p, glyph }, style);
                    st.selected = Some(id);
                    changes.push(Change::Document);
                    changes.push(Change::Selection);
                }
                Tool::CutOut => {
                    let p = st.doc.clamp_point(p);
                    st.band = None;
                    st.drag = Some(Drag::Cut { start: p });
                }
                Tool::TextGrab => {
                    let p = st.doc.clamp_point(p);
                    st.grab = Some(RectF { x: p.x, y: p.y, w: 0.0, h: 0.0 });
                    st.drag = Some(Drag::Grab { start: p });
                }
                Tool::Picker => {
                    let (x, y) = (p.x.floor(), p.y.floor());
                    let picture = seen(&mut st);
                    if x >= 0.0 && y >= 0.0 && (x as u32) < picture.width && (y as u32) < picture.height {
                        let c = picture.pixel(x as u32, y as u32);
                        drop(st);
                        self.request(Request::PickColor(c));
                        return;
                    }
                }
                Tool::Crop => {
                    let p = st.doc.clamp_point(p);
                    let previous = st.crop;
                    st.crop = Some(RectF { x: p.x, y: p.y, w: 0.0, h: 0.0 });
                    st.drag = Some(Drag::Crop { start: p, previous });
                }
                Tool::Pen | Tool::Highlighter => {
                    let kind = if tool == Tool::Pen { Kind::Pen { points: vec![p] } } else { Kind::Highlighter { points: vec![p] } };
                    st.draft = Some(Object { id: 0, kind, style: style_for(&st, tool), angle: 0.0 });
                    st.drag = Some(Drag::Draw { start: p });
                }
                _ => {
                    let style = style_for(&st, tool);
                    st.draft = Some(Object { id: 0, kind: shape_kind(tool, p, p, &style), style, angle: 0.0 });
                    st.drag = Some(Drag::Draw { start: p });
                }
            }
        }
        self.redraw();
        for c in changes {
            self.notify(c);
        }
    }

    fn motion(&self, wx: f64, wy: f64, mods: gdk::ModifierType) {
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        {
            let mut st = self.st.borrow_mut();
            let p = st.view.to_image(wx, wy);
            let tool = st.tool;
            let st = &mut *st;
            match &mut st.drag {
                Some(Drag::Draw { start }) => {
                    if let Some(d) = st.draft.as_mut() {
                        match &mut d.kind {
                            Kind::Pen { points } | Kind::Highlighter { points } => {
                                if points.last().is_none_or(|l| l.distance(p) >= 1.0) {
                                    points.push(p);
                                }
                            }
                            _ => d.kind = shape_kind(tool, *start, constrain(tool, *start, p, shift), &d.style),
                        }
                    }
                }
                Some(Drag::Move { id, start, orig, edited }) => {
                    if !*edited {
                        st.doc.begin_edit();
                        *edited = true;
                    }
                    let (mut dx, mut dy) = (p.x - start.x, p.y - start.y);
                    st.guides.clear();
                    // Snap to the picture and to the other marks, unless Alt is held.
                    if !mods.contains(gdk::ModifierType::ALT_MASK) {
                        let reach = SNAP / st.view.scale;
                        let b = orig.bounds();
                        let moved = RectF { x: b.x + dx, y: b.y + dy, ..b };
                        let (xs, ys) = snap_lines(&st.doc, *id);
                        if let Some((shift, at)) = snap(&[moved.x, moved.x + moved.w / 2.0, moved.x + moved.w], &xs, reach) {
                            dx += shift;
                            st.guides.push((true, at));
                        }
                        if let Some((shift, at)) = snap(&[moved.y, moved.y + moved.h / 2.0, moved.y + moved.h], &ys, reach) {
                            dy += shift;
                            st.guides.push((false, at));
                        }
                    }
                    let mut o = (**orig).clone();
                    o.translate(dx, dy);
                    if let Some(m) = st.doc.scene.get_mut(*id) {
                        *m = o;
                    }
                    st.underlay = None;
                }
                Some(Drag::Handle { id, index, orig, edited }) => {
                    if !*edited {
                        st.doc.begin_edit();
                        *edited = true;
                    }
                    // Marks may reach past the edge (the picture grows); the area a magnifier
                    // looks at stays on the screenshot.
                    let p = if matches!(orig.kind, Kind::Magnify { .. }) && *index < 4 { st.doc.clamp_point(p) } else { p };
                    let changed = orig.with_handle(*index, p);
                    let (id, style) = (*id, changed.style);
                    if let Some(o) = st.doc.scene.get_mut(id) {
                        o.kind = changed.kind;
                        o.style = changed.style;
                    }
                    if matches!(orig.kind, Kind::Text { .. } | Kind::Callout { .. }) {
                        refresh_text_size(&mut st.doc, id, style);
                    }
                    if matches!(orig.kind, Kind::Redact { .. } | Kind::Magnify { .. }) {
                        st.underlay = None;
                    }
                }
                Some(Drag::Rotate { id, orig, start, edited }) => {
                    if !*edited {
                        st.doc.begin_edit();
                        *edited = true;
                    }
                    let c = orig.center();
                    let mut delta = (p.y - c.y).atan2(p.x - c.x) - *start;
                    // Shift: steps of 15 degrees.
                    if shift {
                        let step = 15f64.to_radians();
                        let base = orig.turn();
                        delta = ((base + delta) / step).round() * step - base;
                    }
                    let turned = orig.rotated(delta);
                    if let Some(o) = st.doc.scene.get_mut(*id) {
                        *o = turned;
                    }
                }
                Some(Drag::Crop { start, .. }) => {
                    let end = st.doc.clamp_point(p);
                    let end = match st.crop_ratio {
                        Some(ratio) => with_ratio(*start, end, ratio),
                        None => constrain(Tool::Crop, *start, end, shift),
                    };
                    st.crop = Some(RectF::from_points(*start, st.doc.clamp_point(end)));
                }
                Some(Drag::Cut { start }) => {
                    let end = st.doc.clamp_point(p);
                    st.band = Some(Band::between(*start, end));
                }
                Some(Drag::Grab { start }) => {
                    let end = st.doc.clamp_point(p);
                    st.grab = Some(RectF::from_points(*start, end));
                }
                None => {}
            }
        }
        self.redraw();
    }

    fn release(&self, dragged_px: f64) {
        let mut changes = Vec::new();
        // A callout is placed by the drag; its text is typed next.
        let mut callout: Option<(Point, Point, Style)> = None;
        let mut grab = None;
        {
            let mut st = self.st.borrow_mut();
            let scale = st.view.scale;
            st.guides.clear();
            // Typing keeps the view still until the text is done.
            if st.editing.is_none() {
                st.frozen = None;
            }
            match st.drag.take() {
                Some(Drag::Draw { .. }) => {
                    if let Some(d) = st.draft.take() {
                        if let Kind::Callout { tip, pos, .. } = d.kind {
                            // A click without a drag puts the box up and to the right of the point.
                            let pos = if tip.distance(pos) < 12.0 { tip.offset(70.0, -50.0) } else { pos };
                            callout = Some((tip, pos, d.style));
                        } else if worth_keeping(&d.kind) || matches!(d.kind, Kind::Magnify { .. }) {
                            let kind = match d.kind {
                                // The lens goes beside the area, twice as big. A click without a
                                // drag looks at a small area around the point.
                                Kind::Magnify { source, .. } => {
                                    let (w, h) = (st.doc.base.width as f64, st.doc.base.height as f64);
                                    let source = if source.w < MIN_DRAG || source.h < MIN_DRAG { RectF { x: source.x - 40.0, y: source.y - 25.0, w: 80.0, h: 50.0 } } else { source };
                                    // The area looked at stays inside the picture.
                                    let source = RectF { x: source.x.clamp(0.0, (w - source.w).max(0.0)), y: source.y.clamp(0.0, (h - source.h).max(0.0)), w: source.w.min(w), h: source.h.min(h) };
                                    Kind::Magnify { source, lens: Object::lens_for(source, 2.0, w, h) }
                                }
                                k => k,
                            };
                            st.doc.begin_edit();
                            if matches!(kind, Kind::Redact { .. } | Kind::Magnify { .. }) {
                                st.underlay = None;
                            }
                            let id = st.doc.scene.add(kind, d.style);
                            // Handles show at once, to bend or resize what was just drawn.
                            st.selected = Some(id);
                            changes.push(Change::Document);
                            changes.push(Change::Selection);
                        }
                    }
                }
                Some(Drag::Move { edited: true, .. }) | Some(Drag::Handle { edited: true, .. }) | Some(Drag::Rotate { edited: true, .. }) => changes.push(Change::Document),
                Some(Drag::Move { .. }) | Some(Drag::Handle { .. }) | Some(Drag::Rotate { .. }) => {}
                Some(Drag::Cut { .. }) => {
                    if let Some(b) = st.band.take() {
                        if (b.to - b.from) * scale >= 3.0 && st.doc.cut(b.from as u32, b.to as u32, b.horizontal) {
                            st.underlay = None;
                            st.selected = None;
                            changes.push(Change::Document);
                        }
                    }
                }
                Some(Drag::Grab { .. }) => {
                    let (w, h) = (st.doc.base.width, st.doc.base.height);
                    // A click (or a tiny drag) reads the whole picture.
                    let area = match st.grab.take() {
                        Some(r) if r.w * scale >= 4.0 && r.h * scale >= 4.0 => Rect::new(r.x.floor() as i32, r.y.floor() as i32, r.w.ceil() as u32, r.h.ceil() as u32),
                        _ => Rect::new(0, 0, w, h),
                    };
                    grab = Some(area);
                }
                Some(Drag::Crop { previous, .. }) => {
                    // A plain click keeps the crop that was there; a tiny one is dropped.
                    if dragged_px < 3.0 {
                        st.crop = previous;
                    } else if st.crop.is_some_and(|r| r.w * scale < 4.0 || r.h * scale < 4.0) {
                        st.crop = None;
                    }
                }
                None => {}
            }
        }
        self.redraw();
        for c in changes {
            self.notify(c);
        }
        if let Some((tip, pos, style)) = callout {
            self.start_editing(None, pos, style, "", Some(tip));
        }
        if let Some(area) = grab {
            self.request(Request::GrabText(area));
        }
    }

    fn double_click(&self, wx: f64, wy: f64) {
        // On the selected mark's handles: its round handle stands it upright again; a bend of a
        // line goes. With the Select tool, on a selected line: a new bend there.
        let reshaped = {
            let mut st = self.st.borrow_mut();
            let p = st.view.to_image(wx, wy);
            let upright = rotate_handle_at(&st, wx, wy).filter(|o| o.turns() && o.angle != 0.0).map(|o| o.id);
            let bend = handle_at(&st, wx, wy).filter(|(_, i)| *i >= 2 && matches!(selected_object(&st).map(|o| &o.kind), Some(Kind::Line { .. })));
            let on_line = st.tool == Tool::Select && selected_object(&st).is_some_and(|o| matches!(o.kind, Kind::Line { .. })) && st.selected == st.doc.scene.hit_test(p);
            if let Some(id) = upright {
                st.drag = None;
                st.doc.begin_edit();
                if let Some(o) = st.doc.scene.get_mut(id) {
                    o.angle = 0.0;
                }
                true
            } else if let Some((id, i)) = bend {
                st.drag = None;
                st.doc.begin_edit();
                st.doc.scene.get_mut(id).is_some_and(|o| o.remove_bend(i))
            } else if on_line {
                let id = st.selected.expect("a line is selected");
                st.drag = None;
                st.doc.begin_edit();
                st.doc.scene.get_mut(id).and_then(|o| o.add_bend(p)).is_some()
            } else {
                false
            }
        };
        if reshaped {
            self.redraw();
            self.notify(Change::Document);
            return;
        }
        let tool = self.tool();
        if tool == Tool::Crop {
            self.apply_crop();
            return;
        }
        if tool == Tool::Select {
            self.open_text_at_widget(wx, wy);
        }
    }

    // ---- Text typed on the picture ----

    fn open_text_at_widget(&self, wx: f64, wy: f64) {
        let p = self.st.borrow().view.to_image(wx, wy);
        let existing = {
            let st = self.st.borrow();
            st.doc.scene.hit_test(p).and_then(|id| st.doc.scene.get(id).cloned())
        };
        self.edit_existing(existing);
    }

    fn edit_existing(&self, existing: Option<Object>) {
        match existing {
            Some(Object { id, kind: Kind::Text { pos, text, .. }, style, .. }) => self.start_editing(Some(id), pos, style, &text, None),
            Some(Object { id, kind: Kind::Callout { tip, pos, text, .. }, style, .. }) => self.start_editing(Some(id), pos, style, &text, Some(tip)),
            _ => {}
        }
    }

    /// Text tool, click: edits the text mark there, or starts a new one.
    fn open_text_at(&self, p: Point) {
        let existing = {
            let st = self.st.borrow();
            st.doc.scene.hit_test(p).and_then(|id| st.doc.scene.get(id).cloned())
        };
        if matches!(existing, Some(Object { kind: Kind::Text { .. } | Kind::Callout { .. }, .. })) {
            self.edit_existing(existing);
        } else {
            let style = style_for(&self.st.borrow(), Tool::Text);
            self.start_editing(None, p, style, "", None);
        }
    }

    fn start_editing(&self, id: Option<Id>, at: Point, style: Style, text: &str, tip: Option<Point>) {
        let scale = {
            let mut st = self.st.borrow_mut();
            st.editing = Some(TextEditing { id, at, style, tip });
            // The typing field sits over the picture: the view must not move while typing.
            st.frozen = Some(st.shown);
            st.view.scale
        };
        let (x, y) = self.st.borrow().view.to_widget(at);
        let px = (style.font_size * scale).max(8.0);
        let c = style.text_color.to_hex();
        self.text_css.load_from_string(&format!(
            ".obtura-text {{ font-family: \"{family}\"; font-size: {px}px; font-weight: {weight}; font-style: {italic}; color: {c}; caret-color: {c}; background: transparent; padding: 0; outline: 1px dashed #3584e4; }} .obtura-text text {{ background: transparent; color: {c}; }}",
            family = style.font.family().replace(['"', '\\'], ""),
            weight = if style.bold { "bold" } else { "normal" },
            italic = if style.italic { "italic" } else { "normal" },
        ));
        self.text.buffer().set_text(text);
        self.text.set_margin_start(x.round() as i32);
        self.text.set_margin_top(y.round() as i32);
        self.text.set_visible(true);
        self.text.grab_focus();
        self.redraw();
    }

    /// Turns what was typed into a text mark (or removes the mark if it was emptied).
    pub fn commit_text(&self) {
        let Some(editing) = self.st.borrow_mut().editing.take() else { return };
        self.st.borrow_mut().frozen = None;
        let buffer = self.text.buffer();
        let typed = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
        let typed = typed.trim_end_matches('\n').to_string();
        self.text.set_visible(false);
        buffer.set_text("");
        let mut changed = false;
        {
            let mut st = self.st.borrow_mut();
            match (editing.id, typed.is_empty()) {
                (Some(id), true) => changed = st.doc.remove(id),
                (None, true) => {}
                (id, false) => {
                    let (w, h) = measure_text(&typed, &editing.style);
                    let size = Point::new(w, h);
                    let kind = match editing.tip {
                        Some(tip) => Kind::Callout { tip, pos: editing.at, text: typed, size },
                        None => Kind::Text { pos: editing.at, text: typed, size },
                    };
                    match id {
                        Some(id) => {
                            if st.doc.scene.get(id).is_some_and(|o| o.kind != kind) {
                                st.doc.begin_edit();
                                if let Some(o) = st.doc.scene.get_mut(id) {
                                    o.kind = kind;
                                }
                                changed = true;
                            }
                        }
                        None => {
                            st.doc.begin_edit();
                            st.doc.scene.add(kind, editing.style);
                            changed = true;
                        }
                    }
                }
            }
            st.underlay = None;
        }
        self.area.grab_focus();
        self.redraw();
        if changed {
            self.notify(Change::Document);
        }
    }

    // ---- Drawing ----

    /// The finished picture's area as it is now: the screenshot, grown for the marks, for the
    /// mark being drawn and for the text being typed.
    fn live_canvas(&self, st: &State) -> Rect {
        let (bw, bh) = (st.doc.base.width, st.doc.base.height);
        let mut c = st.doc.scene.canvas(bw, bh);
        if let Some(d) = &st.draft {
            c = union(c, canvas_of(std::slice::from_ref(d), bw, bh));
        }
        if let Some(o) = self.editing_preview(st) {
            c = union(c, canvas_of(std::slice::from_ref(&o), bw, bh));
        }
        c
    }

    /// The box (and the arrow of a callout) around the text being typed, without its text.
    fn editing_preview(&self, st: &State) -> Option<Object> {
        let e = st.editing.as_ref()?;
        let typed = self.typed();
        let (w, h) = measure_text(if typed.is_empty() { " " } else { &typed }, &e.style);
        let size = Point::new(w, h);
        let kind = match e.tip {
            Some(tip) => Kind::Callout { tip, pos: e.at, text: String::new(), size },
            None => Kind::Text { pos: e.at, text: String::new(), size },
        };
        Some(Object { id: 0, kind, style: e.style, angle: 0.0 })
    }

    fn draw(&self, cr: &cairo::Context, w: i32, h: i32) {
        let mut st = self.st.borrow_mut();
        let frame = st.doc.frame;
        let canvas = self.live_canvas(&st);
        let (cx, cy, cw, ch) = (canvas.x as f64, canvas.y as f64, canvas.w as f64, canvas.h as f64);
        let layout = frame.map(|f| f.layout(canvas.w, canvas.h));
        let content = {
            let r = content_rect(frame.as_ref(), canvas.w, canvas.h);
            RectF { x: r.x + cx, y: r.y + cy, ..r }
        };
        let fitted = st.frozen.unwrap_or(content);
        let view = match st.zoom {
            None => View::fit_rect(fitted, w, h),
            Some(z) => View::placed(fitted, z, st.origin, w, h, false),
        };
        st.view = view;
        st.origin = (view.ox, view.oy);
        st.canvas = (w, h);
        st.shown = fitted;
        ensure_underlay(&mut st, canvas);
        let (iw, ih) = (st.doc.base.width as f64, st.doc.base.height as f64);

        let _ = cr.save();
        cr.translate(view.ox, view.oy);
        cr.scale(view.scale, view.scale);
        match (frame, layout) {
            (Some(f), Some(l)) => {
                let size = (canvas.w, canvas.h);
                if !st.backdrop.as_ref().is_some_and(|(bf, bs, _)| *bf == f && *bs == size) {
                    st.backdrop = backdrop(&f, &l).ok().map(|s| (f, size, s));
                }
                if let Some((_, _, b)) = &st.backdrop {
                    let _ = cr.set_source_surface(b, cx - l.pic.x, cy - l.pic.y);
                    let _ = cr.paint();
                }
                // The picture and its marks stay inside the picture's (rounded, torn) outline.
                let _ = cr.save();
                cr.translate(cx - l.pic.x, cy - l.pic.y);
                picture_path(cr, &f, &l);
                let _ = cr.restore();
            }
            _ => cr.rectangle(cx, cy, cw, ch),
        }
        cr.clip();
        if canvas != Rect::new(0, 0, st.doc.base.width, st.doc.base.height) {
            // The room added for marks past the edge, white as in the saved picture.
            cr.rectangle(cx, cy, cw, ch);
            let [r, g, b, a] = obtura_render::MARGIN_FILL;
            cr.set_source_rgba(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a as f64 / 255.0);
            let _ = cr.fill();
        }
        if let Some(u) = &st.underlay {
            let _ = cr.set_source_surface(&u.surface, u.rect.x as f64, u.rect.y as f64);
            // Zoomed far in, every pixel shows as a sharp square.
            cr.source().set_filter(if view.scale >= 2.0 { cairo::Filter::Nearest } else { cairo::Filter::Good });
            let _ = cr.paint();
            let skip = st.editing.as_ref().and_then(|e| e.id);
            let live: Vec<Object> = st.doc.scene.objects()[u.n..].iter().filter(|o| Some(o.id) != skip).cloned().collect();
            draw_objects(cr, &live);
        }
        if let Some(o) = self.editing_preview(&st) {
            // The box (and the arrow of a callout) around what is being typed; the text itself
            // is the typing field's.
            draw_objects(cr, &[o]);
        }
        if let Some(d) = &st.draft {
            match &d.kind {
                Kind::Redact { rect, .. } | Kind::Magnify { source: rect, .. } => {
                    cr.rectangle(rect.x, rect.y, rect.w, rect.h);
                    cr.set_source_rgba(0.1, 0.4, 0.9, 0.25);
                    let _ = cr.fill_preserve();
                    cr.set_source_rgba(0.1, 0.4, 0.9, 0.9);
                    cr.set_line_width(1.0 / view.scale);
                    cr.set_dash(&[4.0 / view.scale, 3.0 / view.scale], 0.0);
                    let _ = cr.stroke();
                }
                _ => draw_objects(cr, std::slice::from_ref(d)),
            }
        }
        let _ = cr.restore();

        if let (Some(f), Some(l)) = (frame, layout) {
            let _ = cr.save();
            cr.translate(view.ox, view.oy);
            cr.scale(view.scale, view.scale);
            cr.translate(cx - l.pic.x, cy - l.pic.y);
            draw_border(cr, &f, &l);
            let _ = cr.restore();
        }
        // A thin edge around the result.
        let (ex, ey) = view.to_widget(Point::new(content.x, content.y));
        cr.rectangle(ex - 0.5, ey - 0.5, content.w * view.scale + 1.0, content.h * view.scale + 1.0);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.3);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        if let Some(o) = selected_object(&st).filter(|_| st.editing.is_none()) {
            // The outline turns with the mark.
            let b = o.upright_bounds().inflate(2.0 / view.scale);
            let corners = [Point::new(b.x, b.y), Point::new(b.x + b.w, b.y), Point::new(b.x + b.w, b.y + b.h), Point::new(b.x, b.y + b.h)];
            for (i, c) in corners.into_iter().enumerate() {
                let (x, y) = view.to_widget(obtura_scene::rotate_point(c, o.center(), o.turn()));
                if i == 0 {
                    cr.move_to(x, y);
                } else {
                    cr.line_to(x, y);
                }
            }
            cr.close_path();
            cr.set_source_rgba(0.21, 0.52, 0.89, 1.0);
            cr.set_line_width(1.5);
            cr.set_dash(&[5.0, 3.0], 0.0);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            // The round handle that turns the mark, on a stalk from its top edge.
            if let Some(r) = o.rotate_handle(ROTATE_GAP / view.scale) {
                let foot = o.rotate_handle(0.0).unwrap_or(r);
                let ((rx, ry), (fx, fy)) = (view.to_widget(r), view.to_widget(foot));
                cr.move_to(fx, fy);
                cr.line_to(rx, ry);
                cr.set_source_rgb(0.21, 0.52, 0.89);
                cr.set_line_width(1.5);
                let _ = cr.stroke();
                cr.arc(rx, ry, 5.5, 0.0, std::f64::consts::TAU);
                cr.set_source_rgb(1.0, 1.0, 1.0);
                let _ = cr.fill_preserve();
                cr.set_source_rgb(0.21, 0.52, 0.89);
                let _ = cr.stroke();
                // A turning arrow inside.
                cr.arc(rx, ry, 2.6, 0.3, 4.9);
                cr.set_line_width(1.2);
                let _ = cr.stroke();
            }
            // Bends of a line are round; every other handle is square.
            let bends_from = if matches!(o.kind, Kind::Line { .. }) { 2 } else { usize::MAX };
            for (i, h) in o.handles().into_iter().enumerate() {
                let (hx, hy) = view.to_widget(h);
                if i >= bends_from {
                    cr.arc(hx, hy, 5.0, 0.0, std::f64::consts::TAU);
                } else {
                    cr.rectangle(hx - 4.5, hy - 4.5, 9.0, 9.0);
                }
                cr.set_source_rgb(1.0, 1.0, 1.0);
                let _ = cr.fill_preserve();
                cr.set_source_rgb(0.21, 0.52, 0.89);
                cr.set_line_width(1.5);
                let _ = cr.stroke();
            }
        }
        if let Some(c) = st.crop {
            draw_crop(cr, &view, c, (iw, ih));
        }
        if let Some(b) = st.band {
            draw_band(cr, &view, b, (iw, ih));
        }
        if let Some(g) = st.grab {
            draw_area(cr, &view, g, "Release to copy the text in here");
        }
        for r in &st.proposals {
            let (x, y) = view.to_widget(Point::new(r.x, r.y));
            cr.rectangle(x - 2.0, y - 2.0, r.w * view.scale + 4.0, r.h * view.scale + 4.0);
            cr.set_source_rgba(0.88, 0.11, 0.14, 0.18);
            let _ = cr.fill_preserve();
            cr.set_source_rgb(0.88, 0.11, 0.14);
            cr.set_line_width(2.0);
            cr.set_dash(&[5.0, 3.0], 0.0);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
        if st.tool == Tool::Picker && st.drag.is_none() {
            if let Some((px, py)) = st.pointer {
                let picture = seen(&mut st);
                draw_loupe(cr, &picture, st.view.to_image(px, py), (px, py), (w as f64, h as f64));
            }
        }
        for &(vertical, at) in &st.guides {
            let (a, b) = if vertical { (Point::new(at, 0.0), Point::new(at, ih)) } else { (Point::new(0.0, at), Point::new(iw, at)) };
            let ((ax, ay), (bx, by)) = (view.to_widget(a), view.to_widget(b));
            cr.move_to(ax.round() + 0.5, ay.round() + 0.5);
            cr.line_to(bx.round() + 0.5, by.round() + 0.5);
            cr.set_source_rgb(0.88, 0.11, 0.6);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        }
    }
}

/// How near (in screen pixels) a mark's edge or centre must come to a guide to snap to it.
const SNAP: f64 = 6.0;

/// The lines a moving mark can snap to: the picture's edges and centre, and the edges and
/// centres of the other marks. (vertical lines' x, horizontal lines' y)
fn snap_lines(doc: &Document, moving: Id) -> (Vec<f64>, Vec<f64>) {
    let (w, h) = (doc.base.width as f64, doc.base.height as f64);
    let (mut xs, mut ys) = (vec![0.0, w / 2.0, w], vec![0.0, h / 2.0, h]);
    for o in doc.scene.objects().iter().filter(|o| o.id != moving) {
        let b = o.bounds();
        xs.extend([b.x, b.x + b.w / 2.0, b.x + b.w]);
        ys.extend([b.y, b.y + b.h / 2.0, b.y + b.h]);
    }
    (xs, ys)
}

/// The smallest shift (within `reach`) that puts one of `edges` on one of `lines`, and that line.
fn snap(edges: &[f64], lines: &[f64], reach: f64) -> Option<(f64, f64)> {
    edges.iter().flat_map(|e| lines.iter().map(move |l| (l - e, *l))).filter(|(d, _)| d.abs() <= reach).min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
}

/// The band about to be cut out: shaded, with dashed scissor lines at its edges.
fn draw_band(cr: &cairo::Context, view: &View, b: Band, (iw, ih): (f64, f64)) {
    let r = if b.horizontal { RectF { x: 0.0, y: b.from, w: iw, h: b.to - b.from } } else { RectF { x: b.from, y: 0.0, w: b.to - b.from, h: ih } };
    let (x, y) = view.to_widget(Point::new(r.x, r.y));
    let (w, h) = (r.w * view.scale, r.h * view.scale);
    cr.rectangle(x, y, w, h);
    cr.set_source_rgba(0.88, 0.11, 0.14, 0.25);
    let _ = cr.fill();
    cr.set_source_rgb(0.88, 0.11, 0.14);
    cr.set_line_width(1.5);
    cr.set_dash(&[6.0, 4.0], 0.0);
    if b.horizontal {
        for yy in [y, y + h] {
            cr.move_to(x, yy);
            cr.line_to(x + w, yy);
        }
    } else {
        for xx in [x, x + w] {
            cr.move_to(xx, y);
            cr.line_to(xx, y + h);
        }
    }
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    let label = format!("Release to cut out {} px", (b.to - b.from).round());
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(13.0);
    if let Ok(ext) = cr.text_extents(&label) {
        let (lx, ly) = (x + 8.0, y + h / 2.0 + 5.0);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.75);
        cr.rectangle(lx - 5.0, ly - 15.0, ext.width() + 10.0, 22.0);
        let _ = cr.fill();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(lx, ly);
        let _ = cr.show_text(&label);
    }
}

/// The picture as it is seen on the screenshot's area, made again when the marks change.
fn seen(st: &mut State) -> Rc<Bitmap> {
    if let Some((base, made_with, picture)) = &st.seen {
        if Rc::ptr_eq(base, &st.doc.base) && made_with.as_slice() == st.doc.scene.objects() {
            return picture.clone();
        }
    }
    let marks = st.doc.scene.objects().to_vec();
    // If drawing fails, nothing can be seen: black, never the bare screenshot.
    let picture = Rc::new(obtura_render::compose(&st.doc.base, &marks).unwrap_or_else(|_| Bitmap::filled(st.doc.base.width, st.doc.base.height, [0, 0, 0, 255])));
    st.seen = Some((st.doc.base.clone(), marks, picture.clone()));
    picture
}

/// An area being chosen (Copy Text): dashed outline, light fill and what releasing does.
fn draw_area(cr: &cairo::Context, view: &View, r: RectF, label: &str) {
    let (x, y) = view.to_widget(Point::new(r.x, r.y));
    let (w, h) = (r.w * view.scale, r.h * view.scale);
    cr.rectangle(x, y, w, h);
    cr.set_source_rgba(0.21, 0.52, 0.89, 0.15);
    let _ = cr.fill_preserve();
    cr.set_source_rgb(0.21, 0.52, 0.89);
    cr.set_line_width(1.5);
    cr.set_dash(&[6.0, 4.0], 0.0);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(13.0);
    if let Ok(ext) = cr.text_extents(label) {
        let (lx, ly) = (x.max(4.0), y + h + 22.0);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.75);
        cr.rectangle(lx - 5.0, ly - 15.0, ext.width() + 10.0, 22.0);
        let _ = cr.fill();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(lx, ly);
        let _ = cr.show_text(label);
    }
}

/// How many pixels across the colour picker's loupe shows, and how big each one is drawn.
const LOUPE_PIXELS: i32 = 11;
const LOUPE_CELL: f64 = 10.0;

/// The colour picker's loupe beside the pointer at `(wx, wy)`: the pixels around picture point
/// `p`, enlarged, the middle one outlined, and its colour as a hex code.
fn draw_loupe(cr: &cairo::Context, picture: &Bitmap, p: Point, (wx, wy): (f64, f64), (cw, ch): (f64, f64)) {
    let size = LOUPE_PIXELS as f64 * LOUPE_CELL;
    let (cx, cy) = (p.x.floor() as i32, p.y.floor() as i32);
    let inside = |x: i32, y: i32| x >= 0 && y >= 0 && (x as u32) < picture.width && (y as u32) < picture.height;
    // Below and to the right of the pointer, or on the other side near the edges.
    let lx = if wx + 24.0 + size > cw { wx - 24.0 - size } else { wx + 24.0 };
    let ly = if wy + 24.0 + size + 40.0 > ch { wy - 24.0 - size - 40.0 } else { wy + 24.0 };
    cr.rectangle(lx - 2.0, ly - 2.0, size + 4.0, size + 44.0);
    cr.set_source_rgba(0.1, 0.1, 0.1, 0.85);
    let _ = cr.fill();
    let half = LOUPE_PIXELS / 2;
    for j in 0..LOUPE_PIXELS {
        for i in 0..LOUPE_PIXELS {
            let (x, y) = (cx - half + i, cy - half + j);
            if inside(x, y) {
                let c = picture.pixel(x as u32, y as u32);
                cr.set_source_rgb(c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0);
            } else {
                cr.set_source_rgb(0.5, 0.5, 0.5);
            }
            cr.rectangle(lx + i as f64 * LOUPE_CELL, ly + j as f64 * LOUPE_CELL, LOUPE_CELL, LOUPE_CELL);
            let _ = cr.fill();
        }
    }
    let (mx, my) = (lx + half as f64 * LOUPE_CELL, ly + half as f64 * LOUPE_CELL);
    cr.rectangle(mx - 0.5, my - 0.5, LOUPE_CELL + 1.0, LOUPE_CELL + 1.0);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    cr.rectangle(mx - 1.5, my - 1.5, LOUPE_CELL + 3.0, LOUPE_CELL + 3.0);
    cr.set_source_rgb(0.0, 0.0, 0.0);
    let _ = cr.stroke();
    let (top, bottom) = if inside(cx, cy) {
        let c = picture.pixel(cx as u32, cy as u32);
        (format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]), format!("{cx}, {cy}"))
    } else {
        ("outside".to_string(), "the picture".to_string())
    };
    cr.select_font_face("Monospace", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(12.0);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.move_to(lx + 4.0, ly + size + 17.0);
    let _ = cr.show_text(&top);
    cr.set_source_rgb(0.75, 0.75, 0.75);
    cr.move_to(lx + 4.0, ly + size + 34.0);
    let _ = cr.show_text(&bottom);
}

/// The end of a drag from `start` adjusted so the rectangle has the ratio `(w, h)`.
fn with_ratio(start: Point, end: Point, (rw, rh): (u32, u32)) -> Point {
    let r = rw as f64 / rh.max(1) as f64;
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    let (w, h) = if dx.abs() / dy.abs().max(1e-9) > r { (dy.abs() * r, dy.abs()) } else { (dx.abs(), dx.abs() / r) };
    Point::new(start.x + w * dx.signum(), start.y + h * dy.signum())
}

fn selected_object(st: &State) -> Option<&Object> {
    st.selected.and_then(|id| st.doc.scene.get(id))
}

fn style_for(st: &State, tool: Tool) -> Style {
    st.styles.get(&tool).copied().unwrap_or_else(|| tool.default_style())
}

/// After a text mark's size or font changed, measure it again.
fn refresh_text_size(doc: &mut Document, id: Id, style: Style) {
    if let Some(Object { kind: Kind::Text { text, size, .. } | Kind::Callout { text, size, .. }, .. }) = doc.scene.get_mut(id) {
        let (w, h) = measure_text(text, &style);
        *size = Point::new(w, h);
    }
}

/// A zoom level near `wanted`: 100 % is a stop on the way, so stepping through it lands on it.
fn nice_zoom(wanted: f64, from: f64) -> f64 {
    if (from < 1.0 && wanted > 1.0) || (from > 1.0 && wanted < 1.0) {
        1.0
    } else {
        wanted
    }
}

/// The handle of the selected mark under the widget point, if any.
fn handle_at(st: &State, wx: f64, wy: f64) -> Option<(Id, usize)> {
    let o = selected_object(st)?;
    o.handles().iter().position(|h| {
        let (hx, hy) = st.view.to_widget(*h);
        (hx - wx).hypot(hy - wy) <= HANDLE_REACH
    }).map(|i| (o.id, i))
}

/// The selected mark, if the widget point is on its round turning handle.
fn rotate_handle_at(st: &State, wx: f64, wy: f64) -> Option<Object> {
    let o = selected_object(st)?;
    let r = o.rotate_handle(ROTATE_GAP / st.view.scale)?;
    let (hx, hy) = st.view.to_widget(r);
    ((hx - wx).hypot(hy - wy) <= HANDLE_REACH).then(|| o.clone())
}

/// A curve that starts from a straight drag: bent to one side by a fifth of its length.
fn bend(a: Point, b: Point) -> Point {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    Point::new((a.x + b.x) / 2.0 + dy * 0.2, (a.y + b.y) / 2.0 - dx * 0.2)
}

/// The smallest rectangle holding both.
fn union(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    let x1 = (a.x as i64 + a.w as i64).max(b.x as i64 + b.w as i64);
    let y1 = (a.y as i64 + a.h as i64).max(b.y as i64 + b.h as i64);
    Rect::new(x0, y0, (x1 - x0 as i64) as u32, (y1 - y0 as i64) as u32)
}

/// Draws the marks below the last redaction or magnifier into the cached underlay, on the room
/// they need. A spotlight among them dims the whole `canvas`, as on export.
fn ensure_underlay(st: &mut State, canvas: Rect) {
    let n = underlay_len(&st.doc.scene);
    // A text being typed again is left out: the typing field shows it.
    let editing = st.editing.as_ref().and_then(|e| e.id);
    let below: Vec<Object> = st.doc.scene.objects()[..n].iter().filter(|o| Some(o.id) != editing).cloned().collect();
    let (bw, bh) = (st.doc.base.width, st.doc.base.height);
    let rect = if below.iter().any(|o| matches!(o.kind, Kind::Spotlight { .. })) { canvas } else { canvas_of(&below, bw, bh) };
    if st.underlay.as_ref().is_some_and(|u| u.n == n && u.rect == rect && Rc::ptr_eq(&u.base, &st.doc.base) && u.marks == below) {
        return;
    }
    let bitmap = if below.is_empty() { Ok(st.doc.base.as_ref().clone()) } else { render_on(&st.doc.base, &below, rect) };
    let (base, marks) = (st.doc.base.clone(), below);
    st.underlay = match bitmap.and_then(|b| to_surface(&b)) {
        Ok(surface) => Some(Underlay { n, rect, base, marks, surface }),
        Err(e) => {
            obtura_core::log::error(&format!("cannot draw the picture: {e}"));
            None
        }
    };
}

fn shape_kind(tool: Tool, a: Point, b: Point, style: &Style) -> Kind {
    let rect = RectF::from_points(a, b);
    match tool {
        Tool::Line => Kind::Line { from: a, to: b, via: vec![] },
        Tool::Spline => Kind::Line { from: a, to: b, via: vec![bend(a, b)] },
        Tool::Callout => {
            let (w, h) = measure_text("Text", style);
            Kind::Callout { tip: a, pos: b, text: "Text".into(), size: Point::new(w, h) }
        }
        Tool::Rect => Kind::Rect { rect },
        Tool::Ellipse => Kind::Ellipse { rect },
        Tool::Spotlight => Kind::Spotlight { rect },
        Tool::Magnify => Kind::Magnify { source: rect, lens: rect },
        Tool::Blur => Kind::Redact { rect, mode: RedactMode::Blur },
        Tool::Pixelate => Kind::Redact { rect, mode: RedactMode::Pixelate },
        Tool::Measure => Kind::Measure { from: a, to: b },
        _ => Kind::Redact { rect, mode: RedactMode::Box },
    }
}

/// Shift: lines and arrows snap to 45 degree steps; boxes become squares.
fn constrain(tool: Tool, start: Point, end: Point, shift: bool) -> Point {
    if !shift {
        return end;
    }
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    match tool {
        Tool::Line | Tool::Spline | Tool::Measure => {
            let len = dx.hypot(dy);
            let step = std::f64::consts::FRAC_PI_4;
            let angle = (dy.atan2(dx) / step).round() * step;
            Point::new(start.x + len * angle.cos(), start.y + len * angle.sin())
        }
        Tool::Rect | Tool::Ellipse | Tool::Blur | Tool::Pixelate | Tool::Box | Tool::Crop | Tool::Spotlight | Tool::Magnify => {
            let side = dx.abs().max(dy.abs());
            Point::new(start.x + side * dx.signum(), start.y + side * dy.signum())
        }
        _ => end,
    }
}

/// Whether a finished drag made something worth adding.
fn worth_keeping(kind: &Kind) -> bool {
    match kind {
        Kind::Line { from, to, .. } | Kind::Measure { from, to } => from.distance(*to) >= MIN_DRAG,
        Kind::Rect { rect } | Kind::Ellipse { rect } | Kind::Redact { rect, .. } | Kind::Spotlight { rect } | Kind::Magnify { source: rect, .. } => rect.w >= MIN_DRAG && rect.h >= MIN_DRAG,
        Kind::Pen { points } | Kind::Highlighter { points } => !points.is_empty(),
        Kind::Text { .. } | Kind::Callout { .. } | Kind::Step { .. } | Kind::Stamp { .. } | Kind::Image { .. } => true,
    }
}

fn draw_crop(cr: &cairo::Context, view: &View, c: RectF, (iw, ih): (f64, f64)) {
    let (x, y) = view.to_widget(Point::new(c.x, c.y));
    let (w, h) = (c.w * view.scale, c.h * view.scale);
    let (px, py) = view.to_widget(Point::new(0.0, 0.0));
    cr.set_fill_rule(cairo::FillRule::EvenOdd);
    cr.rectangle(px, py, iw * view.scale, ih * view.scale);
    cr.rectangle(x, y, w, h);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.5);
    let _ = cr.fill();
    cr.rectangle(x + 0.5, y + 0.5, w, h);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    let label = format!("{} × {}   Enter to crop · Esc to cancel", c.w.round(), c.h.round());
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(13.0);
    if let Ok(ext) = cr.text_extents(&label) {
        let (lx, ly) = (x.max(4.0), (y + h + 22.0).min(py + ih * view.scale - 4.0).max(18.0));
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.75);
        cr.rectangle(lx - 5.0, ly - 15.0, ext.width() + 10.0, 22.0);
        let _ = cr.fill();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(lx, ly);
        let _ = cr.show_text(&label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use obtura_scene::quad_control;

    #[test]
    fn a_mark_reshaped_below_a_redaction_shows_at_once() {
        // The view keeps the marks below the last redaction as one cached picture. A mark in
        // there that is resized by a handle or turned must show its new shape at once.
        let mut st = State::new(Bitmap::filled(100, 80, [255, 255, 255, 255]));
        let filled = Style { fill: Some(obtura_scene::Color::BLACK), ..Tool::Rect.default_style() };
        let id = st.doc.scene.add(Kind::Rect { rect: RectF { x: 10.0, y: 10.0, w: 20.0, h: 20.0 } }, filled);
        st.doc.scene.add(Kind::Redact { rect: RectF { x: 70.0, y: 50.0, w: 20.0, h: 20.0 }, mode: RedactMode::Box }, Tool::Box.default_style());
        let canvas = Rect::new(0, 0, 100, 80);
        let dark_at = |st: &State, x: usize, y: usize| {
            let mut dark = false;
            st.underlay.as_ref().expect("drawn").surface.with_data(|d| dark = d[(y * 100 + x) * 4] < 128).expect("surface data");
            dark
        };
        ensure_underlay(&mut st, canvas);
        assert!(dark_at(&st, 20, 20) && !dark_at(&st, 50, 20));
        // As a handle drag does it: the mark changes, nothing else is told.
        st.doc.scene.get_mut(id).expect("mark").kind = Kind::Rect { rect: RectF { x: 40.0, y: 10.0, w: 20.0, h: 20.0 } };
        ensure_underlay(&mut st, canvas);
        assert!(!dark_at(&st, 20, 20) && dark_at(&st, 50, 20));
        // A mark being typed again (a text box) is left out while the typing field shows it.
        st.editing = Some(TextEditing { id: Some(id), at: Point::new(40.0, 10.0), style: filled, tip: None });
        ensure_underlay(&mut st, canvas);
        assert!(!dark_at(&st, 50, 20));
        st.editing = None;
        ensure_underlay(&mut st, canvas);
        assert!(dark_at(&st, 50, 20));
    }

    #[test]
    fn the_view_fits_a_big_picture_and_never_enlarges_a_small_one() {
        let v = View::fit(2000, 1000, 1032, 600);
        assert!((v.scale - 0.5).abs() < 1e-9);
        let small = View::fit(100, 50, 1000, 800);
        assert_eq!(small.scale, 1.0);
        assert_eq!((small.ox, small.oy), (450.0, 375.0));
    }

    #[test]
    fn a_zoomed_picture_scrolls_only_within_its_edges_and_a_small_one_stays_centred() {
        let content = RectF { x: 0.0, y: 0.0, w: 1000.0, h: 100.0 };
        // 2000 px wide at 2x in a 500 px canvas: scrolled far left, it stops at the right edge.
        let v = View::placed(content, 2.0, (-99999.0, 0.0), 500, 400, false);
        assert_eq!(v.ox, 500.0 - MARGIN - 2000.0);
        let v = View::placed(content, 2.0, (99999.0, 0.0), 500, 400, false);
        assert_eq!(v.ox, MARGIN);
        // 200 px tall fits in 400: centred whatever the scroll.
        assert_eq!(v.oy, 100.0);
    }

    #[test]
    fn a_crop_with_a_ratio_keeps_its_shape_whichever_way_it_is_dragged() {
        let o = Point::new(100.0, 100.0);
        let e = with_ratio(o, Point::new(260.0, 130.0), (16, 9));
        assert!(((e.x - o.x) / (e.y - o.y) - 16.0 / 9.0).abs() < 1e-9);
        let e = with_ratio(o, Point::new(40.0, 20.0), (1, 1));
        assert_eq!(e, Point::new(40.0, 40.0), "up and to the left");
    }

    #[test]
    fn a_mark_snaps_to_the_nearest_line_within_reach() {
        // Edges at 10, 20, 30; lines at 0, 50, 100: 30 is 20 away from 50, too far.
        assert_eq!(snap(&[10.0, 20.0, 30.0], &[0.0, 50.0, 100.0], 6.0), None);
        // Edges at 46, 56, 66: the left edge is 4 from 50, the centre 6 from 50: the left wins.
        assert_eq!(snap(&[46.0, 56.0, 66.0], &[0.0, 50.0, 100.0], 6.0), Some((4.0, 50.0)));
        assert_eq!(snap(&[97.0], &[0.0, 50.0, 100.0], 6.0), Some((3.0, 100.0)));
    }

    #[test]
    fn a_band_runs_the_way_the_drag_went_most() {
        let b = Band::between(Point::new(10.0, 50.0), Point::new(14.0, 20.0));
        assert!(b.horizontal && b.from == 20.0 && b.to == 50.0);
        let b = Band::between(Point::new(10.0, 50.0), Point::new(80.0, 55.0));
        assert!(!b.horizontal && b.from == 10.0 && b.to == 80.0);
    }

    #[test]
    fn zoom_steps_stop_at_one_hundred_percent() {
        assert_eq!(nice_zoom(1.1, 0.9), 1.0);
        assert_eq!(nice_zoom(0.85, 1.05), 1.0);
        assert_eq!(nice_zoom(2.5, 2.0), 2.5);
    }

    #[test]
    fn widget_and_picture_coordinates_convert_both_ways() {
        let v = View { scale: 0.5, ox: 20.0, oy: 10.0 };
        let p = v.to_image(120.0, 60.0);
        assert_eq!((p.x, p.y), (200.0, 100.0));
        assert_eq!(v.to_widget(p), (120.0, 60.0));
    }

    #[test]
    fn shift_snaps_lines_to_45_degrees_and_boxes_to_squares() {
        let o = Point::new(0.0, 0.0);
        let snapped = constrain(Tool::Line, o, Point::new(100.0, 8.0), true);
        assert!((snapped.y).abs() < 1e-6 && snapped.x > 99.0, "{snapped:?}");
        let diagonal = constrain(Tool::Spline, o, Point::new(50.0, 60.0), true);
        assert!((diagonal.x - diagonal.y).abs() < 1e-6);
        let square = constrain(Tool::Rect, o, Point::new(40.0, -10.0), true);
        assert_eq!((square.x, square.y), (40.0, -40.0));
        assert_eq!(constrain(Tool::Rect, o, Point::new(40.0, -10.0), false), Point::new(40.0, -10.0));
    }

    #[test]
    fn tiny_drags_make_nothing() {
        assert!(!worth_keeping(&Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(1.0, 1.0), via: vec![] }));
        assert!(worth_keeping(&Kind::Line { from: Point::new(0.0, 0.0), to: Point::new(30.0, 1.0), via: vec![] }));
        assert!(!worth_keeping(&Kind::Rect { rect: RectF { x: 0.0, y: 0.0, w: 50.0, h: 1.0 } }));
    }

    #[test]
    fn a_new_curve_bulges_to_the_left_of_where_it_is_heading_and_passes_through_the_via_point() {
        let (a, b) = (Point::new(0.0, 100.0), Point::new(100.0, 100.0));
        let via = bend(a, b);
        assert_eq!(via, Point::new(50.0, 80.0), "up when drawn to the right");
        assert_eq!(quad_control(a, via, b), Point::new(50.0, 60.0));
    }
}
