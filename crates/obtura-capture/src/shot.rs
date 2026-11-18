use crate::CaptureError;

/// A captured picture: PNG bytes as the system gave them, and its size in physical pixels.
#[derive(Clone, Debug)]
pub struct Shot {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Shot {
    /// Wraps PNG bytes, reading the size from the header. Fails if they are not a PNG.
    pub fn from_png(png: Vec<u8>) -> Result<Self, CaptureError> {
        let (width, height) = png_size(&png).ok_or_else(|| CaptureError::Failed("the picture is not a PNG file".into()))?;
        Ok(Self { png, width, height })
    }
}

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Width and height from the PNG header (IHDR), or `None` if the bytes are not a PNG.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || png[..8] != SIGNATURE || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}
