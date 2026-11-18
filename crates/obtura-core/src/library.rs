//! The library: every capture kept on this computer, found again by its date or by the words
//! in it. One folder per capture in `~/.local/share/obtura/library/`:
//!
//! * `original.png`: the screenshot as it was taken (never changed);
//! * `picture.png` and `marks.json`: the picture and its marks as last saved from the editor
//!   (only once it was edited), so it opens again ready to edit;
//! * `thumb.png`: a small picture for the library window;
//! * `item.json`: the date, the size, the text read in it, its tags, and whether to keep it
//!   always.
//!
//! Removing an item removes only these files (and the folder if nothing else is in it):
//! never anything Obtura did not put there.

use crate::naming::LocalTime;
use crate::paths;
use obtura_image::{resize, Bitmap};
use obtura_scene::{Frame, Object};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The files of an item, all made by Obtura (each may also have a `.tmp` left by a crash).
const FILES: [&str; 5] = ["original.png", "picture.png", "marks.json", "thumb.png", "item.json"];

/// What is known about a capture.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    /// Local time of the capture, `YYYY-MM-DD HH:MM:SS`.
    pub created: String,
    pub width: u32,
    pub height: u32,
    /// The text read in it; `None` until it was read.
    pub text: Option<String>,
    /// Kept whatever the clean-up setting says.
    pub keep: bool,
    /// It was edited and saved (it has `picture.png` and `marks.json`).
    pub edited: bool,
    /// Words the user gave it ("invoice", "bug 42"), found by the search like its text.
    pub tags: Vec<String>,
}

/// The marks of an edited capture, as saved.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Marks {
    pub marks: Vec<Object>,
    pub frame: Option<Frame>,
}

impl Marks {
    pub fn to_json(&self) -> std::io::Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(std::io::Error::other)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The folder's name: the capture time, sortable (`20261114-193025`, `-2` and so on for
    /// several in one second).
    pub id: String,
    pub meta: Meta,
}

#[derive(Clone, Debug)]
pub struct Library {
    dir: PathBuf,
}

impl Default for Library {
    fn default() -> Self {
        Self::at(paths::data_dir().join("library"))
    }
}

/// The largest thumbnail, in pixels.
pub const THUMB_W: u32 = 320;
pub const THUMB_H: u32 = 200;

/// A small copy of `b` that fits in `THUMB_W` x `THUMB_H` (never enlarged).
pub fn thumbnail(b: &Bitmap) -> Bitmap {
    let k = (THUMB_W as f64 / b.width as f64).min(THUMB_H as f64 / b.height as f64).min(1.0);
    resize(b, ((b.width as f64 * k).round() as u32).max(1), ((b.height as f64 * k).round() as u32).max(1))
}

/// Days from 1970-01-01 to the given date (proleptic Gregorian calendar).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn day_of(created: &str) -> Option<i64> {
    let date = created.get(..10)?;
    let mut it = date.split('-');
    let (y, m, d) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    Some(days_from_civil(y, m, d))
}

fn stamp(t: &LocalTime) -> String {
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

/// Writes through a temporary file, so a crash never leaves half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

/// `s` in lower case without accents, for searching: "Été" and "ete" are the same word, as are
/// "Straße" and "strasse". Other letters stay as they are.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => out.push('c'),
            'ď' | 'đ' => out.push('d'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => out.push('e'),
            'ĝ' | 'ğ' | 'ġ' | 'ģ' => out.push('g'),
            'ĥ' | 'ħ' => out.push('h'),
            'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => out.push('i'),
            'ĵ' => out.push('j'),
            'ķ' => out.push('k'),
            'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => out.push('l'),
            'ñ' | 'ń' | 'ņ' | 'ň' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => out.push('o'),
            'ŕ' | 'ŗ' | 'ř' => out.push('r'),
            'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => out.push('s'),
            'ţ' | 'ť' | 'ŧ' | 'ț' => out.push('t'),
            'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => out.push('u'),
            'ŵ' => out.push('w'),
            'ý' | 'ÿ' | 'ŷ' => out.push('y'),
            'ź' | 'ż' | 'ž' => out.push('z'),
            'ß' => out.push_str("ss"),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            'þ' => out.push_str("th"),
            // Typographic apostrophes and dashes as typed on a keyboard.
            '’' | '‘' => out.push('\''),
            '–' | '—' => out.push('-'),
            c => out.push(c),
        }
    }
    out
}

/// Whether every word of `query` is found in `haystack` (case and accents do not matter).
fn matches(haystack: &str, query: &str) -> bool {
    let h = fold(haystack);
    fold(query).split_whitespace().all(|w| h.contains(w))
}

/// Tags as typed ("invoice, Bug 42 ,, invoice"): trimmed, empty ones and repeats dropped (the
/// first spelling kept), at most 64 characters each.
pub fn clean_tags(typed: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for t in typed.split([',', ';', '\n']) {
        let t: String = t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(64).collect();
        if !t.is_empty() && !tags.iter().any(|x| fold(x) == fold(&t)) {
            tags.push(t);
        }
    }
    tags
}

impl Library {
    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn item_dir(&self, id: &str) -> Option<PathBuf> {
        // Only names Obtura makes: digits and dashes. Nothing can point outside the library.
        (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit() || c == '-')).then(|| self.dir.join(id))
    }

    /// Adds a capture: its PNG as taken and its thumbnail (PNG). Returns the new item.
    pub fn add(&self, png: &[u8], thumb_png: &[u8], width: u32, height: u32, when: &LocalTime) -> std::io::Result<Item> {
        std::fs::create_dir_all(&self.dir)?;
        let base = format!("{:04}{:02}{:02}-{:02}{:02}{:02}", when.year, when.month, when.day, when.hour, when.minute, when.second);
        let mut id = base.clone();
        let mut n = 2;
        // `create_dir` fails if it exists: two captures never share a folder.
        while let Err(e) = std::fs::create_dir(self.dir.join(&id)) {
            if e.kind() != std::io::ErrorKind::AlreadyExists || n > 999 {
                return Err(e);
            }
            id = format!("{base}-{n}");
            n += 1;
        }
        let dir = self.dir.join(&id);
        write_atomic(&dir.join("original.png"), png)?;
        write_atomic(&dir.join("thumb.png"), thumb_png)?;
        let meta = Meta { created: stamp(when), width, height, ..Meta::default() };
        self.write_meta(&id, &meta)?;
        Ok(Item { id, meta })
    }

    fn write_meta(&self, id: &str, meta: &Meta) -> std::io::Result<()> {
        let dir = self.item_dir(id).ok_or_else(|| std::io::Error::other("not a library item"))?;
        write_atomic(&dir.join("item.json"), &serde_json::to_vec_pretty(meta).map_err(std::io::Error::other)?)
    }

    pub fn get(&self, id: &str) -> Option<Item> {
        let dir = self.item_dir(id)?;
        let meta: Meta = serde_json::from_str(&std::fs::read_to_string(dir.join("item.json")).ok()?).ok()?;
        Some(Item { id: id.to_string(), meta })
    }

    /// Every item, newest first. Folders that are not items are left alone.
    pub fn items(&self) -> Vec<Item> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut items: Vec<Item> = entries.filter_map(|e| e.ok()).filter_map(|e| self.get(&e.file_name().to_string_lossy())).collect();
        items.sort_by(|a, b| b.meta.created.cmp(&a.meta.created).then(b.id.cmp(&a.id)));
        items
    }

    /// The items whose text, tags or date hold every word of `query` (all of them for an empty
    /// one).
    pub fn search(&self, query: &str) -> Vec<Item> {
        self.items()
            .into_iter()
            .filter(|i| matches(&format!("{} {} {}", i.meta.created, i.meta.tags.join(" "), i.meta.text.as_deref().unwrap_or("")), query))
            .collect()
    }

    /// Every tag in use, in alphabetical order (for suggestions).
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = Vec::new();
        for item in self.items() {
            for t in item.meta.tags {
                if !tags.iter().any(|x| fold(x) == fold(&t)) {
                    tags.push(t);
                }
            }
        }
        tags.sort_by_key(|t| fold(t));
        tags
    }

    pub fn set_tags(&self, id: &str, tags: Vec<String>) -> std::io::Result<()> {
        self.update(id, |m| m.tags = tags)
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Meta)) -> std::io::Result<()> {
        let mut item = self.get(id).ok_or_else(|| std::io::Error::other("no such library item"))?;
        f(&mut item.meta);
        self.write_meta(id, &item.meta)
    }

    pub fn set_text(&self, id: &str, text: &str) -> std::io::Result<()> {
        self.update(id, |m| m.text = Some(text.to_string()))
    }

    pub fn set_keep(&self, id: &str, keep: bool) -> std::io::Result<()> {
        self.update(id, |m| m.keep = keep)
    }

    /// Path of a file of the item (`original.png`, `thumb.png`...).
    pub fn file(&self, id: &str, name: &str) -> Option<PathBuf> {
        self.item_dir(id).map(|d| d.join(name))
    }

    /// Keeps the edited picture and its marks (and a new thumbnail), so it opens again as left.
    pub fn save_edits(&self, id: &str, picture_png: &[u8], marks: &Marks, thumb_png: &[u8]) -> std::io::Result<()> {
        self.save_edits_json(id, picture_png, &marks.to_json()?, thumb_png)
    }

    /// [`Library::save_edits`] with the marks already made into JSON (marks holding pictures
    /// cannot go to another thread; their JSON can).
    pub fn save_edits_json(&self, id: &str, picture_png: &[u8], marks_json: &[u8], thumb_png: &[u8]) -> std::io::Result<()> {
        let dir = self.item_dir(id).ok_or_else(|| std::io::Error::other("not a library item"))?;
        write_atomic(&dir.join("picture.png"), picture_png)?;
        write_atomic(&dir.join("marks.json"), marks_json)?;
        write_atomic(&dir.join("thumb.png"), thumb_png)?;
        self.update(id, |m| m.edited = true)
    }

    /// The picture to edit and its marks: as last saved, or the capture as taken.
    pub fn load(&self, id: &str) -> Option<(Vec<u8>, Marks)> {
        let item = self.get(id)?;
        let dir = self.item_dir(id)?;
        if item.meta.edited {
            if let (Ok(png), Ok(json)) = (std::fs::read(dir.join("picture.png")), std::fs::read_to_string(dir.join("marks.json"))) {
                if let Ok(marks) = serde_json::from_str::<Marks>(&json) {
                    return Some((png, marks));
                }
            }
        }
        Some((std::fs::read(dir.join("original.png")).ok()?, Marks::default()))
    }

    /// Removes an item: its own files, then its folder if nothing else is in it.
    pub fn remove(&self, id: &str) -> std::io::Result<()> {
        let dir = self.item_dir(id).ok_or_else(|| std::io::Error::other("not a library item"))?;
        if !dir.join("item.json").exists() {
            return Err(std::io::Error::other("not a library item"));
        }
        for f in FILES.iter().flat_map(|f| [f.to_string(), format!("{f}.tmp")]) {
            match std::fs::remove_file(dir.join(f)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        // Fails (and keeps it) if something else was put in there.
        let _ = std::fs::remove_dir(&dir);
        Ok(())
    }

    /// Removes the items taken more than `days` days before `today` that are not kept.
    /// `days` = 0 removes nothing. Returns how many were removed.
    pub fn clean(&self, days: u32, today: &LocalTime) -> usize {
        if days == 0 {
            return 0;
        }
        let now = days_from_civil(today.year, today.month, today.day);
        self.items().into_iter().filter(|i| !i.meta.keep && day_of(&i.meta.created).is_some_and(|d| now - d > days as i64)).filter(|i| self.remove(&i.id).is_ok()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use obtura_scene::{Kind, Point, Style};

    fn scratch(name: &str) -> Library {
        let dir = std::env::temp_dir().join(format!("obtura-library-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Library::at(dir)
    }

    fn at(day: u32, second: u32) -> LocalTime {
        LocalTime { year: 2026, month: 11, day, hour: 10, minute: 0, second }
    }

    #[test]
    fn dates_count_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 11, 14) - days_from_civil(2026, 10, 31), 14);
        assert_eq!(days_from_civil(2024, 3, 1) - days_from_civil(2024, 2, 28), 2, "2024 is a leap year");
    }

    #[test]
    fn captures_are_added_newest_first_and_never_share_a_folder() {
        let lib = scratch("add");
        let a = lib.add(b"png-a", b"thumb", 10, 10, &at(14, 5)).unwrap();
        let b = lib.add(b"png-b", b"thumb", 10, 10, &at(14, 5)).unwrap();
        let c = lib.add(b"png-c", b"thumb", 20, 10, &at(14, 9)).unwrap();
        assert_eq!(a.id, "20261114-100005");
        assert_eq!(b.id, "20261114-100005-2");
        let ids: Vec<String> = lib.items().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![c.id.clone(), b.id, a.id]);
        assert_eq!(lib.load(&c.id).unwrap().0, b"png-c");
        let _ = std::fs::remove_dir_all(lib.dir());
    }

    #[test]
    fn found_by_the_words_in_it_or_its_date() {
        let lib = scratch("search");
        let a = lib.add(b"a", b"t", 1, 1, &at(13, 0)).unwrap();
        let b = lib.add(b"b", b"t", 1, 1, &at(14, 0)).unwrap();
        lib.set_text(&a.id, "Invoice total 42 EUR").unwrap();
        lib.set_text(&b.id, "Meeting notes").unwrap();
        let found = |q: &str| lib.search(q).into_iter().map(|i| i.id).collect::<Vec<_>>();
        assert_eq!(found("invoice"), vec![a.id.clone()]);
        assert_eq!(found("total eur"), vec![a.id.clone()]);
        assert_eq!(found("total notes"), Vec::<String>::new(), "every word must be there");
        assert_eq!(found("2026-11-14"), vec![b.id.clone()]);
        assert_eq!(found("").len(), 2);
        let _ = std::fs::remove_dir_all(lib.dir());
    }

    #[test]
    fn search_ignores_accents_and_finds_tags() {
        let lib = scratch("tags");
        let t = at(16, 0);
        let a = lib.add(b"png", b"thumb", 10, 10, &t).unwrap();
        lib.set_text(&a.id, "Réunion du comité à Yaoundé — Straße").unwrap();
        let b = lib.add(b"png", b"thumb", 10, 10, &t).unwrap();
        lib.set_tags(&b.id, clean_tags("Invoice, bug 42 ,, invoice;  Été  ")).unwrap();
        assert_eq!(lib.get(&b.id).unwrap().meta.tags, ["Invoice", "bug 42", "Été"]);
        let ids = |q: &str| lib.search(q).into_iter().map(|i| i.id).collect::<Vec<_>>();
        assert_eq!(ids("reunion comite yaounde"), [a.id.clone()]);
        assert_eq!(ids("RÉUNION strasse"), [a.id.clone()]);
        assert_eq!(ids("invoice"), [b.id.clone()]);
        assert_eq!(ids("ete bug"), [b.id.clone()]);
        assert!(ids("invoice réunion").is_empty());
        assert_eq!(lib.all_tags(), ["bug 42", "Été", "Invoice"]);
        // An item from before tags existed reads with none.
        let old = r#"{"created":"2026-11-14 10:00:00","width":1,"height":1,"text":null,"keep":false,"edited":false}"#;
        assert_eq!(serde_json::from_str::<Meta>(old).unwrap().tags, Vec::<String>::new());
        let _ = std::fs::remove_dir_all(lib.dir());
    }

    #[test]
    fn edits_are_kept_and_open_again() {
        let lib = scratch("edits");
        let a = lib.add(b"original", b"t", 1, 1, &at(14, 0)).unwrap();
        let mut s = obtura_scene::Scene::new();
        s.add(Kind::Line { from: Point::new(1.0, 2.0), to: Point::new(3.0, 4.0), via: vec![] }, Style::default());
        let marks = Marks { marks: s.objects().to_vec(), frame: Some(Frame::default()) };
        lib.save_edits(&a.id, b"edited", &marks, b"t2").unwrap();
        let (png, back) = lib.load(&a.id).unwrap();
        assert_eq!(png, b"edited");
        assert_eq!(back, marks);
        assert_eq!(std::fs::read(lib.file(&a.id, "original.png").unwrap()).unwrap(), b"original", "the capture as taken stays");
        let _ = std::fs::remove_dir_all(lib.dir());
    }

    #[test]
    fn removing_takes_only_obtura_files_and_clean_up_spares_kept_items() {
        let lib = scratch("remove");
        let old = lib.add(b"a", b"t", 1, 1, &at(1, 0)).unwrap();
        let kept = lib.add(b"b", b"t", 1, 1, &at(2, 0)).unwrap();
        let recent = lib.add(b"c", b"t", 1, 1, &at(13, 0)).unwrap();
        lib.set_keep(&kept.id, true).unwrap();
        // Something the user put in an item's folder is never deleted.
        std::fs::write(lib.dir().join(&old.id).join("mine.txt"), b"mine").unwrap();
        assert_eq!(lib.clean(0, &at(14, 0)), 0, "0 days: keep everything");
        assert_eq!(lib.clean(7, &at(14, 0)), 1);
        let ids: Vec<String> = lib.items().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![recent.id.clone(), kept.id.clone()]);
        assert_eq!(std::fs::read(lib.dir().join(&old.id).join("mine.txt")).unwrap(), b"mine");
        assert!(lib.remove("../../etc").is_err() && lib.remove("").is_err());
        assert!(lib.remove("20990101-000000").is_err(), "not an item");
        let _ = std::fs::remove_dir_all(lib.dir());
    }

    #[test]
    fn a_thumbnail_fits_and_is_never_enlarged() {
        let t = thumbnail(&Bitmap::filled(1920, 1080, [1, 2, 3, 255]));
        assert_eq!((t.width, t.height), (320, 180));
        let small = thumbnail(&Bitmap::filled(100, 50, [1, 2, 3, 255]));
        assert_eq!((small.width, small.height), (100, 50));
    }
}
