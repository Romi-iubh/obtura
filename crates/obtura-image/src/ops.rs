//! Whole-picture operations: soft blur (for shadows), resize, quarter turns, flips, cutting a
//! strip out, adding space around, and placing two pictures side by side.

use crate::Bitmap;

/// Softens the picture with three box blurs of `radius` (close to a Gaussian). Used for
/// shadows; not a redaction (see [`crate::redact`] for that).
pub fn soften(b: &mut Bitmap, radius: u32) {
    if radius == 0 || b.width == 0 || b.height == 0 {
        return;
    }
    for _ in 0..3 {
        box_pass(b, radius, true);
        box_pass(b, radius, false);
    }
}

/// One running-sum box blur along rows (`horizontal`) or columns. Edges repeat the last pixel.
fn box_pass(b: &mut Bitmap, r: u32, horizontal: bool) {
    let (w, h) = (b.width as usize, b.height as usize);
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    let r = r as usize;
    let win = (2 * r + 1) as u32;
    let mut line = vec![[0u8; 4]; len];
    for l in 0..lines {
        let idx = |i: usize| if horizontal { (l * w + i) * 4 } else { (i * w + l) * 4 };
        for (i, px) in line.iter_mut().enumerate() {
            let k = idx(i);
            px.copy_from_slice(&b.rgba[k..k + 4]);
        }
        let at = |i: isize| line[i.clamp(0, len as isize - 1) as usize];
        let mut sum = [0u32; 4];
        for i in -(r as isize)..=(r as isize) {
            let p = at(i);
            for c in 0..4 {
                sum[c] += p[c] as u32;
            }
        }
        for i in 0..len {
            let k = idx(i);
            for c in 0..4 {
                b.rgba[k + c] = ((sum[c] + win / 2) / win) as u8;
            }
            let (out, inn) = (at(i as isize - r as isize), at(i as isize + r as isize + 1));
            for c in 0..4 {
                sum[c] = sum[c] + inn[c] as u32 - out[c] as u32;
            }
        }
    }
}

/// The picture at a new size. Shrinking averages every source pixel that falls in a target
/// pixel (no jagged text); enlarging interpolates between neighbours.
pub fn resize(b: &Bitmap, new_w: u32, new_h: u32) -> Bitmap {
    let (new_w, new_h) = (new_w.max(1), new_h.max(1));
    if (new_w, new_h) == (b.width, b.height) {
        return b.clone();
    }
    let horizontal = resample_axis(b, new_w, true);
    resample_axis(&horizontal, new_h, false)
}

fn resample_axis(b: &Bitmap, new_len: u32, horizontal: bool) -> Bitmap {
    let (w, h) = (b.width as usize, b.height as usize);
    let old_len = if horizontal { w } else { h };
    let (out_w, out_h) = if horizontal { (new_len as usize, h) } else { (w, new_len as usize) };
    let mut out = Bitmap::filled(out_w as u32, out_h as u32, [0, 0, 0, 0]);
    let lines = if horizontal { h } else { w };
    let scale = old_len as f64 / new_len as f64;
    // Weights per target pixel: (first source pixel, weights), with premultiplied alpha.
    let taps: Vec<(usize, Vec<f64>)> = (0..new_len as usize)
        .map(|i| {
            if scale >= 1.0 {
                let (start, end) = (i as f64 * scale, (i as f64 + 1.0) * scale);
                let first = start.floor() as usize;
                let last = (end.ceil() as usize).min(old_len);
                let ws = (first..last).map(|s| (end.min(s as f64 + 1.0) - start.max(s as f64)).max(0.0)).collect();
                (first, ws)
            } else {
                let x = ((i as f64 + 0.5) * scale - 0.5).clamp(0.0, (old_len - 1) as f64);
                let first = x.floor() as usize;
                let t = x - first as f64;
                if first + 1 < old_len {
                    (first, vec![1.0 - t, t])
                } else {
                    (first, vec![1.0])
                }
            }
        })
        .collect();
    for l in 0..lines {
        for (i, (first, ws)) in taps.iter().enumerate() {
            let mut acc = [0.0f64; 4];
            let mut total = 0.0;
            for (k, wgt) in ws.iter().enumerate() {
                let s = first + k;
                let (x, y) = if horizontal { (s, l) } else { (l, s) };
                let p = b.pixel(x as u32, y as u32);
                let a = p[3] as f64 / 255.0;
                for c in 0..3 {
                    acc[c] += p[c] as f64 * a * wgt;
                }
                acc[3] += p[3] as f64 * wgt;
                total += wgt;
            }
            let alpha = acc[3] / total.max(1e-9);
            let px = if alpha <= 0.0 {
                [0, 0, 0, 0]
            } else {
                let un = |v: f64| (v / total / (alpha / 255.0)).round().clamp(0.0, 255.0) as u8;
                [un(acc[0]), un(acc[1]), un(acc[2]), alpha.round().clamp(0.0, 255.0) as u8]
            };
            let (x, y) = if horizontal { (i, l) } else { (l, i) };
            out.set_pixel(x as u32, y as u32, px);
        }
    }
    out
}

/// A quarter turn clockwise.
pub fn rotate_cw(b: &Bitmap) -> Bitmap {
    let mut out = Bitmap::filled(b.height, b.width, [0, 0, 0, 0]);
    for y in 0..b.height {
        for x in 0..b.width {
            out.set_pixel(b.height - 1 - y, x, b.pixel(x, y));
        }
    }
    out
}

/// A quarter turn counter-clockwise.
pub fn rotate_ccw(b: &Bitmap) -> Bitmap {
    let mut out = Bitmap::filled(b.height, b.width, [0, 0, 0, 0]);
    for y in 0..b.height {
        for x in 0..b.width {
            out.set_pixel(y, b.width - 1 - x, b.pixel(x, y));
        }
    }
    out
}

/// Mirror image, left to right.
pub fn flip_horizontal(b: &Bitmap) -> Bitmap {
    let mut out = b.clone();
    for y in 0..b.height {
        for x in 0..b.width {
            out.set_pixel(b.width - 1 - x, y, b.pixel(x, y));
        }
    }
    out
}

/// Upside down.
pub fn flip_vertical(b: &Bitmap) -> Bitmap {
    let row = b.width as usize * 4;
    let mut out = b.clone();
    for y in 0..b.height as usize {
        let src = (b.height as usize - 1 - y) * row;
        out.rgba[y * row..(y + 1) * row].copy_from_slice(&b.rgba[src..src + row]);
    }
    out
}

/// Removes rows `from..to` (`horizontal`: a band across the picture) or columns `from..to`
/// and joins what is left. Returns `None` if nothing would be left or nothing removed.
pub fn cut_band(b: &Bitmap, from: u32, to: u32, horizontal: bool) -> Option<Bitmap> {
    let len = if horizontal { b.height } else { b.width };
    let (from, to) = (from.min(len), to.min(len));
    if to <= from || to - from >= len {
        return None;
    }
    let cut = to - from;
    if horizontal {
        let row = b.width as usize * 4;
        let mut rgba = Vec::with_capacity((b.height - cut) as usize * row);
        rgba.extend_from_slice(&b.rgba[..from as usize * row]);
        rgba.extend_from_slice(&b.rgba[to as usize * row..]);
        Some(Bitmap { width: b.width, height: b.height - cut, rgba })
    } else {
        let mut out = Bitmap::filled(b.width - cut, b.height, [0, 0, 0, 0]);
        for y in 0..b.height {
            for x in 0..out.width {
                let sx = if x < from { x } else { x + cut };
                out.set_pixel(x, y, b.pixel(sx, y));
            }
        }
        Some(out)
    }
}

/// The picture with space added around it (left, top, right, bottom), filled with `fill`.
pub fn extend(b: &Bitmap, left: u32, top: u32, right: u32, bottom: u32, fill: [u8; 4]) -> Bitmap {
    let mut out = Bitmap::filled(b.width + left + right, b.height + top + bottom, fill);
    paste(&mut out, b, left as i64, top as i64);
    out
}

/// Copies `src` into `dst` with its top-left corner at (x, y); what falls outside is dropped.
pub fn paste(dst: &mut Bitmap, src: &Bitmap, x: i64, y: i64) {
    for sy in 0..src.height as i64 {
        let dy = y + sy;
        if dy < 0 || dy >= dst.height as i64 {
            continue;
        }
        let x0 = x.max(0);
        let x1 = (x + src.width as i64).min(dst.width as i64);
        if x1 <= x0 {
            continue;
        }
        let s = ((sy * src.width as i64 + (x0 - x)) * 4) as usize;
        let d = ((dy * dst.width as i64 + x0) * 4) as usize;
        let n = ((x1 - x0) * 4) as usize;
        dst.rgba[d..d + n].copy_from_slice(&src.rgba[s..s + n]);
    }
}

/// `a` and `b` together: `b` to the right of `a` (`beside`) or below it, centred on the other
/// axis, with `gap` pixels between. The space around the smaller one is `fill`.
pub fn join(a: &Bitmap, b: &Bitmap, beside: bool, gap: u32, fill: [u8; 4]) -> (Bitmap, (i64, i64)) {
    let (w, h, bx, by) = if beside {
        let h = a.height.max(b.height);
        (a.width + gap + b.width, h, (a.width + gap) as i64, ((h - b.height) / 2) as i64)
    } else {
        let w = a.width.max(b.width);
        (w, a.height + gap + b.height, ((w - b.width) / 2) as i64, (a.height + gap) as i64)
    };
    let (ax, ay) = if beside { (0, ((h - a.height) / 2) as i64) } else { (((w - a.width) / 2) as i64, 0) };
    let mut out = Bitmap::filled(w, h, fill);
    paste(&mut out, a, ax, ay);
    paste(&mut out, b, bx, by);
    (out, (ax, ay))
}

/// How pictures are put together by [`combine`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrange {
    /// Side by side, left to right.
    Row,
    /// One below the other.
    Column,
    /// Rows of `n` pictures (the last row may be shorter).
    Grid(u32),
}

/// Where a picture goes in its cell: at the start (left or top), centred, or at the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

/// The pictures together, in order, with `gap` pixels between them (and `margin` around), on
/// `fill`. A row's height is its tallest picture's; a column's width its widest's; in a grid
/// every cell of a row is as tall as the row and every cell of a column as wide as the
/// column. Returns the picture and where each one went.
pub fn combine(pictures: &[&Bitmap], arrange: Arrange, gap: u32, margin: u32, align: Align, fill: [u8; 4]) -> (Bitmap, Vec<(i64, i64)>) {
    let n = pictures.len();
    if n == 0 {
        return (Bitmap::filled(1, 1, fill), Vec::new());
    }
    let cols = match arrange {
        Arrange::Row => n,
        Arrange::Column => 1,
        Arrange::Grid(c) => (c.max(1) as usize).min(n),
    };
    let rows = n.div_ceil(cols);
    let mut col_w = vec![0u32; cols];
    let mut row_h = vec![0u32; rows];
    for (i, p) in pictures.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        col_w[c] = col_w[c].max(p.width);
        row_h[r] = row_h[r].max(p.height);
    }
    let w = col_w.iter().sum::<u32>() + gap * (cols as u32 - 1) + 2 * margin;
    let h = row_h.iter().sum::<u32>() + gap * (rows as u32 - 1) + 2 * margin;
    let mut out = Bitmap::filled(w.max(1), h.max(1), fill);
    let place = |room: u32, size: u32| -> i64 {
        match align {
            Align::Start => 0,
            Align::Center => ((room - size) / 2) as i64,
            Align::End => (room - size) as i64,
        }
    };
    let mut at = Vec::with_capacity(n);
    for (i, p) in pictures.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let x0 = margin as i64 + col_w[..c].iter().map(|v| *v as i64 + gap as i64).sum::<i64>();
        let y0 = margin as i64 + row_h[..r].iter().map(|v| *v as i64 + gap as i64).sum::<i64>();
        let (x, y) = (x0 + place(col_w[c], p.width), y0 + place(row_h[r], p.height));
        paste(&mut out, p, x, y);
        at.push((x, y));
    }
    (out, at)
}

/// The part of the picture inside its plain border: rows and columns at the edges that are
/// all the colour of the top-left pixel (within `tolerance` per channel) are left out.
/// `None` if the whole picture is that colour or there is no border.
pub fn trim_bounds(b: &Bitmap, tolerance: u8) -> Option<crate::Rect> {
    if b.width == 0 || b.height == 0 {
        return None;
    }
    let edge = b.pixel(0, 0);
    let plain = |x: u32, y: u32| b.pixel(x, y).iter().zip(edge).all(|(a, e)| a.abs_diff(e) <= tolerance);
    let row_plain = |y: u32| (0..b.width).all(|x| plain(x, y));
    let col_plain = |x: u32, y0: u32, y1: u32| (y0..y1).all(|y| plain(x, y));
    let top = (0..b.height).find(|&y| !row_plain(y))?;
    let bottom = (0..b.height).rev().find(|&y| !row_plain(y))? + 1;
    let left = (0..b.width).find(|&x| !col_plain(x, top, bottom))?;
    let right = (0..b.width).rev().find(|&x| !col_plain(x, top, bottom))? + 1;
    let r = crate::Rect::new(left as i32, top as i32, right - left, bottom - top);
    (r != crate::Rect::new(0, 0, b.width, b.height)).then_some(r)
}

/// Fills `r` with the colour around it (the middle value of the pixels just outside its
/// edges), as if nothing were there: hidden areas look like blank space to text reading,
/// whereas a black box would make it skip whole lines.
pub fn blank(b: &mut Bitmap, r: crate::Rect) {
    let Some(r) = r.clamp_to(b.width, b.height) else { return };
    let (x0, y0, x1, y1) = (r.x as i64, r.y as i64, r.x as i64 + r.w as i64, r.y as i64 + r.h as i64);
    let mut ring: Vec<[u8; 4]> = Vec::new();
    let mut take = |x: i64, y: i64| {
        if x >= 0 && y >= 0 && x < b.width as i64 && y < b.height as i64 {
            ring.push(b.pixel(x as u32, y as u32));
        }
    };
    for x in x0 - 1..=x1 {
        take(x, y0 - 2);
        take(x, y1 + 1);
    }
    for y in y0 - 1..=y1 {
        take(x0 - 2, y);
        take(x1 + 1, y);
    }
    let colour = if ring.is_empty() {
        [255, 255, 255, 255]
    } else {
        let mid = |c: usize| {
            let mut v: Vec<u8> = ring.iter().map(|p| p[c]).collect();
            v.sort_unstable();
            v[v.len() / 2]
        };
        [mid(0), mid(1), mid(2), 255]
    };
    for y in y0..y1 {
        for x in x0..x1 {
            b.set_pixel(x as u32, y as u32, colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_combine_in_a_row_a_column_or_a_grid() {
        let (a, b, c) = (Bitmap::filled(10, 6, [255, 0, 0, 255]), Bitmap::filled(4, 10, [0, 255, 0, 255]), Bitmap::filled(8, 8, [0, 0, 255, 255]));
        let white = [255, 255, 255, 255];
        let (row, at) = combine(&[&a, &b, &c], Arrange::Row, 2, 0, Align::Center, white);
        assert_eq!((row.width, row.height), (10 + 2 + 4 + 2 + 8, 10));
        assert_eq!(at, vec![(0, 2), (12, 0), (18, 1)]);
        assert_eq!(row.pixel(0, 2), [255, 0, 0, 255]);
        assert_eq!(row.pixel(0, 0), white);
        let (col, at) = combine(&[&a, &b], Arrange::Column, 0, 3, Align::Start, white);
        assert_eq!((col.width, col.height), (10 + 6, 6 + 10 + 6));
        assert_eq!(at, vec![(3, 3), (3, 9)]);
        // A grid of two columns: three pictures make two rows, cells as big as their row and column.
        let (grid, at) = combine(&[&a, &b, &c], Arrange::Grid(2), 1, 0, Align::End, white);
        assert_eq!((grid.width, grid.height), (10 + 1 + 4, 10 + 1 + 8));
        assert_eq!(at, vec![(0, 4), (11, 0), (2, 11)]);
        assert_eq!(grid.pixel(14, 18), white, "the empty cell");
    }

    #[test]
    fn blanking_fills_with_the_colour_around() {
        let mut b = Bitmap::filled(40, 30, [250, 240, 230, 255]);
        for y in 10..20 {
            for x in 10..30 {
                b.set_pixel(x, y, [0, 0, 0, 255]);
            }
        }
        blank(&mut b, crate::Rect::new(10, 10, 20, 10));
        assert_eq!(b.pixel(15, 15), [250, 240, 230, 255]);
        assert_eq!(b, Bitmap::filled(40, 30, [250, 240, 230, 255]));
        // At the edge of the picture: the sides that exist are enough.
        let mut c = Bitmap::filled(10, 10, [9, 9, 9, 255]);
        blank(&mut c, crate::Rect::new(0, 0, 10, 4));
        assert_eq!(c.pixel(0, 0), [9, 9, 9, 255]);
    }

    fn numbered(w: u32, h: u32) -> Bitmap {
        let mut b = Bitmap::filled(w, h, [0, 0, 0, 255]);
        for y in 0..h {
            for x in 0..w {
                b.set_pixel(x, y, [x as u8, y as u8, (x * 7 + y * 3) as u8, 255]);
            }
        }
        b
    }

    #[test]
    fn quarter_turns_undo_each_other_and_four_make_a_full_turn() {
        let b = numbered(5, 3);
        let cw = rotate_cw(&b);
        assert_eq!((cw.width, cw.height), (3, 5));
        assert_eq!(cw.pixel(2, 0), b.pixel(0, 0), "the top-left goes to the top-right");
        assert_eq!(rotate_ccw(&cw), b);
        assert_eq!(rotate_cw(&rotate_cw(&rotate_cw(&cw))), b);
    }

    #[test]
    fn flips_mirror_and_undo_themselves() {
        let b = numbered(4, 3);
        assert_eq!(flip_horizontal(&b).pixel(3, 1), b.pixel(0, 1));
        assert_eq!(flip_vertical(&b).pixel(1, 2), b.pixel(1, 0));
        assert_eq!(flip_horizontal(&flip_horizontal(&b)), b);
        assert_eq!(flip_vertical(&flip_vertical(&b)), b);
    }

    #[test]
    fn cutting_a_band_joins_what_was_around_it() {
        let b = numbered(6, 10);
        let rows = cut_band(&b, 3, 7, true).unwrap();
        assert_eq!((rows.width, rows.height), (6, 6));
        assert_eq!(rows.pixel(2, 2), b.pixel(2, 2));
        assert_eq!(rows.pixel(2, 3), b.pixel(2, 7));
        let cols = cut_band(&b, 1, 3, false).unwrap();
        assert_eq!((cols.width, cols.height), (4, 10));
        assert_eq!(cols.pixel(1, 5), b.pixel(3, 5));
        assert!(cut_band(&b, 0, 10, true).is_none(), "nothing would be left");
        assert!(cut_band(&b, 4, 4, true).is_none(), "nothing removed");
    }

    #[test]
    fn shrinking_averages_and_keeps_flat_colours_exact() {
        let flat = Bitmap::filled(40, 30, [10, 200, 30, 255]);
        let small = resize(&flat, 13, 7);
        assert_eq!((small.width, small.height), (13, 7));
        assert!(small.rgba.chunks(4).all(|p| p == [10, 200, 30, 255]));
        // A 2x2 checkerboard of black and white halves to mid grey.
        let mut checks = Bitmap::filled(4, 4, [0, 0, 0, 255]);
        for y in 0..4 {
            for x in 0..4 {
                if (x + y) % 2 == 0 {
                    checks.set_pixel(x, y, [255, 255, 255, 255]);
                }
            }
        }
        let half = resize(&checks, 2, 2);
        assert!(half.rgba.chunks(4).all(|p| p[0].abs_diff(128) <= 1), "{:?}", half.rgba);
        let big = resize(&flat, 100, 61);
        assert!(big.rgba.chunks(4).all(|p| p == [10, 200, 30, 255]));
    }

    #[test]
    fn extending_adds_space_and_keeps_the_picture() {
        let b = numbered(3, 2);
        let e = extend(&b, 2, 1, 0, 3, [9, 9, 9, 255]);
        assert_eq!((e.width, e.height), (5, 6));
        assert_eq!(e.pixel(2, 1), b.pixel(0, 0));
        assert_eq!(e.pixel(0, 0), [9, 9, 9, 255]);
        assert_eq!(e.pixel(4, 5), [9, 9, 9, 255]);
    }

    #[test]
    fn joining_puts_the_second_picture_beside_or_below_centred() {
        let (a, b) = (Bitmap::filled(4, 4, [1, 1, 1, 255]), Bitmap::filled(2, 2, [2, 2, 2, 255]));
        let (j, at) = join(&a, &b, true, 1, [0, 0, 0, 0]);
        assert_eq!((j.width, j.height), (7, 4));
        assert_eq!(at, (0, 0));
        assert_eq!(j.pixel(5, 1), [2, 2, 2, 255]);
        assert_eq!(j.pixel(5, 0), [0, 0, 0, 0], "space above the smaller one");
        let (j, at) = join(&b, &a, false, 0, [0, 0, 0, 0]);
        assert_eq!((j.width, j.height), (4, 6));
        assert_eq!(at, (1, 0), "the first is centred above");
    }

    #[test]
    fn trimming_finds_what_is_inside_a_plain_border() {
        let mut b = Bitmap::filled(20, 10, [255, 255, 255, 255]);
        for y in 3..6 {
            for x in 4..15 {
                b.set_pixel(x, y, [0, 0, 0, 255]);
            }
        }
        b.set_pixel(19, 9, [250, 252, 255, 255]); // within the tolerance: still border
        assert_eq!(trim_bounds(&b, 8), Some(crate::Rect::new(4, 3, 11, 3)));
        assert_eq!(trim_bounds(&Bitmap::filled(5, 5, [1, 2, 3, 255]), 8), None, "all border");
        assert_eq!(trim_bounds(&numbered(5, 5), 0), None, "no border");
    }

    #[test]
    fn softening_spreads_a_dot_but_keeps_flat_areas() {
        let mut b = Bitmap::filled(21, 21, [0, 0, 0, 0]);
        b.set_pixel(10, 10, [0, 0, 0, 255]);
        soften(&mut b, 2);
        assert!(b.pixel(10, 10)[3] < 255 && b.pixel(12, 10)[3] > 0);
        let mut flat = Bitmap::filled(10, 10, [50, 60, 70, 200]);
        soften(&mut flat, 3);
        assert!(flat.rgba.chunks(4).all(|p| p == [50, 60, 70, 200]));
    }
}
