use anyhow::{Context, Result};
use image::{DynamicImage, GenericImageView, imageops::FilterType, load_from_memory};
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag};
use std::cell::RefCell;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use crate::data::fingerprint::audio_fingerprint;
use crate::data::models::Cuid;

const PROBE_BUFFER_CAPACITY: usize = 64 * 1024;

const BLOCK_SIZE: usize = 256 * 1024;
const MAX_BLOCKS: usize = 2;

struct BlockCache {
    file: File,
    len: u64,
    blocks: Vec<(u64, Vec<u8>)>,
}

impl BlockCache {
    fn open(path: &Path) -> Result<Rc<RefCell<Self>>> {
        let file = File::open(path).with_context(|| format!("Failed to open {:?}", path))?;
        let len = file
            .metadata()
            .with_context(|| format!("Failed to stat {:?}", path))?
            .len();
        Ok(Rc::new(RefCell::new(Self {
            file,
            len,
            blocks: Vec::new(),
        })))
    }

    fn reader(cache: &Rc<RefCell<Self>>) -> BlockCacheReader {
        let len = cache.borrow().len;
        BlockCacheReader {
            cache: cache.clone(),
            len,
            pos: 0,
        }
    }

    fn read_at(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
        if pos >= self.len {
            return Ok(0);
        }

        let idx = self
            .blocks
            .iter()
            .position(|(start, block)| pos >= *start && pos < *start + block.len() as u64);

        let idx = match idx {
            Some(idx) => idx,
            None => {
                let want = (BLOCK_SIZE as u64).min(self.len - pos) as usize;
                self.file.seek(SeekFrom::Start(pos))?;
                let mut block = vec![0u8; want];
                self.file.read_exact(&mut block)?;
                if self.blocks.len() >= MAX_BLOCKS {
                    self.blocks.remove(0);
                }
                self.blocks.push((pos, block));
                self.blocks.len() - 1
            }
        };

        let (start, block) = &self.blocks[idx];
        let offset = (pos - start) as usize;
        let n = (block.len() - offset).min(buf.len());
        buf[..n].copy_from_slice(&block[offset..offset + n]);
        Ok(n)
    }
}

struct BlockCacheReader {
    cache: Rc<RefCell<BlockCache>>,
    len: u64,
    pos: u64,
}

impl Read for BlockCacheReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.cache.borrow_mut().read_at(self.pos, buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for BlockCacheReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(p) => self.len as i64 + p,
            SeekFrom::Current(p) => self.pos as i64 + p,
        };
        if new_pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start of file",
            ));
        }
        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}

const COVER_SIZE: u32 = 512;
const JPEG_QUALITY: u8 = 70;

#[derive(Debug, Clone, Default)]
pub struct AudioMetadata {
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub year: Option<i32>,
    pub duration: Duration,
    pub genres: Vec<String>,
    pub lufs: Option<f32>,
}

fn extract_metadata_from_tag(tag: Option<&Tag>, duration: Duration) -> AudioMetadata {
    let (title, artists, album, album_artist, genres, year, track_number, lufs) =
        if let Some(tag) = tag {
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
            let album_artist = tag
                .get_string(ItemKey::AlbumArtist)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
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

            (
                title,
                artists,
                album,
                album_artist,
                genres,
                year,
                track_number,
                lufs,
            )
        } else {
            (None, vec![], None, None, vec![], None, None, None)
        };

    AudioMetadata {
        title,
        artists,
        album,
        album_artist,
        genres,
        year,
        track_number,
        duration,
        lufs,
    }
}

pub(crate) fn image_id_for(bytes: &[u8]) -> String {
    Cuid::for_image(bytes).into_string()
}

fn open_probe(path: &Path) -> Result<Probe<BufReader<File>>> {
    let file = File::open(path).with_context(|| format!("Failed to open {:?}", path))?;
    let reader = BufReader::with_capacity(PROBE_BUFFER_CAPACITY, file);
    Ok(Probe::new(reader))
}

pub fn read_track(path: &Path) -> Result<(AudioMetadata, String)> {
    let cache = BlockCache::open(path)?;

    let tagged_file = Probe::new(BlockCache::reader(&cache))
        .guess_file_type()?
        .options(ParseOptions::new().read_cover_art(false))
        .read()?;

    let duration = tagged_file.properties().duration();
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let metadata = extract_metadata_from_tag(tag, duration);
    let audio_hash = audio_fingerprint(&mut BlockCache::reader(&cache))
        .with_context(|| format!("Failed to fingerprint {:?}", path))?;
    Ok((metadata, audio_hash))
}

const ARTIST_PICTURES: &[PictureType] = &[
    PictureType::LeadArtist,
    PictureType::Artist,
    PictureType::Band,
];

#[derive(Default)]
pub(crate) struct TagImages {
    pub cover: Option<Vec<u8>>,
    pub artist: Option<Vec<u8>>,
}

pub(crate) fn read_tag_images(path: &Path) -> Result<TagImages> {
    let tagged_file = open_probe(path)?
        .guess_file_type()?
        .options(ParseOptions::new().read_properties(false))
        .read()?;

    let Some(tag) = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
    else {
        return Ok(TagImages::default());
    };

    let pictures: Vec<_> = tag
        .pictures()
        .iter()
        .filter(|picture| !picture.data().is_empty())
        .collect();

    let cover = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| {
            pictures
                .iter()
                .find(|p| !ARTIST_PICTURES.contains(&p.pic_type()))
        })
        .map(|picture| picture.data().to_vec());

    let artist = ARTIST_PICTURES
        .iter()
        .find_map(|kind| pictures.iter().find(|p| p.pic_type() == *kind))
        .map(|picture| picture.data().to_vec());

    Ok(TagImages { cover, artist })
}

pub(crate) fn encode_cover(bytes: &[u8]) -> Result<Vec<u8>> {
    convert_to_jpeg(load_from_memory(bytes)?)
}

fn convert_to_jpeg(img: DynamicImage) -> Result<Vec<u8>> {
    let (w, h) = img.dimensions();
    let resized = if w > COVER_SIZE || h > COVER_SIZE {
        img.resize(COVER_SIZE, COVER_SIZE, FilterType::CatmullRom)
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
