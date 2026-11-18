//! The last region chosen on the overlay, so "Same Region Again" can capture it without
//! asking. Kept in `~/.local/state/obtura/last-region.json`.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A rectangle in screen pixels (x, y, width, height).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

pub fn path() -> PathBuf {
    paths::state_dir().join("last-region.json")
}

pub fn load() -> Option<Region> {
    load_from(&path())
}

pub fn load_from(path: &Path) -> Option<Region> {
    let r: Region = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    (r.w > 0 && r.h > 0).then_some(r)
}

pub fn save(r: Region) -> std::io::Result<()> {
    save_to(&path(), r)
}

pub fn save_to(path: &Path, r: Region) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&r).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_nonsense_is_ignored() {
        let dir = std::env::temp_dir().join(format!("obtura-last-region-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("state/last-region.json");
        assert_eq!(load_from(&p), None);
        let r = Region { x: -10, y: 20, w: 300, h: 200 };
        save_to(&p, r).unwrap();
        assert_eq!(load_from(&p), Some(r));
        std::fs::write(&p, "{\"x\":0,\"y\":0,\"w\":0,\"h\":5}").unwrap();
        assert_eq!(load_from(&p), None, "an empty region is no region");
        std::fs::write(&p, "garbage").unwrap();
        assert_eq!(load_from(&p), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
