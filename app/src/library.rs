//! The library window and the work behind it: every capture is recorded (in the background),
//! its text is read so it can be found by its words, and an edited capture keeps its marks so
//! it opens again ready to edit. Everything stays in `~/.local/share/obtura/library/`.

use crate::{capture::now, icons, window::Window};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use obtura_core::{
    library::{thumbnail, Item, Library, Marks},
    settings::Settings,
};
use obtura_image::{decode_png, encode_png, Bitmap};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

/// Tells the library window apart from the others.
pub const NAME: &str = "obtura-library";

thread_local! {
    /// Items waiting for their text to be read, one at a time.
    static TO_READ: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
    static READING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Called when something in the library changed (the window refreshes).
    static CHANGED: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

fn changed() {
    if let Some(f) = CHANGED.with(|c| c.borrow().clone()) {
        f();
    }
}

/// Records a new capture in the library, in the background, if the history is on. `then`
/// gets the new item's id.
pub fn record(image: &Bitmap, settings: &Settings, then: impl FnOnce(String) + 'static) {
    if !settings.history {
        return;
    }
    let image = image.clone();
    let when = now();
    let read_text = settings.history_text;
    glib::spawn_future_local(async move {
        let added = gio::spawn_blocking(move || -> Result<Item, String> {
            let png = encode_png(&image).map_err(|e| e.to_string())?;
            let thumb = encode_png(&thumbnail(&image)).map_err(|e| e.to_string())?;
            Library::default().add(&png, &thumb, image.width, image.height, &when).map_err(|e| e.to_string())
        })
        .await;
        match added {
            Ok(Ok(item)) => {
                let id = item.id.clone();
                then(item.id);
                if read_text {
                    read_later(id);
                }
                changed();
            }
            Ok(Err(e)) => obtura_core::log::error(&format!("could not add the capture to the library: {e}")),
            Err(_) => obtura_core::log::error("could not add the capture to the library"),
        }
    });
}

/// Queues an item for its text to be read (for searching).
pub fn read_later(id: String) {
    TO_READ.with(|q| {
        let mut q = q.borrow_mut();
        if !q.contains(&id) {
            q.push_back(id);
        }
    });
    read_next();
}

fn read_next() {
    if READING.get() {
        return;
    }
    let Some(id) = TO_READ.with(|q| q.borrow_mut().pop_front()) else { return };
    let lib = Library::default();
    let Some(picture) = readable(&lib, &id) else {
        return read_next();
    };
    READING.set(true);
    let langs = Settings::load().ocr_languages;
    crate::inspect::read_text(picture, langs, move |result| {
        READING.set(false);
        match result {
            Ok(words) => {
                let _ = Library::default().set_text(&id, &obtura_core::ocr::text(&words));
                changed();
                read_next();
            }
            // Without tesseract nothing can be read: stop until next time.
            Err(obtura_core::ocr::OcrError::Missing) => TO_READ.with(|q| q.borrow_mut().clear()),
            Err(_) => read_next(),
        }
    });
}

/// What the text of an item is read from: the capture as taken, or once edited, the edited
/// picture with its marks and its hidden areas blank (so a search never finds hidden text).
fn readable(lib: &Library, id: &str) -> Option<Bitmap> {
    let (png, marks) = lib.load(id)?;
    let picture = decode_png(&png).ok()?;
    if marks.marks.is_empty() {
        return Some(picture);
    }
    let mut scene = obtura_scene::Scene::new();
    for o in marks.marks {
        scene.add_object(o);
    }
    obtura_render::picture_to_read(&picture, scene.objects()).ok()
}

/// After a save: the library item keeps the picture as edited and its marks, and its text is
/// read again from what was saved.
pub fn save_edits(w: &Rc<Window>) {
    let Some(id) = w.library_id.borrow().clone() else { return };
    let id_again = id.clone();
    let picture = w.editor.picture().as_ref().clone();
    let Ok(json) = (Marks { marks: w.editor.marks(), frame: w.editor.frame() }).to_json() else { return };
    let finished = w.export();
    glib::spawn_future_local(async move {
        let saved = gio::spawn_blocking(move || -> Result<(), String> {
            let png = encode_png(&picture).map_err(|e| e.to_string())?;
            let thumb = encode_png(&thumbnail(&finished?)).map_err(|e| e.to_string())?;
            Library::default().save_edits_json(&id, &png, &json, &thumb).map_err(|e| e.to_string())
        })
        .await;
        match saved {
            Ok(Ok(())) => {
                if Settings::load().history_text {
                    read_later(id_again);
                }
                changed();
            }
            Ok(Err(e)) => obtura_core::log::error(&format!("could not keep the edits in the library: {e}")),
            Err(_) => {}
        }
    });
}

/// Removes old captures, as the settings ask (pinned "keep" items stay).
pub fn clean_up(settings: &Settings) {
    if settings.history_days > 0 {
        let removed = Library::default().clean(settings.history_days, &now());
        if removed > 0 {
            obtura_core::log::info(&format!("removed {removed} captures older than {} days from the library", settings.history_days));
        }
    }
}

/// Opens a library item in an editor (or brings its editor forward).
pub fn open_item(app: &adw::Application, id: &str) {
    if let Some(w) = crate::window::find_item(id) {
        w.present();
        return;
    }
    let lib = Library::default();
    let Some((png, marks)) = lib.load(id) else { return };
    let Ok(picture) = decode_png(&png) else { return };
    let w = Window::open(app, picture);
    w.editor.init_marks(marks.marks, marks.frame);
    w.library_id.replace(Some(id.to_string()));
    // Opened from the library: nothing new to save until it is changed, and the clipboard
    // is left as it is.
    w.unsaved.set(false);
    w.modified.set(false);
    w.clip_ours.set(false);
    w.clip_dirty.set(false);
    w.present();
}

/// The finished picture of an item: the edited picture with its marks and frame.
fn finished(id: &str) -> Option<Bitmap> {
    let (png, marks) = Library::default().load(id)?;
    let picture = decode_png(&png).ok()?;
    let mut scene = obtura_scene::Scene::new();
    for o in marks.marks {
        scene.add_object(o);
    }
    let out = obtura_render::render(&picture, &scene).ok()?;
    match marks.frame {
        Some(f) => obtura_render::apply_frame(&out, &f).ok(),
        None => Some(out),
    }
}

/// "Today 19:30", "Yesterday 08:12" or "14 Nov 2026 19:30".
fn when_said(created: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let t = now();
    let today = format!("{:04}-{:02}-{:02}", t.year, t.month, t.day);
    let time = created.get(11..16).unwrap_or("");
    let date = created.get(..10).unwrap_or("");
    let day = |s: &str| -> Option<i64> {
        let mut it = s.split('-');
        Some(obtura_core::library::days_from_civil(it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?))
    };
    if date == today {
        return format!("Today {time}");
    }
    if day(&today).zip(day(date)).is_some_and(|(a, b)| a - b == 1) {
        return format!("Yesterday {time}");
    }
    let mut it = date.split('-');
    match (it.next(), it.next().and_then(|m| m.parse::<usize>().ok()), it.next().and_then(|d| d.parse::<u32>().ok())) {
        (Some(y), Some(m), Some(d)) if (1..=12).contains(&m) => format!("{d} {} {y} {time}", MONTHS[m - 1]),
        _ => created.to_string(),
    }
}

/// Opens the library window (or brings it forward).
pub fn open_window(app: &adw::Application) {
    if let Some(w) = app.windows().into_iter().find(|w| w.widget_name() == NAME) {
        w.present();
        return;
    }
    let settings = Settings::load();
    let search = gtk::SearchEntry::builder().placeholder_text("Search by the words in a screenshot, a tag, or a date (2026-11-14)").hexpand(true).build();
    let clamp = adw::Clamp::builder().maximum_size(520).child(&search).build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&clamp));
    let menu = gio::Menu::new();
    menu.append(Some("Open the Library Folder"), Some("library.folder"));
    menu.append(Some("Preferences"), Some("app.preferences"));
    header.pack_end(&gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).tooltip_text("Library menu").build());

    let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(true).min_children_per_line(1).max_children_per_line(8).row_spacing(18).column_spacing(18).valign(gtk::Align::Start).margin_top(18).margin_bottom(18).margin_start(18).margin_end(18).build();
    let scroller = gtk::ScrolledWindow::builder().child(&flow).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    let empty = adw::StatusPage::builder().icon_name("image-x-generic-symbolic").build();
    let stack = gtk::Stack::new();
    stack.add_named(&scroller, Some("items"));
    stack.add_named(&empty, Some("empty"));
    let note = gtk::Label::builder()
        .label(if settings.history {
            "The library keeps each screenshot as it was taken, before any blur, on this computer only. Right-click a screenshot for more."
        } else {
            "The library is turned off: new screenshots are not kept. Turn it on in Preferences."
        })
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&stack);
    body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    body.append(&note);
    let content = adw::ToolbarView::new();
    content.add_top_bar(&header);
    content.set_content(Some(&body));
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&content));
    let win = adw::ApplicationWindow::builder().application(app).title("Obtura Library").default_width(1000).default_height(700).content(&toasts).build();
    win.set_widget_name(NAME);

    let folder = gio::SimpleAction::new("folder", None);
    {
        let win = win.clone();
        folder.connect_activate(move |_, _| {
            let dir = Library::default().dir().to_path_buf();
            let _ = std::fs::create_dir_all(&dir);
            gtk::FileLauncher::new(Some(&gio::File::for_path(dir))).launch(Some(&win), gio::Cancellable::NONE, |_| {});
        });
    }
    let group = gio::SimpleActionGroup::new();
    group.add_action(&folder);
    win.insert_action_group("library", Some(&group));

    let fill: Rc<dyn Fn()> = {
        let (flow, stack, empty, search, app, win, toasts) = (flow.clone(), stack.clone(), empty.clone(), search.clone(), app.clone(), win.clone(), toasts.clone());
        let me: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let me2 = me.clone();
        let f: Rc<dyn Fn()> = Rc::new(move || {
            while let Some(c) = flow.first_child() {
                flow.remove(&c);
            }
            let query = search.text().to_string();
            let lib = Library::default();
            let items = lib.search(&query);
            for item in &items {
                let refresh = me2.borrow().clone().unwrap_or_else(|| Rc::new(|| {}));
                flow.append(&card(&app, &win, &toasts, item, refresh));
                if item.meta.text.is_none() && Settings::load().history_text {
                    read_later(item.id.clone());
                }
            }
            if items.is_empty() {
                if query.trim().is_empty() {
                    empty.set_title("No screenshots yet");
                    empty.set_description(Some("Every screenshot you take is kept here, on this computer only, and can be found again by the words in it."));
                } else {
                    empty.set_title("Nothing matches");
                    empty.set_description(Some("No screenshot has all these words. Text is read a moment after each capture."));
                }
                stack.set_visible_child_name("empty");
            } else {
                stack.set_visible_child_name("items");
            }
        });
        me.replace(Some(f.clone()));
        f
    };
    {
        let fill = fill.clone();
        search.connect_search_changed(move |_| fill());
    }
    {
        let fill = fill.clone();
        CHANGED.with(|c| c.replace(Some(fill)));
    }
    win.connect_destroy(|_| {
        CHANGED.with(|c| c.replace(None));
    });
    // Escape clears the search, then closes the window.
    let keys = gtk::EventControllerKey::new();
    {
        let (win, search) = (win.clone(), search.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if search.text().is_empty() {
                    win.close();
                } else {
                    search.set_text("");
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    win.add_controller(keys);
    fill();
    win.present();
    search.grab_focus();
}

/// One screenshot in the library: thumbnail, date, first words. Click opens it; right-click
/// gives the menu.
fn card(app: &adw::Application, win: &adw::ApplicationWindow, toasts: &adw::ToastOverlay, item: &Item, refresh: Rc<dyn Fn()>) -> gtk::Widget {
    let lib = Library::default();
    let thumb = gtk::Picture::new();
    if let Some(path) = lib.file(&item.id, "thumb.png") {
        thumb.set_filename(Some(path));
    }
    thumb.set_content_fit(gtk::ContentFit::Contain);
    thumb.set_size_request(220, 140);
    thumb.add_css_class("card");
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    title.append(&gtk::Label::builder().label(when_said(&item.meta.created)).xalign(0.0).hexpand(true).css_classes(["heading"]).build());
    if item.meta.keep {
        let pin = icons::image("pin");
        pin.set_tooltip_text(Some("Kept always (not removed by the clean-up)"));
        title.append(&pin);
    }
    if item.meta.edited {
        let pen = icons::image("pen");
        pen.set_tooltip_text(Some("Edited: opens with its marks"));
        title.append(&pen);
    }
    let words = match &item.meta.text {
        Some(t) if !t.trim().is_empty() => crate::inspect::preview(t),
        Some(_) => "No text".to_string(),
        None => "Reading the text…".to_string(),
    };
    let sub = gtk::Label::builder().label(format!("{} × {} · {words}", item.meta.width, item.meta.height)).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(28).css_classes(["dim-label", "caption"]).build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.append(&thumb);
    body.append(&title);
    body.append(&sub);
    if !item.meta.tags.is_empty() {
        let tags = gtk::Label::builder()
            .label(item.meta.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join("  "))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(28)
            .css_classes(["caption", "accent"])
            .build();
        body.append(&tags);
    }
    body.set_tooltip_text(Some("Click to open it for editing · right-click for more"));
    body.set_cursor_from_name(Some("pointer"));

    // The menu of this item.
    let id = item.id.clone();
    let group = gio::SimpleActionGroup::new();
    let add = |name: &str, f: Box<dyn Fn()>| {
        let a = gio::SimpleAction::new(name, None);
        a.connect_activate(move |_, _| f());
        group.add_action(&a);
    };
    {
        let (app, id) = (app.clone(), id.clone());
        add("open", Box::new(move || open_item(&app, &id)));
    }
    {
        let (win, toasts, id) = (win.clone(), toasts.clone(), id.clone());
        add(
            "copy",
            Box::new(move || match finished(&id) {
                Some(b) => {
                    win.clipboard().set_texture(&crate::modules::texture_of(&b));
                    toasts.add_toast(adw::Toast::new("Copied to the clipboard"));
                }
                None => toasts.add_toast(adw::Toast::new("This screenshot cannot be read")),
            }),
        );
    }
    {
        let (app, id) = (app.clone(), id.clone());
        add(
            "pin",
            Box::new(move || {
                if let Some(b) = finished(&id) {
                    crate::pin::open(&app, &b);
                }
            }),
        );
    }
    {
        let (id, keep, refresh) = (id.clone(), item.meta.keep, refresh.clone());
        add(
            "keep",
            Box::new(move || {
                let _ = Library::default().set_keep(&id, !keep);
                refresh();
            }),
        );
    }
    {
        let (win, id, tags, refresh) = (win.clone(), id.clone(), item.meta.tags.clone(), refresh.clone());
        add("tags", Box::new(move || edit_tags(&win, &id, &tags, refresh.clone())));
    }
    {
        let (win, id) = (win.clone(), id.clone());
        add(
            "show",
            Box::new(move || {
                if let Some(p) = Library::default().file(&id, "original.png") {
                    gtk::FileLauncher::new(Some(&gio::File::for_path(p))).open_containing_folder(Some(&win), gio::Cancellable::NONE, |_| {});
                }
            }),
        );
    }
    {
        let (win, id, refresh) = (win.clone(), id.clone(), refresh.clone());
        add(
            "delete",
            Box::new(move || {
                let dialog = adw::AlertDialog::new(Some("Delete this screenshot from the library?"), Some("It is removed from this computer. Pictures you saved elsewhere stay."));
                dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");
                let (id, refresh) = (id.clone(), refresh.clone());
                dialog.connect_response(Some("delete"), move |_, _| {
                    let _ = Library::default().remove(&id);
                    refresh();
                });
                dialog.present(Some(&win));
            }),
        );
    }
    body.insert_action_group("item", Some(&group));
    let model = gio::Menu::new();
    let first = gio::Menu::new();
    first.append(Some("Open for Editing"), Some("item.open"));
    first.append(Some("Copy"), Some("item.copy"));
    first.append(Some("Pin to Screen"), Some("item.pin"));
    model.append_section(None, &first);
    let second = gio::Menu::new();
    second.append(Some("Tags…"), Some("item.tags"));
    second.append(Some(if item.meta.keep { "Stop Keeping Always" } else { "Keep Always" }), Some("item.keep"));
    second.append(Some("Show in Files"), Some("item.show"));
    model.append_section(None, &second);
    let third = gio::Menu::new();
    third.append(Some("Delete…"), Some("item.delete"));
    model.append_section(None, &third);
    let menu = gtk::PopoverMenu::from_model(Some(&model));
    menu.set_has_arrow(false);
    menu.set_parent(&body);
    {
        let menu = menu.clone();
        body.connect_destroy(move |_| menu.unparent());
    }
    let click = gtk::GestureClick::new();
    click.set_button(0);
    {
        let (app, id) = (app.clone(), id.clone());
        click.connect_released(move |g, _, x, y| match g.current_button() {
            1 => open_item(&app, &id),
            3 => {
                menu.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                menu.popup();
            }
            _ => {}
        });
    }
    body.add_controller(click);
    body.upcast()
}

/// Tags of one screenshot: typed, separated by commas; the tags already used elsewhere are one
/// click away.
fn edit_tags(win: &adw::ApplicationWindow, id: &str, tags: &[String], refresh: Rc<dyn Fn()>) {
    let entry = gtk::Entry::builder().text(tags.join(", ")).placeholder_text("invoice, bug 42, holiday").activates_default(true).hexpand(true).build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.append(&entry);
    let used: Vec<String> = Library::default().all_tags().into_iter().filter(|t| !tags.contains(t)).take(12).collect();
    if !used.is_empty() {
        let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(4).column_spacing(4).row_spacing(4).build();
        for t in used {
            let b = gtk::Button::builder().label(&t).css_classes(["pill", "small"]).tooltip_text("Add this tag").build();
            let entry = entry.clone();
            b.connect_clicked(move |b| {
                let mut now = obtura_core::library::clean_tags(&entry.text());
                now.push(t.clone());
                entry.set_text(&obtura_core::library::clean_tags(&now.join(",")).join(", "));
                b.set_sensitive(false);
            });
            flow.append(&b);
        }
        body.append(&gtk::Label::builder().label("Used before").xalign(0.0).css_classes(["dim-label", "caption"]).build());
        body.append(&flow);
    }
    let dialog = adw::AlertDialog::new(Some("Tags"), Some("Words to find this screenshot by. Separate them with commas."));
    dialog.set_extra_child(Some(&body));
    dialog.add_responses(&[("cancel", "Cancel"), ("save", "Save")]);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    let id = id.to_string();
    dialog.connect_response(Some("save"), move |_, _| {
        if let Err(e) = Library::default().set_tags(&id, obtura_core::library::clean_tags(&entry.text())) {
            obtura_core::log::error(&format!("could not save the tags: {e}"));
        }
        refresh();
    });
    dialog.present(Some(win));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_naturally() {
        let t = now();
        let today = format!("{:04}-{:02}-{:02} 19:30:05", t.year, t.month, t.day);
        assert_eq!(when_said(&today), "Today 19:30");
        assert_eq!(when_said("2020-03-07 08:05:00"), "7 Mar 2020 08:05");
    }
}
