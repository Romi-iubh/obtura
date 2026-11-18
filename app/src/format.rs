//! The Formatting button: one popover with the style of the current tool (or of the
//! selected mark). The quick controls come first (a colour swatch row, thickness, size, line
//! ends); everything else is one click away under "More formatting". Only the controls that
//! matter for the tool are shown.

use crate::{
    editor::{Change, Editor},
    tools::{Fields, Slot},
};
use gtk::{cairo, gdk, prelude::*};
use obtura_render::draw_objects;
use obtura_scene::{Cap, Color, Dash, Kind, Object, Point, Shape, Style, CAPS, FONTS};
use std::{cell::Cell, rc::Rc};

const PALETTE: [&str; 8] = ["#e01b24", "#ff7800", "#f5c211", "#33d17a", "#3584e4", "#9141ac", "#ffffff", "#000000"];

/// What the Stamp tool offers. The first seven are plain symbols drawn in the chosen colour;
/// the rest are colour emoji.
/// (U+FE0F asks for the colour form.)
pub const STAMPS: [&str; 24] = [
    "✔", "✘", "★", "♥", "➜", "?", "!", "✅", "❌", "⚠\u{fe0f}", "❓", "❗", "👍\u{fe0f}", "👎\u{fe0f}", "👉\u{fe0f}", "👆\u{fe0f}", "🔥", "💡", "🐞", "🔒", "🎯", "🚫", "😀", "🎉",
];

pub struct Format {
    pub button: gtk::MenuButton,
}

/// What a control does when the style or the tool changes: show or hide itself and show
/// the current value.
type Sync = Box<dyn Fn(&Style, &Fields)>;

struct Panel {
    editor: Rc<Editor>,
    syncing: Rc<Cell<bool>>,
    syncs: Vec<Sync>,
}

pub fn build(editor: &Rc<Editor>) -> Format {
    let mut p = Panel { editor: editor.clone(), syncing: Rc::new(Cell::new(false)), syncs: Vec::new() };

    // The button shows the current colour.
    let dot = gtk::DrawingArea::builder().content_width(14).content_height(14).valign(gtk::Align::Center).build();
    let face = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    face.append(&dot);
    face.append(&gtk::Label::new(Some("Format")));
    let button = gtk::MenuButton::builder().child(&face).tooltip_text("Formatting (F)").build();
    {
        let ed = editor.clone();
        dot.set_draw_func(move |_, cr, w, h| paint_dot(cr, w, h, main_color(&ed.style(), ed.fields().palette), false));
    }

    let quick = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let more = gtk::Box::new(gtk::Orientation::Vertical, 10);

    // ---- Quick ----
    p.palette(&quick);
    p.slider(&quick, 1.0, 40.0, |f| f.thickness, |s| s.width, |s, v| s.width = v);
    p.slider(&quick, 10.0, 120.0, |f| f.size.filter(|_| !f.more.badge_shape), |s| s.font_size, |s, v| s.font_size = v);
    // A step badge: its size and its number grow together.
    p.slider(&quick, 8.0, 60.0, |f| f.size.filter(|_| f.more.badge_shape), |s| s.badge, |s, v| {
        let ratio = if s.badge > 0.0 { v / s.badge } else { 1.0 };
        s.font_size = (s.font_size * ratio).clamp(8.0, 120.0);
        s.badge = v;
    });
    p.stamps(&quick);
    p.crop_ratio(&quick);
    p.slider(&quick, 10.0, 95.0, |f| f.darkness.then_some("How dark (%)"), |s| s.opacity * 100.0, |s, v| s.opacity = (v / 100.0).clamp(0.1, 0.95));
    p.switch(&quick, "Filled", |f| f.filled, |s| s.fill.is_some(), |s, on| s.fill = on.then_some(s.color));
    p.ends(&quick);
    p.choice(
        &quick,
        "Shape",
        |f| f.redact_shape,
        vec![(text_widget("Rectangle"), "Rectangle"), (text_widget("Ellipse"), "Ellipse")],
        |s| usize::from(s.shape == Shape::Ellipse),
        |s, i| s.shape = if i == 1 { Shape::Ellipse } else { Shape::Rect },
    );

    // ---- More formatting: the look of the shape or box first, then the lettering, then the rest ----
    p.choice(
        &more,
        "Badge shape",
        |f| f.more.badge_shape,
        vec![(shape_widget(Shape::Ellipse), "Circle"), (shape_widget(Shape::Rect), "Square"), (shape_widget(Shape::Rounded), "Rounded square"), (shape_widget(Shape::Diamond), "Diamond")],
        |s| match s.shape {
            Shape::Ellipse => 0,
            Shape::Rect => 1,
            Shape::Rounded => 2,
            Shape::Diamond => 3,
        },
        |s, i| s.shape = [Shape::Ellipse, Shape::Rect, Shape::Rounded, Shape::Diamond][i],
    );
    p.colour(&more, "fill", |f| f.more.fill_color, |s| s.fill.unwrap_or(s.color), |s, c| s.fill = Some(c), true);
    p.colour(&more, "line", |f| f.more.line_color, |s| s.color, |s, c| s.color = c, false);
    p.slider(&more, 0.0, 40.0, |f| f.more.border, |s| s.width, |s, v| s.width = v);
    p.slider(&more, 0.0, 40.0, |f| f.more.corner.then_some("Corner radius"), |s| s.corner, |s, v| s.corner = v);
    p.colour(&more, "text", |f| f.more.text_color, |s| s.text_color, |s, c| s.text_color = c, false);
    p.font(&more);
    p.slider(&more, 8.0, 120.0, |f| f.more.font_size, |s| s.font_size, |s, v| s.font_size = v);
    p.choice(
        &more,
        "Line style",
        |f| f.more.dash,
        vec![(dash_widget(Dash::Solid), "Solid"), (dash_widget(Dash::Dashed), "Dashed"), (dash_widget(Dash::Dotted), "Dotted")],
        |s| match s.dash {
            Dash::Solid => 0,
            Dash::Dashed => 1,
            Dash::Dotted => 2,
        },
        |s, i| s.dash = [Dash::Solid, Dash::Dashed, Dash::Dotted][i],
    );
    p.slider(&more, 0.5, 3.0, |f| f.more.cap_size.then_some("Size of the ends"), |s| s.cap_size, |s, v| s.cap_size = (v * 10.0).round() / 10.0);
    p.switch(&more, "Solid ends", |f| f.more.cap_filled, |s| s.cap_filled, |s, on| s.cap_filled = on);
    p.slider(&more, 10.0, 100.0, |f| f.more.opacity.then_some("Opacity (%)"), |s| s.opacity * 100.0, |s, v| s.opacity = (v / 100.0).clamp(0.1, 1.0));
    p.slider(&more, 0.0, 40.0, |f| f.more.strength.then_some("Strength (0 = automatic)"), |s| s.strength, |s, v| s.strength = v.round());

    // The "More formatting" switch.
    let revealer = gtk::Revealer::builder().child(&more).transition_type(gtk::RevealerTransitionType::SlideDown).build();
    let toggle = gtk::ToggleButton::builder().css_classes(["flat"]).build();
    let toggle_face = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let toggle_label = gtk::Label::builder().label("More formatting").hexpand(true).halign(gtk::Align::Start).build();
    let chevron = gtk::Image::from_icon_name("pan-down-symbolic");
    toggle_face.append(&toggle_label);
    toggle_face.append(&chevron);
    toggle.set_child(Some(&toggle_face));
    {
        let (revealer, chevron) = (revealer.clone(), chevron.clone());
        toggle.connect_toggled(move |t| {
            revealer.set_reveal_child(t.is_active());
            chevron.set_icon_name(Some(if t.is_active() { "pan-up-symbolic" } else { "pan-down-symbolic" }));
        });
    }
    let nothing = gtk::Label::builder().label("Nothing to format for this tool.").css_classes(["dim-label"]).build();

    // Keep this look for the tool's next marks, also after Obtura restarts.
    let defaults = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    defaults.set_homogeneous(true);
    let keep = gtk::Button::builder().label("Set as Default").tooltip_text("New marks of this tool start like this, also next time").build();
    let reset = gtk::Button::builder().label("Reset").tooltip_text("Back to Obtura's own style for this tool").build();
    defaults.append(&keep);
    defaults.append(&reset);
    {
        let ed = editor.clone();
        keep.connect_clicked(move |b| {
            let (tool, style) = (ed.style_tool(), ed.style());
            let mut settings = obtura_core::settings::Settings::load();
            settings.tool_styles.insert(tool.id(), style);
            let saved = settings.save().is_ok();
            ed.set_tool_styles([(tool, style)]);
            b.set_tooltip_text(Some(if saved { "Saved: new marks of this tool start like this" } else { "Could not save the setting" }));
        });
        let ed = editor.clone();
        reset.connect_clicked(move |_| {
            let tool = ed.style_tool();
            let mut settings = obtura_core::settings::Settings::load();
            if settings.tool_styles.remove(&tool.id()).is_some() {
                let _ = settings.save();
            }
            ed.reset_tool_style(tool);
        });
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for m in [&content] {
        m.set_margin_top(10);
        m.set_margin_bottom(10);
        m.set_margin_start(12);
        m.set_margin_end(12);
    }
    content.append(&quick);
    content.append(&toggle);
    content.append(&revealer);
    content.append(&defaults);
    content.append(&nothing);
    let scroller = gtk::ScrolledWindow::builder().child(&content).hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).propagate_natural_width(true).max_content_height(560).build();
    let popover = gtk::Popover::new();
    popover.set_child(Some(&scroller));
    button.set_popover(Some(&popover));

    // Show only what matters now and reflect the current style.
    let Panel { editor, syncing, syncs } = p;
    let sync = {
        let (ed, dot, quick) = (editor.clone(), dot.clone(), quick.clone());
        move || {
            let (style, fields) = (ed.style(), ed.fields());
            syncing.set(true);
            for s in &syncs {
                s(&style, &fields);
            }
            syncing.set(false);
            toggle.set_visible(fields.more.any());
            revealer.set_visible(fields.more.any());
            nothing.set_visible(!fields.any());
            defaults.set_visible(fields.any() && !fields.crop_ratio);
            quick.set_visible(fields.quick());
            dot.set_visible(fields.palette.is_some());
            dot.queue_draw();
        }
    };
    sync();
    editor.connect_change(move |c| {
        if matches!(c, Change::Tool | Change::Selection | Change::Style | Change::Document) {
            sync();
        }
    });
    Format { button }
}

/// The colour the quick palette changes.
fn main_color(s: &Style, slot: Option<Slot>) -> Color {
    match slot {
        Some(Slot::Fill) => s.fill.unwrap_or(s.color),
        Some(Slot::Text) => s.text_color,
        _ => s.color,
    }
}

impl Panel {
    fn add(&mut self, f: impl Fn(&Style, &Fields) + 'static) {
        self.syncs.push(Box::new(f));
    }

    /// The row of colour swatches. A swatch sets the tool's main colour (the badge for a step,
    /// the lettering for text, the line for the rest).
    fn palette(&mut self, into: &gtk::Box) {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
        row.append(&caption("Colour"));
        let swatches = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let mut areas = Vec::new();
        for hex in PALETTE {
            let color = Color::from_hex(hex).expect("palette");
            let area = gtk::DrawingArea::builder().content_width(22).content_height(22).build();
            {
                let ed = self.editor.clone();
                area.set_draw_func(move |_, cr, w, h| paint_dot(cr, w, h, color, main_color(&ed.style(), ed.fields().palette) == color));
            }
            let b = gtk::Button::builder().child(&area).css_classes(["flat", "circular"]).tooltip_text(hex).build();
            {
                let ed = self.editor.clone();
                b.connect_clicked(move |_| {
                    let slot = ed.fields().palette;
                    let is_badge = ed.fields().more.badge_shape;
                    ed.apply_style(|s| match slot {
                        Some(Slot::Fill) => {
                            s.fill = Some(color);
                            if is_badge {
                                // The number stays readable on the new badge colour.
                                s.text_color = color.contrast();
                            }
                        }
                        Some(Slot::Text) => s.text_color = color,
                        _ => s.color = color,
                    });
                });
            }
            swatches.append(&b);
            areas.push(area);
        }
        row.append(&swatches);
        into.append(&row);
        self.add(move |_, f| {
            row.set_visible(f.palette.is_some());
            areas.iter().for_each(|a| a.queue_draw());
        });
    }

    /// The shape of the crop rectangle.
    fn crop_ratio(&mut self, into: &gtk::Box) {
        const RATIOS: [(&str, Option<(u32, u32)>); 6] = [("Free", None), ("16:9", Some((16, 9))), ("4:3", Some((4, 3))), ("3:2", Some((3, 2))), ("1:1", Some((1, 1))), ("9:16", Some((9, 16)))];
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        row.append(&caption("Shape"));
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        line.add_css_class("linked");
        let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
        for (label, ratio) in RATIOS {
            let b = gtk::ToggleButton::with_label(label);
            if let Some(first) = buttons.first() {
                b.set_group(Some(first));
            }
            let (ed, sy) = (self.editor.clone(), self.syncing.clone());
            b.connect_toggled(move |b| {
                if b.is_active() && !sy.get() {
                    ed.set_crop_ratio(ratio);
                }
            });
            line.append(&b);
            buttons.push(b);
        }
        row.append(&line);
        into.append(&row);
        let ed = self.editor.clone();
        self.add(move |_, f| {
            row.set_visible(f.crop_ratio);
            let current = ed.crop_ratio();
            if let Some(i) = RATIOS.iter().position(|(_, r)| *r == current) {
                buttons[i].set_active(true);
            }
        });
    }

    /// The grid of emoji and symbols for the Stamp tool.
    fn stamps(&mut self, into: &gtk::Box) {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
        row.append(&caption("Emoji and symbols"));
        let grid = gtk::Grid::builder().row_spacing(2).column_spacing(2).build();
        let mut buttons: Vec<(gtk::ToggleButton, &'static str)> = Vec::new();
        for (i, glyph) in STAMPS.into_iter().enumerate() {
            let label = gtk::Label::new(None);
            let family = if obtura_render::is_emoji(glyph) { obtura_render::EMOJI_FAMILY } else { "Sans" };
            label.set_markup(&format!("<span size='x-large' font_family='{family}'>{glyph}</span>"));
            let b = gtk::ToggleButton::builder().child(&label).css_classes(["flat"]).tooltip_text(glyph).build();
            if let Some((first, _)) = buttons.first() {
                b.set_group(Some(first));
            }
            let (ed, sy) = (self.editor.clone(), self.syncing.clone());
            b.connect_toggled(move |b| {
                if b.is_active() && !sy.get() {
                    ed.set_stamp(glyph);
                }
            });
            grid.attach(&b, (i % 8) as i32, (i / 8) as i32, 1, 1);
            buttons.push((b, glyph));
        }
        row.append(&grid);
        into.append(&row);
        let ed = self.editor.clone();
        self.add(move |_, f| {
            row.set_visible(f.stamps);
            if f.stamps {
                let current = ed.stamp();
                if let Some((b, _)) = buttons.iter().find(|(_, g)| *g == current) {
                    b.set_active(true);
                }
            }
        });
    }

    /// A slider. `label` says whether it shows for the tool and what it is called; `get` reads
    /// the style, `set` writes it.
    fn slider(&mut self, into: &gtk::Box, min: f64, max: f64, label: impl Fn(&Fields) -> Option<&'static str> + 'static, get: impl Fn(&Style) -> f64 + 'static, set: impl Fn(&mut Style, f64) + 'static) {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let title = caption("");
        row.append(&title);
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, if max <= 3.0 { 0.1 } else { 1.0 });
        scale.set_draw_value(true);
        scale.set_value_pos(gtk::PositionType::Right);
        scale.set_width_request(230);
        row.append(&scale);
        into.append(&row);
        let (ed, sy) = (self.editor.clone(), self.syncing.clone());
        scale.connect_value_changed(move |s| {
            if !sy.get() {
                let v = s.value();
                ed.apply_style(|st| set(st, v));
            }
        });
        self.add(move |s, f| {
            let name = label(f);
            row.set_visible(name.is_some());
            if let Some(n) = name {
                title.set_label(n);
                scale.set_value(get(s));
            }
        });
    }

    fn switch(&mut self, into: &gtk::Box, text: &str, show: impl Fn(&Fields) -> bool + 'static, get: impl Fn(&Style) -> bool + 'static, set: impl Fn(&mut Style, bool) + 'static) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(&gtk::Label::builder().label(text).hexpand(true).halign(gtk::Align::Start).build());
        let sw = gtk::Switch::builder().valign(gtk::Align::Center).build();
        row.append(&sw);
        into.append(&row);
        let (ed, sy) = (self.editor.clone(), self.syncing.clone());
        sw.connect_active_notify(move |w| {
            if !sy.get() {
                ed.apply_style(|st| set(st, w.is_active()));
            }
        });
        self.add(move |s, f| {
            row.set_visible(show(f));
            sw.set_active(get(s));
        });
    }

    /// A row of exclusive buttons, one per option.
    fn choice(&mut self, into: &gtk::Box, title: &str, show: impl Fn(&Fields) -> bool + 'static, items: Vec<(gtk::Widget, &'static str)>, get: impl Fn(&Style) -> usize + 'static, set: impl Fn(&mut Style, usize) + 'static) {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        row.append(&caption(title));
        let buttons_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let set = Rc::new(set);
        let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
        for (i, (widget, tip)) in items.into_iter().enumerate() {
            let b = gtk::ToggleButton::builder().child(&widget).tooltip_text(tip).build();
            if let Some(first) = buttons.first() {
                b.set_group(Some(first));
            }
            let (ed, sy, set) = (self.editor.clone(), self.syncing.clone(), set.clone());
            b.connect_toggled(move |b| {
                if b.is_active() && !sy.get() {
                    ed.apply_style(|st| set(st, i));
                }
            });
            buttons_row.append(&b);
            buttons.push(b);
        }
        row.append(&buttons_row);
        into.append(&row);
        self.add(move |s, f| {
            row.set_visible(show(f));
            if let Some(b) = buttons.get(get(s)) {
                b.set_active(true);
            }
        });
    }

    /// What a line starts and ends with: two rows of pictures of the ends.
    fn ends(&mut self, into: &gtk::Box) {
        for (title, at_end) in [("Start", false), ("End", true)] {
            let items: Vec<(gtk::Widget, &'static str)> = CAPS.into_iter().map(|c| (cap_widget(c, at_end), c.name())).collect();
            self.choice(
                into,
                title,
                |f| f.ends,
                items,
                move |s| CAPS.iter().position(|c| *c == if at_end { s.end } else { s.start }).unwrap_or(0),
                move |s, i| {
                    if at_end {
                        s.end = CAPS[i];
                    } else {
                        s.start = CAPS[i];
                    }
                },
            );
        }
    }

    /// Font family (the four everyday ones, or any installed family through "Other Font…"),
    /// bold and italic.
    fn font(&mut self, into: &gtk::Box) {
        const OTHER: &str = "Other Font…";
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        row.append(&caption("Font"));
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let mut names: Vec<&str> = FONTS.iter().map(|f| f.name()).collect();
        names.push(OTHER);
        let list = gtk::StringList::new(&names);
        let family = gtk::DropDown::builder().model(&list).hexpand(true).build();
        let choose = gtk::Button::builder().icon_name("document-edit-symbolic").tooltip_text("Choose any installed font").css_classes(["flat"]).build();
        let bold = gtk::ToggleButton::builder().label("B").tooltip_text("Bold").css_classes(["heading"]).build();
        let italic = gtk::ToggleButton::builder().label("I").tooltip_text("Italic").build();
        italic.add_css_class("italic");
        line.append(&family);
        line.append(&choose);
        line.append(&bold);
        line.append(&italic);
        row.append(&line);
        into.append(&row);
        // GNOME's font chooser; the family picked becomes the style's font.
        let pick = {
            let (ed, family) = (self.editor.clone(), family.clone());
            Rc::new(move || {
                let dialog = gtk::FontDialog::builder().title("Choose a Font").build();
                let parent = family.root().and_downcast::<gtk::Window>();
                let ed = ed.clone();
                dialog.choose_family(parent.as_ref(), None::<&gtk::pango::FontFamily>, gtk::gio::Cancellable::NONE, move |result| {
                    if let Ok(f) = result {
                        let name = obtura_scene::FamilyName::new(&f.name());
                        ed.apply_style(|st| st.font = obtura_scene::Font::Family(name));
                    }
                });
            })
        };
        {
            let (ed, sy, pick1) = (self.editor.clone(), self.syncing.clone(), pick.clone());
            family.connect_selected_notify(move |d| {
                if sy.get() {
                    return;
                }
                match FONTS.get(d.selected() as usize) {
                    Some(f) => {
                        let f = *f;
                        ed.apply_style(|st| st.font = f);
                    }
                    // The last entry: the font chosen before, or the chooser if there is none.
                    None if matches!(ed.style().font, obtura_scene::Font::Family(_)) => {}
                    None => pick1(),
                }
            });
            let pick2 = pick.clone();
            choose.connect_clicked(move |_| pick2());
            let (ed, sy) = (self.editor.clone(), self.syncing.clone());
            bold.connect_toggled(move |b| {
                if !sy.get() {
                    ed.apply_style(|st| st.bold = b.is_active());
                }
            });
            let (ed, sy) = (self.editor.clone(), self.syncing.clone());
            italic.connect_toggled(move |b| {
                if !sy.get() {
                    ed.apply_style(|st| st.italic = b.is_active());
                }
            });
        }
        self.add(move |s, f| {
            row.set_visible(f.more.font);
            // The last entry names the font chosen through the chooser.
            let last = match &s.font {
                obtura_scene::Font::Family(name) => name.as_str().to_string(),
                _ => OTHER.to_string(),
            };
            list.splice(FONTS.len() as u32, 1, &[last.as_str()]);
            family.set_selected(FONTS.iter().position(|x| *x == s.font).unwrap_or(FONTS.len()) as u32);
            bold.set_active(s.bold);
            italic.set_active(s.italic);
        });
    }

    /// A colour of the style with the full colour chooser. `fill` colours may be transparent
    /// and may be switched off.
    #[allow(clippy::too_many_arguments)]
    fn colour(&mut self, into: &gtk::Box, _id: &str, label: impl Fn(&Fields) -> Option<&'static str> + 'static, get: impl Fn(&Style) -> Color + 'static, set: impl Fn(&mut Style, Color) + 'static, fill: bool) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = gtk::Label::builder().hexpand(true).halign(gtk::Align::Start).build();
        row.append(&title);
        let dialog = gtk::ColorDialog::new();
        dialog.set_with_alpha(fill);
        let button = gtk::ColorDialogButton::new(Some(dialog));
        let none = gtk::CheckButton::with_label("None");
        if fill {
            row.append(&none);
        }
        row.append(&button);
        into.append(&row);
        let set = Rc::new(set);
        {
            let (ed, sy, set) = (self.editor.clone(), self.syncing.clone(), set.clone());
            button.connect_rgba_notify(move |b| {
                if !sy.get() {
                    let c = b.rgba();
                    let to8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                    let color = Color { r: to8(c.red()), g: to8(c.green()), b: to8(c.blue()), a: to8(c.alpha()) };
                    ed.apply_style(|st| set(st, color));
                }
            });
        }
        if fill {
            let (ed, sy, set, button) = (self.editor.clone(), self.syncing.clone(), set, button.clone());
            none.connect_toggled(move |n| {
                if !sy.get() {
                    if n.is_active() {
                        ed.apply_style(|st| st.fill = None);
                    } else {
                        let c = button.rgba();
                        let to8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                        let color = Color { r: to8(c.red()), g: to8(c.green()), b: to8(c.blue()), a: to8(c.alpha()) };
                        ed.apply_style(|st| set(st, color));
                    }
                }
            });
        }
        self.add(move |s, f| {
            let name = label(f);
            row.set_visible(name.is_some());
            if let Some(n) = name {
                title.set_label(n);
                let c = get(s);
                button.set_rgba(&gdk::RGBA::new(c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0, c.a as f32 / 255.0));
                if fill {
                    none.set_active(s.fill.is_none());
                    button.set_sensitive(s.fill.is_some());
                }
            }
        });
    }
}

fn caption(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).halign(gtk::Align::Start).css_classes(["caption-heading"]).build()
}

fn text_widget(text: &str) -> gtk::Widget {
    gtk::Label::new(Some(text)).upcast()
}

/// The colour text has in the current theme.
fn foreground(w: &gtk::DrawingArea) -> Color {
    let c = w.color();
    Color::rgb((c.red() * 255.0) as u8, (c.green() * 255.0) as u8, (c.blue() * 255.0) as u8)
}

/// A small picture of one end of a line, drawn by the same code that draws the real thing.
fn cap_widget(cap: Cap, at_end: bool) -> gtk::Widget {
    let area = gtk::DrawingArea::builder().content_width(36).content_height(20).build();
    area.set_draw_func(move |a, cr, _w, h| {
        let c = foreground(a);
        let y = h as f64 / 2.0;
        let style = Style { color: c, width: 2.0, cap_size: 0.75, start: if at_end { Cap::None } else { cap }, end: if at_end { cap } else { Cap::None }, ..Style::default() };
        let (from, to) = (Point::new(4.0, y), Point::new(32.0, y));
        draw_objects(cr, &[Object { id: 0, kind: Kind::Line { from, to, via: vec![] }, style, angle: 0.0 }]);
    });
    area.upcast()
}

fn dash_widget(dash: Dash) -> gtk::Widget {
    let area = gtk::DrawingArea::builder().content_width(36).content_height(20).build();
    area.set_draw_func(move |a, cr, _w, h| {
        let style = Style { color: foreground(a), width: 2.5, dash, ..Style::default() };
        let y = h as f64 / 2.0;
        draw_objects(cr, &[Object { id: 0, kind: Kind::Line { from: Point::new(4.0, y), to: Point::new(32.0, y), via: vec![] }, style, angle: 0.0 }]);
    });
    area.upcast()
}

fn shape_widget(shape: Shape) -> gtk::Widget {
    let area = gtk::DrawingArea::builder().content_width(26).content_height(20).build();
    area.set_draw_func(move |a, cr, w, h| {
        let style = Style { fill: Some(foreground(a)), width: 0.0, shape, ..Style::default() };
        let (cx, cy, r) = (w as f64 / 2.0, h as f64 / 2.0, 7.0);
        draw_objects(cr, &[Object { id: 0, kind: Kind::Step { center: Point::new(cx, cy), number: 0 }, style: Style { badge: r, font_size: 1.0, text_color: Color { a: 0, ..Color::WHITE }, ..style }, angle: 0.0 }]);
    });
    area.upcast()
}

fn paint_dot(cr: &cairo::Context, w: i32, h: i32, color: Color, selected: bool) {
    let (cx, cy, r) = (w as f64 / 2.0, h as f64 / 2.0, (w.min(h) as f64 / 2.0) - 1.5);
    cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
    cr.set_source_rgb(color.r as f64 / 255.0, color.g as f64 / 255.0, color.b as f64 / 255.0);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(0.5, 0.5, 0.5, 0.6);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    if selected {
        cr.arc(cx, cy, r + 1.5, 0.0, std::f64::consts::TAU);
        cr.set_source_rgb(0.21, 0.52, 0.89);
        cr.set_line_width(2.0);
        let _ = cr.stroke();
    }
}
