use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::tag::Accessor;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default)]
pub struct AudioMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub year: Option<i32>,
    pub duration: Duration,
    pub genre: Option<String>,
    pub track_lufs: Option<f32>,
}

impl AudioMetadata {
    pub fn from_path(path: &Path) -> Result<Self> {
        let tagged_file = lofty::read_from_path(path)
            .with_context(|| format!("Failed to read audio file: {:?}", path))?;

        let properties = tagged_file.properties();
        let duration = properties.duration();

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        let mut metadata = AudioMetadata {
            duration,
            ..Default::default()
        };

        if let Some(tag) = tag {
            metadata.title = tag.title().map(|s| s.to_string());
            metadata.artist = tag.artist().map(|s| s.to_string());
            metadata.album = tag.album().map(|s| s.to_string());
            metadata.album_artist = tag
                .get_string(&lofty::tag::ItemKey::AlbumArtist)
                .map(|s| s.to_string());
            metadata.track_number = tag.track();
            metadata.year = tag.year().map(|y| y as i32);
            metadata.genre = tag.genre().map(|s| s.to_string());

            metadata.track_lufs = tag
                .get_string(&lofty::tag::ItemKey::ReplayGainTrackGain)
                .and_then(|s| parse_replaygain_to_lufs(s))
                .or_else(|| {
                    tag.get_string(&lofty::tag::ItemKey::Unknown("LUFS".to_string()))
                        .and_then(|s| s.trim().parse().ok())
                });
        }

        if metadata.title.is_none() {
            metadata.title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string());
        }

        Ok(metadata)
    }
}

fn parse_replaygain_to_lufs(s: &str) -> Option<f32> {
    let gain_db = s
        .trim()
        .trim_end_matches(" dB")
        .trim_end_matches("dB")
        .parse::<f32>()
        .ok()?;

    Some(-18.0 + gain_db)
}

pub fn extract_and_save_cover(audio_path: &Path, covers_dir: &Path) -> Result<Option<PathBuf>> {
    let tagged_file = lofty::read_from_path(audio_path)
        .with_context(|| format!("Failed to read audio file for cover: {:?}", audio_path))?;

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let Some(tag) = tag else {
        return Ok(None);
    };

    let picture = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first());

    let Some(picture) = picture else {
        return Ok(None);
    };

    let data = picture.data();
    if data.is_empty() {
        return Ok(None);
    }

    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hasher.finalize();
    let filename = format!("{:x}.jpg", hash);
    let cover_path = covers_dir.join(&filename);

    if cover_path.exists() {
        return Ok(Some(cover_path));
    }

    fs::create_dir_all(covers_dir)
        .with_context(|| format!("Failed to create covers directory: {:?}", covers_dir))?;

    let img = image::load_from_memory(data).with_context(|| "Failed to decode cover image")?;

    let img = if img.width() > 512 || img.height() > 512 {
        img.resize(512, 512, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let mut file = std::fs::File::create(&cover_path)
        .with_context(|| format!("Failed to create cover file: {:?}", cover_path))?;

    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 85);
    encoder
        .encode_image(&img)
        .with_context(|| format!("Failed to save cover as JPEG: {:?}", cover_path))?;

    Ok(Some(cover_path))
}
