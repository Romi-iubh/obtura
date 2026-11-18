use cairo::{Format, ImageSurface};
use obtura_image::Bitmap;

/// A cairo surface holding `bitmap` (cairo keeps colours premultiplied by alpha).
pub fn to_surface(bitmap: &Bitmap) -> Result<ImageSurface, String> {
    let (w, h) = (bitmap.width as i32, bitmap.height as i32);
    let mut surface = ImageSurface::create(Format::ARgb32, w, h).map_err(|e| format!("cannot make a {w} x {h} surface: {e}"))?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().map_err(|e| e.to_string())?;
        for y in 0..bitmap.height as usize {
            let src = &bitmap.rgba[y * bitmap.width as usize * 4..][..bitmap.width as usize * 4];
            let dst = &mut data[y * stride..][..bitmap.width as usize * 4];
            for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                let a = s[3] as u32;
                let premul = |c: u8| ((c as u32 * a + 127) / 255) as u32;
                let v = (a << 24) | (premul(s[0]) << 16) | (premul(s[1]) << 8) | premul(s[2]);
                d.copy_from_slice(&v.to_ne_bytes());
            }
        }
    }
    Ok(surface)
}

/// The picture on a cairo surface, with straight (not premultiplied) alpha.
pub fn from_surface(surface: &mut ImageSurface) -> Result<Bitmap, String> {
    surface.flush();
    let (w, h, stride) = (surface.width() as usize, surface.height() as usize, surface.stride() as usize);
    let data = surface.data().map_err(|e| e.to_string())?;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for px in data[y * stride..][..w * 4].chunks_exact(4) {
            let v = u32::from_ne_bytes([px[0], px[1], px[2], px[3]]);
            let a = v >> 24;
            let un = |c: u32| if a == 0 { 0 } else { ((c * 255 + a / 2) / a).min(255) as u8 };
            rgba.extend_from_slice(&[un((v >> 16) & 255), un((v >> 8) & 255), un(v & 255), a as u8]);
        }
    }
    Ok(Bitmap { width: w as u32, height: h as u32, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_pictures_survive_the_round_trip_exactly() {
        let mut b = Bitmap::filled(7, 5, [0, 0, 0, 255]);
        for y in 0..5 {
            for x in 0..7 {
                b.set_pixel(x, y, [(x * 36) as u8, (y * 50) as u8, (x * y * 7) as u8, 255]);
            }
        }
        let mut s = to_surface(&b).unwrap();
        assert_eq!(from_surface(&mut s).unwrap(), b);
    }

    #[test]
    fn transparency_is_kept() {
        let b = Bitmap::filled(2, 2, [200, 100, 50, 128]);
        let mut s = to_surface(&b).unwrap();
        let back = from_surface(&mut s).unwrap();
        let p = back.pixel(0, 0);
        assert_eq!(p[3], 128);
        assert!(p[0].abs_diff(200) <= 2 && p[1].abs_diff(100) <= 2 && p[2].abs_diff(50) <= 2, "{p:?}");
    }
}
