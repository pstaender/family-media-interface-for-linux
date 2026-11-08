//! Recently played files per configured folder: `$XDG_DATA_HOME/curator/recent.json`.
//!
//! The folder browser shows the last `SHOWN` of them at the end of the folder's first page.
//! Paths are stored relative to the folder (subfolder names + file name) and checked again
//! when shown (`library::file_entry`), so a moved folder or a changed filter never shows
//! anything outside the folder or anything the folder doesn't allow.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::store;

/// How many the folder page shows.
pub const SHOWN: usize = 5;
/// How many are kept per folder (some may be gone or hidden by the filter by now).
const KEPT: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// Subfolder names below the configured folder.
    pub sub: Vec<String>,
    pub name: String,
    pub kind: Kind,
    /// Unix seconds.
    pub played_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recent {
    /// Folder id → newest first.
    #[serde(default)]
    pub folders: BTreeMap<String, Vec<Item>>,
}

impl Recent {
    pub fn load() -> Self {
        store::recent_path()
            .and_then(|p| store::load(&p))
            .unwrap_or_default()
    }

    pub fn items(&self, folder_id: &str) -> &[Item] {
        self.folders.get(folder_id).map_or(&[], Vec::as_slice)
    }

    /// Put a file at the top. Music that plays on through a folder counts once: the next
    /// track of the same folder replaces the previous one.
    pub fn add(&mut self, folder_id: &str, item: Item) {
        let list = self.folders.entry(folder_id.to_owned()).or_default();
        if item.kind == Kind::Audio
            && list
                .first()
                .is_some_and(|f| f.kind == Kind::Audio && f.sub == item.sub)
        {
            list.remove(0);
        }
        list.retain(|i| !(i.sub == item.sub && i.name == item.name));
        list.insert(0, item);
        list.truncate(KEPT);
    }
}

/// Remember that a file was played (read, add, write; small file, called on play only).
pub fn record(folder_id: &str, sub: &[String], name: &str, kind: Kind) {
    let Some(path) = store::recent_path() else {
        return;
    };
    let mut recent = Recent::load();
    let played_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    recent.add(
        folder_id,
        Item {
            sub: sub.to_vec(),
            name: name.to_owned(),
            kind,
            played_at,
        },
    );
    if let Err(e) = store::save(&path, &recent) {
        log::warn!("could not save the recently played list: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(sub: &[&str], name: &str, kind: Kind) -> Item {
        Item {
            sub: sub.iter().map(|s| s.to_string()).collect(),
            name: name.into(),
            kind,
            played_at: 0,
        }
    }

    fn names(r: &Recent, id: &str) -> Vec<String> {
        r.items(id).iter().map(|i| i.name.clone()).collect()
    }

    #[test]
    fn newest_first_without_duplicates() {
        let mut r = Recent::default();
        r.add("f", item(&[], "a.mkv", Kind::Video));
        r.add("f", item(&["S1"], "b.mkv", Kind::Video));
        r.add("f", item(&[], "a.mkv", Kind::Video));
        assert_eq!(names(&r, "f"), ["a.mkv", "b.mkv"]);
        // Same name in another subfolder is another file.
        r.add("f", item(&["S2"], "b.mkv", Kind::Video));
        assert_eq!(names(&r, "f"), ["b.mkv", "a.mkv", "b.mkv"]);
        assert!(r.items("other").is_empty());
    }

    #[test]
    fn music_playing_on_counts_once() {
        let mut r = Recent::default();
        r.add("f", item(&["Album"], "01.mp3", Kind::Audio));
        r.add("f", item(&["Album"], "02.mp3", Kind::Audio));
        r.add("f", item(&["Album"], "03.mp3", Kind::Audio));
        assert_eq!(names(&r, "f"), ["03.mp3"]);
        r.add("f", item(&[], "movie.mkv", Kind::Video));
        r.add("f", item(&["Album"], "04.mp3", Kind::Audio));
        assert_eq!(names(&r, "f"), ["04.mp3", "movie.mkv", "03.mp3"]);
        r.add("f", item(&["Other"], "01.mp3", Kind::Audio));
        assert_eq!(names(&r, "f"), ["01.mp3", "04.mp3", "movie.mkv", "03.mp3"]);
    }

    #[test]
    fn keeps_a_limited_number() {
        let mut r = Recent::default();
        for i in 0..30 {
            r.add("f", item(&[], &format!("{i}.mkv"), Kind::Video));
        }
        assert_eq!(r.items("f").len(), KEPT);
        assert_eq!(r.items("f")[0].name, "29.mkv");
    }
}
