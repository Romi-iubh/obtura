//! The window shown when Obtura is started from the app grid: capture buttons, nothing else.

use adw::prelude::*;

/// Tells the start window apart from the editors.
pub const NAME: &str = "obtura-home";

pub fn open(app: &adw::Application) -> adw::ApplicationWindow {
    let region = gtk::Button::builder().label("Capture Region").tooltip_text("Ctrl+Shift+R").build();
    region.add_css_class("suggested-action");
    region.add_css_class("pill");
    region.set_action_name(Some("app.capture-region"));
    let screen = gtk::Button::builder().label("Capture Screen").tooltip_text("Ctrl+Shift+F").build();
    screen.add_css_class("pill");
    screen.set_action_name(Some("app.capture-screen"));
    let again = gtk::Button::builder().label("Same Region Again").tooltip_text("The region chosen last time, without asking").build();
    again.add_css_class("pill");
    again.set_action_name(Some("app.capture-last-region"));
    again.set_visible(obtura_core::last_region::load().is_some());
    let buttons = gtk::Box::new(gtk::Orientation::Vertical, 12);
    buttons.set_halign(gtk::Align::Center);
    buttons.append(&region);
    buttons.append(&again);
    buttons.append(&screen);
    // The other things Obtura does with the screen, smaller, below.
    let more = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).min_children_per_line(4).max_children_per_line(4).homogeneous(true).column_spacing(6).row_spacing(6).halign(gtk::Align::Center).margin_top(12).build();
    for (icon, label, tip, action) in [
        ("scroll", "Scrolling", "Select something that scrolls, scroll it, and get it all in one picture", "app.capture-scrolling"),
        ("ocr", "Copy Text", "Select a region; its text goes to the clipboard (Ctrl+Shift+T)", "app.capture-text"),
        ("qr", "Read QR", "Select a region; what its QR codes hold goes to the clipboard", "app.capture-qr"),
        ("picker", "Pick a Colour", "Click anywhere on the screen to copy that colour", "app.pick-color"),
        ("measure", "Measure", "Drag across the screen to measure it in pixels", "app.measure-screen"),
        ("record", "Record", "Record the screen or an area as a video or a GIF (Ctrl+Shift+E)", "app.record"),
        ("pin", "Pin", "Select a region and keep it in view in a small window", "app.capture-pin"),
        ("library", "Library", "Every screenshot taken, searchable by its text (Ctrl+L)", "app.library"),
    ] {
        let face = gtk::Box::new(gtk::Orientation::Vertical, 4);
        face.append(&crate::icons::image(icon));
        face.append(&gtk::Label::new(Some(label)));
        let b = gtk::Button::builder().child(&face).tooltip_text(tip).css_classes(["flat"]).build();
        b.set_action_name(Some(action));
        let child = gtk::FlowBoxChild::builder().child(&b).focusable(false).build();
        more.append(&child);
    }
    buttons.append(&more);
    if let Some(keys) = keys_line() {
        buttons.append(&keys);
    }

    let page = adw::StatusPage::builder()
        .icon_name(crate::app::APP_ID)
        .title("Obtura")
        .description("Capture, annotate and share your screen.\nNothing leaves this computer.")
        .child(&buttons)
        .build();
    let menu = crate::help::menu_section();
    let menu_button = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).tooltip_text("Main menu").build();
    let header = adw::HeaderBar::new();
    header.pack_end(&menu_button);
    let content = adw::ToolbarView::new();
    content.add_top_bar(&header);
    content.set_content(Some(&page));
    // Room for every button and the keys line below them without scrolling (the content needs
    // about 755 pixels with Same Region Again shown), with some air around them.
    let win = adw::ApplicationWindow::builder().application(app).title("Obtura").default_width(600).default_height(800).content(&content).build();
    win.set_widget_name(NAME);
    // Ctrl+W (an app-wide key) closes it like any Obtura window.
    let close = gtk::gio::SimpleAction::new("close", None);
    close.connect_activate({
        let win = win.downgrade();
        move |_, _| {
            if let Some(w) = win.upgrade() {
                w.close();
            }
        }
    });
    win.add_action(&close);
    win.present();
    win
}

/// The key that captures a region from any app; or, when there is none yet, a button that adds
/// Obtura's keys to GNOME's shortcuts (see `global_keys`).
fn keys_line() -> Option<gtk::Widget> {
    if !crate::global_keys::available() {
        return None;
    }
    let said = |keys: &[(&str, String)]| keys.iter().map(|(what, key)| format!("{key}: {}", what.to_lowercase())).collect::<Vec<_>>().join("   ·   ");
    let label = gtk::Label::builder().css_classes(["dim-label", "caption"]).wrap(true).justify(gtk::Justification::Center).margin_top(12).build();
    let on = crate::global_keys::current();
    if !on.is_empty() {
        label.set_label(&format!("In every app: {}", said(&on)));
        return Some(label.upcast());
    }
    let first = &crate::global_keys::KEYS[0];
    let add = gtk::Button::builder()
        .label(format!("Use {} in Every App", crate::global_keys::label(first.binding)))
        .tooltip_text("Adds Obtura's keys to GNOME's keyboard shortcuts, so they work while other apps are in front (Preferences can remove them)")
        .css_classes(["flat"])
        .halign(gtk::Align::Center)
        .margin_top(12)
        .build();
    let place = gtk::Box::new(gtk::Orientation::Vertical, 0);
    place.append(&add);
    add.connect_clicked(move |b| {
        match crate::global_keys::set(true) {
            Ok(notes) => {
                let on = crate::global_keys::current();
                let mut text = if on.is_empty() { String::new() } else { format!("In every app: {}", said(&on)) };
                for n in notes {
                    text.push_str(&format!("\n{n}"));
                }
                label.set_label(&text);
            }
            Err(e) => label.set_label(&format!("Could not add the keys: {e}")),
        }
        if let Some(place) = b.parent().and_downcast::<gtk::Box>() {
            place.remove(b);
            place.append(&label);
        }
    });
    Some(place.upcast())
}
