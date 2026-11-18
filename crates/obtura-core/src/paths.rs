//! Where Obtura keeps things (XDG folders). The owner's data folders are only ever read
//! from or written into by Obtura's own code paths for its own files: never deleted.

use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// `$VAR` when it is a non-empty absolute path, else `$HOME/<fallback>`.
fn xdg_dir(var: Option<&str>, fallback: &str, home: &Path) -> PathBuf {
    match var {
        Some(v) if Path::new(v).is_absolute() => PathBuf::from(v),
        _ => home.join(fallback),
    }
}

fn env(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

/// `~/.config/obtura/`: settings.
pub fn config_dir() -> PathBuf {
    xdg_dir(env("XDG_CONFIG_HOME").as_deref(), ".config", &home()).join("obtura")
}

/// `~/.local/share/obtura/`: library, presets.
pub fn data_dir() -> PathBuf {
    xdg_dir(env("XDG_DATA_HOME").as_deref(), ".local/share", &home()).join("obtura")
}

/// `~/.local/state/obtura/`: logs and other state.
pub fn state_dir() -> PathBuf {
    xdg_dir(env("XDG_STATE_HOME").as_deref(), ".local/state", &home()).join("obtura")
}

/// `~/.cache/obtura/`: scratch files of work under way (a recording's parts), removed when done.
pub fn cache_dir() -> PathBuf {
    xdg_dir(env("XDG_CACHE_HOME").as_deref(), ".cache", &home()).join("obtura")
}

/// The user's Pictures folder, from `~/.config/user-dirs.dirs`, else `~/Pictures`.
pub fn pictures_dir() -> PathBuf {
    user_dir("XDG_PICTURES_DIR", "Pictures")
}

/// The user's Videos folder, from `~/.config/user-dirs.dirs`, else `~/Videos`.
pub fn videos_dir() -> PathBuf {
    user_dir("XDG_VIDEOS_DIR", "Videos")
}

fn user_dir(key: &str, fallback: &str) -> PathBuf {
    let home = home();
    let file = xdg_dir(env("XDG_CONFIG_HOME").as_deref(), ".config", &home).join("user-dirs.dirs");
    std::fs::read_to_string(file).ok().and_then(|c| parse_user_dir(&c, key, &home)).unwrap_or_else(|| home.join(fallback))
}

/// Reads `KEY="$HOME/Some Folder"` from the text of `user-dirs.dirs`.
fn parse_user_dir(content: &str, key: &str, home: &Path) -> Option<PathBuf> {
    content.lines().find_map(|line| {
        let value = line.trim().strip_prefix(key)?.strip_prefix('=')?.trim().trim_matches('"');
        let path = match value.strip_prefix("$HOME") {
            Some(rest) => PathBuf::from(format!("{}{rest}", home.display())),
            None => PathBuf::from(value),
        };
        path.is_absolute().then_some(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_variable_wins_only_if_absolute() {
        let home = Path::new("/home/u");
        assert_eq!(xdg_dir(Some("/x/cfg"), ".config", home), PathBuf::from("/x/cfg"));
        assert_eq!(xdg_dir(Some("relative"), ".config", home), PathBuf::from("/home/u/.config"));
        assert_eq!(xdg_dir(None, ".config", home), PathBuf::from("/home/u/.config"));
    }

    #[test]
    fn reads_the_pictures_folder() {
        let home = Path::new("/home/u");
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_PICTURES_DIR=\"$HOME/Images and Photos\"\n";
        assert_eq!(parse_user_dir(text, "XDG_PICTURES_DIR", home), Some(PathBuf::from("/home/u/Images and Photos")));
        assert_eq!(parse_user_dir("XDG_PICTURES_DIR=\"/data/pics\"", "XDG_PICTURES_DIR", home), Some(PathBuf::from("/data/pics")));
        assert_eq!(parse_user_dir(text, "XDG_VIDEOS_DIR", home), None);
    }
}
