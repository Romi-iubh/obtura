//! Text recognition with the `tesseract` program (Ubuntu package `tesseract-ocr`), as Foliox
//! does. The picture goes to it through a pipe: nothing is written to disk, nothing leaves the
//! computer.

use obtura_image::{encode_png, resize, Bitmap};
use std::io::Write;
use std::process::{Command, Stdio};

/// One recognised word and where it is, in pixels of the picture given.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// How sure tesseract is, 0 to 100.
    pub conf: f32,
    /// Which text block, paragraph and line it belongs to (in reading order).
    pub block: u32,
    pub par: u32,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OcrError {
    /// The `tesseract` program is not installed.
    Missing,
    Failed(String),
}

impl std::fmt::Display for OcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OcrError::Missing if cfg!(windows) => write!(f, "Text recognition needs tesseract.exe, installed with Obtura: install Obtura again"),
            OcrError::Missing => write!(f, "Text recognition needs the tesseract program. Install it with: sudo apt install tesseract-ocr"),
            OcrError::Failed(why) => write!(f, "Text recognition failed: {why}"),
        }
    }
}

/// The program, from the system (never a copy shadowing it elsewhere on the PATH).
#[cfg(unix)]
fn command() -> Command {
    Command::new(if std::path::Path::new("/usr/bin/tesseract").exists() { "/usr/bin/tesseract" } else { "tesseract" })
}

/// Windows: the copy installed with Obtura (beside `obtura.exe`, its languages in
/// `..\share\tessdata`), started without a console window.
#[cfg(windows)]
fn command() -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let bin = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    let mut cmd = Command::new(bin.join("tesseract.exe"));
    if let Some(prefix) = bin.parent() {
        cmd.env("TESSDATA_PREFIX", prefix.join("share").join("tessdata"));
    }
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// The languages tesseract can read here (without its helper data "osd" and "equ").
pub fn languages() -> Result<Vec<String>, OcrError> {
    let out = command().arg("--list-langs").stdin(Stdio::null()).output().map_err(|_| OcrError::Missing)?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().skip(1).map(str::trim).filter(|l| !l.is_empty() && *l != "osd" && *l != "equ").map(String::from).collect())
}

/// The `-l` argument: English first if installed, and every other installed language
/// (at most three in all: each one slows recognition down).
pub fn default_languages(installed: &[String]) -> String {
    let mut langs: Vec<&str> = Vec::new();
    if installed.iter().any(|l| l == "eng") {
        langs.push("eng");
    }
    for l in installed {
        if langs.len() < 3 && !langs.contains(&l.as_str()) {
            langs.push(l);
        }
    }
    langs.join("+")
}

/// Screen text is small: tesseract reads it best enlarged. Pictures up to this many pixels
/// are read at twice their size.
const ENLARGE_UP_TO: u64 = 2_600_000;

/// Reads the words in `picture`. `langs` is tesseract's `-l` value ("eng", "eng+fra"); empty
/// picks the installed languages.
pub fn recognize(picture: &Bitmap, langs: &str) -> Result<Vec<Word>, OcrError> {
    let langs = if langs.is_empty() { default_languages(&languages()?) } else { langs.to_string() };
    let k = if (picture.width as u64) * (picture.height as u64) <= ENLARGE_UP_TO { 2 } else { 1 };
    let input = if k == 1 { encode_png(picture) } else { encode_png(&resize(picture, picture.width * k, picture.height * k)) };
    let png = input.map_err(|e| OcrError::Failed(e.to_string()))?;
    let mut cmd = command();
    cmd.args(["stdin", "stdout", "--dpi", &(96 * k).to_string()]);
    if !langs.is_empty() {
        cmd.args(["-l", &langs]);
    }
    cmd.arg("tsv");
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|_| OcrError::Missing)?;
    // Written from another thread: tesseract may start answering before it has read it all.
    let mut stdin = child.stdin.take().expect("piped");
    let writer = std::thread::spawn(move || stdin.write_all(&png));
    let out = child.wait_with_output().map_err(|e| OcrError::Failed(e.to_string()))?;
    let _ = writer.join();
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").to_string();
        return Err(OcrError::Failed(if why.is_empty() { format!("tesseract ended with {}", out.status) } else { why }));
    }
    let mut words = parse_tsv(&String::from_utf8_lossy(&out.stdout));
    if k > 1 {
        for w in &mut words {
            w.x /= k;
            w.y /= k;
            w.w = w.w.div_ceil(k);
            w.h = w.h.div_ceil(k);
        }
    }
    Ok(words)
}

/// The words of tesseract's TSV output (level 5 rows with some text).
pub fn parse_tsv(tsv: &str) -> Vec<Word> {
    tsv.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(12, '\t').collect();
            if f.len() < 12 || f[0] != "5" {
                return None;
            }
            let n = |i: usize| f[i].trim().parse::<u32>().ok();
            let text = f[11].trim();
            let conf = f[10].trim().parse::<f32>().ok()?;
            if text.is_empty() || conf < 0.0 {
                return None;
            }
            Some(Word { text: text.to_string(), x: n(6)?, y: n(7)?, w: n(8)?, h: n(9)?, conf, block: n(2)?, par: n(3)?, line: n(4)? })
        })
        .collect()
}

/// The words grouped in lines, in reading order.
pub fn lines(words: &[Word]) -> Vec<Vec<&Word>> {
    let mut out: Vec<Vec<&Word>> = Vec::new();
    for w in words {
        match out.last_mut() {
            Some(line) if line[0].block == w.block && line[0].par == w.par && line[0].line == w.line => line.push(w),
            _ => out.push(vec![w]),
        }
    }
    out
}

/// Words below this confidence that hold no letter or digit are specks, not text.
const SPECK: f32 = 50.0;

/// The text: words joined by spaces, one line per line, a blank line between blocks.
pub fn text(words: &[Word]) -> String {
    let mut out = String::new();
    let mut last_block = None;
    for line in lines(words) {
        let kept: Vec<&str> = line.iter().filter(|w| w.conf >= SPECK || w.text.chars().any(char::is_alphanumeric)).map(|w| w.text.as_str()).collect();
        if kept.is_empty() {
            continue;
        }
        if let Some(b) = last_block {
            out.push('\n');
            if b != line[0].block {
                out.push('\n');
            }
        }
        last_block = Some(line[0].block);
        out.push_str(&kept.join(" "));
    }
    out
}

/// The words whose middle lies inside the rectangle `(x, y, w, h)`.
pub fn inside(words: &[Word], (x, y, w, h): (u32, u32, u32, u32)) -> Vec<Word> {
    words
        .iter()
        .filter(|wd| {
            let (cx, cy) = (wd.x + wd.w / 2, wd.y + wd.h / 2);
            cx >= x && cx < x + w && cy >= y && cy < y + h
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext
1\t1\t0\t0\t0\t0\t0\t0\t800\t500\t-1\t
4\t1\t1\t1\t1\t0\t157\t232\t485\t37\t-1\t
5\t1\t1\t1\t1\t1\t157\t232\t144\t37\t92.25\tHello
5\t1\t1\t1\t1\t2\t316\t233\t128\t36\t96.7\tworld
5\t1\t1\t1\t2\t1\t157\t280\t60\t30\t91.0\tagain
5\t1\t2\t1\t1\t1\t741\t384\t1\t78\t95.0\t
5\t1\t3\t1\t1\t1\t10\t400\t30\t30\t88.0\tnext
5\t1\t3\t1\t1\t2\t50\t400\t6\t6\t12.0\t~
";

    #[test]
    fn reads_the_words_and_their_boxes_from_tesseract_output() {
        let words = parse_tsv(TSV);
        assert_eq!(words.len(), 5, "empty and non-word rows are skipped");
        assert_eq!(words[0], Word { text: "Hello".into(), x: 157, y: 232, w: 144, h: 37, conf: 92.25, block: 1, par: 1, line: 1 });
    }

    #[test]
    fn text_keeps_lines_and_separates_blocks_and_drops_specks() {
        assert_eq!(text(&parse_tsv(TSV)), "Hello world\nagain\n\nnext");
    }

    #[test]
    fn words_inside_a_rectangle() {
        let words = parse_tsv(TSV);
        let found: Vec<String> = inside(&words, (150, 220, 200, 50)).into_iter().map(|w| w.text).collect();
        assert_eq!(found, vec!["Hello"]);
    }

    #[test]
    fn english_comes_first_and_three_languages_at_most() {
        let l = |v: &[&str]| default_languages(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(l(&["deu", "eng", "fra", "spa"]), "eng+deu+fra");
        assert_eq!(l(&["fra"]), "fra");
        assert_eq!(l(&[]), "");
    }

    #[test]
    fn reads_the_test_picture_when_tesseract_is_installed() {
        if languages().is_err() {
            eprintln!("tesseract is not installed: skipped");
            return;
        }
        let png = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/sample.png")).unwrap();
        let picture = obtura_image::decode_png(&png).unwrap();
        let words = recognize(&picture, "").unwrap();
        let found = text(&words);
        assert!(found.contains("Obtura") && found.contains("fixture"), "{found:?}");
        let obtura = words.iter().find(|w| w.text == "Obtura").unwrap();
        // The word is at about (157, 232) in the 800 x 500 picture, whatever the enlargement.
        assert!(obtura.x.abs_diff(157) <= 3 && obtura.y.abs_diff(232) <= 3, "{obtura:?}");
    }
}
