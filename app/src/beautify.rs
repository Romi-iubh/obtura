//! The Beautify button: a frame around the screenshot (background colour, gradient or
//! picture, padding, corners, shadow, border, window bar, device, torn edge, size ratio), in
//! one popover. One click on a preset turns it on.

use crate::{
    editor::{Change, Editor},
    format::{leave_popover, ColourButton},
};
use gtk::{cairo, prelude::*};
use obtura_scene::{Background, Bar, Color, Device, Frame};
use std::{cell::Cell, rc::Rc};

/// The backgrounds offered as one-click presets: (name, background).
pub fn presets() -> Vec<(&'static str, Background)> {
    let c = |h: &str| Color::from_hex(h).expect("preset colour");
    vec![
        ("Twilight", Background::Gradient(c("#625ee8"), c("#d15cb5"))),
        ("Ocean", Background::Gradient(c("#2193b0"), c("#6dd5ed"))),
        ("Sunset", Background::Gradient(c("#ff7e5f"), c("#feb47b"))),
        ("Mint", Background::Gradient(c("#11998e"), c("#38ef7d"))),
        ("Peach", Background::Gradient(c("#ee9ca7"), c("#ffdde1"))),
        ("Night", Background::Gradient(c("#232526"), c("#414345"))),
        ("White", Background::Solid(Color::WHITE)),
        ("Light grey", Background::Solid(c("#ececef"))),
        ("Dark", Background::Solid(c("#1e1e20"))),
        ("Transparent", Background::None),
    ]
}

/// Size ratios: (label, ratio).
const RATIOS: [(&str, Option<(u32, u32)>); 7] = [("Free", None), ("16:9 (wide)", Some((16, 9))), ("4:3", Some((4, 3))), ("1:1 (square)", Some((1, 1))), ("4:5 (portrait post)", Some((4, 5))), ("9:16 (story)", Some((9, 16))), ("3:2", Some((3, 2)))];

type Sync = Box<dyn Fn(Option<&Frame>)>;

struct Panel {
    editor: Rc<Editor>,
    syncing: Rc<Cell<bool>>,
    syncs: Vec<Sync>,
}

impl Panel {
    /// Changes the frame (turning it on if it was off).
    fn change(editor: &Editor, f: impl FnOnce(&mut Frame)) {
        let mut frame = editor.frame().unwrap_or_default();
        f(&mut frame);
        editor.set_frame(Some(frame));
    }

    fn slider(&mut self, into: &gtk::Box, title: &str, max: f64, get: impl Fn(&Frame) -> f64 + 'static, set: impl Fn(&mut Frame, f64) + 'static) {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        row.append(&caption(title));
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, max, 1.0);
        scale.set_draw_value(true);
        scale.set_value_pos(gtk::PositionType::Right);
        scale.set_width_request(260);
        row.append(&scale);
        into.append(&row);
        let (ed, sy) = (self.editor.clone(), self.syncing.clone());
        scale.connect_value_changed(move |s| {
            if !sy.get() {
                let v = s.value().round();
                Panel::change(&ed, |f| set(f, v));
            }
        });
        self.syncs.push(Box::new(move |f| {
            row.set_sensitive(f.is_some());
            scale.set_value(get(&f.copied().unwrap_or_default()));
        }));
    }
}

pub fn build(editor: &Rc<Editor>) -> gtk::MenuButton {
    let mut p = Panel { editor: editor.clone(), syncing: Rc::new(Cell::new(false)), syncs: Vec::new() };
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // On / off.
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let title = gtk::Label::builder().label("Frame around the picture").hexpand(true).halign(gtk::Align::Start).css_classes(["heading"]).build();
    let on = gtk::Switch::builder().valign(gtk::Align::Center).tooltip_text("Turn the frame on or off").build();
    top.append(&title);
    top.append(&on);
    content.append(&top);
    {
        let (ed, sy) = (editor.clone(), p.syncing.clone());
        on.connect_active_notify(move |s| {
            if !sy.get() {
                ed.set_frame(s.is_active().then(|| ed.frame().unwrap_or_default()));
            }
        });
        let on = on.clone();
        p.syncs.push(Box::new(move |f| on.set_active(f.is_some())));
    }

    // Background presets: one click turns the frame on with that background.
    content.append(&caption("Background"));
    let grid = gtk::Grid::builder().row_spacing(4).column_spacing(4).build();
    for (i, (name, bg)) in presets().into_iter().enumerate() {
        let area = gtk::DrawingArea::builder().content_width(36).content_height(26).build();
        area.set_draw_func(move |_, cr, w, h| paint_background(cr, w as f64, h as f64, bg));
        let b = gtk::Button::builder().child(&area).tooltip_text(name).css_classes(["flat"]).build();
        let ed = editor.clone();
        b.connect_clicked(move |_| Panel::change(&ed, |f| f.background = bg));
        grid.attach(&b, (i % 5) as i32, (i / 5) as i32, 1, 1);
    }
    content.append(&grid);
    // Own colours: one colour, or two for a gradient.
    let own = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    own.append(&gtk::Label::builder().label("Own colours").hexpand(true).halign(gtk::Align::Start).build());
    // Both colours make the background: one plain colour, or a gradient from the first to
    // the second.
    let pair = Rc::new(Cell::new((Color::WHITE, Color::WHITE)));
    let pick = {
        let (ed, pair) = (editor.clone(), pair.clone());
        Rc::new(move |first: Option<Color>, second: Option<Color>| {
            let (a, b) = pair.get();
            let (a, b) = (first.unwrap_or(a), second.unwrap_or(b));
            Panel::change(&ed, |f| f.background = if a == b { Background::Solid(a) } else { Background::Gradient(a, b) });
        })
    };
    let from = {
        let pick = pick.clone();
        ColourButton::new(false, move |c| pick(Some(c), None))
    };
    let to = ColourButton::new(false, move |c| pick(None, Some(c)));
    from.button.set_tooltip_text(Some("Top-left colour"));
    to.button.set_tooltip_text(Some("Bottom-right colour (the same for a plain background)"));
    own.append(&from.button);
    own.append(&to.button);
    content.append(&own);
    p.syncs.push(Box::new(move |f| {
        let (a, b) = match f.map(|f| f.background) {
            Some(Background::Solid(c)) => (c, c),
            Some(Background::Gradient(a, b)) => (a, b),
            _ => (Color::WHITE, Color::WHITE),
        };
        pair.set((a, b));
        from.set_color(a);
        to.set_color(b);
    }));

    p.slider(&content, "Space around (padding)", 200.0, |f| f.padding, |f, v| f.padding = v);
    p.slider(&content, "Rounded corners", 48.0, |f| f.corner, |f, v| f.corner = v);
    p.slider(&content, "Shadow", 80.0, |f| f.shadow, |f, v| f.shadow = v);
    p.slider(&content, "Border", 16.0, |f| f.border, |f, v| f.border = v);
    let border_colour = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    border_colour.append(&gtk::Label::builder().label("Border colour").hexpand(true).halign(gtk::Align::Start).build());
    let border_button = {
        let ed = editor.clone();
        ColourButton::new(false, move |c| {
            Panel::change(&ed, |f| {
                f.border_color = c;
                if f.border <= 0.0 {
                    f.border = 2.0;
                }
            })
        })
    };
    border_colour.append(&border_button.button);
    content.append(&border_colour);
    p.syncs.push(Box::new(move |f| {
        border_colour.set_sensitive(f.is_some());
        border_button.set_color(f.copied().unwrap_or_default().border_color);
    }));

    // A picture of your own as the background.
    let picture_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    picture_row.append(&gtk::Label::builder().label("Picture").hexpand(true).halign(gtk::Align::Start).build());
    let shown = gtk::DrawingArea::builder().content_width(36).content_height(26).build();
    let choose_picture = gtk::Button::builder().label("Choose…").tooltip_text("Use a picture of your own as the background (it covers the whole frame)").build();
    picture_row.append(&shown);
    picture_row.append(&choose_picture);
    content.append(&picture_row);
    {
        let ed = editor.clone();
        choose_picture.connect_clicked(move |b| {
            let (parent, back) = leave_popover(b);
            let ed = ed.clone();
            crate::modules::choose_picture_file(parent.as_ref(), move |picture| {
                back();
                let Some(picture) = picture else { return };
                let ed = ed.clone();
                gtk::glib::spawn_future_local(async move {
                    // Kept in Obtura's picture store (made smaller if huge), off the main thread.
                    match gtk::gio::spawn_blocking(move || obtura_core::backgrounds::add(&picture)).await {
                        Ok(Ok(key)) => Panel::change(&ed, |f| f.background = Background::Picture(key)),
                        Ok(Err(e)) => obtura_core::log::error(&format!("could not keep the background picture: {e}")),
                        Err(_) => obtura_core::log::error("could not keep the background picture"),
                    }
                });
            });
        });
        let current = Rc::new(Cell::new(None::<Background>));
        let c2 = current.clone();
        shown.set_draw_func(move |_, cr, w, h| {
            if let Some(bg) = c2.get() {
                paint_background(cr, w as f64, h as f64, bg);
            }
        });
        p.syncs.push(Box::new(move |f| {
            let bg = f.map(|f| f.background).filter(|b| matches!(b, Background::Picture(_)));
            current.set(bg);
            shown.set_visible(bg.is_some());
            shown.queue_draw();
        }));
    }

    // Window bar.
    let bar_row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    bar_row.append(&caption("Window bar"));
    let bars = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    bars.add_css_class("linked");
    let mut bar_buttons: Vec<gtk::ToggleButton> = Vec::new();
    for (label, bar) in [("None", Bar::None), ("Light", Bar::Light), ("Dark", Bar::Dark), ("Browser", Bar::Browser)] {
        let b = gtk::ToggleButton::with_label(label);
        if let Some(first) = bar_buttons.first() {
            b.set_group(Some(first));
        }
        let (ed, sy) = (editor.clone(), p.syncing.clone());
        b.connect_toggled(move |b| {
            if b.is_active() && !sy.get() {
                Panel::change(&ed, |f| f.bar = bar);
            }
        });
        bars.append(&b);
        bar_buttons.push(b);
    }
    bar_row.append(&bars);
    content.append(&bar_row);
    p.syncs.push(Box::new(move |f| {
        bar_row.set_sensitive(f.is_some());
        let i = match f.map(|f| f.bar).unwrap_or_default() {
            Bar::None => 0,
            Bar::Light => 1,
            Bar::Dark => 2,
            Bar::Browser => 3,
        };
        bar_buttons[i].set_active(true);
    }));

    // A device around the picture.
    let device_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    device_row.append(&gtk::Label::builder().label("Device").hexpand(true).halign(gtk::Align::Start).build());
    let names: Vec<&str> = Device::ALL.iter().map(|d| d.name()).collect();
    let device = gtk::DropDown::from_strings(&names);
    device.set_tooltip_text(Some("Shows the picture on the screen of a phone, a tablet, a laptop or a monitor"));
    device_row.append(&device);
    content.append(&device_row);
    {
        let (ed, sy) = (editor.clone(), p.syncing.clone());
        device.connect_selected_notify(move |d| {
            if !sy.get() {
                let chosen = Device::ALL[(d.selected() as usize).min(Device::ALL.len() - 1)];
                Panel::change(&ed, |f| f.device = chosen);
            }
        });
        p.syncs.push(Box::new(move |f| {
            device_row.set_sensitive(f.is_some());
            let d = f.map(|f| f.device).unwrap_or_default();
            device.set_selected(Device::ALL.iter().position(|x| *x == d).unwrap_or(0) as u32);
        }));
    }

    // Torn bottom edge.
    let torn_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    torn_row.append(&gtk::Label::builder().label("Torn bottom edge").hexpand(true).halign(gtk::Align::Start).build());
    let torn = gtk::Switch::builder().valign(gtk::Align::Center).build();
    torn_row.append(&torn);
    content.append(&torn_row);
    {
        let (ed, sy) = (editor.clone(), p.syncing.clone());
        torn.connect_active_notify(move |s| {
            if !sy.get() {
                let on = s.is_active();
                Panel::change(&ed, |f| f.torn = on);
            }
        });
        p.syncs.push(Box::new(move |f| {
            // A device has no torn edge.
            torn_row.set_sensitive(f.is_some_and(|f| f.device == Device::None));
            torn.set_active(f.is_some_and(|f| f.torn));
        }));
    }

    // Size ratio.
    let ratio_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    ratio_row.append(&gtk::Label::builder().label("Size ratio").hexpand(true).halign(gtk::Align::Start).build());
    let labels: Vec<&str> = RATIOS.iter().map(|(l, _)| *l).collect();
    let ratio = gtk::DropDown::from_strings(&labels);
    ratio.set_tooltip_text(Some("Adds background so the result has this shape (for social posts and slides)"));
    ratio_row.append(&ratio);
    content.append(&ratio_row);
    {
        let (ed, sy) = (editor.clone(), p.syncing.clone());
        ratio.connect_selected_notify(move |d| {
            if !sy.get() {
                let r = RATIOS[(d.selected() as usize).min(RATIOS.len() - 1)].1;
                Panel::change(&ed, |f| f.ratio = r);
            }
        });
        p.syncs.push(Box::new(move |f| {
            ratio_row.set_sensitive(f.is_some());
            let r = f.and_then(|f| f.ratio);
            ratio.set_selected(RATIOS.iter().position(|(_, x)| *x == r).unwrap_or(0) as u32);
        }));
    }

    // My frame: kept in the settings, and optionally put around every new capture.
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let mine = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    mine.set_homogeneous(true);
    let save_mine = gtk::Button::builder().label("Save as My Frame").tooltip_text("Keep this frame to use again").build();
    let use_mine = gtk::Button::builder().label("Use My Frame").tooltip_text("Put your saved frame around this picture").build();
    mine.append(&save_mine);
    mine.append(&use_mine);
    content.append(&mine);
    let auto_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    auto_row.append(&gtk::Label::builder().label("Add my frame to every new capture").hexpand(true).halign(gtk::Align::Start).wrap(true).build());
    let auto = gtk::Switch::builder().valign(gtk::Align::Center).build();
    auto_row.append(&auto);
    content.append(&auto_row);
    {
        let settings = obtura_core::settings::Settings::load();
        use_mine.set_sensitive(settings.frame.is_some());
        auto.set_active(settings.auto_frame);
        auto.set_sensitive(settings.frame.is_some());
        let (ed, use_mine2, auto2) = (editor.clone(), use_mine.clone(), auto.clone());
        save_mine.connect_clicked(move |b| {
            let Some(frame) = ed.frame() else {
                b.set_tooltip_text(Some("Turn the frame on first"));
                return;
            };
            let mut settings = obtura_core::settings::Settings::load();
            settings.frame = Some(frame);
            if settings.save().is_ok() {
                use_mine2.set_sensitive(true);
                auto2.set_sensitive(true);
                b.set_tooltip_text(Some("Saved as your frame"));
            }
        });
        let ed = editor.clone();
        use_mine.connect_clicked(move |_| {
            if let Some(f) = obtura_core::settings::Settings::load().frame {
                ed.set_frame(Some(f));
            }
        });
        auto.connect_active_notify(|s| {
            let mut settings = obtura_core::settings::Settings::load();
            if settings.auto_frame != s.is_active() {
                settings.auto_frame = s.is_active();
                let _ = settings.save();
            }
        });
    }

    let scroller = gtk::ScrolledWindow::builder().child(&content).hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).propagate_natural_width(true).max_content_height(620).build();
    let popover = gtk::Popover::new();
    popover.set_child(Some(&scroller));
    let face = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    face.append(&crate::icons::image("beautify"));
    face.append(&gtk::Label::new(Some("Beautify")));
    let button = gtk::MenuButton::builder().child(&face).popover(&popover).tooltip_text("Beautify: background, shadow, corners, window bar (Shift+B)").build();

    let Panel { editor, syncing, syncs } = p;
    let sync = {
        let ed = editor.clone();
        move || {
            let f = ed.frame();
            syncing.set(true);
            for s in &syncs {
                s(f.as_ref());
            }
            syncing.set(false);
        }
    };
    sync();
    editor.connect_change(move |c| {
        if c == Change::Document {
            sync();
        }
    });
    button
}

fn caption(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).halign(gtk::Align::Start).css_classes(["caption-heading"]).build()
}

/// A swatch of a background: a rounded rectangle; a checkerboard for "transparent".
fn paint_background(cr: &cairo::Context, w: f64, h: f64, bg: Background) {
    let r = 5.0;
    let q = std::f64::consts::FRAC_PI_2;
    let path = |cr: &cairo::Context| {
        cr.new_sub_path();
        cr.arc(w - r - 1.0, r + 1.0, r, -q, 0.0);
        cr.arc(w - r - 1.0, h - r - 1.0, r, 0.0, q);
        cr.arc(r + 1.0, h - r - 1.0, r, q, 2.0 * q);
        cr.arc(r + 1.0, r + 1.0, r, 2.0 * q, 3.0 * q);
        cr.close_path();
    };
    path(cr);
    cr.clip();
    match bg {
        Background::None => {
            for y in 0..(h as i32 / 6 + 1) {
                for x in 0..(w as i32 / 6 + 1) {
                    let light = (x + y) % 2 == 0;
                    cr.set_source_rgb(if light { 0.95 } else { 0.75 }, if light { 0.95 } else { 0.75 }, if light { 0.95 } else { 0.75 });
                    cr.rectangle(x as f64 * 6.0, y as f64 * 6.0, 6.0, 6.0);
                    let _ = cr.fill();
                }
            }
        }
        Background::Solid(c) => {
            cr.set_source_rgb(c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0);
            let _ = cr.paint();
        }
        Background::Gradient(a, b) => {
            let g = cairo::LinearGradient::new(0.0, 0.0, w, h);
            g.add_color_stop_rgb(0.0, a.r as f64 / 255.0, a.g as f64 / 255.0, a.b as f64 / 255.0);
            g.add_color_stop_rgb(1.0, b.r as f64 / 255.0, b.g as f64 / 255.0, b.b as f64 / 255.0);
            let _ = cr.set_source(&g);
            let _ = cr.paint();
        }
        Background::Picture(key) => {
            if let Some(b) = obtura_scene::picture(&key) {
                let small = obtura_image::resize(&b, (w as u32 * 2).max(1), ((w * 2.0) * b.height as f64 / b.width.max(1) as f64).round().max(1.0) as u32);
                if let Ok(surface) = obtura_render::to_surface(&small) {
                    let k = (w / small.width as f64).max(h / small.height as f64);
                    // (save/restore, not identity_matrix: the context GTK gives may be moved
                    // or scaled already, on a HiDPI screen say.)
                    let _ = cr.save();
                    cr.translate((w - small.width as f64 * k) / 2.0, (h - small.height as f64 * k) / 2.0);
                    cr.scale(k, k);
                    let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                    let _ = cr.paint();
                    let _ = cr.restore();
                }
            }
        }
    }
    cr.reset_clip();
    path(cr);
    cr.set_source_rgba(0.5, 0.5, 0.5, 0.5);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
}
