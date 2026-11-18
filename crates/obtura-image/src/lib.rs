//! Pixel operations: decode and encode, crop, and later blur, pixelate, resize, trim,
//! stitch, colour, animated GIF, QR codes. No GTK, no portals.
//!
//! Pictures are [`Bitmap`]s: 8-bit RGBA, straight (not premultiplied) alpha, row by row,
//! top to bottom.

mod bitmap;
pub mod gif;
mod ops;
mod png;
mod qr;
mod redact;
pub mod stitch;

pub use bitmap::{Bitmap, Rect};
pub use ops::{blank, combine, cut_band, extend, Align, Arrange, flip_horizontal, flip_vertical, join, paste, resize, rotate_ccw, rotate_cw, soften, trim_bounds};
pub use redact::{redact, redact_shaped, suggested_block, Redaction};
pub use png::{decode_png, encode_png, ImageError};
pub use qr::{decode_qr, QrCode};
