//! Obtura's log file, `~/.local/state/obtura/obtura.log`: what went wrong, so the owner does
//! not have to describe it. It stays on this computer (nothing is ever sent anywhere).
//!
//! What goes in: the start of each run (version, system), errors and warnings, crashes
//! (with where they happened), the messages GTK and GLib print. What never goes in: the
//! picture, the text read from it, what was typed on it, the clipboard. At most 1 MB is
//! kept, plus the previous 1 MB in `obtura.log.1`.

use crate::naming::LocalTime;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The log is moved to `obtura.log.1` at the start of a run when it is larger than this.
const MAX_BYTES: u64 = 1_000_000;

static LOG: Mutex<Option<File>> = Mutex::new(None);
/// Local time, supplied by the program (this crate has no time zone); none until `init`.
static CLOCK: Mutex<Option<fn() -> LocalTime>> = Mutex::new(None);

pub fn path() -> PathBuf {
    crate::paths::state_dir().join("obtura.log")
}

/// Opens the log, writes the start of the run and makes crashes (panics) land in it.
/// `clock` gives the local time. Never fails: without a log file, Obtura still runs.
pub fn init(clock: fn() -> LocalTime) {
    *CLOCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(clock);
    open_at(&path());
    info(&session_header());
    install_panic_hook();
}

/// A crash (panic) is written to the log, with where it happened, before the usual message.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let what = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        error(&format!("CRASH (panic) at {place}: {what}\n{}", std::backtrace::Backtrace::force_capture()));
        previous(info);
    }));
}

fn open_at(path: &Path) {
    let Some(dir) = path.parent() else { return };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    if fs::metadata(path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let mut old = path.as_os_str().to_owned();
        old.push(".1");
        let _ = fs::rename(path, old);
    }
    if let Ok(f) = OpenOptions::new().create(true).append(true).open(path) {
        *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }
}

/// Version and system, the first lines of every run.
fn session_header() -> String {
    let os = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
        .unwrap_or_else(|| "unknown system".into());
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| "?".into());
    format!(
        "---- Obtura {} started (process {}) on {os}; desktop {}, session {}, display {}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        var("XDG_CURRENT_DESKTOP"),
        var("XDG_SESSION_TYPE"),
        if std::env::var_os("WAYLAND_DISPLAY").is_some() { "Wayland" } else if std::env::var_os("DISPLAY").is_some() { "X11" } else { "none" },
    )
}

fn stamp() -> String {
    match *CLOCK.lock().unwrap_or_else(|e| e.into_inner()) {
        Some(clock) => {
            let t = clock();
            format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
        }
        None => "-".into(),
    }
}

fn line(level: &str, msg: &str) -> String {
    format!("{} [{level}] {}\n", stamp(), msg.trim_end())
}

fn write(level: &str, msg: &str) {
    let text = line(level, msg);
    if let Some(f) = LOG.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = f.write_all(text.as_bytes());
    }
}

pub fn info(msg: &str) {
    write("info", msg);
}

/// Into the file only (for messages something else already prints, such as GLib's).
pub fn note(level: &str, msg: &str) {
    write(level, msg);
}

/// Something unexpected that Obtura worked around.
pub fn warn(msg: &str) {
    write("warning", msg);
    eprintln!("obtura: {msg}");
}

/// Something that did not work.
pub fn error(msg: &str) {
    write("ERROR", msg);
    eprintln!("obtura: {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("obtura-log-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn lines_have_the_time_and_the_level() {
        *CLOCK.lock().unwrap() = Some(|| LocalTime { year: 2026, month: 11, day: 5, hour: 9, minute: 3, second: 7 });
        assert_eq!(line("ERROR", "no luck\n"), "2026-11-05 09:03:07 [ERROR] no luck\n");
    }

    #[test]
    fn a_big_log_moves_aside_and_a_small_one_is_added_to() {
        let d = dir("rotate");
        let p = d.join("obtura.log");
        fs::create_dir_all(&d).unwrap();
        fs::write(&p, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        open_at(&p);
        assert!(d.join("obtura.log.1").is_file());
        assert_eq!(fs::metadata(&p).unwrap().len(), 0);
        write("info", "one");
        open_at(&p);
        write("info", "two");
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("one") && text.contains("two"), "{text}");
        assert!(!d.join("obtura.log.2").exists());
        // A crash lands in the log, with its place (same test: the log is one global).
        install_panic_hook();
        let _ = std::panic::catch_unwind(|| panic!("test crash"));
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("CRASH (panic)") && text.contains("test crash") && text.contains("log.rs"), "{text}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_header_names_the_version() {
        assert!(session_header().contains(env!("CARGO_PKG_VERSION")));
    }
}
