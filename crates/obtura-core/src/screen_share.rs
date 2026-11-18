//! The "restore token" GNOME gives for the screens the user chose to share with Obtura, so it
//! does not ask again at every recording. Kept in `~/.local/state/obtura/screen-share-token`.
//! Forgetting it makes GNOME ask again (to choose other screens).

use crate::paths;
use std::path::{Path, PathBuf};

pub fn path() -> PathBuf {
    paths::state_dir().join("screen-share-token")
}

pub fn load() -> Option<String> {
    load_from(&path())
}

pub fn load_from(path: &Path) -> Option<String> {
    let t = std::fs::read_to_string(path).ok()?;
    let t = t.trim();
    (!t.is_empty() && t.len() < 4096).then(|| t.to_string())
}

pub fn save(token: &str) -> std::io::Result<()> {
    save_to(&path(), token)
}

pub fn save_to(path: &Path, token: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, token)
}

/// Forgets the screens chosen: GNOME asks again next time. Removes only this one file.
pub fn forget() -> std::io::Result<()> {
    match std::fs::remove_file(path()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_nonsense_is_ignored() {
        let dir = std::env::temp_dir().join(format!("obtura-share-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("state/screen-share-token");
        assert_eq!(load_from(&p), None);
        save_to(&p, "abc-123").unwrap();
        assert_eq!(load_from(&p), Some("abc-123".into()));
        std::fs::write(&p, "  \n").unwrap();
        assert_eq!(load_from(&p), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
