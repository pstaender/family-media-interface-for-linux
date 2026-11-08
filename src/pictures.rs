//! Loading pictures for the built-in viewer (read-only).

use std::io::{BufRead, Cursor, Seek};
use std::path::Path;

use image::{DynamicImage, ImageDecoder, ImageReader, imageops::FilterType};

/// A decoded picture, ready to upload as a texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    /// RGBA, 4 bytes per pixel.
    pub rgba: Vec<u8>,
}

/// Decode a picture, turn it upright (EXIF orientation of phone photos) and shrink it to fit
/// `max_width` × `max_height` (never enlarged). Slow for big photos: call from a thread.
pub fn load(path: &Path, max_width: u32, max_height: u32) -> Result<Picture, String> {
    let reader = ImageReader::open(path)
        .and_then(ImageReader::with_guessed_format)
        .map_err(|e| e.to_string())?;
    decode(reader, max_width, max_height)
}

/// Like [`load`], for a picture in memory (e.g. cover art embedded in a music file).
pub fn load_bytes(bytes: &[u8], max_width: u32, max_height: u32) -> Result<Picture, String> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    decode(reader, max_width, max_height)
}

fn decode<R: BufRead + Seek>(
    reader: ImageReader<R>,
    max_width: u32,
    max_height: u32,
) -> Result<Picture, String> {
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    let (max_width, max_height) = (max_width.max(1), max_height.max(1));
    if img.width() > max_width || img.height() > max_height {
        img = img.resize(max_width, max_height, FilterType::Triangle);
    }
    let rgba = img.to_rgba8();
    Ok(Picture {
        width: rgba.width() as usize,
        height: rgba.height() as usize,
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_shrinks_but_never_enlarges() {
        let dir = std::env::temp_dir().join(format!("curator-pictures-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.png");
        image::RgbImage::from_pixel(400, 200, image::Rgb([200, 10, 10]))
            .save(&path)
            .unwrap();

        let p = load(&path, 100, 100).unwrap();
        assert_eq!((p.width, p.height), (100, 50)); // aspect ratio kept
        assert_eq!(p.rgba.len(), 100 * 50 * 4);
        assert_eq!(&p.rgba[..4], &[200, 10, 10, 255]);

        let p = load(&path, 4000, 4000).unwrap();
        assert_eq!((p.width, p.height), (400, 200));

        let p = load_bytes(&std::fs::read(&path).unwrap(), 40, 40).unwrap();
        assert_eq!((p.width, p.height), (40, 20));
        assert!(load_bytes(b"not a picture", 10, 10).is_err());

        std::fs::write(&path, b"not a picture").unwrap();
        assert!(load(&path, 100, 100).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
