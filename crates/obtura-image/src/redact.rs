//! Redaction that is real: the pixels are replaced, so nothing of the original under the
//! rectangle can be recovered from the result, or from a file saved from it.
//!
//! * `Box` writes one flat colour.
//! * `Pixelate` replaces every block with its average colour.
//! * `Blur` first reduces the area to block averages exactly as `Pixelate` does, and only
//!   then smooths between them. A "blur" that was a mild filter over the original could
//!   be partly undone; this one keeps nothing finer than a block.
//!
//! Every output pixel therefore depends only on block averages (or on nothing, for `Box`).
//! The tests prove that by feeding in different pictures that share their averages and
//! checking that the results are identical.

use crate::{Bitmap, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redaction {
    /// A flat, opaque colour.
    Box { rgb: [u8; 3] },
    /// Square blocks of `block` pixels, each one flat.
    Pixelate { block: u32 },
    /// Smooth, built from averages of `block` x `block` pixels.
    Blur { block: u32 },
}

/// The block size that suits a rectangle: big enough that text under it cannot be read,
/// scaled to the rectangle so a small one is not erased to a single colour.
pub fn suggested_block(rect_w: u32, rect_h: u32) -> u32 {
    (rect_w.min(rect_h) / 6).clamp(6, 24)
}

/// Redacts the part of `rect` that lies inside `bitmap`. The result is fully opaque there.
pub fn redact(bitmap: &mut Bitmap, rect: Rect, mode: Redaction) {
    redact_shaped(bitmap, rect, mode, false);
}

/// Like [`redact`], but with `ellipse` only the ellipse that fits in `rect` is redacted; the
/// corners keep the original pixels. Inside the ellipse the result is exactly what
/// [`redact`] gives, so it still depends only on block averages.
pub fn redact_shaped(bitmap: &mut Bitmap, rect: Rect, mode: Redaction, ellipse: bool) {
    if !ellipse {
        return redact_rect(bitmap, rect, mode);
    }
    let Some(r) = rect.clamp_to(bitmap.width, bitmap.height) else { return };
    let original = bitmap.crop(r).expect("clamped to the picture");
    redact_rect(bitmap, rect, mode);
    // Put back what the ellipse does not cover (a pixel counts if its centre is inside).
    let (cx, cy) = (rect.x as f64 + rect.w as f64 / 2.0, rect.y as f64 + rect.h as f64 / 2.0);
    let (rx, ry) = ((rect.w as f64 / 2.0).max(0.5), (rect.h as f64 / 2.0).max(0.5));
    for y in 0..r.h {
        for x in 0..r.w {
            let (px, py) = ((r.x + x as i32) as f64 + 0.5, (r.y + y as i32) as f64 + 0.5);
            if ((px - cx) / rx).powi(2) + ((py - cy) / ry).powi(2) > 1.0 {
                bitmap.set_pixel((r.x + x as i32) as u32, (r.y + y as i32) as u32, original.pixel(x, y));
            }
        }
    }
}

fn redact_rect(bitmap: &mut Bitmap, rect: Rect, mode: Redaction) {
    let Some(r) = rect.clamp_to(bitmap.width, bitmap.height) else { return };
    match mode {
        Redaction::Box { rgb } => fill(bitmap, r, [rgb[0], rgb[1], rgb[2], 255]),
        Redaction::Pixelate { block } => {
            let grid = block_averages(bitmap, r, block.max(1));
            for y in 0..r.h {
                for x in 0..r.w {
                    let b = grid.at(x / grid.block, y / grid.block);
                    bitmap.set_pixel(r.x as u32 + x, r.y as u32 + y, b);
                }
            }
        }
        Redaction::Blur { block } => {
            let grid = block_averages(bitmap, r, block.max(1));
            for y in 0..r.h {
                for x in 0..r.w {
                    let p = grid.smooth(x, y);
                    bitmap.set_pixel(r.x as u32 + x, r.y as u32 + y, p);
                }
            }
        }
    }
}

fn fill(bitmap: &mut Bitmap, r: Rect, rgba: [u8; 4]) {
    for y in r.y as u32..r.y as u32 + r.h {
        for x in r.x as u32..r.x as u32 + r.w {
            bitmap.set_pixel(x, y, rgba);
        }
    }
}

/// Average colours of the blocks of a rectangle, as opaque pixels.
struct Grid {
    block: u32,
    cols: u32,
    rows: u32,
    cells: Vec<[u8; 4]>,
    /// Width and height of the rectangle, to find the centre of the last, smaller, blocks.
    w: u32,
    h: u32,
}

fn block_averages(bitmap: &Bitmap, r: Rect, block: u32) -> Grid {
    let (cols, rows) = (r.w.div_ceil(block), r.h.div_ceil(block));
    let mut cells = Vec::with_capacity((cols * rows) as usize);
    for by in 0..rows {
        for bx in 0..cols {
            let (x0, y0) = (bx * block, by * block);
            let (x1, y1) = ((x0 + block).min(r.w), (y0 + block).min(r.h));
            let mut sum = [0u64; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = bitmap.pixel(r.x as u32 + x, r.y as u32 + y);
                    // Straight alpha: weigh by opacity so a transparent pixel adds no colour.
                    let a = p[3] as u64;
                    for c in 0..3 {
                        sum[c] += p[c] as u64 * a;
                    }
                }
            }
            let weight: u64 = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| bitmap.pixel(r.x as u32 + x, r.y as u32 + y)[3] as u64).sum();
            let avg = |s: u64| if weight == 0 { 0 } else { ((s + weight / 2) / weight) as u8 };
            cells.push([avg(sum[0]), avg(sum[1]), avg(sum[2]), 255]);
        }
    }
    Grid { block, cols, rows, cells, w: r.w, h: r.h }
}

impl Grid {
    fn at(&self, bx: u32, by: u32) -> [u8; 4] {
        self.cells[(by.min(self.rows - 1) * self.cols + bx.min(self.cols - 1)) as usize]
    }

    /// Bilinear blend between the centres of the four nearest blocks.
    fn smooth(&self, x: u32, y: u32) -> [u8; 4] {
        let centre = |i: u32, n_px: u32| {
            let start = i * self.block;
            let end = (start + self.block).min(n_px);
            (start + end) as f64 / 2.0
        };
        // Which pair of blocks lies around this pixel along one axis, and the blend weight.
        let axis = |p: u32, n_cells: u32, n_px: u32| -> (u32, u32, f64) {
            let pos = p as f64 + 0.5;
            let i = (p / self.block).min(n_cells - 1);
            let c = centre(i, n_px);
            let (a, b) = if pos < c { (i.saturating_sub(1), i) } else { (i, (i + 1).min(n_cells - 1)) };
            if a == b {
                return (a, b, 0.0);
            }
            let (ca, cb) = (centre(a, n_px), centre(b, n_px));
            (a, b, ((pos - ca) / (cb - ca)).clamp(0.0, 1.0))
        };
        let (x0, x1, tx) = axis(x, self.cols, self.w);
        let (y0, y1, ty) = axis(y, self.rows, self.h);
        let mut out = [0u8; 4];
        for (c, o) in out.iter_mut().enumerate() {
            let top = self.at(x0, y0)[c] as f64 * (1.0 - tx) + self.at(x1, y0)[c] as f64 * tx;
            let bottom = self.at(x0, y1)[c] as f64 * (1.0 - tx) + self.at(x1, y1)[c] as f64 * tx;
            *o = (top * (1.0 - ty) + bottom * ty).round() as u8;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random picture, so tests need no random crate.
    fn noise(w: u32, h: u32, seed: u32) -> Bitmap {
        let mut b = Bitmap::filled(w, h, [0, 0, 0, 255]);
        let mut s = seed.wrapping_mul(2654435761).wrapping_add(1);
        for y in 0..h {
            for x in 0..w {
                let mut p = [0u8; 4];
                for c in p.iter_mut().take(3) {
                    s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                    *c = (s >> 24) as u8;
                }
                p[3] = 255;
                b.set_pixel(x, y, p);
            }
        }
        b
    }

    /// The same picture with the pixels inside every `block` x `block` square of `r`
    /// (counted from its corner, the last ones cut off by its edge) shuffled around: all
    /// the block averages are unchanged, but individual pixels are different.
    fn shuffle_inside_blocks(b: &Bitmap, r: Rect, block: u32) -> Bitmap {
        let mut out = b.clone();
        let (x_end, y_end) = (r.x as u32 + r.w, r.y as u32 + r.h);
        for by in (r.y as u32..y_end).step_by(block as usize) {
            for bx in (r.x as u32..x_end).step_by(block as usize) {
                let (x1, y1) = ((bx + block).min(x_end), (by + block).min(y_end));
                let coords: Vec<(u32, u32)> = (by..y1).flat_map(|y| (bx..x1).map(move |x| (x, y))).collect();
                for (i, &(x, y)) in coords.iter().enumerate() {
                    let (sx, sy) = coords[coords.len() - 1 - i]; // reversing is always a permutation
                    out.set_pixel(x, y, b.pixel(sx, sy));
                }
            }
        }
        out
    }

    /// Like `assert_eq!` for pictures, without printing every byte when it fails.
    fn assert_same(a: &[u8], b: &[u8], what: &str) {
        let differing = a.iter().zip(b).filter(|(x, y)| x != y).count();
        assert!(a.len() == b.len() && differing == 0, "{what}: {differing} bytes differ");
    }

    fn region_bytes(b: &Bitmap, r: Rect) -> Vec<u8> {
        b.crop(r).unwrap().rgba
    }

    #[test]
    fn box_erases_everything_under_it_and_nothing_else() {
        let original = noise(40, 30, 1);
        let mut b = original.clone();
        let r = Rect::new(5, 6, 20, 10);
        redact(&mut b, r, Redaction::Box { rgb: [0, 0, 0] });
        for y in 0..30 {
            for x in 0..40 {
                let inside = (5..25).contains(&x) && (6..16).contains(&y);
                assert_eq!(b.pixel(x, y), if inside { [0, 0, 0, 255] } else { original.pixel(x, y) }, "at {x},{y}");
            }
        }
    }

    #[test]
    fn box_result_does_not_depend_on_the_original_at_all() {
        let r = Rect::new(2, 2, 30, 20);
        let (mut a, mut b) = (noise(40, 30, 1), noise(40, 30, 2));
        redact(&mut a, r, Redaction::Box { rgb: [10, 20, 30] });
        redact(&mut b, r, Redaction::Box { rgb: [10, 20, 30] });
        assert_same(&region_bytes(&a, r), &region_bytes(&b, r), "result");
    }

    #[test]
    fn pixelate_keeps_only_block_averages() {
        let r = Rect::new(3, 4, 32, 24);
        let original = noise(50, 40, 3);
        let shuffled = shuffle_inside_blocks(&original, r, 8);
        assert!(region_bytes(&original, r) != region_bytes(&shuffled, r), "the test pictures must differ");
        let (mut a, mut b) = (original.clone(), shuffled);
        redact(&mut a, r, Redaction::Pixelate { block: 8 });
        redact(&mut b, r, Redaction::Pixelate { block: 8 });
        assert_same(&region_bytes(&a, r), &region_bytes(&b, r), "pixelate: pictures with equal block averages");
        // At most one colour per block: 4 x 3 blocks.
        let mut colours: Vec<[u8; 4]> = (0..r.h).flat_map(|y| (0..r.w).map(move |x| (x, y))).map(|(x, y)| a.pixel(3 + x, 4 + y)).collect();
        colours.sort();
        colours.dedup();
        assert!(colours.len() <= 12, "{} colours", colours.len());
    }

    #[test]
    fn blur_keeps_only_block_averages_too() {
        let r = Rect::new(3, 4, 32, 24);
        let original = noise(50, 40, 4);
        let shuffled = shuffle_inside_blocks(&original, r, 8);
        assert!(region_bytes(&original, r) != region_bytes(&shuffled, r), "the test pictures must differ");
        let (mut a, mut b) = (original, shuffled);
        redact(&mut a, r, Redaction::Blur { block: 8 });
        redact(&mut b, r, Redaction::Blur { block: 8 });
        assert_same(&region_bytes(&a, r), &region_bytes(&b, r), "result");
    }

    #[test]
    fn blur_of_odd_sized_areas_also_keeps_only_block_averages() {
        // Partial blocks at the right and bottom edges.
        let r = Rect::new(1, 2, 37, 19);
        let original = noise(50, 40, 5);
        let shuffled = shuffle_inside_blocks(&original, r, 10);
        let (mut a, mut b) = (original, shuffled);
        redact(&mut a, r, Redaction::Blur { block: 10 });
        redact(&mut b, r, Redaction::Blur { block: 10 });
        assert_same(&region_bytes(&a, r), &region_bytes(&b, r), "result");
    }

    #[test]
    fn text_like_detail_is_gone_after_blur_and_pixelate() {
        // Black "text" strokes on white: after redaction no pixel may be pure black or
        // pure white inside the area (every pixel is an average over a block).
        let mut b = Bitmap::filled(60, 30, [255, 255, 255, 255]);
        for y in 0..30 {
            for x in (0..60).step_by(3) {
                b.set_pixel(x, y, [0, 0, 0, 255]);
            }
        }
        for mode in [Redaction::Pixelate { block: 12 }, Redaction::Blur { block: 12 }] {
            let mut c = b.clone();
            redact(&mut c, Rect::new(0, 0, 60, 30), mode);
            let extremes = (0..30u32).flat_map(|y| (0..60u32).map(move |x| (x, y))).filter(|&(x, y)| matches!(c.pixel(x, y)[0], 0 | 255)).count();
            assert_eq!(extremes, 0, "{mode:?} left original-looking pixels");
        }
    }

    #[test]
    fn a_round_redaction_leaves_the_corners_alone_and_still_keeps_only_block_averages() {
        let r = Rect::new(3, 4, 32, 24);
        let original = noise(50, 40, 8);
        let shuffled = shuffle_inside_blocks(&original, r, 8);
        let inside_ellipse = |x: u32, y: u32| {
            let (cx, cy) = (3.0 + 16.0, 4.0 + 12.0);
            (((x as f64 + 0.5 - cx) / 16.0).powi(2) + ((y as f64 + 0.5 - cy) / 12.0).powi(2)) <= 1.0
        };
        for mode in [Redaction::Pixelate { block: 8 }, Redaction::Blur { block: 8 }, Redaction::Box { rgb: [9, 9, 9] }] {
            let (mut a, mut b) = (original.clone(), shuffled.clone());
            redact_shaped(&mut a, r, mode, true);
            redact_shaped(&mut b, r, mode, true);
            let mut redacted = 0;
            for y in 0..40 {
                for x in 0..50 {
                    let in_rect = (3..35).contains(&x) && (4..28).contains(&y);
                    if in_rect && inside_ellipse(x, y) {
                        redacted += 1;
                        // Inside the ellipse: the same for pictures that share block averages.
                        assert_eq!(a.pixel(x, y), b.pixel(x, y), "{mode:?} at {x},{y} still depends on the original pixels");
                    } else {
                        assert_eq!(a.pixel(x, y), original.pixel(x, y), "{mode:?} at {x},{y}: outside the ellipse nothing changes");
                    }
                }
            }
            assert!(redacted > 400, "the ellipse covers most of the middle: {redacted}");
        }
    }

    #[test]
    fn only_the_rectangle_changes() {
        let original = noise(40, 30, 6);
        for mode in [Redaction::Pixelate { block: 6 }, Redaction::Blur { block: 6 }] {
            let mut b = original.clone();
            redact(&mut b, Rect::new(10, 10, 12, 8), mode);
            for y in 0..30 {
                for x in 0..40 {
                    if !((10..22).contains(&x) && (10..18).contains(&y)) {
                        assert_eq!(b.pixel(x, y), original.pixel(x, y));
                    }
                }
            }
        }
    }

    #[test]
    fn a_rectangle_partly_outside_is_clamped_and_one_wholly_outside_is_ignored() {
        let original = noise(20, 20, 7);
        let mut b = original.clone();
        redact(&mut b, Rect::new(15, 15, 50, 50), Redaction::Box { rgb: [1, 2, 3] });
        assert_eq!(b.pixel(19, 19), [1, 2, 3, 255]);
        assert_eq!(b.pixel(14, 14), original.pixel(14, 14));
        let mut c = original.clone();
        redact(&mut c, Rect::new(100, 100, 5, 5), Redaction::Box { rgb: [1, 2, 3] });
        assert_eq!(c, original);
    }

    #[test]
    fn the_suggested_block_scales_with_the_area() {
        assert_eq!(suggested_block(10, 10), 6);
        assert_eq!(suggested_block(120, 60), 10);
        assert_eq!(suggested_block(2000, 2000), 24);
    }
}
