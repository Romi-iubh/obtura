//! Animated GIF from a sequence of frames, made small (the "GIF optimiser"):
//! * a frame identical to the one before is not written again: the one before lasts longer;
//! * a frame only holds the rectangle that changed, and inside it the pixels that did not
//!   change are transparent, so they compress to almost nothing;
//! * each frame gets its own palette of up to 256 colours (exact when it has no more than
//!   that, NeuQuant otherwise);
//! * frame times add up exactly: each frame's delay is rounded so the total never drifts.

use crate::Bitmap;
use std::io::Write;

pub struct GifWriter<W: Write> {
    encoder: gif::Encoder<W>,
    width: u32,
    height: u32,
    /// How long one frame lasts, in hundredths of a second.
    frame_cs: f64,
    /// NeuQuant speed (1 best and slowest, 30 fastest).
    speed: i32,
    /// The picture as shown once the pending frame is drawn.
    shown: Option<Vec<u8>>,
    /// A frame made but not written yet: its delay grows while identical frames follow.
    pending: Option<gif::Frame<'static>>,
    pending_end: f64,
    /// Time so far, in hundredths of a second: of the frames given, and of the delays written.
    clock: f64,
    written: u64,
    frames_written: usize,
}

impl<W: Write> GifWriter<W> {
    /// A GIF of `width` x `height` that plays `fps` frames per second, looping forever.
    pub fn new(out: W, width: u32, height: u32, fps: f64) -> Result<Self, String> {
        if width == 0 || height == 0 || width > u16::MAX as u32 || height > u16::MAX as u32 {
            return Err(format!("a GIF cannot be {width} x {height}"));
        }
        let mut encoder = gif::Encoder::new(out, width as u16, height as u16, &[]).map_err(|e| e.to_string())?;
        encoder.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
        Ok(Self { encoder, width, height, frame_cs: 100.0 / fps.clamp(1.0, 50.0), speed: 10, shown: None, pending: None, pending_end: 0.0, clock: 0.0, written: 0, frames_written: 0 })
    }

    /// Adds the next frame (it must be `width` x `height`), lasting one frame.
    pub fn add(&mut self, frame: &Bitmap) -> Result<(), String> {
        self.add_lasting(frame, self.frame_cs)
    }

    /// Adds the next frame, lasting `cs` hundredths of a second.
    pub fn add_lasting(&mut self, frame: &Bitmap, cs: f64) -> Result<(), String> {
        if (frame.width, frame.height) != (self.width, self.height) {
            return Err(format!("a frame of {} x {} in a GIF of {} x {}", frame.width, frame.height, self.width, self.height));
        }
        let end = self.clock + cs.max(0.0);
        self.clock = end;
        if self.shown.as_deref() == Some(&frame.rgba[..]) {
            self.pending_end = end;
            return Ok(());
        }
        self.flush()?;
        let made = make_frame(self.shown.as_deref(), frame, self.speed);
        self.pending = Some(made);
        self.pending_end = end;
        self.shown = Some(frame.rgba.clone());
        Ok(())
    }

    /// Writes the pending frame with its delay.
    fn flush(&mut self) -> Result<(), String> {
        let Some(mut frame) = self.pending.take() else { return Ok(()) };
        let end = self.pending_end.round().max(0.0) as u64;
        // At least 2 hundredths: players slow shorter delays down to a tenth of a second.
        frame.delay = end.saturating_sub(self.written).clamp(2, u16::MAX as u64) as u16;
        self.written += frame.delay as u64;
        self.encoder.write_frame(&frame).map_err(|e| e.to_string())?;
        self.frames_written += 1;
        Ok(())
    }

    /// Writes what is left and gives the output back, with the number of frames written.
    pub fn finish(mut self) -> Result<(W, usize), String> {
        self.flush()?;
        if self.frames_written == 0 {
            return Err("there was no frame to make a GIF of".into());
        }
        let n = self.frames_written;
        let out = self.encoder.into_inner().map_err(|e| e.to_string())?;
        Ok((out, n))
    }
}

/// Reads a GIF frame by frame as whole pictures (each frame drawn over what was shown before, as
/// a player shows it) and calls `f` with each picture and how long it lasts (hundredths of a
/// second). `f` answers whether to go on. Gives the GIF's size.
pub fn for_each_frame(bytes: &[u8], mut f: impl FnMut(&Bitmap, u16) -> Result<bool, String>) -> Result<(u32, u32), String> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut d = opts.read_info(bytes).map_err(|e| e.to_string())?;
    let (w, h) = (d.width() as u32, d.height() as u32);
    let mut canvas = Bitmap::filled(w, h, [0, 0, 0, 0]);
    // What the frame before asked to be done once it has been shown.
    let mut undo: Option<(gif::DisposalMethod, (u32, u32, u32, u32), Option<Vec<u8>>)> = None;
    while let Some(frame) = d.read_next_frame().map_err(|e| e.to_string())? {
        match undo.take() {
            Some((gif::DisposalMethod::Background, (x, y, fw, fh), _)) => {
                for row in y..(y + fh).min(h) {
                    for col in x..(x + fw).min(w) {
                        let o = ((row * w + col) * 4) as usize;
                        canvas.rgba[o..o + 4].copy_from_slice(&[0, 0, 0, 0]);
                    }
                }
            }
            Some((gif::DisposalMethod::Previous, _, Some(saved))) => canvas.rgba = saved,
            _ => {}
        }
        let rect = (frame.left as u32, frame.top as u32, frame.width as u32, frame.height as u32);
        let saved = (frame.dispose == gif::DisposalMethod::Previous).then(|| canvas.rgba.clone());
        for j in 0..rect.3 {
            for i in 0..rect.2 {
                let (cx, cy) = (rect.0 + i, rect.1 + j);
                let s = ((j * rect.2 + i) * 4) as usize;
                if cx < w && cy < h && frame.buffer[s + 3] != 0 {
                    let o = ((cy * w + cx) * 4) as usize;
                    canvas.rgba[o..o + 4].copy_from_slice(&frame.buffer[s..s + 4]);
                }
            }
        }
        undo = Some((frame.dispose, rect, saved));
        if !f(&canvas, frame.delay)? {
            break;
        }
    }
    Ok((w, h))
}

/// The part of a GIF from `start` to `end` (hundredths of a second), as a new GIF, made small
/// the same way. Gives the number of frames written.
pub fn trim<W: Write>(bytes: &[u8], start: f64, end: f64, out: W) -> Result<(W, usize), String> {
    let mut writer: Option<GifWriter<W>> = None;
    let mut out = Some(out);
    let mut t = 0.0;
    for_each_frame(bytes, |picture, delay| {
        let (a, b) = (t, t + delay as f64);
        t = b;
        let (from, to) = (a.max(start), b.min(end));
        if to > from {
            if writer.is_none() {
                writer = Some(GifWriter::new(out.take().expect("one writer"), picture.width, picture.height, 10.0)?);
            }
            writer.as_mut().expect("made").add_lasting(picture, to - from)?;
        }
        Ok(b < end)
    })?;
    writer.ok_or_else(|| "nothing is left between those two moments".to_string())?.finish()
}

/// The smallest rectangle (x, y, w, h) holding every pixel that differs between `a` and `b`
/// (RGBA, `width` wide), or `None` if they are the same.
fn changed(a: &[u8], b: &[u8], width: u32) -> Option<(u32, u32, u32, u32)> {
    let w = width as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (y, (ra, rb)) in a.chunks_exact(w * 4).zip(b.chunks_exact(w * 4)).enumerate() {
        if ra == rb {
            continue;
        }
        let first = ra.chunks_exact(4).zip(rb.chunks_exact(4)).position(|(p, q)| p != q).unwrap_or(0);
        let last = w - 1 - ra.chunks_exact(4).rev().zip(rb.chunks_exact(4).rev()).position(|(p, q)| p != q).unwrap_or(0);
        x0 = x0.min(first);
        x1 = x1.max(last);
        y0 = y0.min(y);
        y1 = y;
    }
    (x0 != usize::MAX).then(|| (x0 as u32, y0 as u32, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32))
}

/// The GIF frame that turns `before` (if any) into `now`: the changed rectangle only, with the
/// unchanged pixels in it transparent.
fn make_frame(before: Option<&[u8]>, now: &Bitmap, speed: i32) -> gif::Frame<'static> {
    let (x, y, w, h) = match before {
        Some(b) => changed(b, &now.rgba, now.width).unwrap_or((0, 0, 1, 1)),
        None => (0, 0, now.width, now.height),
    };
    let stride = now.width as usize * 4;
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y..y + h {
        let start = row as usize * stride + x as usize * 4;
        let line = &now.rgba[start..start + w as usize * 4];
        match before {
            Some(b) => {
                let old = &b[start..start + w as usize * 4];
                for (p, q) in line.chunks_exact(4).zip(old.chunks_exact(4)) {
                    let alpha = if p == q { 0 } else { 255 };
                    pixels.extend_from_slice(&[p[0], p[1], p[2], alpha]);
                }
            }
            None => {
                for p in line.chunks_exact(4) {
                    pixels.extend_from_slice(&[p[0], p[1], p[2], 255]);
                }
            }
        }
    }
    let mut frame = gif::Frame::from_rgba_speed(w as u16, h as u16, &mut pixels, speed);
    frame.left = x as u16;
    frame.top = y as u16;
    frame.dispose = gif::DisposalMethod::Keep;
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(colour: [u8; 4]) -> Bitmap {
        Bitmap::filled(40, 30, colour)
    }

    fn with_square(mut b: Bitmap, x: u32, y: u32, size: u32, colour: [u8; 4]) -> Bitmap {
        for j in y..y + size {
            for i in x..x + size {
                let o = ((j * b.width + i) * 4) as usize;
                b.rgba[o..o + 4].copy_from_slice(&colour);
            }
        }
        b
    }

    /// (left, top, width, height, delay) of every frame, and the canvas after the last one.
    fn decode(bytes: &[u8]) -> (Vec<(u16, u16, u16, u16, u16)>, Vec<u8>) {
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::RGBA);
        let mut d = opts.read_info(bytes).unwrap();
        let (cw, ch) = (d.width() as usize, d.height() as usize);
        let mut canvas = vec![0u8; cw * ch * 4];
        let mut frames = Vec::new();
        while let Some(f) = d.read_next_frame().unwrap() {
            frames.push((f.left, f.top, f.width, f.height, f.delay));
            for j in 0..f.height as usize {
                for i in 0..f.width as usize {
                    let s = (j * f.width as usize + i) * 4;
                    if f.buffer[s + 3] != 0 {
                        let o = ((j + f.top as usize) * cw + i + f.left as usize) * 4;
                        canvas[o..o + 4].copy_from_slice(&f.buffer[s..s + 4]);
                    }
                }
            }
        }
        (frames, canvas)
    }

    #[test]
    fn identical_frames_are_merged_and_only_the_change_is_written() {
        let a = picture([20, 40, 60, 255]);
        let b = with_square(a.clone(), 10, 5, 4, [250, 10, 10, 255]);
        let mut gif = GifWriter::new(Vec::new(), 40, 30, 10.0).unwrap();
        for f in [&a, &a, &a, &b] {
            gif.add(f).unwrap();
        }
        let (bytes, n) = gif.finish().unwrap();
        assert_eq!(n, 2);
        let (frames, canvas) = decode(&bytes);
        // The first frame lasted three frames (0.3 s); the second holds the square only.
        assert_eq!(frames, vec![(0, 0, 40, 30, 30), (10, 5, 4, 4, 10)]);
        // What is shown at the end is exactly the last frame (few colours: no loss).
        assert_eq!(canvas, b.rgba);
    }

    #[test]
    fn delays_add_up_without_drift() {
        // 15 frames a second: 6.67 hundredths each. 30 different frames last 2 s = 200.
        let mut gif = GifWriter::new(Vec::new(), 40, 30, 15.0).unwrap();
        for i in 0..30u8 {
            gif.add(&with_square(picture([0, 0, 0, 255]), i as u32, 0, 2, [255, 255, 255, 255])).unwrap();
        }
        let (bytes, n) = gif.finish().unwrap();
        assert_eq!(n, 30);
        let (frames, _) = decode(&bytes);
        let total: u32 = frames.iter().map(|f| f.4 as u32).sum();
        assert_eq!(total, 200);
        assert!(frames.iter().all(|f| f.4 == 6 || f.4 == 7));
    }

    #[test]
    fn many_colours_are_reduced_to_a_close_palette() {
        let mut a = picture([0, 0, 0, 255]);
        for (i, p) in a.rgba.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[(i * 7 % 256) as u8, (i * 3 % 256) as u8, (i % 256) as u8, 255]);
        }
        let mut gif = GifWriter::new(Vec::new(), 40, 30, 10.0).unwrap();
        gif.add(&a).unwrap();
        let (bytes, _) = gif.finish().unwrap();
        let (_, canvas) = decode(&bytes);
        let err: f64 = canvas.iter().zip(&a.rgba).map(|(x, y)| (*x as f64 - *y as f64).abs()).sum::<f64>() / canvas.len() as f64;
        assert!(err < 24.0, "mean error {err}");
    }

    #[test]
    fn a_gif_is_read_back_as_whole_pictures_and_cut_by_time() {
        // Ten frames at 10 a second, each with a square further right: 1 s in all.
        let frames: Vec<Bitmap> = (0..10).map(|i| with_square(picture([10, 20, 30, 255]), i * 3, 4, 3, [250, 250, 0, 255])).collect();
        let mut gif = GifWriter::new(Vec::new(), 40, 30, 10.0).unwrap();
        for f in &frames {
            gif.add(f).unwrap();
        }
        let (bytes, _) = gif.finish().unwrap();
        let mut seen = Vec::new();
        for_each_frame(&bytes, |p, delay| {
            seen.push((p.clone(), delay));
            Ok(true)
        })
        .unwrap();
        assert_eq!(seen.len(), 10);
        for (i, (p, delay)) in seen.iter().enumerate() {
            assert_eq!(p.rgba, frames[i].rgba, "frame {i} drawn over the ones before");
            assert_eq!(*delay, 10);
        }
        // From 0.25 s to 0.6 s: frames 2 (half), 3, 4, 5 = 35 hundredths.
        let (cut, n) = trim(&bytes, 25.0, 60.0, Vec::new()).unwrap();
        assert_eq!(n, 4);
        let mut got = Vec::new();
        for_each_frame(&cut, |p, delay| {
            got.push((p.clone(), delay));
            Ok(true)
        })
        .unwrap();
        assert_eq!(got.iter().map(|g| g.1 as u32).sum::<u32>(), 35);
        assert_eq!(got[0].0.rgba, frames[2].rgba);
        assert_eq!(got[3].0.rgba, frames[5].rgba);
        assert!(trim(&bytes, 200.0, 300.0, Vec::new()).is_err());
    }

    #[test]
    fn wrong_sizes_are_refused() {
        assert!(GifWriter::new(Vec::new(), 0, 10, 10.0).is_err());
        let mut gif = GifWriter::new(Vec::new(), 40, 30, 10.0).unwrap();
        assert!(gif.add(&Bitmap::filled(10, 10, [0, 0, 0, 255])).is_err());
        assert!(GifWriter::new(Vec::new(), 4, 4, 10.0).unwrap().finish().is_err());
    }

    #[test]
    fn finds_the_changed_rectangle() {
        let a = picture([1, 2, 3, 255]);
        assert_eq!(changed(&a.rgba, &a.rgba, 40), None);
        let b = with_square(with_square(a.clone(), 2, 3, 1, [9, 9, 9, 255]), 30, 20, 2, [9, 9, 9, 255]);
        assert_eq!(changed(&a.rgba, &b.rgba, 40), Some((2, 3, 30, 19)));
    }
}
