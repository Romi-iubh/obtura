//! Feature modules. Each module installs its actions, shortcuts and widgets into the
//! editor window. New features are added as new `Module` implementations registered in
//! [`install_all`].

use crate::{
    editor::Change,
    format, icons,
    tools::{self, Tool},
    window::Window,
};
use adw::prelude::*;
use gtk::{gio, glib};
use crate::capture::now;
use obtura_core::{
    naming::{format_name, unique_path},
    settings::Settings,
};
use obtura_image::{encode_png, Bitmap};
use std::{path::{Path, PathBuf}, rc::Rc};

pub trait Module {
    fn install(&self, w: &Rc<Window>);
}

pub fn install_all(w: &Rc<Window>) {
    let modules: Vec<Box<dyn Module>> = vec![Box::new(EditModule), Box::new(FileModule), Box::new(InspectModule), Box::new(ZoomModule), Box::new(BeautifyModule), Box::new(ImageModule), Box::new(RecordModule)];
    for m in modules {
        m.install(w);
    }
}

/// Registers `win.<name>` running `f` with the window.
fn action(w: &Rc<Window>, name: &str, f: impl Fn(&Rc<Window>) + 'static) {
    let a = gio::SimpleAction::new(name, None);
    let wk = Rc::downgrade(w);
    a.connect_activate(move |_, _| {
        if let Some(w) = wk.upgrade() {
            f(&w);
        }
    });
    w.win.add_action(&a);
}

/// Tools, Formatting, undo and redo, and the keys that go with them.
struct EditModule;

impl Module for EditModule {
    fn install(&self, w: &Rc<Window>) {
        // Tool buttons in two columns, one group per block; the active one stays pressed.
        let grid = gtk::Grid::builder().row_spacing(2).column_spacing(2).build();
        let mut first: Option<gtk::ToggleButton> = None;
        let mut buttons: Vec<(Tool, gtk::ToggleButton)> = Vec::new();
        let (mut group, mut row, mut col) = (tools::ALL[0].group(), 0, 0);
        for tool in tools::ALL {
            if tool.group() != group {
                group = tool.group();
                row += if col > 0 { 1 } else { 0 };
                let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
                sep.set_margin_top(3);
                sep.set_margin_bottom(3);
                grid.attach(&sep, 0, row, 2, 1);
                (row, col) = (row + 1, 0);
            }
            let b = gtk::ToggleButton::builder().child(&icons::image(tool.icon())).tooltip_text(format!("{} ({})", tool.name(), tool.key())).css_classes(["flat"]).build();
            match &first {
                Some(f) => b.set_group(Some(f)),
                None => first = Some(b.clone()),
            }
            let wk = Rc::downgrade(w);
            b.connect_toggled(move |b| {
                if let (Some(w), true) = (wk.upgrade(), b.is_active()) {
                    w.editor.set_tool(tool);
                }
            });
            grid.attach(&b, col, row, 1, 1);
            (row, col) = if col == 1 { (row + 1, 0) } else { (row, 1) };
            buttons.push((tool, b));
            if tool == Tool::Box && col == 1 {
                // Smart Redact sits with the redaction tools.
                grid.attach(&w.redact_slot, col, row, 1, 1);
                (row, col) = (row + 1, 0);
            }
        }
        w.toolbar.append(&grid);
        let show_tool = {
            let editor = w.editor.clone();
            move || {
                let current = editor.tool();
                for (t, b) in &buttons {
                    b.set_active(*t == current);
                }
            }
        };
        show_tool();
        w.editor.connect_change(move |c| {
            if c == Change::Tool {
                show_tool();
            }
        });

        let fmt = format::build(&w.editor);
        w.header.pack_start(&fmt.button);

        let undo = icons::button("undo", "Undo (Ctrl+Z)");
        undo.set_action_name(Some("win.undo"));
        let redo = icons::button("redo", "Redo (Ctrl+Shift+Z)");
        redo.set_action_name(Some("win.redo"));
        w.header.pack_start(&undo);
        w.header.pack_start(&redo);
        let history = Rc::new({
            let editor = w.editor.clone();
            move || {
                undo.set_sensitive(editor.can_undo());
                redo.set_sensitive(editor.can_redo());
            }
        });
        let on_change = history.clone();
        w.editor.connect_change(move |c| {
            if c == Change::Document {
                on_change();
            }
        });
        action(w, "undo", |w| w.editor.undo());
        action(w, "redo", |w| w.editor.redo());
        // After the actions exist: adding an action makes its button sensitive again.
        history();
        action(w, "delete", |w| w.editor.delete_selected());
        if let Some(app) = w.win.application() {
            app.set_accels_for_action("win.undo", &["<Ctrl>Z"]);
            app.set_accels_for_action("win.redo", &["<Ctrl><Shift>Z", "<Ctrl>Y"]);
        }

        // Plain keys: tool letters, Delete, Escape, Enter (crop), F (Formatting). A text
        // field being typed in gets its keys first, so letters never switch tools there.
        let keys = gtk::EventControllerKey::new();
        let wk = Rc::downgrade(w);
        let format_button = fmt.button.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(w) = wk.upgrade() else { return glib::Propagation::Proceed };
            // Ctrl+V here, not as a shortcut, so a text being typed gets its own paste.
            if mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) && matches!(key, gtk::gdk::Key::v | gtk::gdk::Key::V) {
                let _ = WidgetExt::activate_action(&w.win, "win.paste", None);
                return glib::Propagation::Stop;
            }
            if mods.intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK) {
                return glib::Propagation::Proceed;
            }
            use gtk::gdk::Key;
            let handled = match key {
                Key::Escape => w.editor.escape(),
                Key::Delete | Key::BackSpace => {
                    w.editor.delete_selected();
                    true
                }
                Key::Return | Key::KP_Enter if w.editor.has_crop() => {
                    w.editor.apply_crop();
                    true
                }
                _ => match key.to_unicode() {
                    Some(c) if c.eq_ignore_ascii_case(&'f') => {
                        format_button.popup();
                        true
                    }
                    Some('B') if mods.contains(gtk::gdk::ModifierType::SHIFT_MASK) => {
                        let _ = WidgetExt::activate_action(&w.win, "win.beautify", None);
                        true
                    }
                    Some(c) => match Tool::from_key(c) {
                        Some(t) => {
                            w.editor.set_tool(t);
                            true
                        }
                        None => false,
                    },
                    None => false,
                },
            };
            if handled {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        w.win.add_controller(keys);
    }
}

/// The Image menu: turn, flip, resize, add space, trim borders, and combine with other
/// pictures (attach beside or below, place on top, paste).
struct ImageModule;

impl Module for ImageModule {
    fn install(&self, w: &Rc<Window>) {
        action(w, "rotate-left", |w| {
            w.editor.edit_picture(|d| {
                d.rotate(false);
                true
            });
        });
        action(w, "rotate-right", |w| {
            w.editor.edit_picture(|d| {
                d.rotate(true);
                true
            });
        });
        action(w, "flip-horizontal", |w| {
            w.editor.edit_picture(|d| {
                d.flip(true);
                true
            });
        });
        action(w, "flip-vertical", |w| {
            w.editor.edit_picture(|d| {
                d.flip(false);
                true
            });
        });
        action(w, "trim", |w| {
            let done = w.editor.edit_picture(|d| match obtura_image::trim_bounds(&d.base, 10) {
                Some(r) => d.crop(r),
                None => false,
            });
            if !done {
                w.toast("There is no plain border to trim");
            }
        });
        action(w, "resize", resize_dialog);
        action(w, "extend", extend_dialog);
        action(w, "attach-right", |w| pick_picture(w, |w, b| attach(w, &b, true)));
        action(w, "attach-below", |w| pick_picture(w, |w, b| attach(w, &b, false)));
        action(w, "insert-picture", |w| pick_picture(w, |w, b| w.editor.insert_picture(b)));
        action(w, "combine", combine_dialog);
        action(w, "paste", paste);

        let menu = gio::Menu::new();
        let turn = gio::Menu::new();
        turn.append(Some("Rotate Left"), Some("win.rotate-left"));
        turn.append(Some("Rotate Right"), Some("win.rotate-right"));
        turn.append(Some("Flip Left to Right"), Some("win.flip-horizontal"));
        turn.append(Some("Flip Upside Down"), Some("win.flip-vertical"));
        menu.append_section(None, &turn);
        let size = gio::Menu::new();
        size.append(Some("Resize…"), Some("win.resize"));
        size.append(Some("Add Space Around…"), Some("win.extend"));
        size.append(Some("Trim Plain Borders"), Some("win.trim"));
        menu.append_section(None, &size);
        let combine = gio::Menu::new();
        combine.append(Some("Combine with Several Pictures…"), Some("win.combine"));
        combine.append(Some("Attach a Picture on the Right…"), Some("win.attach-right"));
        combine.append(Some("Attach a Picture Below…"), Some("win.attach-below"));
        combine.append(Some("Place a Picture on Top…"), Some("win.insert-picture"));
        combine.append(Some("Paste a Picture or Text"), Some("win.paste"));
        menu.append_section(None, &combine);
        let face = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        face.append(&icons::image("image"));
        face.append(&gtk::Label::new(Some("Image")));
        let button = gtk::MenuButton::builder().child(&face).menu_model(&menu).tooltip_text("Turn, resize, add space, combine pictures").build();
        w.header.pack_end(&button);
        if let Some(app) = w.win.application() {
            app.set_accels_for_action("win.rotate-left", &["<Ctrl>bracketleft"]);
            app.set_accels_for_action("win.rotate-right", &["<Ctrl>bracketright"]);
            app.set_accels_for_action("win.resize", &["<Ctrl><Alt>R"]);
        }
    }
}

/// Reads a picture file of any kind GTK can open (PNG, JPEG, WebP...).
fn load_picture(file: &gio::File) -> Result<Bitmap, String> {
    let texture = gtk::gdk::Texture::from_file(file).map_err(|e| e.to_string())?;
    Ok(bitmap_of(&texture))
}

/// The pixels of a texture as a picture.
fn bitmap_of(texture: &gtk::gdk::Texture) -> Bitmap {
    let mut down = gtk::gdk::TextureDownloader::new(texture);
    down.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = down.download_bytes();
    let (w, h) = (texture.width() as usize, texture.height() as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        rgba.extend_from_slice(&bytes[y * stride..y * stride + w * 4]);
    }
    Bitmap { width: w as u32, height: h as u32, rgba }
}

/// The file dialog for pictures (PNG, JPEG, WebP... whatever GTK opens), starting in `dir`.
fn picture_dialog(title: &str, dir: &Path, multiple: bool) -> gtk::FileDialog {
    let dialog = gtk::FileDialog::builder().title(title).accept_label("Choose").modal(true).build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Pictures"));
    filter.add_pixbuf_formats();
    filter.add_mime_type("image/*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
    if multiple {
        dialog.set_title(&format!("{title} (several: Ctrl+click)"));
    }
    dialog
}

/// Combine: several pictures chosen at once, put together with this one (first) in a row, a
/// column or a grid, with a gap, an alignment and a background colour.
fn combine_dialog(w: &Rc<Window>) {
    let dialog = picture_dialog("Choose the Pictures to Combine", &Settings::load().save_dir(), true);
    let wk = Rc::downgrade(w);
    dialog.open_multiple(Some(&w.win), gio::Cancellable::NONE, move |result| {
        let (Some(w), Ok(files)) = (wk.upgrade(), result) else { return };
        let mut pictures = Vec::new();
        for f in files.iter::<gio::File>().flatten() {
            match load_picture(&f) {
                Ok(b) => pictures.push(b),
                Err(e) => return w.toast(&format!("Cannot open {}: {e}", f.basename().map(|b| b.display().to_string()).unwrap_or_default())),
            }
        }
        if !pictures.is_empty() {
            combine_options(&w, pictures);
        }
    });
}

fn combine_options(w: &Rc<Window>, pictures: Vec<Bitmap>) {
    let n = pictures.len() + 1;
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let ways = gtk::DropDown::from_strings(&["In a row", "In a column", "In a grid"]);
    ways.set_selected(if n >= 4 { 2 } else { 0 });
    let way_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    way_row.append(&gtk::Label::builder().label("Arrange").hexpand(true).halign(gtk::Align::Start).build());
    way_row.append(&ways);
    body.append(&way_row);
    let (cols_row, cols) = spin_row("Pictures per row", (n as f64).sqrt().ceil(), n as f64);
    cols.set_range(1.0, n as f64);
    body.append(&cols_row);
    {
        let cols_row = cols_row.clone();
        let show = move |d: &gtk::DropDown| cols_row.set_visible(d.selected() == 2);
        show(&ways);
        ways.connect_selected_notify(show);
    }
    let (gap_row, gap) = spin_row("Space between (pixels)", 16.0, 500.0);
    body.append(&gap_row);
    let (margin_row, margin) = spin_row("Space around (pixels)", 0.0, 500.0);
    body.append(&margin_row);
    let aligns = gtk::DropDown::from_strings(&["Start (left or top)", "Centre", "End (right or bottom)"]);
    aligns.set_selected(1);
    let align_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    align_row.append(&gtk::Label::builder().label("Line up").hexpand(true).halign(gtk::Align::Start).build());
    align_row.append(&aligns);
    body.append(&align_row);
    let colour_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    colour_row.append(&gtk::Label::builder().label("Background").hexpand(true).halign(gtk::Align::Start).build());
    let cd = gtk::ColorDialog::new();
    cd.set_with_alpha(true);
    let colour = gtk::ColorDialogButton::new(Some(cd));
    colour.set_rgba(&gtk::gdk::RGBA::WHITE);
    colour_row.append(&colour);
    body.append(&colour_row);
    let dialog = adw::AlertDialog::new(Some(&format!("Combine {n} Pictures")), Some("This picture comes first; the others follow in the order chosen. The marks stay on this picture."));
    dialog.set_extra_child(Some(&body));
    dialog.add_responses(&[("cancel", "Cancel"), ("combine", "Combine")]);
    dialog.set_response_appearance("combine", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("combine"));
    dialog.set_close_response("cancel");
    let wk = Rc::downgrade(w);
    dialog.connect_response(Some("combine"), move |_, _| {
        let Some(w) = wk.upgrade() else { return };
        let arrange = match ways.selected() {
            0 => obtura_image::Arrange::Row,
            1 => obtura_image::Arrange::Column,
            _ => obtura_image::Arrange::Grid(cols.value() as u32),
        };
        let align = [obtura_image::Align::Start, obtura_image::Align::Center, obtura_image::Align::End][(aligns.selected() as usize).min(2)];
        let c = colour.rgba();
        let to8 = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u8;
        let fill = [to8(c.red()), to8(c.green()), to8(c.blue()), to8(c.alpha())];
        let (gap, margin) = (gap.value() as u32, margin.value() as u32);
        w.editor.edit_picture(|d| {
            d.combine(&pictures, arrange, gap, margin, align, fill);
            true
        });
    });
    dialog.present(Some(&w.win));
}

/// Asks for a picture file, then hands it to `then`.
fn pick_picture(w: &Rc<Window>, then: impl Fn(&Rc<Window>, Bitmap) + 'static) {
    let dialog = picture_dialog("Choose a Picture", &Settings::load().save_dir(), false);
    let wk = Rc::downgrade(w);
    dialog.open(Some(&w.win), gio::Cancellable::NONE, move |result| {
        let (Some(w), Ok(file)) = (wk.upgrade(), result) else { return };
        match load_picture(&file) {
            Ok(b) => then(&w, b),
            Err(e) => w.toast(&format!("Cannot open that picture: {e}")),
        }
    });
}

/// Asks for a picture file (from any window), then hands its pixels to `then`. A file that
/// cannot be read is logged and ignored.
pub fn choose_picture_file(parent: Option<&gtk::Window>, then: impl Fn(Bitmap) + 'static) {
    let dialog = picture_dialog("Choose a Picture", &obtura_core::paths::pictures_dir(), false);
    dialog.open(parent, gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        match load_picture(&file) {
            Ok(b) => then(b),
            Err(e) => obtura_core::log::warn(&format!("cannot open that picture: {e}")),
        }
    });
}

fn attach(w: &Rc<Window>, other: &Bitmap, beside: bool) {
    w.editor.edit_picture(|d| {
        d.attach(other, beside, [255, 255, 255, 255]);
        true
    });
}

/// Ctrl+V: a picture on the clipboard is placed on top of the screenshot; text becomes a text
/// box (a note pasted from anywhere).
fn paste(w: &Rc<Window>) {
    let clipboard = w.win.clipboard();
    let has_picture = clipboard.formats().contains_type(gtk::gdk::Texture::static_type());
    let wk = Rc::downgrade(w);
    if has_picture {
        clipboard.read_texture_async(gio::Cancellable::NONE, move |result| {
            let Some(w) = wk.upgrade() else { return };
            match result {
                Ok(Some(texture)) => w.editor.insert_picture(bitmap_of(&texture)),
                _ => w.toast("The picture on the clipboard cannot be read"),
            }
        });
        return;
    }
    clipboard.read_text_async(gio::Cancellable::NONE, move |result| {
        let Some(w) = wk.upgrade() else { return };
        match result {
            Ok(Some(text)) if !text.trim().is_empty() => w.editor.paste_text(&text),
            _ => w.toast("There is no picture or text on the clipboard"),
        }
    });
}

/// A number field with a label, in a row.
fn spin_row(label: &str, value: f64, max: f64) -> (gtk::Box, gtk::SpinButton) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.append(&gtk::Label::builder().label(label).hexpand(true).halign(gtk::Align::Start).build());
    let spin = gtk::SpinButton::with_range(0.0, max, 1.0);
    spin.set_value(value);
    row.append(&spin);
    (row, spin)
}

/// Resize: width and height, keeping the proportions, or a percentage.
fn resize_dialog(w: &Rc<Window>) {
    let pic = w.editor.picture();
    let (pw, ph) = (pic.width as f64, pic.height as f64);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let (wrow, wspin) = spin_row("Width (pixels)", pw, 20000.0);
    let (hrow, hspin) = spin_row("Height (pixels)", ph, 20000.0);
    wspin.set_range(1.0, 20000.0);
    hspin.set_range(1.0, 20000.0);
    body.append(&wrow);
    body.append(&hrow);
    let quick = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    quick.add_css_class("linked");
    quick.set_halign(gtk::Align::Center);
    for pct in [25.0, 50.0, 75.0, 150.0, 200.0] {
        let b = gtk::Button::with_label(&format!("{pct:.0} %"));
        let wspin = wspin.clone();
        b.connect_clicked(move |_| wspin.set_value((pw * pct / 100.0).round()));
        quick.append(&b);
    }
    body.append(&quick);
    // The two numbers keep the picture's proportions.
    let busy = Rc::new(std::cell::Cell::new(false));
    {
        let (hspin, busy) = (hspin.clone(), busy.clone());
        wspin.connect_value_changed(move |s| {
            if !busy.replace(true) {
                hspin.set_value((s.value() * ph / pw).round().max(1.0));
                busy.set(false);
            }
        });
    }
    {
        let (wspin, busy) = (wspin.clone(), busy.clone());
        hspin.connect_value_changed(move |s| {
            if !busy.replace(true) {
                wspin.set_value((s.value() * pw / ph).round().max(1.0));
                busy.set(false);
            }
        });
    }
    let dialog = adw::AlertDialog::new(Some("Resize the Picture"), Some("The marks are resized with it."));
    dialog.set_extra_child(Some(&body));
    dialog.add_responses(&[("cancel", "Cancel"), ("resize", "Resize")]);
    dialog.set_response_appearance("resize", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("resize"));
    dialog.set_close_response("cancel");
    let wk = Rc::downgrade(w);
    dialog.connect_response(Some("resize"), move |_, _| {
        if let Some(w) = wk.upgrade() {
            let new_w = wspin.value() as u32;
            w.editor.edit_picture(|d| d.resize(new_w));
        }
    });
    dialog.present(Some(&w.win));
}

/// Add space around: four sides and a colour.
fn extend_dialog(w: &Rc<Window>) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let mut spins = Vec::new();
    for side in ["Top", "Right", "Bottom", "Left"] {
        let (row, spin) = spin_row(&format!("{side} (pixels)"), 40.0, 5000.0);
        body.append(&row);
        spins.push(spin);
    }
    let colour_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    colour_row.append(&gtk::Label::builder().label("Colour").hexpand(true).halign(gtk::Align::Start).build());
    let cd = gtk::ColorDialog::new();
    cd.set_with_alpha(true);
    let colour = gtk::ColorDialogButton::new(Some(cd));
    colour.set_rgba(&gtk::gdk::RGBA::WHITE);
    colour_row.append(&colour);
    body.append(&colour_row);
    let dialog = adw::AlertDialog::new(Some("Add Space Around"), Some("Room for notes and arrows beside the picture."));
    dialog.set_extra_child(Some(&body));
    dialog.add_responses(&[("cancel", "Cancel"), ("add", "Add Space")]);
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");
    let wk = Rc::downgrade(w);
    dialog.connect_response(Some("add"), move |_, _| {
        let Some(w) = wk.upgrade() else { return };
        let v: Vec<u32> = spins.iter().map(|s| s.value() as u32).collect();
        let c = colour.rgba();
        let to8 = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u8;
        let fill = [to8(c.red()), to8(c.green()), to8(c.blue()), to8(c.alpha())];
        w.editor.edit_picture(|d| d.extend(v[3], v[0], v[1], v[2], fill));
    });
    dialog.present(Some(&w.win));
}

/// Reading the picture: Copy Text (tool G and Ctrl+Shift+C for all of it), the colour
/// picker's results, and Smart Redact (beside the redaction tools, Ctrl+Shift+X).
struct InspectModule;

impl Module for InspectModule {
    fn install(&self, w: &Rc<Window>) {
        crate::inspect::install(w);
        let smart = crate::inspect::smart_redact_button(w);
        w.redact_slot.append(&smart);
        action(w, "smart-redact", move |_| smart.popup());
        action(w, "copy-text", |w| {
            let (pw, ph) = w.editor.size();
            crate::inspect::grab_text(w, obtura_image::Rect::new(0, 0, pw, ph));
        });
        let section = gio::Menu::new();
        action(w, "read-qr", crate::inspect::grab_qr);
        section.append(Some("Copy Text in the Picture"), Some("win.copy-text"));
        section.append(Some("Read QR Codes"), Some("win.read-qr"));
        section.append(Some("Smart Redact…"), Some("win.smart-redact"));
        w.main_menu.append_section(None, &section);
        if let Some(app) = w.win.application() {
            app.set_accels_for_action("win.copy-text", &["<Ctrl><Shift>C"]);
            app.set_accels_for_action("win.smart-redact", &["<Ctrl><Shift>X"]);
            app.set_accels_for_action("win.read-qr", &["<Ctrl><Shift>Q"]);
        }
    }
}

/// Recording the screen: the Record window, from the main menu (Ctrl+Shift+E).
struct RecordModule;

impl Module for RecordModule {
    fn install(&self, w: &Rc<Window>) {
        let section = gio::Menu::new();
        section.append(Some("Record the Screen…"), Some("app.record"));
        w.main_menu.append_section(None, &section);
    }
}

/// Beautify: the frame around the picture (Shift+B opens it).
struct BeautifyModule;

impl Module for BeautifyModule {
    fn install(&self, w: &Rc<Window>) {
        let button = crate::beautify::build(&w.editor);
        w.header.pack_end(&button);
        let b = button.clone();
        action(w, "beautify", move |_| b.popup());
    }
}

/// Zoom in and out, fit, actual size: keys and a menu button showing the zoom level.
struct ZoomModule;

impl Module for ZoomModule {
    fn install(&self, w: &Rc<Window>) {
        action(w, "zoom-in", |w| w.editor.zoom_in());
        action(w, "zoom-out", |w| w.editor.zoom_out());
        action(w, "zoom-fit", |w| w.editor.zoom_fit());
        action(w, "zoom-100", |w| w.editor.zoom_to(1.0, None));
        if let Some(app) = w.win.application() {
            app.set_accels_for_action("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"]);
            app.set_accels_for_action("win.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]);
            app.set_accels_for_action("win.zoom-fit", &["<Ctrl>0", "<Ctrl>KP_0"]);
            app.set_accels_for_action("win.zoom-100", &["<Ctrl>1", "<Ctrl>KP_1"]);
        }
        let menu = gio::Menu::new();
        menu.append(Some("Zoom In"), Some("win.zoom-in"));
        menu.append(Some("Zoom Out"), Some("win.zoom-out"));
        menu.append(Some("Fit to Window"), Some("win.zoom-fit"));
        menu.append(Some("Actual Size (100 %)"), Some("win.zoom-100"));
        let button = gtk::MenuButton::builder().label("Fit").menu_model(&menu).tooltip_text("Zoom (Ctrl+wheel, Ctrl+0 fits, Ctrl+1 is 100 %)").build();
        button.add_css_class("flat");
        w.header.pack_end(&button);
        let editor = w.editor.clone();
        w.editor.connect_change(move |c| {
            if c == Change::View {
                let pct = (editor.scale() * 100.0).round();
                button.set_label(&if editor.is_fit() { format!("Fit ({pct:.0} %)") } else { format!("{pct:.0} %") });
            }
        });
    }
}

/// Copy, Save, Save As, and the question asked when closing an unsaved picture.
struct FileModule;

impl Module for FileModule {
    fn install(&self, w: &Rc<Window>) {
        action(w, "copy", copy);
        action(w, "save", |w| save_default(w, |_| {}));
        action(w, "save-as", save_as);
        action(w, "close", |w| w.win.close());

        install_new_screenshot(w);
        let save = icons::button("save", "Save (Ctrl+S)");
        save.set_action_name(Some("win.save"));
        let copy_button = icons::button("copy", "Copy to clipboard (Ctrl+C)");
        copy_button.set_action_name(Some("win.copy"));
        w.header.pack_start(&copy_button);
        w.header.pack_start(&save);
        action(w, "pin", |w| match (w.export(), w.win.application().and_downcast::<adw::Application>()) {
            (Ok(picture), Some(app)) => {
                crate::pin::open(&app, &picture);
            }
            (Err(e), _) => w.toast(&format!("Cannot pin: {e}")),
            _ => {}
        });
        let file = gio::Menu::new();
        file.append(Some("Save As…"), Some("win.save-as"));
        file.append(Some("Pin to Screen"), Some("win.pin"));
        file.append(Some("Library"), Some("app.library"));
        w.main_menu.append_section(None, &file);
        let app_items = crate::help::menu_section();
        // Last, after the sections other modules add.
        let wk = Rc::downgrade(w);
        glib::idle_add_local_once(move || {
            if let Some(w) = wk.upgrade() {
                w.main_menu.append_section(None, &app_items);
            }
        });
        let menu_button = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&w.main_menu).tooltip_text("Main menu").build();
        w.header.pack_end(&menu_button);

        if let Some(app) = w.win.application() {
            app.set_accels_for_action("win.copy", &["<Ctrl>C"]);
            app.set_accels_for_action("win.close", &["<Ctrl>W"]);
            app.set_accels_for_action("win.save", &["<Ctrl>S"]);
            app.set_accels_for_action("win.save-as", &["<Ctrl><Shift>S"]);
            app.set_accels_for_action("win.pin", &["<Ctrl><Shift>P"]);
        }

        keep_clipboard_current(w);

        // Nothing is ever lost: ask before closing a picture that was not saved.
        let wk = Rc::downgrade(w);
        w.win.connect_close_request(move |win| {
            let Some(w) = wk.upgrade() else { return glib::Propagation::Proceed };
            if !w.unsaved.get() {
                return glib::Propagation::Proceed;
            }
            let dialog = adw::AlertDialog::new(Some("Save this picture?"), Some("It has not been saved. If you discard it, it is gone."));
            dialog.add_responses(&[("cancel", "Cancel"), ("discard", "Discard"), ("save", "Save")]);
            dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("save"));
            dialog.set_close_response("cancel");
            let (wk2, win2) = (Rc::downgrade(&w), win.clone());
            dialog.connect_response(None, move |_, response| match response {
                "discard" => {
                    if let Some(w) = wk2.upgrade() {
                        w.unsaved.set(false);
                    }
                    win2.close();
                }
                "save" => {
                    if let Some(w) = wk2.upgrade() {
                        let win3 = win2.clone();
                        save_default(&w, move |saved| {
                            if saved {
                                win3.close();
                            }
                        });
                    }
                }
                _ => {}
            });
            dialog.present(Some(win));
            glib::Propagation::Stop
        });
    }
}

pub fn texture_of(image: &Bitmap) -> gtk::gdk::MemoryTexture {
    gtk::gdk::MemoryTexture::new(image.width as i32, image.height as i32, gtk::gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from(&image.rgba[..]), image.width as usize * 4)
}

fn copy(w: &Rc<Window>) {
    match put_on_clipboard(w) {
        Ok(()) => w.toast("Copied to the clipboard"),
        Err(e) => w.toast(&format!("Cannot copy: {e}")),
    }
}

fn put_on_clipboard(w: &Rc<Window>) -> Result<(), String> {
    let image = w.export()?;
    w.win.clipboard().set_texture(&texture_of(&image));
    w.clip_ours.set(true);
    w.clip_dirty.set(false);
    Ok(())
}

/// The capture is put on the clipboard the moment it is taken (see `capture::deliver`). This
/// keeps it right afterwards:
/// * the first time the window gets the focus the picture is put there again, because a
///   compositor may refuse a clipboard change from a program that never had the focus;
/// * if the picture is annotated and the clipboard still holds it, the annotated version
///   replaces it when the user leaves for another program to paste it. If something else was
///   copied meanwhile, the clipboard is left alone.
fn keep_clipboard_current(w: &Rc<Window>) {
    w.clip_ours.set(w.settings.copy_after_capture);
    let clipboard = w.win.clipboard();
    let wk = Rc::downgrade(w);
    clipboard.connect_changed(move |c| {
        if let (Some(w), false) = (wk.upgrade(), c.is_local()) {
            w.clip_ours.set(false);
        }
    });
    let wk = Rc::downgrade(w);
    let first = std::cell::Cell::new(true);
    w.win.connect_is_active_notify(move |win| {
        let Some(w) = wk.upgrade() else { return };
        if !w.clip_ours.get() {
            return;
        }
        let refresh = if win.is_active() { first.replace(false) } else { w.clip_dirty.get() };
        if refresh {
            let _ = put_on_clipboard(&w);
        }
    });
}

/// What the New button can take: (the name kept in the settings, what the chooser beside it
/// says, the app action, its delay in seconds).
pub const NEW_KINDS: [(&str, &str, &str, Option<i32>); 9] = [
    ("region", "Region", "capture-region", None),
    ("last-region", "Same Region", "capture-last-region", None),
    ("screen", "Full Screen", "capture-screen", None),
    ("window", "A Window", "capture-window", None),
    ("scrolling", "Scrolling", "capture-scrolling", None),
    ("region-3", "Region in 3 s", "capture-region-in", Some(3)),
    ("region-5", "Region in 5 s", "capture-region-in", Some(5)),
    ("region-10", "Region in 10 s", "capture-region-in", Some(10)),
    ("screen-5", "Full Screen in 5 s", "capture-screen-in", Some(5)),
];

/// Where `kind` is in [`NEW_KINDS`] (a region for anything unknown).
fn new_kind_index(kind: &str) -> usize {
    NEW_KINDS.iter().position(|k| k.0 == kind).unwrap_or(0)
}

/// The New button and, beside it, the chooser of what it takes (it shows the choice). New
/// takes another screenshot of that kind and replaces this window with it; if this picture
/// was changed and not saved, it asks first. The choice is remembered: the kind of the last
/// capture that opened an editor, or what was chosen here.
fn install_new_screenshot(w: &Rc<Window>) {
    let new = gtk::Button::builder()
        .child(&{
            let face = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            face.append(&icons::image("new"));
            face.append(&gtk::Label::new(Some("New")));
            face
        })
        .tooltip_text("Take a new screenshot of the kind chosen beside it, in place of this one (Ctrl+N)")
        .build();
    new.set_action_name(Some("win.new"));
    let labels: Vec<&str> = NEW_KINDS.iter().map(|k| k.1).collect();
    let chooser = gtk::DropDown::from_strings(&labels);
    chooser.set_tooltip_text(Some("What New takes: a region, the same region again, the full screen, a window, a scrolling capture, or after a delay"));
    let pair = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    pair.add_css_class("linked");
    pair.append(&new);
    pair.append(&chooser);
    w.header.pack_start(&pair);

    // The choice is the window action `new-kind` (its state), shown by the chooser.
    let current = NEW_KINDS[new_kind_index(&Settings::load().new_capture)].0;
    let kind = gio::SimpleAction::new_stateful("new-kind", Some(glib::VariantTy::STRING), &current.to_variant());
    {
        let chooser = chooser.clone();
        kind.connect_change_state(move |a, v| {
            let Some(id) = v.and_then(|v| v.get::<String>()).filter(|id| NEW_KINDS.iter().any(|k| k.0 == id)) else { return };
            a.set_state(&id.to_variant());
            let i = new_kind_index(&id) as u32;
            if chooser.selected() != i {
                chooser.set_selected(i);
            }
            let mut settings = Settings::load();
            if settings.new_capture != id {
                settings.new_capture = id;
                let _ = settings.save();
            }
        });
    }
    kind.connect_activate(|a, p| {
        if let Some(p) = p {
            a.change_state(p);
        }
    });
    chooser.set_selected(new_kind_index(current) as u32);
    {
        let kind = kind.clone();
        chooser.connect_selected_notify(move |c| {
            if let Some(k) = NEW_KINDS.get(c.selected() as usize) {
                kind.change_state(&k.0.to_variant());
            }
        });
    }
    w.win.add_action(&kind);
    // Coming forward, the window shows what was chosen meanwhile in another one.
    {
        let kind = kind.clone();
        w.win.connect_is_active_notify(move |win| {
            let id = Settings::load().new_capture;
            if win.is_active() && kind.state().and_then(|s| s.get::<String>()).as_deref() != Some(id.as_str()) {
                kind.change_state(&id.to_variant());
            }
        });
    }
    action(w, "new", move |w| {
        let id = kind.state().and_then(|s| s.get::<String>()).unwrap_or_default();
        let (_, _, app_action, delay) = NEW_KINDS[new_kind_index(&id)];
        new_screenshot_with(w, app_action, delay.map(|d| d.to_variant()));
    });
    // Each kind as its own action too (Same Region Again has its key).
    action(w, "new-region", |w| new_screenshot(w, "capture-region"));
    action(w, "new-screen", |w| new_screenshot(w, "capture-screen"));
    action(w, "new-last-region", |w| new_screenshot(w, "capture-last-region"));
    action(w, "new-scrolling", |w| new_screenshot(w, "capture-scrolling"));
    action(w, "new-window", |w| new_screenshot(w, "capture-window"));
    if let Some(app) = w.win.application() {
        app.set_accels_for_action("win.new", &["<Ctrl>N"]);
        app.set_accels_for_action("win.new-last-region", &["<Ctrl><Shift>N"]);
    }
}

fn new_screenshot(w: &Rc<Window>, app_action: &'static str) {
    new_screenshot_with(w, app_action, None)
}

fn new_screenshot_with(w: &Rc<Window>, app_action: &'static str, param: Option<glib::Variant>) {
    if crate::capture::is_busy() {
        return w.toast("Another capture is under way");
    }
    let go = Rc::new(move |w: &Rc<Window>| {
        if let Some(app) = w.win.application() {
            w.replace.set(true);
            app.activate_action(app_action, param.as_ref());
        }
    });
    if !w.modified.get() {
        return go(w);
    }
    let dialog = adw::AlertDialog::new(Some("Save changes to this screenshot?"), Some("It has annotations that are not saved. If you don't save them, they are lost."));
    dialog.add_responses(&[("cancel", "Cancel"), ("discard", "Discard Changes"), ("save", "Save")]);
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    let wk = Rc::downgrade(w);
    dialog.connect_response(None, move |_, response| {
        let Some(w) = wk.upgrade() else { return };
        match response {
            "discard" => go(&w),
            "save" => {
                let (wk, go) = (Rc::downgrade(&w), go.clone());
                save_default(&w, move |saved| {
                    if let (Some(w), true) = (wk.upgrade(), saved) {
                        go(&w);
                    }
                });
            }
            _ => {}
        }
    });
    dialog.present(Some(&w.win));
}

/// Saves into the save folder under the naming template, then calls `then(saved)`.
fn save_default(w: &Rc<Window>, then: impl FnOnce(bool) + 'static) {
    // The Preferences as they are now (they may have changed since this window opened).
    let settings = Settings::load();
    let path = unique_path(&settings.save_dir(), &format_name(&settings.name_template, &now()), "png");
    then(write_png(w, &path));
}

fn save_as(w: &Rc<Window>) {
    let dialog = gtk::FileDialog::builder().title("Save Picture").accept_label("Save").build();
    let settings = Settings::load();
    let dir = settings.save_dir();
    dialog.set_initial_folder(Some(&gio::File::for_path(&dir)));
    dialog.set_initial_name(Some(&format!("{}.png", format_name(&settings.name_template, &now()))));
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("PNG picture"));
    filter.add_mime_type("image/png");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    let wk = Rc::downgrade(w);
    dialog.save(Some(&w.win), gio::Cancellable::NONE, move |result| {
        let (Some(w), Ok(file)) = (wk.upgrade(), result) else { return };
        if let Some(path) = file.path() {
            // The file dialog already asked about overwriting.
            write_png(&w, &with_png_extension(path));
        }
    });
}

fn with_png_extension(path: PathBuf) -> PathBuf {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("png") => path,
        _ => PathBuf::from(format!("{}.png", path.display())),
    }
}

/// Writes the finished picture as a PNG; tells the user where, or why not.
fn write_png(w: &Rc<Window>, path: &Path) -> bool {
    let result = w.export().and_then(|image| encode_png(&image).map_err(|e| e.to_string())).and_then(|bytes| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    });
    match result {
        Ok(()) => {
            w.unsaved.set(false);
            w.modified.set(false);
            crate::library::save_edits(w);
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let toast = adw::Toast::new(&format!("Saved {name}"));
            toast.set_button_label(Some("Show in Files"));
            let (file, win) = (gio::File::for_path(path), w.win.clone());
            toast.connect_button_clicked(move |_| gtk::FileLauncher::new(Some(&file)).open_containing_folder(Some(&win), gio::Cancellable::NONE, |_| {}));
            w.toasts.add_toast(toast);
            true
        }
        Err(e) => {
            obtura_core::log::error(&format!("could not save the picture: {e}"));
            let dialog = adw::AlertDialog::new(Some("Could not save the picture"), Some(&e));
            dialog.add_response("ok", "OK");
            dialog.present(Some(&w.win));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{new_kind, Mode, Output, Request};

    /// What a capture leaves for New to take again is always one of the choices beside it,
    /// and every choice runs a capture the app has.
    #[test]
    fn new_remembers_the_kind_of_the_last_capture() {
        let app_source = include_str!("app.rs");
        for (id, _, app_action, _) in NEW_KINDS {
            assert!(app_source.contains(&format!("(\"{app_action}\"")), "no app action {app_action} for {id}");
        }
        let kinds = |mode, delay| new_kind(&Request { mode, delay, output: Output::Editor });
        for (mode, delay, want) in [
            (Mode::Region, 0, "region"),
            (Mode::Region, 5, "region-5"),
            (Mode::Region, 7, "region"),
            (Mode::LastRegion, 0, "last-region"),
            (Mode::Screen, 0, "screen"),
            (Mode::Screen, 5, "screen-5"),
            (Mode::Window, 0, "window"),
            (Mode::Scrolling, 0, "scrolling"),
        ] {
            let got = kinds(mode, delay).unwrap();
            assert_eq!(got, want);
            assert_eq!(NEW_KINDS[new_kind_index(got)].0, got, "{got} is a choice");
        }
        // Captures that open no editor change nothing.
        assert_eq!(new_kind(&Request { mode: Mode::Region, delay: 0, output: Output::Text }), None);
        assert_eq!(new_kind_index("nonsense"), 0);
    }
}
