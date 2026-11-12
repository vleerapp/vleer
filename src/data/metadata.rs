use anyhow::{Context, Result};
use image::{DynamicImage, GenericImageView, imageops::FilterType, load_from_memory};
use lofty::ape::ApeTag;
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::id3::v2::{Frame, Id3v2Tag};
use lofty::mp4::{AtomData, AtomIdent, Ilst};
use lofty::ogg::tag::VorbisComments;
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag, TagType};
use rodio::Decoder;
use rodio::decoder::DecoderBuilder;
use std::cell::RefCell;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;
use symphonia_adapter_libopus::OpusDecoder;

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
    pub isrc: Option<String>,
    pub upc: Option<String>,
    pub omm_song_id: Option<String>,
    pub omm_album_id: Option<String>,
    pub omm_artist_id: Option<String>,
}

pub fn normalize_omm_id(raw: &str) -> Option<String> {
    let digits = raw.trim();
    ((1..=18).contains(&digits.len())
        && !digits.starts_with('0')
        && digits.bytes().all(|b| b.is_ascii_digit()))
    .then(|| digits.to_string())
}

fn custom_fields<const N: usize>(tag: &Tag, names: [&str; N]) -> [Option<String>; N] {
    if !tag.has_format_specific_items() {
        return [const { None }; N];
    }
    let lookup = |find: &dyn Fn(&str) -> Option<String>| names.map(find);
    match tag.tag_type() {
        TagType::VorbisComments => {
            let comments = VorbisComments::from(tag.clone());
            lookup(&|name| comments.get(name).map(str::to_string))
        }
        TagType::Id3v2 => {
            let id3 = Id3v2Tag::from(tag.clone());
            lookup(&|name| {
                (&id3).into_iter().find_map(|frame| match frame {
                    Frame::UserText(text) if text.description.eq_ignore_ascii_case(name) => {
                        Some(text.content.to_string())
                    }
                    _ => None,
                })
            })
        }
        TagType::Mp4Ilst => {
            let ilst = Ilst::from(tag.clone());
            lookup(&|name| {
                let ident = AtomIdent::Freeform {
                    mean: "com.apple.iTunes".into(),
                    name: name.into(),
                };
                ilst.get(&ident)?.data().find_map(|data| match data {
                    AtomData::UTF8(text) | AtomData::UTF16(text) => Some(text.clone()),
                    _ => None,
                })
            })
        }
        TagType::Ape => {
            let ape = ApeTag::from(tag.clone());
            lookup(&|name| ape.get(name)?.text_values()?.next().map(str::to_string))
        }
        _ => [const { None }; N],
    }
}

pub fn normalize_isrc(raw: &str) -> Option<String> {
    let code: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    let valid = code.len() == 12
        && code[..2].chars().all(|c| c.is_ascii_alphabetic())
        && code[2..5].chars().all(|c| c.is_ascii_alphanumeric())
        && code[5..].chars().all(|c| c.is_ascii_digit());
    valid.then_some(code)
}

pub fn normalize_upc(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    ((8..=14).contains(&digits.len()) && digits.chars().any(|c| c != '0')).then_some(digits)
}

fn clean_name(value: &str) -> Option<String> {
    let name = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = name.trim_matches(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '/' | '&'));
    let unmatched_close = |close: char, open: char| {
        name.ends_with(close) && name.matches(open).count() < name.matches(close).count()
    };
    let name = if unmatched_close(')', '(') || unmatched_close(']', '[') {
        &name[..name.len() - 1]
    } else {
        name
    };
    (!name.is_empty()).then(|| name.to_string())
}

fn unique_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names
        .into_iter()
        .filter_map(clean_name)
        .filter(|name| seen.insert(name.to_lowercase()))
        .collect()
}

fn split_on(part: &str, delimiter: &str, rejoin_the: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for piece in part
        .split(delimiter)
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        match (rejoin_the, out.last_mut()) {
            (Some(joiner), Some(last)) if piece.to_lowercase().starts_with("the ") => {
                last.push_str(joiner);
                last.push_str(piece);
            }
            _ => out.push(piece.to_string()),
        }
    }
    out
}

fn split_credit(credit: &str) -> Vec<String> {
    let credit = credit.replace('\0', ";");
    let words: Vec<&str> = credit.split_whitespace().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let marker = word.trim_start_matches(['(', '[']).to_lowercase();
        let featuring = matches!(
            marker.as_str(),
            "feat." | "feat" | "ft." | "ft" | "featuring"
        );
        let crossover = matches!(*word, "x" | "×") && i > 0 && i + 1 < words.len();
        if featuring || crossover {
            parts.push(current.join(" "));
            current.clear();
        } else {
            current.push(word);
        }
    }
    parts.push(current.join(" "));

    for (delimiter, rejoin_the) in [
        (";", None),
        (" / ", None),
        (",", Some(", ")),
        (" & ", Some(" & ")),
    ] {
        parts = parts
            .iter()
            .flat_map(|part| split_on(part, delimiter, rejoin_the))
            .collect();
    }
    parts
}

fn extract_artists(tag: &Tag) -> Vec<String> {
    let split_list = |values: &[&str]| {
        unique_names(
            values
                .iter()
                .flat_map(|value| value.split(['\0', ';']).collect::<Vec<_>>()),
        )
    };

    let listed: Vec<&str> = tag.get_strings(ItemKey::TrackArtists).collect();
    let credited: Vec<&str> = tag.get_strings(ItemKey::TrackArtist).collect();

    let names = split_list(&listed);
    if !names.is_empty() {
        return names;
    }
    match credited.as_slice() {
        [] => Vec::new(),
        [credit] => {
            let names = split_credit(credit);
            unique_names(names.iter().map(String::as_str))
        }
        many => split_list(many),
    }
}

fn extract_metadata_from_tag(tag: Option<&Tag>, duration: Duration) -> AudioMetadata {
    let (title, artists, album, album_artist, genres, year, track_number, lufs, isrc, upc, omm_ids) =
        if let Some(tag) = tag {
            let title = tag.title().map(|s| s.to_string());
            let artists = extract_artists(tag);
            let album = tag.album().map(|s| s.to_string());
            let album_artist = tag.get_string(ItemKey::AlbumArtist).and_then(clean_name);
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

            let isrc = tag.get_string(ItemKey::Isrc).and_then(normalize_isrc);
            let upc = tag.get_string(ItemKey::Barcode).and_then(normalize_upc);
            let [song, album_id, artist] =
                custom_fields(tag, ["OMM_SONG_ID", "OMM_ALBUM_ID", "OMM_ARTIST_ID"]);
            let omm_ids = (
                song.and_then(|raw| normalize_omm_id(&raw)),
                album_id.and_then(|raw| normalize_omm_id(&raw)),
                artist.and_then(|raw| normalize_omm_id(&raw)),
            );

            (
                title,
                artists,
                album,
                album_artist,
                genres,
                year,
                track_number,
                lufs,
                isrc,
                upc,
                omm_ids,
            )
        } else {
            (
                None,
                vec![],
                None,
                None,
                vec![],
                None,
                None,
                None,
                None,
                None,
                (None, None, None),
            )
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
        isrc,
        upc,
        omm_song_id: omm_ids.0,
        omm_album_id: omm_ids.1,
        omm_artist_id: omm_ids.2,
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

pub(crate) fn convert_to_jpeg(img: DynamicImage) -> Result<Vec<u8>> {
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

pub struct Offset<R> {
    inner: R,
    start: u64,
    len: u64,
}

impl<R: Seek> Offset<R> {
    fn new(mut inner: R, start: u64, len: u64) -> std::io::Result<Self> {
        inner.seek(SeekFrom::Start(start))?;
        Ok(Self { inner, start, len })
    }
}

impl<R: Read> Read for Offset<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for Offset<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let absolute = match pos {
            SeekFrom::Start(offset) => SeekFrom::Start(self.start.saturating_add(offset)),
            SeekFrom::End(offset) => {
                let end = (self.start + self.len) as i64;
                SeekFrom::Start(end.saturating_add(offset).max(self.start as i64) as u64)
            }
            current @ SeekFrom::Current(_) => current,
        };
        Ok(self.inner.seek(absolute)?.saturating_sub(self.start))
    }
}

fn id3v2_len(file: &mut File, file_len: u64) -> std::io::Result<u64> {
    let mut start = 0u64;
    loop {
        let mut header = [0u8; 10];
        file.seek(SeekFrom::Start(start))?;
        if file.read_exact(&mut header).is_err() || &header[..3] != b"ID3" {
            return Ok(start);
        }
        let size = header[6..10]
            .iter()
            .fold(0u64, |acc, byte| (acc << 7) | u64::from(byte & 0x7f));
        let footer = if header[5] & 0x10 != 0 { 10 } else { 0 };
        let next = start + 10 + size + footer;
        if next >= file_len {
            return Ok(start);
        }
        start = next;
    }
}

pub fn open_decoder(path: &str) -> Result<Decoder<Offset<BufReader<File>>>> {
    let mut file = File::open(path).with_context(|| format!("open {path}"))?;
    let file_len = file.metadata()?.len();
    let start = id3v2_len(&mut file, file_len)?;
    let len = file_len - start;
    let reader = Offset::new(BufReader::new(file), start, len)?;
    DecoderBuilder::new()
        .with_decoder::<OpusDecoder>()
        .with_data(reader)
        .with_byte_len(len)
        .build()
        .context("decode")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::TextEncoding;
    use lofty::id3::v2::ExtendedTextFrame;

    #[test]
    fn normalizes_omm_ids() {
        assert_eq!(
            normalize_omm_id(" 1024303286 ").as_deref(),
            Some("1024303286")
        );
        assert_eq!(normalize_omm_id("omm:song:1024303286"), None);
        assert_eq!(normalize_omm_id("0123"), None);
        assert_eq!(normalize_omm_id("12a"), None);
        assert_eq!(normalize_omm_id(""), None);
    }

    #[test]
    fn reads_omm_ids_from_id3_user_text() {
        let mut id3 = Id3v2Tag::new();
        for (name, value) in [
            ("OMM_SONG_ID", "1024303286"),
            ("OMM_ALBUM_ID", "1028859113"),
            ("OMM_ARTIST_ID", "1005090652"),
        ] {
            id3.insert(Frame::UserText(ExtendedTextFrame::new(
                TextEncoding::UTF8,
                name,
                value,
            )));
        }
        let tag = Tag::from(id3);
        let meta = extract_metadata_from_tag(Some(&tag), Duration::ZERO);
        assert_eq!(meta.omm_song_id.as_deref(), Some("1024303286"));
        assert_eq!(meta.omm_album_id.as_deref(), Some("1028859113"));
        assert_eq!(meta.omm_artist_id.as_deref(), Some("1005090652"));
    }

    #[test]
    fn reads_omm_ids_from_vorbis_comments() {
        let mut comments = VorbisComments::new();
        comments.push("omm_song_id".into(), "1024303286".into());
        comments.push("OMM_ARTIST_ID".into(), "1005090652".into());
        let tag = Tag::from(comments);
        let meta = extract_metadata_from_tag(Some(&tag), Duration::ZERO);
        assert_eq!(meta.omm_song_id.as_deref(), Some("1024303286"));
        assert_eq!(meta.omm_album_id, None);
        assert_eq!(meta.omm_artist_id.as_deref(), Some("1005090652"));
    }

    #[test]
    fn normalizes_isrc_and_upc() {
        assert_eq!(
            normalize_isrc("us-rc1-76-07839").as_deref(),
            Some("USRC17607839")
        );
        assert_eq!(normalize_isrc("not an isrc"), None);
        assert_eq!(
            normalize_upc("0602 5778 1234 5").as_deref(),
            Some("0602577812345")
        );
        assert_eq!(normalize_upc("0000000000000"), None);
        assert_eq!(normalize_upc("12"), None);
    }

    #[test]
    fn offset_reads_and_seeks_relative_to_the_start() {
        let mut view = Offset::new(Cursor::new(b"0123456789".to_vec()), 4, 6).unwrap();
        let mut buf = [0u8; 2];
        view.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"45");
        assert_eq!(view.seek(SeekFrom::Start(3)).unwrap(), 3);
        view.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"78");
        assert_eq!(view.seek(SeekFrom::End(0)).unwrap(), 6);
        assert_eq!(view.seek(SeekFrom::Current(-1)).unwrap(), 5);
    }
}
