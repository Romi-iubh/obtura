use crate::Bitmap;
use image::{ImageEncoder, ImageFormat};

#[derive(Debug)]
pub struct ImageError(pub String);

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImageError {}

pub fn decode_png(bytes: &[u8]) -> Result<Bitmap, ImageError> {
    let img = image::load_from_memory_with_format(bytes, ImageFormat::Png).map_err(|e| ImageError(format!("cannot read the PNG: {e}")))?;
    let rgba = img.into_rgba8();
    Ok(Bitmap { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
}

pub fn encode_png(bitmap: &Bitmap) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&bitmap.rgba, bitmap.width, bitmap.height, image::ExtendedColorType::Rgba8)
        .map_err(|e| ImageError(format!("cannot write the PNG: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/sample.png");

    #[test]
    fn decodes_the_fixture() {
        let b = decode_png(FIXTURE).unwrap();
        assert_eq!((b.width, b.height), (800, 500));
        assert_eq!(b.rgba.len(), 800 * 500 * 4);
    }

    #[test]
    fn encode_then_decode_is_lossless() {
        let b = decode_png(FIXTURE).unwrap();
        assert_eq!(decode_png(&encode_png(&b).unwrap()).unwrap(), b);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode_png(b"definitely not a png").is_err());
    }
}
