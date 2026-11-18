/// An integer rectangle in pixels. `x` and `y` may be negative only before clamping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// The part of this rectangle that lies inside a `width` x `height` picture, or `None`
    /// if nothing of it does.
    pub fn clamp_to(self, width: u32, height: u32) -> Option<Rect> {
        let x0 = self.x.max(0) as i64;
        let y0 = self.y.max(0) as i64;
        let x1 = (self.x as i64 + self.w as i64).min(width as i64);
        let y1 = (self.y as i64 + self.h as i64).min(height as i64);
        (x1 > x0 && y1 > y0).then(|| Rect { x: x0 as i32, y: y0 as i32, w: (x1 - x0) as u32, h: (y1 - y0) as u32 })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes: R, G, B, A.
    pub rgba: Vec<u8>,
}

impl Bitmap {
    /// A picture of one colour.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width as usize * height as usize {
            data.extend_from_slice(&rgba);
        }
        Self { width, height, rgba: data }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, p: [u8; 4]) {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba[i..i + 4].copy_from_slice(&p);
    }

    /// A copy of the part inside `rect` (clamped to the picture), or `None` if it is empty.
    pub fn crop(&self, rect: Rect) -> Option<Bitmap> {
        let r = rect.clamp_to(self.width, self.height)?;
        let mut data = Vec::with_capacity(r.w as usize * r.h as usize * 4);
        for y in r.y as usize..(r.y as usize + r.h as usize) {
            let start = (y * self.width as usize + r.x as usize) * 4;
            data.extend_from_slice(&self.rgba[start..start + r.w as usize * 4]);
        }
        Some(Bitmap { width: r.w, height: r.h, rgba: data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Bitmap {
        let mut b = Bitmap::filled(w, h, [0, 0, 0, 255]);
        for y in 0..h {
            for x in 0..w {
                b.set_pixel(x, y, [x as u8, y as u8, (x + y) as u8, 255]);
            }
        }
        b
    }

    #[test]
    fn crop_copies_the_right_pixels() {
        let b = gradient(10, 8);
        let c = b.crop(Rect::new(3, 2, 4, 5)).unwrap();
        assert_eq!((c.width, c.height), (4, 5));
        assert_eq!(c.pixel(0, 0), b.pixel(3, 2));
        assert_eq!(c.pixel(3, 4), b.pixel(6, 6));
    }

    #[test]
    fn crop_is_clamped_to_the_picture() {
        let b = gradient(10, 8);
        let c = b.crop(Rect::new(-5, 6, 8, 10)).unwrap();
        assert_eq!((c.width, c.height), (3, 2));
        assert_eq!(c.pixel(0, 0), b.pixel(0, 6));
    }

    #[test]
    fn crop_outside_or_empty_is_none() {
        let b = gradient(10, 8);
        assert!(b.crop(Rect::new(10, 0, 3, 3)).is_none());
        assert!(b.crop(Rect::new(2, 2, 0, 3)).is_none());
        assert!(b.crop(Rect::new(-9, -9, 5, 5)).is_none());
    }
}
