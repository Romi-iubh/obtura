//! Pictures used as a Beautify background, kept in `~/.local/share/obtura/backgrounds/` as
//! `<key>.png` (the key names the pixels, see `obtura_scene::PictureKey`). A frame keeps only
//! the key, so a saved frame stays small and the same picture is stored once. Obtura never
//! removes these files: a frame saved in the library may still use one.

use crate::paths;
use obtura_image::{decode_png, encode_png, Bitmap};
use obtura_scene::PictureKey;
use std::path::PathBuf;

/// The largest side a background picture is kept at (bigger ones are made smaller first).
pub const MAX_SIDE: u32 = 3840;

pub fn dir() -> PathBuf {
    paths::data_dir().join("backgrounds")
}

fn file(key: &PictureKey) -> PathBuf {
    dir().join(format!("{}.png", key.hex()))
}

/// Keeps `picture` (made smaller if it is huge) and returns its key.
pub fn add(picture: &Bitmap) -> std::io::Result<PictureKey> {
    add_in(&dir(), picture)
}

fn add_in(dir: &std::path::Path, picture: &Bitmap) -> std::io::Result<PictureKey> {
    let k = (MAX_SIDE as f64 / picture.width.max(picture.height).max(1) as f64).min(1.0);
    let picture = if k < 1.0 { obtura_image::resize(picture, ((picture.width as f64 * k).round() as u32).max(1), ((picture.height as f64 * k).round() as u32).max(1)) } else { picture.clone() };
    let key = obtura_scene::register_picture(picture.clone());
    let path = dir.join(format!("{}.png", key.hex()));
    if !path.exists() {
        std::fs::create_dir_all(dir)?;
        let png = encode_png(&picture).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("png.tmp");
        std::fs::write(&tmp, png)?;
        std::fs::rename(tmp, &path)?;
    }
    Ok(key)
}

/// Reads a kept picture (the loader given to `obtura_scene::set_picture_loader`).
pub fn load(key: &PictureKey) -> Option<Bitmap> {
    decode_png(&std::fs::read(file(key)).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_kept_once_under_its_key_and_read_back() {
        let dir = std::env::temp_dir().join(format!("obtura-backgrounds-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let b = Bitmap::filled(4, 3, [1, 2, 3, 255]);
        let key = add_in(&dir, &b).unwrap();
        let again = add_in(&dir, &b).unwrap();
        assert_eq!(key, again);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let png = std::fs::read(dir.join(format!("{}.png", key.hex()))).unwrap();
        assert_eq!(decode_png(&png).unwrap(), b);
        // A huge picture is kept smaller.
        let big = add_in(&dir, &Bitmap::filled(MAX_SIDE * 2, 10, [0, 0, 0, 255])).unwrap();
        assert_eq!(obtura_scene::picture(&big).unwrap().width, MAX_SIDE);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
