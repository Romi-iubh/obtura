//! File names from a template such as `Screenshot %Y-%m-%d %H.%M.%S`.

use std::path::{Path, PathBuf};

/// A local date and time, supplied by the caller (this crate has no clock or time zone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

pub const DEFAULT_TEMPLATE: &str = "Screenshot %Y-%m-%d %H.%M.%S";

/// Fills in `%Y` (year), `%y` (two digits), `%m` `%d` `%H` `%M` `%S` and `%%`. Anything
/// else is kept as typed. The result is safe to use as a file name (no extension).
pub fn format_name(template: &str, t: &LocalTime) -> String {
    let mut out = String::new();
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&format!("{:04}", t.year)),
            Some('y') => out.push_str(&format!("{:02}", t.year.rem_euclid(100))),
            Some('m') => out.push_str(&format!("{:02}", t.month)),
            Some('d') => out.push_str(&format!("{:02}", t.day)),
            Some('H') => out.push_str(&format!("{:02}", t.hour)),
            Some('M') => out.push_str(&format!("{:02}", t.minute)),
            Some('S') => out.push_str(&format!("{:02}", t.second)),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    sanitize(&out)
}

/// Removes what a file name cannot hold (`/`, control characters), leading dots and
/// spaces, and anything past 150 bytes. Never returns an empty name.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '-' } else { c }).collect();
    let mut s = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    while s.len() > 150 {
        s.pop();
    }
    if s.is_empty() {
        "Screenshot".into()
    } else {
        s
    }
}

/// A path in `dir` for `stem.ext` that does not exist yet: `stem.ext`, `stem (2).ext`, ...
///
/// `Screenshot.png` is never returned: it is the name the desktop's screenshot portal
/// writes in the Pictures folder, and Obtura removes that file after every capture.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let reserved = |name: &str| name == "Screenshot.png";
    let mut n = 1u32;
    loop {
        let name = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} ({n}).{ext}") };
        if !reserved(&name) && !dir.join(&name).exists() {
            return dir.join(name);
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: LocalTime = LocalTime { year: 2026, month: 11, day: 4, hour: 9, minute: 5, second: 7 };

    #[test]
    fn fills_the_default_template() {
        assert_eq!(format_name(DEFAULT_TEMPLATE, &T), "Screenshot 2026-11-04 09.05.07");
    }

    #[test]
    fn handles_short_year_percent_and_unknown_codes() {
        assert_eq!(format_name("%y%m%d 100%% %Q %", &T), "261104 100% %Q %");
    }

    #[test]
    fn a_template_cannot_escape_the_folder() {
        assert_eq!(format_name("../evil/%Y", &T), "-evil-2026");
        assert_eq!(format_name("", &T), "Screenshot");
        assert_eq!(format_name("...", &T), "Screenshot");
    }

    #[test]
    fn long_names_are_cut() {
        assert!(format_name(&"a".repeat(500), &T).len() <= 150);
    }

    #[test]
    fn unique_paths_count_up_and_skip_the_portals_name() {
        let dir = std::env::temp_dir().join(format!("obtura-naming-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "shot", "png"), dir.join("shot.png"));
        std::fs::write(dir.join("shot.png"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "shot", "png"), dir.join("shot (2).png"));
        std::fs::write(dir.join("shot (2).png"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "shot", "png"), dir.join("shot (3).png"));
        assert_eq!(unique_path(&dir, "Screenshot", "png"), dir.join("Screenshot (2).png"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
