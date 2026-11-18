//! The tools of the editor: what each one is called, its key, its icon and which style
//! settings it uses.

use obtura_scene::{Cap, Color, Kind, RedactMode, Shape, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Select,
    Line,
    Spline,
    Rect,
    Ellipse,
    Pen,
    Highlighter,
    Text,
    Callout,
    Step,
    Stamp,
    Spotlight,
    Magnify,
    Blur,
    Pixelate,
    Box,
    Crop,
    /// Removes a band across the picture and joins the rest.
    CutOut,
    /// Reads the text in an area (or the whole picture) and copies it.
    TextGrab,
    /// Shows a pixel's colour under the pointer; a click copies it.
    Picker,
    /// A dimension line showing its length in pixels.
    Measure,
    /// Not a button: what a picture placed on the screenshot is formatted as.
    Picture,
}

/// The tools in the toolbar, in order.
pub const ALL: [Tool; 21] = [
    Tool::Select,
    Tool::Line,
    Tool::Spline,
    Tool::Rect,
    Tool::Ellipse,
    Tool::Pen,
    Tool::Highlighter,
    Tool::Text,
    Tool::Callout,
    Tool::Step,
    Tool::Stamp,
    Tool::Spotlight,
    Tool::Magnify,
    Tool::Blur,
    Tool::Pixelate,
    Tool::Box,
    Tool::Crop,
    Tool::CutOut,
    Tool::TextGrab,
    Tool::Picker,
    Tool::Measure,
];

/// Which colour of a style the quick palette changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Lines and outlines.
    Line,
    /// The inside: a shape, a step badge.
    Fill,
    /// The lettering.
    Text,
}

/// Controls only shown under "More formatting".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct More {
    /// Each colour control, with its label.
    pub line_color: Option<&'static str>,
    pub fill_color: Option<&'static str>,
    pub text_color: Option<&'static str>,
    /// A thickness for a border or an outline, with its label.
    pub border: Option<&'static str>,
    pub opacity: bool,
    pub dash: bool,
    pub corner: bool,
    pub cap_size: bool,
    pub cap_filled: bool,
    /// Font family, bold and italic.
    pub font: bool,
    /// A separate size slider (the number inside a step badge), with its label.
    pub font_size: Option<&'static str>,
    /// The shape of a step badge.
    pub badge_shape: bool,
    pub strength: bool,
}

impl More {
    pub fn any(&self) -> bool {
        *self != More::default()
    }
}

/// Which controls the Formatting panel shows. The quick ones come first; the rest are
/// behind "More formatting".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fields {
    pub palette: Option<Slot>,
    pub thickness: Option<&'static str>,
    /// A size slider (text size, or the size of a step badge), with its label.
    pub size: Option<&'static str>,
    pub filled: bool,
    /// What the line starts and ends with.
    pub ends: bool,
    /// A redaction's shape: rectangle or ellipse.
    pub redact_shape: bool,
    /// How dark a spotlight's shade is.
    pub darkness: bool,
    /// The grid of emoji and symbols.
    pub stamps: bool,
    /// The shape of the crop rectangle (free or a ratio).
    pub crop_ratio: bool,
    pub more: More,
}

impl Fields {
    pub fn any(&self) -> bool {
        self.quick() || self.more.any()
    }

    /// Whether any of the quick controls shows.
    pub fn quick(&self) -> bool {
        self.palette.is_some() || self.thickness.is_some() || self.size.is_some() || self.filled || self.ends || self.redact_shape || self.darkness || self.stamps || self.crop_ratio
    }
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select and Move",
            Tool::Line => "Line and Arrow",
            Tool::Spline => "Curved Line and Arrow",
            Tool::Rect => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Pen => "Pen",
            Tool::Highlighter => "Highlighter",
            Tool::Text => "Text",
            Tool::Callout => "Callout",
            Tool::Step => "Numbered Step",
            Tool::Stamp => "Emoji and Symbols",
            Tool::Spotlight => "Spotlight (dim the rest)",
            Tool::Magnify => "Magnifier",
            Tool::Picture => "Picture",
            Tool::Blur => "Blur",
            Tool::Pixelate => "Pixelate",
            Tool::Box => "Cover with a Box",
            Tool::Crop => "Crop",
            Tool::CutOut => "Cut Out a Strip",
            Tool::TextGrab => "Copy Text (drag over it, or click for all)",
            Tool::Picker => "Colour Picker (click copies the colour)",
            Tool::Measure => "Measure",
        }
    }

    /// The key that picks the tool (shown in its tooltip).
    pub fn key(self) -> char {
        match self {
            Tool::Select => 'V',
            Tool::Line => 'L',
            Tool::Spline => 'S',
            Tool::Rect => 'R',
            Tool::Ellipse => 'E',
            Tool::Pen => 'P',
            Tool::Highlighter => 'H',
            Tool::Text => 'T',
            Tool::Callout => 'K',
            Tool::Step => 'N',
            Tool::Stamp => 'M',
            Tool::Spotlight => 'D',
            Tool::Magnify => 'Z',
            Tool::Picture => '\0',
            Tool::Blur => 'B',
            Tool::Pixelate => 'X',
            Tool::Box => 'O',
            Tool::Crop => 'C',
            Tool::CutOut => 'U',
            Tool::TextGrab => 'G',
            Tool::Picker => 'I',
            Tool::Measure => 'Q',
        }
    }

    /// The name the tool's saved style is kept under.
    pub fn id(self) -> String {
        format!("{self:?}")
    }

    /// The tool a saved style belongs to.
    pub fn from_id(id: &str) -> Option<Tool> {
        ALL.into_iter().chain([Tool::Picture]).find(|t| t.id() == id)
    }

    /// The tool a key picks. `A` still picks the line tool, as it picked the arrow tool.
    pub fn from_key(c: char) -> Option<Tool> {
        let c = c.to_ascii_uppercase();
        if c == 'A' {
            return Some(Tool::Line);
        }
        ALL.into_iter().find(|t| t.key() == c)
    }

    /// Tools of the same group sit together in the toolbar.
    pub fn group(self) -> u8 {
        match self {
            Tool::Select => 0,
            Tool::Line | Tool::Spline | Tool::Rect | Tool::Ellipse | Tool::Pen | Tool::Highlighter => 1,
            Tool::Text | Tool::Callout | Tool::Step | Tool::Stamp => 2,
            Tool::Spotlight | Tool::Magnify | Tool::Picture => 3,
            Tool::Blur | Tool::Pixelate | Tool::Box => 4,
            Tool::Crop | Tool::CutOut => 5,
            Tool::TextGrab | Tool::Picker | Tool::Measure => 6,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Line => "line",
            Tool::Spline => "spline",
            Tool::Rect => "rect",
            Tool::Ellipse => "ellipse",
            Tool::Pen => "pen",
            Tool::Highlighter => "highlighter",
            Tool::Text => "text",
            Tool::Callout => "callout",
            Tool::Step => "step",
            Tool::Stamp => "stamp",
            Tool::Spotlight => "spotlight",
            Tool::Magnify => "magnify",
            Tool::Picture => "picture",
            Tool::Blur => "blur",
            Tool::Pixelate => "pixelate",
            Tool::Box => "box",
            Tool::Crop => "crop",
            Tool::CutOut => "cut",
            Tool::TextGrab => "ocr",
            Tool::Picker => "picker",
            Tool::Measure => "measure",
        }
    }

    pub fn fields(self) -> Fields {
        match self {
            Tool::Line | Tool::Spline => Fields {
                palette: Some(Slot::Line),
                thickness: Some("Thickness"),
                ends: true,
                more: More { line_color: Some("Line colour"), dash: true, cap_size: true, cap_filled: true, opacity: true, ..More::default() },
                ..Fields::default()
            },
            Tool::Rect => Fields {
                palette: Some(Slot::Line),
                thickness: Some("Thickness"),
                filled: true,
                more: More { line_color: Some("Outline colour"), fill_color: Some("Fill colour"), corner: true, dash: true, opacity: true, ..More::default() },
                ..Fields::default()
            },
            Tool::Ellipse => Fields {
                palette: Some(Slot::Line),
                thickness: Some("Thickness"),
                filled: true,
                more: More { line_color: Some("Outline colour"), fill_color: Some("Fill colour"), dash: true, opacity: true, ..More::default() },
                ..Fields::default()
            },
            Tool::Pen | Tool::Highlighter => Fields { palette: Some(Slot::Line), thickness: Some("Thickness"), more: More { line_color: Some("Colour"), opacity: true, ..More::default() }, ..Fields::default() },
            Tool::Text => Fields {
                palette: Some(Slot::Text),
                size: Some("Text size"),
                more: More { text_color: Some("Text colour"), font: true, fill_color: Some("Background"), line_color: Some("Border colour"), border: Some("Border thickness"), corner: true, opacity: true, ..More::default() },
                ..Fields::default()
            },
            Tool::Callout => Fields {
                palette: Some(Slot::Line),
                size: Some("Text size"),
                more: More { line_color: Some("Border and line colour"), text_color: Some("Text colour"), fill_color: Some("Background"), border: Some("Border and line thickness"), corner: true, font: true, opacity: true, ..More::default() },
                ..Fields::default()
            },
            Tool::Step => Fields {
                palette: Some(Slot::Fill),
                size: Some("Badge size"),
                more: More {
                    fill_color: Some("Badge colour"),
                    line_color: Some("Outline colour"),
                    border: Some("Outline thickness"),
                    badge_shape: true,
                    text_color: Some("Number colour"),
                    font: true,
                    font_size: Some("Number size"),
                    opacity: true,
                    ..More::default()
                },
                ..Fields::default()
            },
            Tool::Stamp => Fields { stamps: true, size: Some("Size"), more: More { line_color: Some("Colour of symbols"), opacity: true, ..More::default() }, ..Fields::default() },
            Tool::Spotlight => Fields { redact_shape: true, darkness: true, more: More { line_color: Some("Shade colour"), ..More::default() }, ..Fields::default() },
            Tool::Magnify => Fields { palette: Some(Slot::Line), thickness: Some("Border thickness"), redact_shape: true, ..Fields::default() },
            Tool::Picture => Fields { thickness: Some("Border thickness"), palette: Some(Slot::Line), more: More { line_color: Some("Border colour"), corner: true, opacity: true, ..More::default() }, ..Fields::default() },
            Tool::Box => Fields { palette: Some(Slot::Line), redact_shape: true, more: More { line_color: Some("Box colour"), ..More::default() }, ..Fields::default() },
            Tool::Blur | Tool::Pixelate => Fields { redact_shape: true, more: More { strength: true, ..More::default() }, ..Fields::default() },
            Tool::Crop => Fields { crop_ratio: true, ..Fields::default() },
            Tool::Measure => Fields { palette: Some(Slot::Line), thickness: Some("Thickness"), size: Some("Text size"), more: More { line_color: Some("Line colour"), text_color: Some("Text colour"), fill_color: Some("Label background"), opacity: true, ..More::default() }, ..Fields::default() },
            Tool::Select | Tool::CutOut | Tool::TextGrab | Tool::Picker => Fields::default(),
        }
    }

    /// The style a new mark of this tool starts with.
    pub fn default_style(self) -> Style {
        let base = Style::default();
        let white = Color::WHITE;
        match self {
            Tool::Line | Tool::Spline => Style { end: Cap::Arrow, ..base },
            Tool::Highlighter => Style { color: Color::rgb(0xf5, 0xc2, 0x11), width: 22.0, opacity: 0.4, ..base },
            Tool::Text => Style { width: 0.0, corner: 4.0, ..base },
            Tool::Callout => Style { fill: Some(white), width: 3.0, text_color: Color::rgb(0x1e, 0x1e, 0x1e), corner: 6.0, font_size: 22.0, bold: false, ..base },
            Tool::Step => Style { shape: Shape::Ellipse, fill: Some(base.color), color: white, width: 0.0, text_color: white, font_size: 24.0, badge: 21.0, ..base },
            Tool::Box => Style { color: Color::BLACK, ..base },
            Tool::Stamp => Style { font_size: 48.0, ..base },
            Tool::Spotlight => Style { color: Color::BLACK, opacity: 0.6, ..base },
            Tool::Magnify => Style { shape: Shape::Ellipse, color: Color::rgb(0x3d, 0x38, 0x46), width: 4.0, ..base },
            Tool::Picture => Style { width: 0.0, color: Color::rgb(0x3d, 0x38, 0x46), ..base },
            Tool::Measure => Style { width: 2.0, color: Color::rgb(0x1c, 0x71, 0xd8), text_color: white, fill: Some(Color::rgb(0x1c, 0x71, 0xd8)), font_size: 15.0, start: Cap::Bar, end: Cap::Bar, ..base },
            _ => base,
        }
    }
}

/// The tool that draws marks of this kind (so a change to the style of a mark can also become
/// the tool's style).
pub fn tool_for(kind: &Kind) -> Tool {
    match kind {
        Kind::Line { via, .. } if via.is_empty() => Tool::Line,
        Kind::Line { .. } => Tool::Spline,
        Kind::Rect { .. } => Tool::Rect,
        Kind::Ellipse { .. } => Tool::Ellipse,
        Kind::Pen { .. } => Tool::Pen,
        Kind::Highlighter { .. } => Tool::Highlighter,
        Kind::Text { .. } => Tool::Text,
        Kind::Callout { .. } => Tool::Callout,
        Kind::Step { .. } => Tool::Step,
        Kind::Redact { mode: RedactMode::Blur, .. } => Tool::Blur,
        Kind::Redact { mode: RedactMode::Pixelate, .. } => Tool::Pixelate,
        Kind::Redact { mode: RedactMode::Box, .. } => Tool::Box,
        Kind::Stamp { .. } => Tool::Stamp,
        Kind::Spotlight { .. } => Tool::Spotlight,
        Kind::Magnify { .. } => Tool::Magnify,
        Kind::Image { .. } => Tool::Picture,
        Kind::Measure { .. } => Tool::Measure,
    }
}

/// The controls that apply to a mark that already exists.
pub fn fields_of(kind: &Kind) -> Fields {
    tool_for(kind).fields()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_its_own_key_and_it_finds_the_tool_back() {
        let mut keys: Vec<char> = ALL.iter().map(|t| t.key()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), ALL.len());
        for t in ALL {
            assert_eq!(Tool::from_key(t.key()), Some(t));
            assert_eq!(Tool::from_key(t.key().to_ascii_lowercase()), Some(t));
        }
        assert_eq!(Tool::from_key('?'), None);
        assert_eq!(Tool::from_key('a'), Some(Tool::Line), "the old arrow key still works");
    }

    #[test]
    fn a_new_arrow_has_a_head_and_a_new_step_is_a_filled_badge() {
        assert_eq!(Tool::Line.default_style().end, Cap::Arrow);
        assert_eq!(Tool::Spline.default_style().end, Cap::Arrow);
        let step = Tool::Step.default_style();
        assert!(step.fill.is_some() && step.shape == Shape::Ellipse);
    }

    #[test]
    fn every_drawing_tool_offers_more_formatting_and_the_others_nothing() {
        for t in ALL {
            let f = t.fields();
            match t {
                Tool::Select | Tool::CutOut | Tool::TextGrab | Tool::Picker => assert!(!f.any(), "{t:?}"),
                Tool::Crop => assert!(f.crop_ratio && !f.more.any(), "the crop tool offers only its shape"),
                Tool::Picture => unreachable!("not in the toolbar"),
                _ => assert!(f.any(), "{t:?} has nothing to format"),
            }
        }
        assert!(Tool::Step.fields().more.badge_shape && Tool::Step.fields().more.font_size.is_some());
        assert!(Tool::Text.fields().more.fill_color.is_some() && Tool::Text.fields().more.border.is_some());
        assert!(Tool::Blur.fields().redact_shape);
    }

    #[test]
    fn a_mark_finds_its_tool_back() {
        use obtura_scene::Point;
        let p = Point::new(0.0, 0.0);
        assert_eq!(tool_for(&Kind::Line { from: p, to: p, via: vec![] }), Tool::Line);
        assert_eq!(tool_for(&Kind::Line { from: p, to: p, via: vec![p] }), Tool::Spline);
    }
}
