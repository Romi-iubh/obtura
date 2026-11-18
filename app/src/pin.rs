//! Pin to screen: a picture in a small borderless window that stays where it is put, to keep
//! something in view while working. Drag it to move it, scroll to make it bigger or smaller,
//! double-click or Escape to close it, right-click for the menu.
//!
//! GNOME on Wayland does not let a program put its own window above the others: the menu
//! tells how to (Alt+Space, then "Always on Top", GNOME's own window menu).

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use obtura_image::Bitmap;
use std::{cell::Cell, rc::Rc};

/// The largest size a pin opens at (logical pixels); bigger pictures are shrunk to fit.
const MAX_W: f64 = 800.0;
const MAX_H: f64 = 600.0;

const CSS: &str = ".obtura-pin { border: 1px solid alpha(#000, 0.35); background: transparent; } .obtura-pin-window { background: transparent; }";

pub fn open(app: &adw::Application, image: &Bitmap) -> gtk::Window {
    if let Some(display) = gdk::Display::default() {
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    let texture = crate::modules::texture_of(image);
    let (iw, ih) = (image.width as f64, image.height as f64);
    let fit = (MAX_W / iw).min(MAX_H / ih).min(1.0);
    let zoom = Rc::new(Cell::new(fit));

    let picture = gtk::Picture::for_paintable(&texture);
    picture.set_content_fit(gtk::ContentFit::Fill);
    picture.set_can_shrink(true);
    picture.add_css_class("obtura-pin");
    picture.set_tooltip_text(Some("Pinned picture: drag to move · scroll to resize · double-click or Esc to close · right-click for more"));
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&picture));

    let win = gtk::Window::builder()
        .application(app)
        .title("Obtura: pinned picture")
        .decorated(false)
        .resizable(false)
        .default_width((iw * fit).round() as i32)
        .default_height((ih * fit).round() as i32)
        .child(&handle)
        .build();
    win.add_css_class("obtura-pin-window");

    // Scroll: bigger or smaller, from a tenth to twice the picture's size.
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    {
        let (win, zoom) = (win.clone(), zoom.clone());
        scroll.connect_scroll(move |_, _, dy| {
            let z = (zoom.get() * if dy < 0.0 { 1.1 } else { 1.0 / 1.1 }).clamp(0.1, 2.0);
            zoom.set(z);
            win.set_default_size((iw * z).round().max(24.0) as i32, (ih * z).round().max(24.0) as i32);
            glib::Propagation::Stop
        });
    }
    picture.add_controller(scroll);

    let click = gtk::GestureClick::new();
    click.set_button(0);
    let menu = menu_for(&win, image.clone());
    menu.set_parent(&picture);
    {
        let (win, menu) = (win.clone(), menu.clone());
        click.connect_pressed(move |g, n, x, y| match g.current_button() {
            1 if n == 2 => win.close(),
            3 => {
                menu.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                menu.popup();
            }
            _ => {}
        });
    }
    picture.add_controller(click);

    let keys = gtk::EventControllerKey::new();
    {
        let win = win.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            if key == gdk::Key::Escape {
                win.close();
                return glib::Propagation::Stop;
            }
            if mods.contains(gdk::ModifierType::CONTROL_MASK) && matches!(key, gdk::Key::c | gdk::Key::C) {
                let _ = WidgetExt::activate_action(&win, "pin.copy", None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    win.add_controller(keys);
    {
        let menu = menu.clone();
        win.connect_destroy(move |_| menu.unparent());
    }
    win.present();
    win
}

/// The right-click menu: copy, fade, how to keep it on top, close.
fn menu_for(win: &gtk::Window, image: Bitmap) -> gtk::PopoverMenu {
    let group = gio::SimpleActionGroup::new();
    let copy = gio::SimpleAction::new("copy", None);
    {
        let win = win.clone();
        copy.connect_activate(move |_, _| win.clipboard().set_texture(&crate::modules::texture_of(&image)));
    }
    group.add_action(&copy);
    let faded = gio::SimpleAction::new_stateful("fade", None, &false.to_variant());
    {
        let win = win.clone();
        faded.connect_activate(move |a, _| {
            let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            a.set_state(&on.to_variant());
            win.set_opacity(if on { 0.6 } else { 1.0 });
        });
    }
    group.add_action(&faded);
    let close = gio::SimpleAction::new("close", None);
    {
        let win = win.clone();
        close.connect_activate(move |_, _| win.close());
    }
    group.add_action(&close);
    let on_top = gio::SimpleAction::new("on-top", None);
    on_top.set_enabled(false);
    group.add_action(&on_top);
    win.insert_action_group("pin", Some(&group));

    let model = gio::Menu::new();
    let main = gio::Menu::new();
    main.append(Some("Copy"), Some("pin.copy"));
    main.append(Some("See Through"), Some("pin.fade"));
    model.append_section(None, &main);
    let hint = gio::Menu::new();
    hint.append(Some("To keep it above other windows: Alt+Space, then Always on Top"), Some("pin.on-top"));
    model.append_section(None, &hint);
    let end = gio::Menu::new();
    end.append(Some("Close"), Some("pin.close"));
    model.append_section(None, &end);
    let menu = gtk::PopoverMenu::from_model(Some(&model));
    menu.set_has_arrow(false);
    menu
}
