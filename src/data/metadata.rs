use anyhow::{Context, Result};
use image::{GenericImageView, ImageFormat, imageops::FilterType, load_from_memory};
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey};
use sha2::{Digest, Sha256};
use std::fs;
use std::fs::File;
use std::path::Path;
use std::time::Duration;

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

        let (title, artist, album, album_artist, genre, year, track_number, track_lufs) =
            if let Some(tag) = tag {
                let title = tag.title().map(|s| s.to_string());
                let artist = tag.artist().map(|s| s.to_string());
                let album = tag.album().map(|s| s.to_string());
                let album_artist = tag.get_string(&ItemKey::AlbumArtist).map(|s| s.to_string());
                let genre = tag.genre().map(|s| s.to_string());
                let year = tag.year().map(|y| y as i32);
                let track_number = tag.track();

                let track_lufs = tag.get_string(&ItemKey::ReplayGainTrackGain).and_then(|s| {
                    s.trim_end_matches(" dB")
                        .parse::<f64>()
                        .ok()
                        .map(|gain| (-18.0 + gain) as f32)
                });

                (
                    title,
                    artist,
                    album,
                    album_artist,
                    genre,
                    year,
                    track_number,
                    track_lufs,
                )
            } else {
                (None, None, None, None, None, None, None, None)
            };

        Ok(Self {
            title,
            artist,
            album,
            album_artist,
            genre,
            year,
            track_number,
            duration,
            track_lufs,
        })
    }
}

pub fn extract_and_save_cover(audio_path: &Path, covers_dir: &Path) -> Result<Option<String>> {
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

    let data = picture.data();
    if data.is_empty() {
        return Ok(None);
    }

    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = format!("{:x}", hasher.finalize());
    let filename = format!("{}.jpg", hash);
    let cover_path = covers_dir.join(&filename);

    if !cover_path.exists() {
        fs::create_dir_all(covers_dir)
            .with_context(|| format!("Failed to create covers directory: {:?}", covers_dir))?;

        let mut file = File::create(&cover_path)
            .with_context(|| format!("Failed to create cover file: {:?}", cover_path))?;

        std::io::copy(&mut &data[..], &mut file)
            .with_context(|| format!("Failed to save cover: {:?}", cover_path))?;

        if let Ok(mut img) = load_from_memory(data) {
            const MAX_DIM: u32 = 512;
            let (w, h) = img.dimensions();
            if w > MAX_DIM || h > MAX_DIM {
                img = img.resize(MAX_DIM, MAX_DIM, FilterType::Lanczos3);
            }

            let _ = img.save_with_format(&cover_path, ImageFormat::Jpeg);
        }
    }

    Ok(Some(hash))
}
