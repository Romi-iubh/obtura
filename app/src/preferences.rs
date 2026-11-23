//! Preferences and the Keyboard Shortcuts window. Every change is saved at once; there is no OK button.

use adw::prelude::*;
use gtk::gio;
use obtura_core::{
    naming::{format_name, DEFAULT_TEMPLATE},
    settings::Settings,
};
use std::{cell::RefCell, rc::Rc};

pub fn show(app: &adw::Application) {
    let settings = Rc::new(RefCell::new(Settings::load()));
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");
    let page = adw::PreferencesPage::new();

    // --- Saving ---
    let saving = adw::PreferencesGroup::builder().title("Saving").build();
    let folder = adw::ActionRow::builder().title("Folder").subtitle(settings.borrow().save_dir().display().to_string()).subtitle_selectable(true).build();
    let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
    folder.add_suffix(&choose);
    saving.add(&folder);
    {
        let (settings, dialog, folder) = (settings.clone(), dialog.clone(), folder.clone());
        choose.connect_clicked(move |b| {
            let chooser = gtk::FileDialog::builder().title("Folder for Saved Pictures").accept_label("Choose").build();
            chooser.set_initial_folder(Some(&gio::File::for_path(settings.borrow().save_dir())));
            let parent = b.root().and_downcast::<gtk::Window>();
            let (settings, dialog, folder) = (settings.clone(), dialog.clone(), folder.clone());
            chooser.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |result| {
                if let Some(path) = result.ok().and_then(|f| f.path()) {
                    settings.borrow_mut().save_dir = Some(path);
                    folder.set_subtitle(&settings.borrow().save_dir().display().to_string());
                    store(&settings, &dialog);
                }
            });
        });
    }

    let template = adw::EntryRow::builder().title("File name").text(settings.borrow().name_template.clone()).build();
    let preview = adw::ActionRow::builder().title("Example").css_classes(["property"]).build();
    let show_example = {
        let (preview, template) = (preview.clone(), template.clone());
        move || preview.set_subtitle(&format!("{}.png", format_name(&template.text(), &crate::capture::now())))
    };
    show_example();
    {
        let settings = settings.clone();
        let dialog = dialog.clone();
        template.connect_changed(move |t| {
            show_example();
            let text = t.text().to_string();
            settings.borrow_mut().name_template = if text.trim().is_empty() { DEFAULT_TEMPLATE.into() } else { text };
            store(&settings, &dialog);
        });
    }
    saving.add(&template);
    saving.add(&preview);
    saving.add(&adw::ActionRow::builder().title("Codes").subtitle("%Y year · %m month · %d day · %H hour · %M minute · %S second").css_classes(["property"]).build());
    page.add(&saving);

    // --- Behaviour ---
    let behaviour = adw::PreferencesGroup::builder().title("Behaviour").build();
    behaviour.add(&switch_row(&settings, &dialog, "Copy to the clipboard after a capture", "The picture is on the clipboard the moment it is taken, ready to paste anywhere.", |s| s.copy_after_capture, |s, v| s.copy_after_capture = v));
    behaviour.add(&switch_row(&settings, &dialog, "Keep running in the background", "Makes the next capture instant. Nothing is sent anywhere; Obtura just stays loaded.", |s| s.keep_running, |s, v| s.keep_running = v));
    // GNOME only: Windows takes screenshots without a flash or a question.
    if !cfg!(windows) {
        behaviour.add(&switch_row(&settings, &dialog, "Use screen sharing (no white flash)", "Recommended. Turned off, Obtura uses GNOME's screenshot service instead, and the screen flashes white at each screenshot.", |s| s.quiet_capture, |s, v| s.quiet_capture = v));
    }
    let screens = adw::ActionRow::builder().title("Screens Obtura may see").subtitle("GNOME asked once and remembers. Choose again to add a screen.").build();
    let again = gtk::Button::builder().label("Choose Again").valign(gtk::Align::Center).sensitive(obtura_core::screen_share::load().is_some()).build();
    {
        let dialog = dialog.clone();
        again.connect_clicked(move |b| {
            let _ = obtura_core::screen_share::forget();
            b.set_sensitive(false);
            dialog.add_toast(adw::Toast::new("GNOME will ask at the next screenshot or recording"));
        });
    }
    screens.add_suffix(&again);
    if !cfg!(windows) {
        behaviour.add(&screens);
    }
    page.add(&behaviour);

    // --- Library ---
    let library = adw::PreferencesGroup::builder()
        .title("Library")
        .description(format!(
            "Every screenshot is kept as it was taken (before any blur), on this computer only, in {}. Open it with Ctrl+L.",
            if cfg!(windows) { obtura_core::paths::data_dir().join("library").display().to_string() } else { "~/.local/share/obtura/library".to_string() }
        ))
        .build();
    library.add(&switch_row(&settings, &dialog, "Keep every screenshot", "Turned off, new screenshots are not kept; those already there stay until you delete them.", |s| s.history, |s, v| s.history = v));
    library.add(&switch_row(&settings, &dialog, "Read the text of each screenshot", "So you can find a screenshot by the words in it. Done on this computer, in the background.", |s| s.history_text, |s, v| s.history_text = v));
    let days = adw::SpinRow::builder()
        .title("Remove screenshots older than (days)")
        .subtitle("0 keeps them all. Screenshots marked “Keep Always” are never removed.")
        .adjustment(&gtk::Adjustment::new(settings.borrow().history_days as f64, 0.0, 3650.0, 1.0, 30.0, 0.0))
        .build();
    {
        let (settings, dialog) = (settings.clone(), dialog.clone());
        days.connect_value_notify(move |r| {
            settings.borrow_mut().history_days = r.value().round() as u32;
            store(&settings, &dialog);
        });
    }
    library.add(&days);
    page.add(&library);

    // --- Text reading ---
    let reading = adw::PreferencesGroup::builder().title("Text reading").build();
    let installed = obtura_core::ocr::languages();
    let langs = adw::EntryRow::builder().title("Languages (empty: those installed)").text(settings.borrow().ocr_languages.clone()).build();
    {
        let (settings, dialog) = (settings.clone(), dialog.clone());
        langs.connect_changed(move |e| {
            settings.borrow_mut().ocr_languages = e.text().split(|c: char| c == '+' || c == ',' || c.is_whitespace()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("+");
            store(&settings, &dialog);
        });
    }
    reading.add(&langs);
    let about = match &installed {
        // Windows: English comes with Obtura; other languages are files put in its folder.
        Ok(list) if !list.is_empty() && cfg!(windows) => format!(
            "Installed: {}. Write them joined by + (for example eng+fra). More languages: put their .traineddata files (github.com/tesseract-ocr/tessdata) in {}.",
            list.join(", "),
            std::env::current_exe().ok().and_then(|p| Some(p.parent()?.parent()?.join("share").join("tessdata").display().to_string())).unwrap_or_else(|| "share\\tessdata".into())
        ),
        Ok(list) if !list.is_empty() => format!("Installed: {}. Write them joined by + (for example eng+fra). More languages: sudo apt install tesseract-ocr-fra (and so on).", list.join(", ")),
        Ok(_) | Err(_) if cfg!(windows) => obtura_core::ocr::OcrError::Missing.to_string(),
        Ok(_) => "No language is installed for the tesseract program. Install one: sudo apt install tesseract-ocr-eng".to_string(),
        Err(_) => "Text reading needs the tesseract program: sudo apt install tesseract-ocr".to_string(),
    };
    reading.add(&adw::ActionRow::builder().title("Copy Text, Smart Redact and the library search read text with tesseract, on this computer.").subtitle(about).css_classes(["property"]).build());
    page.add(&reading);

    // --- Colour picker ---
    let colours = adw::PreferencesGroup::builder().title("Colour picker").build();
    let formats = gtk::StringList::new(&obtura_core::color::ColorFormat::ALL.map(|f| f.name()));
    let format_row = adw::ComboRow::builder().title("Copies the colour as").model(&formats).build();
    let current = obtura_core::color::ColorFormat::ALL.iter().position(|f| *f == settings.borrow().color_format).unwrap_or(0);
    format_row.set_selected(current as u32);
    {
        let (settings, dialog) = (settings.clone(), dialog.clone());
        format_row.connect_selected_notify(move |r| {
            settings.borrow_mut().color_format = obtura_core::color::ColorFormat::ALL.get(r.selected() as usize).copied().unwrap_or_default();
            store(&settings, &dialog);
        });
    }
    colours.add(&format_row);
    page.add(&colours);

    // --- Recording ---
    let recording = adw::PreferencesGroup::builder().title("Recording").description("Format, sound, pointer and countdown are chosen in the Record window (Ctrl+Shift+E).").build();
    let rec_folder = adw::ActionRow::builder().title("Folder").subtitle(settings.borrow().record.dir().display().to_string()).subtitle_selectable(true).build();
    let rec_choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
    rec_folder.add_suffix(&rec_choose);
    recording.add(&rec_folder);
    {
        let (settings, dialog, rec_folder) = (settings.clone(), dialog.clone(), rec_folder.clone());
        rec_choose.connect_clicked(move |b| {
            let chooser = gtk::FileDialog::builder().title("Folder for Recordings").accept_label("Choose").build();
            chooser.set_initial_folder(Some(&gio::File::for_path(settings.borrow().record.dir())));
            let parent = b.root().and_downcast::<gtk::Window>();
            let (settings, dialog, rec_folder) = (settings.clone(), dialog.clone(), rec_folder.clone());
            chooser.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |result| {
                if let Some(path) = result.ok().and_then(|f| f.path()) {
                    settings.borrow_mut().record.dir = Some(path);
                    rec_folder.set_subtitle(&settings.borrow().record.dir().display().to_string());
                    store(&settings, &dialog);
                }
            });
        });
    }
    let rec_template = adw::EntryRow::builder().title("File name").text(settings.borrow().record.template.clone()).build();
    {
        let (settings, dialog) = (settings.clone(), dialog.clone());
        rec_template.connect_changed(move |t| {
            let text = t.text().to_string();
            settings.borrow_mut().record.template = if text.trim().is_empty() { obtura_core::settings::DEFAULT_RECORD_TEMPLATE.into() } else { text };
            store(&settings, &dialog);
        });
    }
    recording.add(&rec_template);
    // Recording is not in the Windows version yet.
    if !cfg!(windows) {
        page.add(&recording);
    }

    // --- Keys that work in every app ---
    if crate::global_keys::available() {
        let everywhere = adw::PreferencesGroup::builder()
            .title("Keys that work in every app")
            .description(if cfg!(windows) {
                "Obtura's own keys (Ctrl+Shift+R and the others) work only while an Obtura window is in front. These work in every app; Obtura then stays running, its icon in the notification area."
            } else {
                "Obtura's own keys (Ctrl+Shift+R and the others) work only while an Obtura window is in front. These work everywhere, even when Obtura is closed."
            })
            .build();
        let keys_row = adw::SwitchRow::builder().title("Keys in every app").active(crate::global_keys::enabled()).build();
        let describe = {
            let keys_row = keys_row.clone();
            move || {
                let on = crate::global_keys::current();
                let text = if on.is_empty() {
                    crate::global_keys::KEYS.iter().map(|k| format!("{}: {}", crate::global_keys::label(k.binding), k.what)).collect::<Vec<_>>().join("\n")
                } else {
                    on.iter().map(|(what, key)| format!("{key}: {what}")).collect::<Vec<_>>().join("\n")
                };
                let where_ = if cfg!(windows) { "Windows gives them to Obtura while it runs." } else { "Added to GNOME's keyboard shortcuts, where they can be changed." };
                keys_row.set_subtitle(&format!("{text}\n{where_}"));
            }
        };
        describe();
        {
            let dialog = dialog.clone();
            keys_row.connect_active_notify(move |r| {
                match crate::global_keys::set(r.is_active()) {
                    Ok(notes) => {
                        for note in notes {
                            dialog.add_toast(adw::Toast::new(&note));
                        }
                    }
                    Err(e) => {
                        obtura_core::log::error(&format!("keyboard shortcuts: {e}"));
                        dialog.add_toast(adw::Toast::new(&format!("Could not change the keys: {e}")));
                    }
                }
                describe();
            });
        }
        everywhere.add(&keys_row);
        page.add(&everywhere);
    }

    // --- Other commands ---
    let keys = adw::PreferencesGroup::builder()
        .title("Other keys")
        .description(if cfg!(windows) {
            "To give another command a key of its own, make a shortcut to it: right-click the desktop › New › Shortcut, paste one of these commands, give it a name; then in the shortcut's Properties click Shortcut key and press the key you want."
        } else {
            "To give another command a key of its own, add a custom shortcut: open Settings › Keyboard › View and Customize Shortcuts › Custom Shortcuts, press +, give it a name, paste one of these commands, and press the key you want."
        })
        .build();
    // Windows: the full path of obtura.exe (no program folder on the PATH).
    let program = if cfg!(windows) { std::env::current_exe().map(|p| format!("\"{}\"", p.display())).unwrap_or_else(|_| "obtura".into()) } else { "obtura".to_string() };
    for (title, command) in [
        ("Capture a region", "obtura --region"),
        ("Capture the whole screen", "obtura --screen"),
        ("Capture a window", "obtura --window"),
        ("Scrolling capture (press again to finish)", "obtura --scroll"),
        ("Copy the text of a region", "obtura --ocr"),
        ("Read the QR codes of a region", "obtura --qr"),
        ("Pick a colour on the screen", "obtura --pick-color"),
        ("Measure on the screen", "obtura --measure"),
        ("Record an area (press again to stop)", "obtura --record"),
        ("Record a screen (press again to stop)", "obtura --record --screen"),
        ("Record a window (press again to stop)", "obtura --record --window"),
    ] {
        if cfg!(windows) && command.contains("--record") {
            continue;
        }
        let command = command.replacen("obtura", &program, 1);
        keys.add(&command_row(title, &command, &dialog));
    }
    page.add(&keys);

    dialog.add(&page);
    if let Some(parent) = app.active_window() {
        dialog.present(Some(&parent));
    } else {
        // No window to attach to: show the start window and attach to that.
        let home = crate::home::open(app);
        dialog.present(Some(&home));
    }
}

/// Saves the fields this window shows. The file is read again first, so what other parts
/// of Obtura saved meanwhile (tool styles, my frame) is kept.
fn store(settings: &Rc<RefCell<Settings>>, dialog: &adw::PreferencesDialog) {
    let mine = settings.borrow();
    let mut file = Settings::load();
    file.save_dir = mine.save_dir.clone();
    file.name_template = mine.name_template.clone();
    file.copy_after_capture = mine.copy_after_capture;
    file.keep_running = mine.keep_running;
    file.quiet_capture = mine.quiet_capture;
    file.history = mine.history;
    file.history_text = mine.history_text;
    file.history_days = mine.history_days;
    file.ocr_languages = mine.ocr_languages.clone();
    file.color_format = mine.color_format;
    file.record.dir = mine.record.dir.clone();
    file.record.template = mine.record.template.clone();
    if let Err(e) = file.save() {
        dialog.add_toast(adw::Toast::new(&format!("Could not save the settings: {e}")));
    }
}

fn switch_row(settings: &Rc<RefCell<Settings>>, dialog: &adw::PreferencesDialog, title: &str, subtitle: &str, get: impl Fn(&Settings) -> bool, set: impl Fn(&mut Settings, bool) + 'static) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).subtitle(subtitle).active(get(&settings.borrow())).build();
    let (settings, dialog) = (settings.clone(), dialog.clone());
    row.connect_active_notify(move |r| {
        set(&mut settings.borrow_mut(), r.is_active());
        store(&settings, &dialog);
    });
    row
}

/// A row showing a command with a button that copies it.
fn command_row(title: &str, command: &str, dialog: &adw::PreferencesDialog) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle(command).subtitle_selectable(true).css_classes(["monospace"]).build();
    let copy = gtk::Button::builder().icon_name("edit-copy-symbolic").tooltip_text("Copy the command").valign(gtk::Align::Center).css_classes(["flat"]).build();
    let (command, dialog) = (command.to_string(), dialog.clone());
    copy.connect_clicked(move |b| {
        b.clipboard().set_text(&command);
        dialog.add_toast(adw::Toast::new("Command copied"));
    });
    row.add_suffix(&copy);
    row
}

/// The Keyboard Shortcuts window (Ctrl+?): every key, by what it is for.
pub fn shortcuts(app: &adw::Application) {
    let page = adw::PreferencesPage::new();
    let group = |title: &str, rows: &[(&str, String)]| {
        let g = adw::PreferencesGroup::builder().title(title).build();
        for (what, keys) in rows {
            let row = adw::ActionRow::builder().title(*what).build();
            let label = gtk::Label::builder().label(keys.as_str()).css_classes(["dim-label", "monospace"]).build();
            row.add_suffix(&label);
            g.add(&row);
        }
        page.add(&g);
    };
    let s = |x: &str| x.to_string();
    let everywhere = crate::global_keys::current();
    if !everywhere.is_empty() {
        let rows: Vec<(&str, String)> = everywhere.iter().map(|(what, key)| (*what, key.clone())).collect();
        group("In every app", &rows);
    }
    group("Capture", &[("Select a region", s("Ctrl+Shift+R")), ("Whole screen", s("Ctrl+Shift+F")), ("Copy the text of a region", s("Ctrl+Shift+T")), ("New screenshot from the editor", s("Ctrl+N")), ("Same region again, from the editor", s("Ctrl+Shift+N")), ("Scrolling capture: finish", s(if cfg!(windows) { "Print Screen, or Obtura's icon by the clock" } else { "Obtura's icon in the top bar" })), ("Library", s("Ctrl+L"))]);
    if !cfg!(windows) {
        group("Recording", &[("Record the screen", s("Ctrl+Shift+E")), ("Pause", s("Click Obtura's icon")), ("Stop and save", s("Top-bar sharing icon")), ("Start or stop from anywhere", s("obtura --record"))]);
    }
    let tools: Vec<(&str, String)> = crate::tools::ALL.iter().map(|t| (t.name(), t.key().to_string())).collect();
    group("Tools", &tools);
    group(
        "Editing",
        &[
            ("Formatting", s("F")),
            ("Beautify", s("Shift+B")),
            ("Undo", s("Ctrl+Z")),
            ("Redo", s("Ctrl+Shift+Z")),
            ("Delete the selected mark", s("Delete")),
            ("Back out one step", s("Escape")),
            ("Apply the crop", s("Enter")),
            ("Square, circle, 45° lines", s("Shift while drawing")),
            ("Move without snapping", s("Alt while moving")),
            ("Paste a picture, or text as a text box", s("Ctrl+V")),
            ("Turn the selected mark", s("Drag its round handle (Shift: 15° steps)")),
            ("Add a bend to a line (Select tool)", s("Double-click the line")),
            ("Remove a bend", s("Double-click the bend")),
            ("Read the QR codes in the picture", s("Ctrl+Shift+Q")),
            ("Rotate left / right", s("Ctrl+[  Ctrl+]")),
            ("Resize the picture", s("Ctrl+Alt+R")),
            ("Copy all the text in the picture", s("Ctrl+Shift+C")),
            ("Smart Redact", s("Ctrl+Shift+X")),
            ("Pin to the screen", s("Ctrl+Shift+P")),
        ],
    );
    group("View", &[("Zoom in / out", s("Ctrl+wheel  Ctrl++  Ctrl+-")), ("Fit to window", s("Ctrl+0")), ("Actual size", s("Ctrl+1")), ("Scroll", s("Wheel, Shift+wheel, middle button"))]);
    group("File", &[("Copy to the clipboard", s("Ctrl+C")), ("Save", s("Ctrl+S")), ("Save as", s("Ctrl+Shift+S")), ("Close", s("Ctrl+W")), ("Preferences", s("Ctrl+,")), ("Keyboard shortcuts", s("Ctrl+?  or  Ctrl+F1")), ("User guide", s("F1"))]);
    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder().title("Keyboard Shortcuts").content_width(480).content_height(640).child(&view).build();
    match app.active_window() {
        Some(w) => dialog.present(Some(&w)),
        None => dialog.present(Some(&crate::home::open(app))),
    }
}
