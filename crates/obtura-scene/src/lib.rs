//! The document: a base picture plus marks (objects) drawn over it, with undo and redo.
//! Marks are never painted into the base picture until export. No GTK.

mod document;
mod frame;
mod geometry;
mod object;
mod scene;

pub use document::Document;
pub use frame::{content_rect, picture, register_picture, set_picture_loader, Background, Bar, Device, Frame, FrameLayout, PictureKey, TORN_DEPTH};
pub use geometry::{flatten_pieces, flatten_quad, line_pieces, nearest_side_midpoint, quad_at, quad_control, rotate_point, rotated_bounds, Cubic, Point, RectF};
pub use object::{normal_angle, t_back_from_end, Cap, Color, Dash, FamilyName, Font, Id, Kind, Object, Pixels, RedactMode, Shape, Style, CAPS, FONTS};
pub use scene::{canvas_of, Scene, MAX_GROWTH};
