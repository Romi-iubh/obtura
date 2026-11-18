//! Scrolling capture: frames of the same part of the screen, taken while its content scrolls,
//! joined into one tall picture.
//!
//! Each frame is compared with the last one that was used: the content that moved up by `d`
//! rows matches the old frame `d` rows lower. Rows are compared in bands (strips side by side,
//! about 24 pixels wide) by each band's mean brightness, with a little tolerance. A row lines
//! up when its bands with detail do, give or take one (a scroll bar's thumb, which moves at
//! its own pace). Plain bands (one colour: a page's margins, a scroll bar's track) say
//! nothing. A movement is taken when most rows with detail line up: some may differ (a row
//! highlighted under the pointer, a picture that has just loaded), more when the page goes on
//! moving as it did.
//!
//! Rows that stay where they are at the top and at the bottom when the page first scrolls (a
//! sticky header, a status bar) are kept once: the header above everything, the footer below.
//!
//! Scrolling back up is followed, and what comes into view above where it started is added at
//! the top. A frame that does not overlap the last one used (scrolled too fast) is set aside:
//! the caller asks the user to scroll back a little, and joining goes on once frames overlap
//! again.

use crate::Bitmap;

/// What happened to a frame given to [`Stitcher::push`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    /// The first frame.
    First,
    /// The content moved by this many rows (down the page when positive); `added` new rows
    /// came into the picture.
    Moved { by: i32, added: u32 },
    /// Nothing moved.
    Same,
    /// No overlap with the last frame used: scrolled too fast. Not used.
    Lost,
    /// Not the size of the first frame, or the picture is as tall as it may be. Not used.
    Refused,
}

/// The tallest picture made, in rows.
pub const MAX_HEIGHT: u32 = 40_000;

/// The width of a band, in pixels (at most [`MAX_BANDS`] bands across).
const BAND: usize = 24;
const MAX_BANDS: usize = 48;
/// Two bands whose mean brightness (0-255) differs by no more than this are the same.
const TOLERANCE: u8 = 2;
/// A band whose brightest and darkest pixels differ by no more than this is plain.
const PLAIN: u8 = 8;
/// The fewest rows with detail that must line up for a movement to be trusted.
const MIN_ROWS: usize = 6;
/// Rows with detail compared for each movement in the quick first pass.
const SAMPLES: usize = 128;
/// Movements kept from the quick pass for the full comparison.
const CANDIDATES: usize = 8;

/// One frame as it is compared: each row cut into bands, each band's mean brightness and
/// whether it is plain.
struct Rows {
    bands: usize,
    mean: Vec<u8>,
    plain: Vec<bool>,
    /// The rows with detail (a band that is not plain), top to bottom.
    detail: Vec<u32>,
}

/// How a row of one frame compares with a row of another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    /// The same content.
    Same,
    /// Other content.
    Differ,
    /// Plain wherever they meet: says nothing.
    Blank,
}

impl Rows {
    fn of(b: &Bitmap) -> Rows {
        let (w, h) = (b.width as usize, b.height as usize);
        let wide = w.div_ceil((w / BAND).clamp(1, MAX_BANDS)).max(1);
        let bands = w.div_ceil(wide);
        let mut mean = vec![0u8; h * bands];
        let mut plain = vec![true; h * bands];
        let mut detail = Vec::new();
        for y in 0..h {
            let row = &b.rgba[y * w * 4..(y + 1) * w * 4];
            let mut any = false;
            for k in 0..bands {
                let (x0, x1) = (k * wide, ((k + 1) * wide).min(w));
                let (mut sum, mut lo, mut hi) = (0u32, u8::MAX, 0u8);
                for p in row[x0 * 4..x1 * 4].chunks_exact(4) {
                    let l = ((p[0] as u32 * 2 + p[1] as u32 * 5 + p[2] as u32) / 8) as u8;
                    sum += l as u32;
                    lo = lo.min(l);
                    hi = hi.max(l);
                }
                mean[y * bands + k] = (sum / (x1 - x0) as u32) as u8;
                let flat = hi - lo <= PLAIN;
                plain[y * bands + k] = flat;
                any |= !flat;
            }
            if any {
                detail.push(y as u32);
            }
        }
        Rows { bands, mean, plain, detail }
    }

    /// Row `a` of this frame against row `b` of `old`.
    fn compare(&self, a: usize, old: &Rows, b: usize) -> Row {
        let k = self.bands;
        let (am, ap) = (&self.mean[a * k..(a + 1) * k], &self.plain[a * k..(a + 1) * k]);
        let (bm, bp) = (&old.mean[b * k..(b + 1) * k], &old.plain[b * k..(b + 1) * k]);
        // One band may differ (a scroll bar's thumb over a line of text); more may not.
        let allowed = (k / 8).max(1);
        let (mut same, mut differ) = (0usize, 0usize);
        for i in 0..k {
            if ap[i] && bp[i] {
                continue;
            }
            if ap[i] == bp[i] && am[i].abs_diff(bm[i]) <= TOLERANCE {
                same += 1;
            } else {
                differ += 1;
                if differ > allowed {
                    return Row::Differ;
                }
            }
        }
        match (same, differ) {
            // Nothing, or only a band or two that differ (a scroll bar beside a blank line).
            (0, _) => Row::Blank,
            (s, d) if d <= ((s + d) / 8).max(1) => Row::Same,
            _ => Row::Differ,
        }
    }
}

/// How many rows with detail line up, how many do not, and the longest stretch of rows
/// without one that differs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Fit {
    same: usize,
    differ: usize,
    /// Rows compared.
    rows: usize,
    /// The longest stretch of compared rows without a row that differs, and its rows that
    /// line up.
    stretch: usize,
    stretch_same: usize,
}

impl Fit {
    fn score(self) -> i64 {
        self.same as i64 * 4 - self.differ as i64 * 8
    }

    /// Trusted: nearly every row lines up; or most do, and the ones that differ are together
    /// (something changed in one place while the page scrolled: a picture that loaded, a row
    /// highlighted under the pointer), leaving a long stretch, a third of the `band` at least,
    /// where all line up. Look-alike items of a list (the same icon, other names) differ all
    /// along, so they are not taken for one another.
    fn trusted(self, band: usize) -> bool {
        let long = self.stretch * 5 >= self.rows * 2 && self.stretch * 3 >= band && self.stretch_same >= MIN_ROWS;
        self.same >= MIN_ROWS && (self.differ * 12 <= self.same || (self.differ <= self.same && long))
    }
}

/// Rows `rows` of `new` (in order) against `old` moved by `d` rows.
fn fit(new: &Rows, old: &Rows, rows: impl Iterator<Item = usize>, d: i32) -> Fit {
    let mut f = Fit::default();
    let (mut run, mut run_same) = (0, 0);
    for i in rows {
        f.rows += 1;
        match new.compare(i, old, (i as i64 + d as i64) as usize) {
            Row::Differ => {
                f.differ += 1;
                run = 0;
                run_same = 0;
                continue;
            }
            Row::Same => {
                f.same += 1;
                run_same += 1;
            }
            Row::Blank => {}
        }
        run += 1;
        if run > f.stretch {
            f.stretch = run;
            f.stretch_same = run_same;
        }
    }
    f
}

pub struct Stitcher {
    width: u32,
    height: u32,
    /// The last frame used, as compared.
    last: Option<(Bitmap, Rows)>,
    /// Rows that never move at the top and the bottom, known from the first scroll.
    band: Option<(u32, u32)>,
    /// The frames at the top and at the bottom of what was joined: the picture's header comes
    /// from the first, its footer from the second.
    head: Option<Bitmap>,
    tail: Option<Bitmap>,
    /// The content (the moving band) joined so far, row by row.
    content: Vec<u8>,
    rows: u32,
    /// Where the last frame's band starts in the content.
    pos: i64,
    /// The last movement, preferred when several fit equally (repeating content: table rows).
    last_move: i32,
}

impl Default for Stitcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Stitcher {
    pub fn new() -> Self {
        Stitcher { width: 0, height: 0, last: None, band: None, head: None, tail: None, content: Vec::new(), rows: 0, pos: 0, last_move: 0 }
    }

    /// Height of the picture made so far.
    pub fn height(&self) -> u32 {
        match self.band {
            Some((top, bottom)) => top + self.rows + bottom,
            None => self.height,
        }
    }

    pub fn push(&mut self, frame: &Bitmap) -> Pushed {
        if frame.width == 0 || frame.height < 4 {
            return Pushed::Refused;
        }
        let Some((last, last_rows)) = self.last.as_ref() else {
            self.width = frame.width;
            self.height = frame.height;
            self.head = Some(frame.clone());
            self.tail = Some(frame.clone());
            self.last = Some((frame.clone(), Rows::of(frame)));
            return Pushed::First;
        };
        if frame.width != self.width || frame.height != self.height {
            return Pushed::Refused;
        }
        if frame.rgba == last.rgba {
            return Pushed::Same;
        }
        let rows = Rows::of(frame);
        let n = self.height as usize;
        // The band that moves: known after the first scroll; until then, the rows that did not
        // change at the top and the bottom.
        let (top, bottom) = self.band.unwrap_or_else(|| fixed_rows(last_rows, &rows, n));
        let band = n - (top + bottom) as usize;
        let Some(by) = best_move(last_rows, &rows, top as usize, band, self.last_move) else {
            return Pushed::Lost;
        };
        if by == 0 {
            // Something changed in place (a blinking cursor, an animation): not a scroll.
            self.last = Some((frame.clone(), rows));
            return Pushed::Same;
        }
        let w = self.width as usize * 4;
        if self.band.is_none() {
            // The first scroll: the content so far is the first frame's band.
            self.band = Some((top, bottom));
            let first = self.head.as_ref().expect("the first frame is kept");
            self.content = first.rgba[top as usize * w..(top as usize + band) * w].to_vec();
            self.rows = band as u32;
            self.pos = 0;
        }
        let mut pos = self.pos + by as i64;
        let mut added = 0;
        if pos < 0 {
            // Above where it started: what came into view goes on top.
            let extra = (-pos) as u32;
            if self.height() + extra > MAX_HEIGHT {
                return Pushed::Refused;
            }
            let from = top as usize * w;
            let mut content = Vec::with_capacity(self.content.len() + extra as usize * w);
            content.extend_from_slice(&frame.rgba[from..from + extra as usize * w]);
            content.extend_from_slice(&self.content);
            self.content = content;
            self.rows += extra;
            added = extra;
            pos = 0;
            self.head = Some(frame.clone());
        }
        let end = pos + band as i64;
        if end > self.rows as i64 {
            let extra = (end - self.rows as i64) as u32;
            if self.height() + extra > MAX_HEIGHT {
                return Pushed::Refused;
            }
            let from = top as usize + band - extra as usize;
            self.content.extend_from_slice(&frame.rgba[from * w..(from + extra as usize) * w]);
            self.rows += extra;
            added += extra;
        }
        if end >= self.rows as i64 {
            self.tail = Some(frame.clone());
        }
        self.pos = pos;
        self.last_move = by;
        self.last = Some((frame.clone(), rows));
        Pushed::Moved { by, added }
    }

    /// The picture: the header of the frame at the top of what was joined, the content, the
    /// footer of the frame at its bottom.
    pub fn picture(&self) -> Option<Bitmap> {
        let head = self.head.as_ref()?;
        let Some((top, bottom)) = self.band else { return Some(head.clone()) };
        let tail = self.tail.as_ref()?;
        let w = self.width as usize * 4;
        let mut rgba = Vec::with_capacity(self.height() as usize * w);
        rgba.extend_from_slice(&head.rgba[..top as usize * w]);
        rgba.extend_from_slice(&self.content);
        rgba.extend_from_slice(&tail.rgba[(self.height - bottom) as usize * w..]);
        Some(Bitmap { width: self.width, height: self.height(), rgba })
    }
}

/// The rows of `n` that did not change from `old` to `new`, at the top and at the bottom (a
/// header and a footer each at most a third of the frame).
fn fixed_rows(old: &Rows, new: &Rows, n: usize) -> (u32, u32) {
    let still = |y: usize| new.compare(y, old, y) != Row::Differ;
    let top = (0..n).take_while(|&y| still(y)).count();
    let bottom = (0..n - top).take_while(|&k| still(n - 1 - k)).count();
    (top.min(n / 3) as u32, bottom.min(n / 3) as u32)
}

/// How many rows the content of the band (`band` rows from `top`) moved from `old` to `new`:
/// the movement whose rows with detail line up best. Positive: the content went up (the page
/// scrolled down). `None` when no movement fits.
fn best_move(old: &Rows, new: &Rows, top: usize, band: usize, prefer: i32) -> Option<i32> {
    // Frames must overlap by an eighth of the band at least.
    let overlap = (band / 8).max(12).min(band);
    let reach = (band - overlap) as i32;
    let range = |d: i32| (top + (-d).max(0) as usize, top + band - d.max(0) as usize);
    let pull = |d: i32| ((d - prefer).abs() as i64).min(200) / 20;
    // Quick pass: every movement, on a sample of the rows with detail.
    let mut quick: Vec<(i64, i32)> = Vec::new();
    for d in -reach..=reach {
        let (from, to) = range(d);
        let lo = new.detail.partition_point(|&y| (y as usize) < from);
        let hi = new.detail.partition_point(|&y| (y as usize) < to);
        if hi == lo {
            continue;
        }
        let step = (hi - lo).div_ceil(SAMPLES);
        let f = fit(new, old, new.detail[lo..hi].iter().step_by(step).map(|&y| y as usize), d);
        if f.same > 0 && f.differ <= f.same {
            quick.push((f.score() - pull(d), d));
        }
    }
    quick.sort_by(|a, b| b.0.cmp(&a.0));
    let mut candidates: Vec<i32> = quick.iter().take(CANDIDATES).map(|&(_, d)| d).collect();
    if prefer != 0 && prefer.abs() <= reach && !candidates.contains(&prefer) {
        candidates.push(prefer);
    }
    // Full pass on the best of them: every row.
    let mut best: Option<(i64, i32)> = None;
    for d in candidates {
        let (from, to) = range(d);
        let f = fit(new, old, from..to, d);
        if !f.trusted(band) {
            continue;
        }
        let score = f.score() - pull(d);
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, d));
        }
    }
    if best.is_some() {
        return best.map(|(_, d)| d);
    }
    // Plain where they overlap (a blank part of the page): nothing tells where the content
    // is, so it is taken to have moved as it did last time (or not at all), if nothing
    // contradicts that.
    let mut guesses = vec![0];
    if prefer != 0 && prefer.abs() <= reach {
        guesses.insert(0, prefer);
    }
    guesses.into_iter().find(|&d| {
        let (from, to) = range(d);
        (from..to).all(|i| new.compare(i, old, (i as i64 + d as i64) as usize) != Row::Differ)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page `w` wide and `h` tall: lines of "text" (random dark specks on white) with plain
    /// white gaps, like a web page.
    fn page(w: u32, h: u32, seed: u32) -> Bitmap {
        let mut b = Bitmap::filled(w, h, [255, 255, 255, 255]);
        let mut s = seed.wrapping_mul(2654435761) | 1;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        let mut y = 4;
        while y + 14 < h {
            let len = 40 + next() % (w - 60);
            for yy in y..y + 12 {
                for x in 10..10 + len {
                    if next() % 3 == 0 {
                        b.set_pixel(x, yy, [(next() % 90) as u8, 30, 60, 255]);
                    }
                }
            }
            y += 12 + 6 + next() % 14;
        }
        b
    }

    /// What the screen shows with the page scrolled to `at`: a header, the page, a footer.
    fn view(content: &Bitmap, at: u32, w: u32, h: u32, header: &Bitmap, footer: &Bitmap) -> Bitmap {
        let mut f = Bitmap::filled(w, h, [0, 0, 0, 255]);
        crate::paste(&mut f, header, 0, 0);
        let band = content.crop(crate::Rect::new(0, at as i32, w, h - header.height - footer.height)).unwrap();
        crate::paste(&mut f, &band, 0, header.height as i64);
        crate::paste(&mut f, footer, 0, (h - footer.height) as i64);
        f
    }

    fn plain_view(content: &Bitmap, at: u32, w: u32, h: u32) -> Bitmap {
        let none = Bitmap::filled(w, 0, [0; 4]);
        view(content, at, w, h, &none, &none)
    }

    fn fill(b: &mut Bitmap, x: u32, y: u32, w: u32, h: u32, c: [u8; 4]) {
        for yy in y..(y + h).min(b.height) {
            for xx in x..(x + w).min(b.width) {
                b.set_pixel(xx, yy, c);
            }
        }
    }

    /// A browser's scroll bar at the right of a view of a page `page_h` tall scrolled to `at`:
    /// a grey track and a darker thumb that moves at its own pace.
    fn with_scroll_bar(mut v: Bitmap, at: u32, page_h: u32) -> Bitmap {
        let (w, h) = (v.width, v.height);
        fill(&mut v, w - 14, 0, 14, h, [241, 241, 241, 255]);
        let thumb = (h as u64 * h as u64 / page_h as u64) as u32;
        let y = (at as u64 * h as u64 / page_h as u64) as u32;
        fill(&mut v, w - 12, y, 10, thumb, [160, 160, 160, 255]);
        v
    }

    fn assert_page_part(pic: &Bitmap, content: &Bitmap, x: u32, w: u32, rows: u32) {
        assert_eq!(pic.crop(crate::Rect::new(x as i32, 0, w, rows)).unwrap(), content.crop(crate::Rect::new(x as i32, 0, w, rows)).unwrap(), "the joined picture differs from the page");
    }

    #[test]
    fn frames_of_a_scrolled_page_join_into_the_whole_page_with_its_header_once() {
        let (w, h) = (300, 240);
        let content = page(w, 1500, 7);
        let header = Bitmap::filled(w, 30, [40, 90, 200, 255]);
        let mut footer = Bitmap::filled(w, 20, [230, 230, 230, 255]);
        footer.set_pixel(5, 5, [0, 0, 0, 255]);
        let band = h - 30 - 20;
        let mut s = Stitcher::new();
        let mut at = 0;
        // Irregular steps, a pause, a step back up, then on to the end.
        for step in [0, 37, 37, 0, 64, 120, -40, 15, 150, 150, 150, 150, 150, 150, 150, 150, 150, 150] {
            at = (at as i32 + step).clamp(0, (content.height - band) as i32) as u32;
            let r = s.push(&view(&content, at, w, h, &header, &footer));
            assert_ne!(r, Pushed::Lost, "at {at}");
        }
        let pic = s.picture().unwrap();
        let expected_rows = at + band;
        assert_eq!(pic.height, 30 + expected_rows + 20);
        // Header once at the top, then exactly the page, then the footer.
        assert_eq!(pic.crop(crate::Rect::new(0, 0, w, 30)).unwrap(), header);
        assert_eq!(pic.crop(crate::Rect::new(0, 30, w, expected_rows)).unwrap(), content.crop(crate::Rect::new(0, 0, w, expected_rows)).unwrap());
        assert_eq!(pic.crop(crate::Rect::new(0, (30 + expected_rows) as i32, w, 20)).unwrap(), footer);
    }

    #[test]
    fn a_page_without_header_and_with_wide_blank_gaps_still_joins() {
        let (w, h) = (200, 160);
        let mut content = page(w, 900, 3);
        // A big blank part in the middle of the page.
        fill(&mut content, 0, 300, w, 120, [255, 255, 255, 255]);
        let mut s = Stitcher::new();
        for at in (0..=740).step_by(50) {
            assert_ne!(s.push(&plain_view(&content, at, w, h)), Pushed::Lost, "at {at}");
        }
        let pic = s.picture().unwrap();
        assert_eq!(pic.height, 700 + h);
        assert_eq!(pic, content.crop(crate::Rect::new(0, 0, w, 860)).unwrap());
    }

    #[test]
    fn scrolling_too_fast_is_said_and_joining_goes_on_after_going_back() {
        let (w, h) = (200, 150);
        let content = page(w, 1200, 11);
        let mut s = Stitcher::new();
        assert_eq!(s.push(&plain_view(&content, 0, w, h)), Pushed::First);
        assert!(matches!(s.push(&plain_view(&content, 60, w, h)), Pushed::Moved { by: 60, added: 60 }));
        // A jump of 400 rows: no overlap.
        assert_eq!(s.push(&plain_view(&content, 460, w, h)), Pushed::Lost);
        // Back near the last frame used: it goes on.
        assert!(matches!(s.push(&plain_view(&content, 150, w, h)), Pushed::Moved { by: 90, .. }));
        assert_eq!(s.picture().unwrap(), content.crop(crate::Rect::new(0, 0, w, 300)).unwrap());
        // The same frame again, or another size: nothing happens.
        assert_eq!(s.push(&plain_view(&content, 150, w, h)), Pushed::Same);
        assert_eq!(s.push(&Bitmap::filled(10, 10, [0; 4])), Pushed::Refused);
    }

    #[test]
    fn one_frame_is_the_picture_itself() {
        let f = page(100, 80, 5);
        let mut s = Stitcher::new();
        assert_eq!(s.picture(), None);
        s.push(&f);
        assert_eq!(s.picture().unwrap(), f);
        assert_eq!(s.height(), 80);
    }

    /// The scroll bar was left in the chosen part, on a short page (a long thumb that moves
    /// with every step): one wheel notch at a time (about 100 rows) joins without a miss.
    #[test]
    fn a_scroll_bar_in_the_chosen_part_does_not_stop_the_joining() {
        let (w, h) = (400, 300);
        let content = page(w, 700, 21);
        let mut s = Stitcher::new();
        let mut at = 0;
        s.push(&with_scroll_bar(plain_view(&content, 0, w, h), 0, content.height));
        while at < content.height - h {
            at = (at + 100).min(content.height - h);
            let r = s.push(&with_scroll_bar(plain_view(&content, at, w, h), at, content.height));
            assert!(matches!(r, Pushed::Moved { .. } | Pushed::First), "at {at}: {r:?}");
        }
        let pic = s.picture().unwrap();
        assert_eq!(pic.height, content.height);
        // The page itself is exact (the scroll bar shows wherever it was).
        assert_page_part(&pic, &content, 0, w - 14, content.height);
    }

    /// Things that change on the page while it scrolls: a row highlighted under the still
    /// pointer, a picture that loads, slight noise in every frame.
    #[test]
    fn changes_on_the_page_while_it_scrolls_do_not_stop_the_joining() {
        let (w, h) = (360, 280);
        let content = page(w, 1400, 5);
        let mut s = Stitcher::new();
        let mut losts = 0;
        let mut at = 0;
        let mut n = 0u32;
        while at < content.height - h {
            let mut f = plain_view(&content, at, w, h);
            // The row under the pointer (always at the same place on the screen) is highlighted.
            for y in 150..180 {
                for x in 0..w {
                    let p = f.pixel(x, y);
                    f.set_pixel(x, y, [p[0].saturating_sub(20), p[1].saturating_sub(10), p[2], 255]);
                }
            }
            // From the fourth frame on, a picture has loaded where the page was blank.
            if n >= 3 {
                let top = 500i64 - at as i64;
                if top < h as i64 && top + 90 > 0 {
                    let y0 = top.max(0) as u32;
                    for y in y0..((top + 90).min(h as i64) as u32) {
                        for x in 60..300 {
                            f.set_pixel(x, y, [(x * 3 % 256) as u8, (y as i64 - top) as u8 * 2, 90, 255]);
                        }
                    }
                }
            }
            // Noise of one level here and there.
            for (i, b) in f.rgba.iter_mut().enumerate() {
                if (i / 4 + n as usize) % 7 == 0 && i % 4 == 1 {
                    *b = b.saturating_sub(1);
                }
            }
            if s.push(&f) == Pushed::Lost {
                losts += 1;
            }
            at = (at + 45).min(content.height - h);
            n += 1;
            if at == content.height - h {
                assert_ne!(s.push(&plain_view(&content, at, w, h)), Pushed::Lost);
                break;
            }
        }
        assert_eq!(losts, 0);
        assert_eq!(s.picture().unwrap().height, content.height);
    }

    /// Started in the middle of the page, scrolled up to its top, then down again: the part
    /// above the start is in the picture too.
    #[test]
    fn what_is_above_the_start_is_added_on_top() {
        let (w, h) = (240, 200);
        let content = page(w, 1000, 9);
        let mut s = Stitcher::new();
        for at in [400, 340, 280, 200, 120, 40, 0, 80, 180, 300, 420, 540, 660] {
            assert_ne!(s.push(&plain_view(&content, at, w, h)), Pushed::Lost, "at {at}");
        }
        let pic = s.picture().unwrap();
        assert_eq!(pic, content.crop(crate::Rect::new(0, 0, w, 660 + h)).unwrap());
    }

    /// A list whose items all have the same icon and frame (only their names differ): a jump
    /// past the overlap is said, never joined at a wrong place.
    #[test]
    fn a_jump_on_a_list_of_look_alike_items_is_not_joined_wrongly() {
        let (w, h) = (300, 240);
        let names = page(w, 2000, 13);
        let mut list = Bitmap::filled(w, 2000, [255, 255, 255, 255]);
        for item in 0..2000 / 40 {
            let y = item * 40;
            fill(&mut list, 8, y + 6, 28, 28, [30, 120, 220, 255]);
            fill(&mut list, 0, y + 39, w, 1, [210, 210, 210, 255]);
            // The name: a piece of "text".
            let name = names.crop(crate::Rect::new(50, (y + 14) as i32, 200, 12)).unwrap();
            crate::paste(&mut list, &name, 50, (y + 14) as i64);
        }
        let mut s = Stitcher::new();
        s.push(&plain_view(&list, 0, w, h));
        assert!(matches!(s.push(&plain_view(&list, 40, w, h)), Pushed::Moved { by: 40, .. }));
        // 400 rows down in one go: nothing in common with the last view.
        assert_eq!(s.push(&plain_view(&list, 440, w, h)), Pushed::Lost);
        assert!(matches!(s.push(&plain_view(&list, 120, w, h)), Pushed::Moved { by: 80, .. }));
        assert_eq!(s.picture().unwrap(), list.crop(crate::Rect::new(0, 0, w, 120 + h)).unwrap());
    }
}

