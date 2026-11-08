//! Cover art for the music player (read-only): the picture embedded in the music file (ID3,
//! FLAC, MP4, Ogg, … via `lofty`), otherwise a cover image in the track's folder
//! (`cover.jpg`, `folder.jpg`, `front.png`, …, or the folder's only picture).

use std::path::{Path, PathBuf};

use lofty::config::ParseOptions;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::probe::Probe;

use crate::library;
use crate::pictures::{self, Picture};

/// Usual names of cover files, best first (compared without extension, ignoring case).
/// A name that only starts with one of these ranks after all exact matches, e.g.
/// `Folder_large`, `AlbumArt_{…}_Large` (Windows Media Player), `cover (front)`.
const NAMES: &[&str] = &["cover", "folder", "front", "album", "albumart", "thumb"];

/// Where a cover comes from. The same folder image isn't decoded again for the next track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Embedded(PathBuf),
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cover {
    pub source: Source,
    /// `None`: same source as the cover already shown (`current` in [`load`]); keep it.
    pub picture: Option<Picture>,
}

/// The cover for `track`, shrunk to `max` × `max` pixels. `root` is the configured folder:
/// cover files must be inside it (symlinks pointing outside are ignored). Reads files only.
/// Slow (decoding): call from a thread.
pub fn load(track: &Path, root: &Path, current: Option<&Source>, max: u32) -> Option<Cover> {
    if let Some(bytes) = embedded(track) {
        match pictures::load_bytes(&bytes, max, max) {
            Ok(picture) => {
                return Some(Cover {
                    source: Source::Embedded(track.to_path_buf()),
                    picture: Some(picture),
                });
            }
            Err(e) => log::debug!("cover in {}: {e}", track.display()),
        }
    }
    let file = folder_image(track.parent()?, root)?;
    let source = Source::File(file.clone());
    if current == Some(&source) {
        return Some(Cover {
            source,
            picture: None,
        });
    }
    match pictures::load(&file, max, max) {
        Ok(picture) => Some(Cover {
            source,
            picture: Some(picture),
        }),
        Err(e) => {
            log::debug!("cover {}: {e}", file.display());
            None
        }
    }
}

/// The picture embedded in a music file: the front cover, else the first picture.
pub fn embedded(track: &Path) -> Option<Vec<u8>> {
    let tagged = Probe::open(track)
        .ok()?
        .options(ParseOptions::new().read_properties(false))
        .guess_file_type()
        .ok()?
        .read()
        .ok()?;
    let pictures: Vec<_> = tagged.tags().iter().flat_map(|t| t.pictures()).collect();
    pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or(pictures.first())
        .map(|p| p.data().to_vec())
}

/// The cover image in `dir`: the best name from [`NAMES`]; without one, the folder's only
/// picture. Hidden files and pictures outside `root` are ignored.
pub fn folder_image(dir: &Path, root: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let mut best: Option<(usize, String, PathBuf)> = None;
    let mut images = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !library::is_image(&name) {
            continue;
        }
        let Ok(path) = entry.path().canonicalize() else {
            continue;
        };
        if !path.starts_with(&root) || !path.is_file() {
            continue;
        }
        let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
        if let Some(rank) = rank(&stem.to_lowercase())
            && best
                .as_ref()
                .is_none_or(|(r, n, _)| (rank, &name) < (*r, n))
        {
            best = Some((rank, name, path.clone()));
        }
        images.push(path);
    }
    match best {
        Some((_, _, path)) => Some(path),
        None if images.len() == 1 => images.pop(),
        None => None,
    }
}

fn rank(stem: &str) -> Option<usize> {
    NAMES.iter().position(|n| *n == stem).or_else(|| {
        NAMES
            .iter()
            .position(|n| stem.starts_with(n))
            .map(|i| NAMES.len() + i)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::config::WriteOptions;
    use lofty::picture::MimeType;
    use lofty::tag::{Tag, TagType};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("curator-cover-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png(path: &Path, w: u32, h: u32) {
        image::RgbImage::from_pixel(w, h, image::Rgb([10, 200, 10]))
            .save(path)
            .unwrap();
    }

    fn touch(dir: &Path, names: &[&str]) {
        for n in names {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
    }

    #[test]
    fn picks_the_best_cover_file_name() {
        let dir = temp_dir("names");
        touch(
            &dir,
            &[
                "back.jpg",
                "Folder.JPG",
                "cover.txt",
                ".cover.jpg",
                "01.mp3",
            ],
        );
        assert_eq!(folder_image(&dir, &dir), Some(dir.join("Folder.JPG")));
        touch(&dir, &["Cover.png"]);
        assert_eq!(folder_image(&dir, &dir), Some(dir.join("Cover.png")));
        let _ = std::fs::remove_dir_all(dir);

        let dir = temp_dir("prefix");
        touch(&dir, &["photo.jpg", "AlbumArt_{1234}_Large.jpg"]);
        assert_eq!(
            folder_image(&dir, &dir),
            Some(dir.join("AlbumArt_{1234}_Large.jpg"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_picture_counts_but_not_one_of_many() {
        let dir = temp_dir("single");
        touch(&dir, &["Artist - Album.jpg", "01.mp3"]);
        assert_eq!(
            folder_image(&dir, &dir),
            Some(dir.join("Artist - Album.jpg"))
        );
        touch(&dir, &["holiday.png"]);
        assert_eq!(folder_image(&dir, &dir), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ignores_covers_outside_the_shared_folder() {
        let outside = temp_dir("outside");
        touch(&outside, &["secret.jpg"]);
        let root = temp_dir("root");
        std::os::unix::fs::symlink(outside.join("secret.jpg"), root.join("cover.jpg")).unwrap();
        assert_eq!(folder_image(&root, &root), None);
        let _ = std::fs::remove_dir_all(outside);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn embedded_cover_wins_over_the_folder_image() {
        let dir = temp_dir("embedded");
        png(&dir.join("cover.png"), 50, 50);
        let mut art = Vec::new();
        image::RgbImage::from_pixel(80, 40, image::Rgb([200, 10, 10]))
            .write_to(&mut std::io::Cursor::new(&mut art), image::ImageFormat::Png)
            .unwrap();
        // An MP3: ID3v2 tag with the picture, then one MPEG frame (128 kbit/s, 44.1 kHz).
        let mut tag = Tag::new(TagType::Id3v2);
        tag.push_picture(
            lofty::picture::Picture::unchecked(art)
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
        let mut mp3 = Vec::new();
        tag.dump_to(&mut mp3, WriteOptions::default()).unwrap();
        mp3.extend([0xFF, 0xFB, 0x90, 0x64]);
        mp3.resize(mp3.len() + 413, 0);
        let track = dir.join("01.mp3");
        std::fs::write(&track, &mp3).unwrap();
        let before = std::fs::read(&track).unwrap();

        let cover = load(&track, &dir, None, 1000).unwrap();
        assert_eq!(cover.source, Source::Embedded(track.clone()));
        let p = cover.picture.unwrap();
        assert_eq!((p.width, p.height), (80, 40));
        assert_eq!(std::fs::read(&track).unwrap(), before, "read-only");

        // Without an embedded picture: the folder's cover, not decoded again if shown already.
        let plain = dir.join("02.mp3");
        std::fs::write(&plain, b"not really music").unwrap();
        let cover = load(&plain, &dir, None, 20).unwrap();
        let source = Source::File(dir.join("cover.png"));
        assert_eq!(cover.source, source);
        assert_eq!(cover.picture.map(|p| (p.width, p.height)), Some((20, 20)));
        let again = load(&plain, &dir, Some(&source), 20).unwrap();
        assert_eq!(again.picture, None);

        std::fs::remove_file(dir.join("cover.png")).unwrap();
        assert_eq!(load(&plain, &dir, None, 20), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
