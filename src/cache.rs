//! The last read, kept on disk: the panel opens with what it already had and
//! updates when the network answers.
//!
//! `~/Library/Application Support/CodexResets/cache.json`, written atomically
//! (a temporary file renamed over the old one) so a crash never leaves half a
//! file behind. A cache that does not parse is ignored, not fatal.

use std::{fs, path::PathBuf};

use crate::model::{Reset, StatusData};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Cache {
    /// When the status and the first page were read.
    pub saved_at: i64,
    pub status: StatusData,
    pub history: Vec<Reset>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

impl Cache {
    pub fn age(&self, now: i64) -> i64 {
        (now - self.saved_at).max(0)
    }
}

fn file() -> Option<PathBuf> {
    // Isolated cache for cold-start/offline integration checks.
    if let Some(path) = std::env::var_os("CODEX_RESET_CACHE_PATH") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/Application Support/CodexResets")
            .join("cache.json"),
    )
}

pub fn load() -> Option<Cache> {
    let text = fs::read_to_string(file()?).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(cache: &Cache) {
    let Some(path) = file() else { return };
    let Some(directory) = path.parent() else {
        return;
    };
    if fs::create_dir_all(directory).is_err() {
        return;
    }
    let Ok(text) = serde_json::to_string(cache) else {
        return;
    };
    let temporary = path.with_extension("json.tmp");
    if fs::write(&temporary, text).is_ok() {
        let _ = fs::rename(&temporary, &path);
    }
}
