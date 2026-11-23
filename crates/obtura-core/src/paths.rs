//! Where Obtura keeps things (XDG folders; on Windows, AppData and the known folders). The
//! owner's data folders are only ever read from or written into by Obtura's own code paths for
//! its own files: never deleted.

use std::path::{Path, PathBuf};

#[cfg(unix)]
fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// `$VAR` when it is a non-empty absolute path, else `$HOME/<fallback>`.
#[cfg_attr(windows, allow(dead_code))]
fn xdg_dir(var: Option<&str>, fallback: &str, home: &Path) -> PathBuf {
    match var {
        Some(v) if Path::new(v).is_absolute() => PathBuf::from(v),
        _ => home.join(fallback),
    }
}

#[cfg(unix)]
fn env(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

/// `~/.config/obtura/`: settings.
#[cfg(unix)]
pub fn config_dir() -> PathBuf {
    xdg_dir(env("XDG_CONFIG_HOME").as_deref(), ".config", &home()).join("obtura")
}

/// `~/.local/share/obtura/`: library, presets.
#[cfg(unix)]
pub fn data_dir() -> PathBuf {
    xdg_dir(env("XDG_DATA_HOME").as_deref(), ".local/share", &home()).join("obtura")
}

/// `~/.local/state/obtura/`: logs and other state.
#[cfg(unix)]
pub fn state_dir() -> PathBuf {
    xdg_dir(env("XDG_STATE_HOME").as_deref(), ".local/state", &home()).join("obtura")
}

/// `~/.cache/obtura/`: scratch files of work under way (a recording's parts), removed when done.
#[cfg(unix)]
pub fn cache_dir() -> PathBuf {
    xdg_dir(env("XDG_CACHE_HOME").as_deref(), ".cache", &home()).join("obtura")
}

/// The user's Pictures folder, from `~/.config/user-dirs.dirs`, else `~/Pictures`.
#[cfg(unix)]
pub fn pictures_dir() -> PathBuf {
    user_dir("XDG_PICTURES_DIR", "Pictures")
}

/// The user's Videos folder, from `~/.config/user-dirs.dirs`, else `~/Videos`.
#[cfg(unix)]
pub fn videos_dir() -> PathBuf {
    user_dir("XDG_VIDEOS_DIR", "Videos")
}

#[cfg(unix)]
fn user_dir(key: &str, fallback: &str) -> PathBuf {
    let home = home();
    let file = xdg_dir(env("XDG_CONFIG_HOME").as_deref(), ".config", &home).join("user-dirs.dirs");
    std::fs::read_to_string(file).ok().and_then(|c| parse_user_dir(&c, key, &home)).unwrap_or_else(|| home.join(fallback))
}

#[cfg(windows)]
pub use self::windows::*;

/// Windows: settings in the roaming AppData (`%APPDATA%\Obtura`), the library and the rest in
/// the local one (`%LOCALAPPDATA%\Obtura\Data`, `State`, `Cache`), pictures in the user's
/// Pictures folder wherever Windows keeps it (OneDrive may have moved it).
#[cfg(windows)]
mod windows {
    use std::{ffi::c_void, os::windows::ffi::OsStringExt, path::PathBuf};

    fn profile() -> PathBuf {
        std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\"))
    }

    fn app_data(var: &str, fallback: &str) -> PathBuf {
        std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| profile().join(fallback)).join("Obtura")
    }

    /// `%APPDATA%\Obtura`: settings.
    pub fn config_dir() -> PathBuf {
        app_data("APPDATA", "AppData\\Roaming")
    }

    /// `%LOCALAPPDATA%\Obtura\Data`: library, presets.
    pub fn data_dir() -> PathBuf {
        app_data("LOCALAPPDATA", "AppData\\Local").join("Data")
    }

    /// `%LOCALAPPDATA%\Obtura\State`: log and other state.
    pub fn state_dir() -> PathBuf {
        app_data("LOCALAPPDATA", "AppData\\Local").join("State")
    }

    /// `%LOCALAPPDATA%\Obtura\Cache`: scratch files of work under way.
    pub fn cache_dir() -> PathBuf {
        app_data("LOCALAPPDATA", "AppData\\Local").join("Cache")
    }

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);

    const FOLDERID_PICTURES: Guid = Guid(0x33E2_8130, 0x4E1E, 0x4676, [0x83, 0x5A, 0x98, 0x39, 0x5C, 0x3B, 0xC3, 0xBB]);
    const FOLDERID_VIDEOS: Guid = Guid(0x1898_9B1D, 0x99B5, 0x455B, [0x84, 0x1C, 0xAB, 0x7C, 0x74, 0xE4, 0xDD, 0xFC]);

    #[link(name = "shell32")]
    extern "system" {
        fn SHGetKnownFolderPath(id: *const Guid, flags: u32, token: *mut c_void, path: *mut *mut u16) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(p: *mut c_void);
    }

    fn known_folder(id: &Guid) -> Option<PathBuf> {
        let mut path: *mut u16 = std::ptr::null_mut();
        // SAFETY: Windows writes a string it allocated, freed below.
        unsafe {
            let ok = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut path) == 0 && !path.is_null();
            let found = ok.then(|| {
                let len = (0..).take_while(|&i| *path.add(i) != 0).count();
                PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(path, len)))
            });
            CoTaskMemFree(path.cast());
            found
        }
    }

    /// The user's Pictures folder.
    pub fn pictures_dir() -> PathBuf {
        known_folder(&FOLDERID_PICTURES).unwrap_or_else(|| profile().join("Pictures"))
    }

    /// The user's Videos folder.
    pub fn videos_dir() -> PathBuf {
        known_folder(&FOLDERID_VIDEOS).unwrap_or_else(|| profile().join("Videos"))
    }
}

/// Reads `KEY="$HOME/Some Folder"` from the text of `user-dirs.dirs`.
#[cfg_attr(windows, allow(dead_code))]
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
