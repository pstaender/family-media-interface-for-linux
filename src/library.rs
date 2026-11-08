//! Browsing configured folders — strictly read-only.
//!
//! Only `read_dir`, `metadata` and `canonicalize` are used here; nothing in this module may
//! create, change or delete files (see "Non-destructive" in CLAUDE.md).
//!
//! Confinement: every listed or opened path is canonicalized and must stay inside the
//! (canonicalized) folder root, so `..` and symlinks pointing outside are refused.

use std::cmp::Ordering;
use std::path::{Component, Path, PathBuf};

use crate::config::FolderSettings;

const VIDEO: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "avi", "mov", "wmv", "mpg", "mpeg", "ts", "m2ts", "mts", "flv",
    "ogv", "3gp", "vob",
];
const AUDIO: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "m4a", "m4b", "aac", "wav", "wma", "aiff", "aif", "ape",
    "mka", "alac",
];
const IMAGES: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    Video,
    Audio,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    /// Canonical path (inside the folder root).
    pub path: PathBuf,
}

/// Which files a folder shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub video: bool,
    pub audio: bool,
    pub images: bool,
}

impl From<&FolderSettings> for Filter {
    fn from(f: &FolderSettings) -> Self {
        Self {
            video: f.video,
            audio: f.audio,
            images: f.images,
        }
    }
}

impl Filter {
    /// The kind of a file, or `None` if this folder doesn't show it.
    pub fn kind_of(&self, file_name: &str) -> Option<EntryKind> {
        let ext = file_name.rsplit_once('.')?.1.to_ascii_lowercase();
        if self.video && VIDEO.contains(&ext.as_str()) {
            Some(EntryKind::Video)
        } else if self.audio && AUDIO.contains(&ext.as_str()) {
            Some(EntryKind::Audio)
        } else if self.images && IMAGES.contains(&ext.as_str()) {
            Some(EntryKind::Image)
        } else {
            None
        }
    }
}

/// Is this a picture the built-in viewer can show (by extension)?
pub fn is_image(file_name: &str) -> bool {
    file_name
        .rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGES.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Resolve `sub` (folder names below the root) to a canonical directory inside `root`.
pub fn resolve(root: &Path, sub: &[String]) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("{}: {e}", root.display()))?;
    let mut path = root.clone();
    for part in sub {
        let mut components = Path::new(part).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => path.push(part),
            _ => return Err(format!("invalid folder name: {part:?}")),
        }
    }
    let path = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !path.starts_with(&root) {
        return Err("this folder is outside the shared folder".into());
    }
    Ok(path)
}

/// List a directory inside `root`: subfolders and the files the filter shows, folders first,
/// then by name (numbers compared by value, so "Episode 2" comes before "Episode 10").
/// Hidden entries and symlinks pointing outside the root are skipped.
pub fn list(root: &Path, sub: &[String], filter: &Filter) -> Result<Vec<Entry>, String> {
    let dir = resolve(root, sub)?;
    let root = root
        .canonicalize()
        .map_err(|e| format!("{}: {e}", root.display()))?;
    let read = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut entries = Vec::new();
    for item in read.flatten() {
        let Ok(name) = item.file_name().into_string() else {
            continue; // not valid UTF-8: can't be shown reliably
        };
        if name.starts_with('.') {
            continue;
        }
        // Follow symlinks, but only if they stay inside the root.
        let Ok(path) = item.path().canonicalize() else {
            continue; // broken link
        };
        if !path.starts_with(&root) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let kind = if meta.is_dir() {
            Some(EntryKind::Folder)
        } else if meta.is_file() {
            filter.kind_of(&name)
        } else {
            None // sockets, devices, …
        };
        if let Some(kind) = kind {
            entries.push(Entry { name, kind, path });
        }
    }
    entries.sort_by(|a, b| {
        (b.kind == EntryKind::Folder)
            .cmp(&(a.kind == EntryKind::Folder))
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
    Ok(entries)
}

/// Case-insensitive comparison that compares runs of digits by their numeric value.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (da, db) = (take(&mut a), take(&mut b));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// Build a test tree (the only place where this module writes: test temp dirs).
    fn tree() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "familynovice-library-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        for dir in ["root/Season 1", "root/.hidden-dir", "outside"] {
            std::fs::create_dir_all(base.join(dir)).unwrap();
        }
        for file in [
            "root/Episode 10.mkv",
            "root/Episode 2.MKV",
            "root/song.flac",
            "root/photo.jpg",
            "root/notes.pdf",
            "root/readme.txt",
            "root/.secret.mp4",
            "root/Season 1/a.mp4",
            "outside/secret.mp4",
        ] {
            std::fs::write(base.join(file), b"x").unwrap();
        }
        symlink(base.join("outside"), root.join("escape")).unwrap();
        symlink(base.join("outside/secret.mp4"), root.join("escape.mp4")).unwrap();
        symlink(root.join("Season 1"), root.join("Also season 1")).unwrap();
        root
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn filter_by_kind_and_extension() {
        let f = Filter {
            video: true,
            images: true,
            ..Default::default()
        };
        assert_eq!(f.kind_of("a.MKV"), Some(EntryKind::Video));
        assert_eq!(f.kind_of("a.jpeg"), Some(EntryKind::Image));
        assert_eq!(f.kind_of("a.pdf"), None); // only media
        assert_eq!(f.kind_of("a.heic"), None); // can't be displayed
        assert_eq!(f.kind_of("a.flac"), None); // audio not enabled
        assert_eq!(f.kind_of("mkv"), None); // no extension
    }

    #[test]
    fn lists_visible_entries_sorted() {
        let root = tree();
        let filter = Filter {
            video: true,
            ..Default::default()
        };
        let entries = list(&root, &[], &filter).unwrap();
        assert_eq!(
            names(&entries),
            [
                "Also season 1",
                "Season 1",
                "Episode 2.MKV",
                "Episode 10.mkv"
            ]
        );
        assert_eq!(entries[0].kind, EntryKind::Folder);
        assert_eq!(entries[3].kind, EntryKind::Video);
    }

    #[test]
    fn subfolders_and_confinement() {
        let root = tree();
        let filter = Filter {
            video: true,
            ..Default::default()
        };
        let sub = list(&root, &["Season 1".into()], &filter).unwrap();
        assert_eq!(names(&sub), ["a.mp4"]);
        // Symlinks pointing outside are neither listed nor enterable.
        assert!(resolve(&root, &["escape".into()]).is_err());
        for bad in ["..", ".", "a/b", "/etc", ""] {
            assert!(resolve(&root, &[bad.into()]).is_err(), "{bad}");
        }
    }

    #[test]
    fn natural_sorting() {
        let mut v = vec![
            "Episode 10",
            "episode 2",
            "Episode 1",
            "Bonus",
            "Episode 02b",
        ];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            v,
            [
                "Bonus",
                "Episode 1",
                "episode 2",
                "Episode 02b",
                "Episode 10"
            ]
        );
    }

    #[test]
    fn missing_root_is_an_error() {
        assert!(list(Path::new("/does/not/exist"), &[], &Filter::default()).is_err());
    }
}
