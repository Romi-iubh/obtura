//! Scene + base picture to pixels, for the live view and for export. Uses cairo and pango
//! (drawing libraries, not GTK).
//!
//! Marks are drawn in stacking order on top of the base picture. A redaction (blur,
//! pixelate, box) is applied to everything under it at that point in the order, marks
//! included, so nothing below a redaction can be read in the result.

mod compose;
mod draw;
mod frame;
mod surface;

pub use compose::{blank_redactions, compose, picture_to_read, render, render_on, render_on_canvas, underlay_len, MARGIN_FILL};
pub use frame::{apply_frame, backdrop, card_path, draw_border, picture_path};
pub use draw::{draw_objects, is_emoji, measure_text, EMOJI_FAMILY};
pub use surface::{from_surface, to_surface};
