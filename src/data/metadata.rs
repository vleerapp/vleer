use anyhow::{Context, Result};
use image::{DynamicImage, GenericImageView, imageops::FilterType, load_from_memory};
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

const COVER_SIZE: u32 = 1024;
const JPEG_QUALITY: u8 = 70;

#[derive(Debug, Clone, Default)]
pub struct AudioMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub year: Option<i32>,
    pub duration: Duration,
    pub genre: Option<String>,
    pub lufs: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct ImageData {
    pub id: String,
    pub data: Vec<u8>,
}

impl AudioMetadata {
    pub fn from_path_with_options(path: &Path, read_pictures: bool) -> Result<Self> {
        let parse_options = if read_pictures {
            ParseOptions::new()
        } else {
            ParseOptions::new().read_cover_art(false)
        };

        let tagged_file = Probe::open(path)?.options(parse_options).read()?;

        let properties = tagged_file.properties();
        let duration = properties.duration();

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        let (title, artist, album, genre, year, track_number, lufs) = if let Some(tag) = tag {
            let title = tag.title().map(|s| s.to_string());
            let artist = tag.artist().map(|s| s.to_string());
            let album = tag.album().map(|s| s.to_string());
            let genre = tag.genre().map(|s| s.to_string());
            let year = tag.date().map(|d| d.year as i32);
            let track_number = tag.track();

            let lufs = tag.get_string(ItemKey::ReplayGainTrackGain).and_then(|s| {
                s.trim_end_matches(" dB")
                    .parse::<f32>()
                    .ok()
                    .map(|gain| -18.0 - (gain))
            });

            (title, artist, album, genre, year, track_number, lufs)
        } else {
            (None, None, None, None, None, None, None)
        };

        Ok(Self {
            title,
            artist,
            album,
            genre,
            year,
            track_number,
            duration,
            lufs,
        })
    }
}

pub fn extract_image_data(audio_path: &Path) -> Result<Option<ImageData>> {
    let tagged_file = lofty::read_from_path(audio_path)
        .with_context(|| format!("Failed to read audio file for cover: {:?}", audio_path))?;

    let tag = match tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
    {
        Some(t) => t,
        None => return Ok(None),
    };

    let picture = match tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())
    {
        Some(p) => p,
        None => return Ok(None),
    };

    let original_data = picture.data();
    if original_data.is_empty() {
        return Ok(None);
    }

    let mut hasher = Sha256::new();
    hasher.update(original_data);
    let id = format!("{:x}", hasher.finalize());

    let img = load_from_memory(original_data).context("Failed to load cover image")?;

    let optimized_data = convert_to_jpeg(img)?;

    Ok(Some(ImageData {
        id,
        data: optimized_data,
    }))
}

fn convert_to_jpeg(img: DynamicImage) -> Result<Vec<u8>> {
    let (w, h) = img.dimensions();
    let resized = if w > COVER_SIZE || h > COVER_SIZE {
        img.resize(COVER_SIZE, COVER_SIZE, FilterType::Lanczos3)
    } else {
        img
    };

    let mut buffer = Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, JPEG_QUALITY);

    resized
        .write_with_encoder(encoder)
        .context("Failed to encode image as JPEG")?;

    Ok(buffer.into_inner())
}
