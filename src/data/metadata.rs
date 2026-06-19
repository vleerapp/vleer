use anyhow::{Context, Result};
use image::{DynamicImage, GenericImageView, imageops::FilterType, load_from_memory};
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::Path;
use std::time::Duration;

const PROBE_BUFFER_CAPACITY: usize = 64 * 1024;

const COVER_SIZE: u32 = 1024;
const JPEG_QUALITY: u8 = 70;

#[derive(Debug, Clone, Default)]
pub struct AudioMetadata {
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub year: Option<i32>,
    pub duration: Duration,
    pub genres: Vec<String>,
    pub lufs: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct ImageData {
    pub id: String,
    pub data: Vec<u8>,
}

fn extract_metadata_from_tag(tag: Option<&Tag>, duration: Duration) -> AudioMetadata {
    let (title, artists, album, genres, year, track_number, lufs) = if let Some(tag) = tag {
        let title = tag.title().map(|s| s.to_string());
        let artists = tag
            .artist()
            .map(|s| {
                s.split([',', ';', '/', '&'])
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let album = tag.album().map(|s| s.to_string());
        let genres = tag
            .genre()
            .map(|s| {
                s.split([',', ';', '/'])
                    .map(|g| g.trim().to_string())
                    .filter(|g| !g.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let year = tag.date().map(|d| d.year as i32);
        let track_number = tag.track();

        let lufs = tag.get_string(ItemKey::ReplayGainTrackGain).and_then(|s| {
            s.trim_end_matches(" dB")
                .parse::<f32>()
                .ok()
                .map(|gain| -18.0 - (gain))
        });

        (title, artists, album, genres, year, track_number, lufs)
    } else {
        (None, vec![], None, vec![], None, None, None)
    };

    AudioMetadata {
        title,
        artists,
        album,
        genres,
        year,
        track_number,
        duration,
        lufs,
    }
}

fn picture_to_image_data(picture: &Picture) -> Option<ImageData> {
    let original_data = picture.data();
    if original_data.is_empty() {
        return None;
    }

    let mut hasher = Sha256::new();
    hasher.update(original_data);
    let id = hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();

    let img = load_from_memory(original_data).ok()?;
    let optimized_data = convert_to_jpeg(img).ok()?;

    Some(ImageData {
        id,
        data: optimized_data,
    })
}

fn open_probe(path: &Path) -> Result<Probe<BufReader<File>>> {
    let file = File::open(path).with_context(|| format!("Failed to open {:?}", path))?;
    let reader = BufReader::with_capacity(PROBE_BUFFER_CAPACITY, file);
    Ok(Probe::new(reader))
}

impl AudioMetadata {
    pub fn from_path_with_options(path: &Path, read_pictures: bool) -> Result<Self> {
        let parse_options = if read_pictures {
            ParseOptions::new()
        } else {
            ParseOptions::new().read_cover_art(false)
        };

        let tagged_file = open_probe(path)?
            .guess_file_type()?
            .options(parse_options)
            .read()?;

        let duration = tagged_file.properties().duration();
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        Ok(extract_metadata_from_tag(tag, duration))
    }
}

pub fn read_metadata_and_image(path: &Path) -> Result<(AudioMetadata, Option<ImageData>)> {
    let tagged_file = open_probe(path)?.guess_file_type()?.read()?;

    let duration = tagged_file.properties().duration();
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let metadata = extract_metadata_from_tag(tag, duration);

    let image_data = tag.and_then(|tag| {
        tag.pictures()
            .iter()
            .find(|p| p.pic_type() == PictureType::CoverFront)
            .or_else(|| tag.pictures().first())
            .and_then(picture_to_image_data)
    });

    Ok((metadata, image_data))
}

pub fn extract_image_data(audio_path: &Path) -> Result<Option<ImageData>> {
    Ok(read_metadata_and_image(audio_path)?.1)
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
