//! The editor window, one per picture. Modules install their actions and widgets into it.

use crate::{
    editor::{Change, Editor},
    modules,
};
use adw::prelude::*;
use obtura_core::settings::Settings;
use obtura_image::Bitmap;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

thread_local! {
    /// Open editor windows. Their actions only hold weak references, so this is what keeps
    /// a window's state alive until the window is destroyed.
    static OPEN: RefCell<Vec<Rc<Window>>> = const { RefCell::new(Vec::new()) };
}

pub struct Window {
    pub win: adw::ApplicationWindow,
    pub header: adw::HeaderBar,
    /// The column of tool buttons at the left of the picture.
    pub toolbar: gtk::Box,
    /// The free place beside the redaction tools in the tool column (Smart Redact goes there).
    pub redact_slot: gtk::Box,
    pub toasts: adw::ToastOverlay,
    /// The main menu (the button at the right of the header); modules add their sections.
    pub main_menu: gtk::gio::Menu,
    pub editor: Rc<Editor>,
    pub settings: Settings,
    /// True while the picture has not been saved since it was captured or changed.
    pub unsaved: Cell<bool>,
    /// True once marks were added or the picture was changed, until it is saved.
    pub modified: Cell<bool>,
    /// Set while a new screenshot is being taken to replace this one: if it succeeds this
    /// window closes (the user already chose what to do with the changes).
    pub replace: Cell<bool>,
    /// The clipboard holds this picture (as captured or as last copied), not something else.
    pub clip_ours: Cell<bool>,
    /// The picture changed since it was put on the clipboard.
    pub clip_dirty: Cell<bool>,
    /// The library item this picture is (its edits are kept there when it is saved).
    pub library_id: RefCell<Option<String>>,
}

impl Window {
    pub fn open(app: &adw::Application, image: Bitmap) -> Rc<Self> {
        let (iw, ih) = (image.width as i32, image.height as i32);
        let editor = Editor::new(image);
        let toolbar = gtk::Box::new(gtk::Orientation::Vertical, 2);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);
        toolbar.set_margin_start(6);
        toolbar.set_margin_end(2);
        toolbar.set_valign(gtk::Align::Start);

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&editor.widget));
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        // A short window scrolls the tool column rather than cutting it off.
        let tools = gtk::ScrolledWindow::builder().child(&toolbar).hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_width(true).build();
        body.append(&tools);
        body.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        body.append(&toasts);
        let header = adw::HeaderBar::new();
        // The middle of the header shows the size only: the buttons need the room.
        let size_label = gtk::Label::builder().label(format!("{iw} × {ih}")).css_classes(["dim-label"]).build();
        header.set_title_widget(Some(&size_label));
        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.set_content(Some(&body));
        let win = adw::ApplicationWindow::builder()
            .application(app)
            .title(title(iw, ih))
            .default_width(default_size(iw + 170, 874, 1725, 0))
            .default_height(default_size(ih + 120, 598, 1093, 1))
            .content(&content)
            .build();
        let redact_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let w = Rc::new(Self { win, header, toolbar, redact_slot, toasts, main_menu: gtk::gio::Menu::new(), editor, settings: Settings::load(), unsaved: Cell::new(true), modified: Cell::new(false), replace: Cell::new(false), clip_ours: Cell::new(false), clip_dirty: Cell::new(false), library_id: RefCell::new(None) });

        // The window itself reacts first to document changes.
        let wk = Rc::downgrade(&w);
        w.editor.connect_change(move |c| {
            if let (Some(w), Change::Document) = (wk.upgrade(), c) {
                w.unsaved.set(true);
                w.modified.set(true);
                w.clip_dirty.set(true);
                let (iw, ih) = w.editor.size();
                w.win.set_title(Some(&title(iw as i32, ih as i32)));
                show_size(&size_label, &w.editor);
            }
        });
        let size_label = w.header.title_widget().and_downcast::<gtk::Label>();
        // The user's saved tool styles and frame.
        let saved: Vec<_> = w.settings.tool_styles.iter().filter_map(|(id, s)| crate::tools::Tool::from_id(id).map(|t| (t, *s))).collect();
        w.editor.set_tool_styles(saved);
        if w.settings.auto_frame {
            w.editor.init_frame(w.settings.frame);
            if let Some(label) = size_label {
                show_size(&label, &w.editor);
            }
        }
        modules::install_all(&w);
        OPEN.with(|o| o.borrow_mut().push(w.clone()));
        let wk = Rc::downgrade(&w);
        w.win.connect_destroy(move |_| {
            if let Some(w) = wk.upgrade() {
                OPEN.with(|o| o.borrow_mut().retain(|x| !Rc::ptr_eq(x, &w)));
            }
        });
        w
    }

    pub fn present(&self) {
        self.win.present();
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// The finished picture: the base image with every mark applied.
    pub fn export(&self) -> Result<Bitmap, String> {
        self.editor.export()
    }
}

/// The open editor of a library item, if any.
pub fn find_item(id: &str) -> Option<Rc<Window>> {
    OPEN.with(|o| o.borrow().iter().find(|w| w.library_id.borrow().as_deref() == Some(id)).cloned())
}

fn find(win: &gtk::Window) -> Option<Rc<Window>> {
    OPEN.with(|o| o.borrow().iter().find(|w| w.win.upcast_ref::<gtk::Window>() == win).cloned())
}

/// Whether `win` is an editor that a new screenshot is about to replace.
pub fn is_replaced(win: &gtk::Window) -> bool {
    find(win).is_some_and(|w| w.replace.get())
}

/// The new screenshot did not happen: the window stays.
pub fn keep(win: &gtk::Window) {
    if let Some(w) = find(win) {
        w.replace.set(false);
    }
}

/// Closes an editor without asking: the user already decided what to do with its changes.
pub fn retire(win: &gtk::Window) {
    if let Some(w) = find(win) {
        w.unsaved.set(false);
    }
    win.close();
}

/// The size in the header: the screenshot's, and the saved picture's when that differs (marks
/// past the edge add room; a frame adds more).
fn show_size(label: &gtk::Label, editor: &Editor) {
    let (iw, ih) = editor.size();
    let (cw, ch) = editor.canvas_size();
    let grown = (cw, ch) != (iw, ih);
    let text = match editor.frame() {
        Some(f) => {
            let l = f.layout(cw, ch);
            format!("{iw} × {ih}  →  {} × {} framed", l.width, l.height)
        }
        None if grown => format!("{iw} × {ih}  →  {cw} × {ch}"),
        None => format!("{iw} × {ih}"),
    };
    label.set_label(&text);
    label.set_tooltip_text(grown.then_some("Marks reach past the edge of the screenshot: white room is added around it to hold them"));
}

/// A new editor's size along one side (0 across, 1 down): room for the picture and the tool
/// column, 15 % more than the picture strictly needs so the tools rarely have to scroll, at
/// least `min` and at most `max`, and never more than the smallest screen allows.
fn default_size(needed: i32, min: i32, max: i32, side: usize) -> i32 {
    let screen = crate::overlay::desktop().ok().and_then(|d| d.screens.iter().map(|m| if side == 0 { m.w } else { m.h }).reduce(f64::min));
    let max = match screen {
        // Leave room for GNOME's top bar and the window's own shadow.
        Some(s) => max.min((s * 0.94) as i32).max(min),
        None => max,
    };
    ((needed as f64 * 1.15) as i32).clamp(min, max)
}

fn title(w: i32, h: i32) -> String {
    format!("Obtura: {w} × {h}")
}
