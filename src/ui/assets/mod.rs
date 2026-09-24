pub mod bundled;
pub mod image_cache;

use crate::data::db::repo::Database;
use crate::data::images::{NO_IMAGE, decode_limit, resolve_album_image, resolve_song_image};
use crate::data::models::Cuid;
use crate::ui::assets::bundled::BundledAssets;
use gpui::{App, Asset, ImageCacheError, RenderImage, Resource};
use gpui::{AssetSource, Result as GpuiResult};
use image::imageops::FilterType;
use image::{Frame, ImageError};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rusqlite::{OptionalExtension, params};
use std::borrow::Cow;
use std::sync::{Arc, OnceLock};
use url::Url;

const RENDER_SCALE: u32 = 2;

const TRACK_NAMESPACE: &str = "track";
const ALBUM_NAMESPACE: &str = "album";
const GENRE_NAMESPACE: &str = "genre";

const GENRE_TINTS: [[f32; 3]; 10] = [
    [0.0, 190.0, 110.0],
    [240.0, 100.0, 50.0],
    [255.0, 0.0, 60.0],
    [230.0, 180.0, 0.0],
    [240.0, 90.0, 140.0],
    [90.0, 130.0, 240.0],
    [140.0, 70.0, 200.0],
    [0.0, 150.0, 160.0],
    [190.0, 200.0, 0.0],
    [255.0, 120.0, 0.0],
];

#[derive(Debug, PartialEq)]
pub enum ImageRequest {
    Stored(String),
    Track(Cuid),
    Album(Cuid),
    Genre(Cuid),
}

pub enum VleerImageLoader {}

impl Asset for VleerImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let db = cx.global::<Database>().clone();
        let image_conn = db.image_conn.clone();
        let executor = cx.background_executor().clone();

        async move {
            let path = match &source {
                Resource::Embedded(p) => p.as_ref().to_string(),
                _ => {
                    return Err(ImageCacheError::Asset(
                        "vleer loader expects embedded resource".into(),
                    ));
                }
            };
            let normalized = path.strip_prefix('!').unwrap_or(&path);
            let (request, target) = parse_image_request(normalized).ok_or_else(|| {
                ImageCacheError::Asset(format!("invalid image uri: {}", path).into())
            })?;

            executor
                .spawn(async move {
                    if let ImageRequest::Genre(genre_id) = &request {
                        return compose_genre_cover(&db, &image_conn, genre_id, target).await;
                    }

                    let image_id = match request {
                        ImageRequest::Stored(id) => id,
                        ImageRequest::Track(song_id) => resolve_song_image(db, song_id)
                            .await
                            .map_err(|e| ImageCacheError::Asset(e.to_string().into()))?,
                        ImageRequest::Album(album_id) => resolve_album_image(db, album_id)
                            .await
                            .map_err(|e| ImageCacheError::Asset(e.to_string().into()))?,
                        ImageRequest::Genre(_) => unreachable!("genre covers are composed above"),
                    };

                    let bytes: Option<Vec<u8>> = {
                        let conn = image_conn.lock();
                        conn.query_row(
                            "SELECT data FROM images WHERE id = ?1",
                            params![image_id],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|e| ImageCacheError::Asset(format!("rusqlite: {}", e).into()))?
                    };
                    let bytes =
                        bytes.ok_or_else(|| ImageCacheError::Asset("image not found".into()))?;

                    let _permit = decode_limit().acquire().await;
                    decode_bytes(&bytes, target)
                })
                .await
        }
    }
}

pub fn bucket_size(size: f32) -> u32 {
    ((size.max(1.0) as u32).div_ceil(64)) * 64
}

pub fn cover_uri(image_id: Option<&str>, fallback: ImageRequest) -> String {
    match image_id {
        Some(id) => format!("!image://{id}"),
        None => match fallback {
            ImageRequest::Stored(id) => format!("!image://{id}"),
            ImageRequest::Track(id) => format!("!image://{TRACK_NAMESPACE}/{id}"),
            ImageRequest::Album(id) => format!("!image://{ALBUM_NAMESPACE}/{id}"),
            ImageRequest::Genre(id) => format!("!image://{GENRE_NAMESPACE}/{id}"),
        },
    }
}

pub fn genre_cover_uri(genre_id: &Cuid) -> String {
    cover_uri(None, ImageRequest::Genre(genre_id.clone()))
}

pub fn is_vleer_image(resource: &Resource) -> bool {
    match resource {
        Resource::Embedded(p) => {
            let s = p.as_ref();
            let normalized = s.strip_prefix('!').unwrap_or(s);
            normalized.starts_with("image://")
        }
        _ => false,
    }
}

fn genre_seed(genre_id: &Cuid) -> u64 {
    static SESSION: OnceLock<u64> = OnceLock::new();
    let session = *SESSION.get_or_init(rand::random::<u64>);
    xxhash_rust::xxh3::xxh3_64_with_seed(genre_id.to_string().as_bytes(), session)
}

async fn compose_genre_cover(
    db: &Database,
    image_conn: &Arc<parking_lot::Mutex<rusqlite::Connection>>,
    genre_id: &Cuid,
    target: Option<u32>,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    let mut image_ids = db
        .genre_cover_image_ids(genre_id)
        .map_err(|e| ImageCacheError::Asset(e.to_string().into()))?;
    if image_ids.is_empty() {
        return Err(ImageCacheError::Asset(NO_IMAGE.into()));
    }

    let seed = genre_seed(genre_id);
    image_ids.sort();
    image_ids.shuffle(&mut StdRng::seed_from_u64(seed));
    image_ids.truncate(4);

    let mut blobs = Vec::with_capacity(image_ids.len());
    for image_id in &image_ids {
        let bytes: Option<Vec<u8>> = {
            let conn = image_conn.lock();
            conn.query_row(
                "SELECT data FROM images WHERE id = ?1",
                params![image_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| ImageCacheError::Asset(format!("rusqlite: {}", e).into()))?
        };
        if let Some(bytes) = bytes {
            blobs.push(bytes);
        }
    }
    if blobs.is_empty() {
        return Err(ImageCacheError::Asset(NO_IMAGE.into()));
    }

    let _permit = decode_limit().acquire().await;
    let size = target
        .map(|t| t.saturating_mul(RENDER_SCALE))
        .unwrap_or(512)
        .clamp(64, 1024)
        & !1;
    let tint = GENRE_TINTS[(seed >> 32) as usize % GENRE_TINTS.len()];
    compose_tinted_grid(&blobs, size, tint)
}

fn compose_tinted_grid(
    blobs: &[Vec<u8>],
    size: u32,
    tint: [f32; 3],
) -> Result<Arc<RenderImage>, ImageCacheError> {
    let tiles = blobs
        .iter()
        .map(|bytes| {
            let format = image::guess_format(bytes).map_err(image_err)?;
            image::load_from_memory_with_format(bytes, format).map_err(image_err)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut canvas = image::RgbaImage::new(size, size);
    if tiles.len() == 1 {
        let tile = tiles[0].resize_to_fill(size, size, FilterType::Triangle);
        image::imageops::replace(&mut canvas, &tile.into_rgba8(), 0, 0);
    } else {
        let half = size / 2;
        for slot in 0..4u32 {
            let tile = tiles[slot as usize % tiles.len()]
                .resize_to_fill(half, half, FilterType::Triangle)
                .into_rgba8();
            image::imageops::replace(
                &mut canvas,
                &tile,
                i64::from((slot % 2) * half),
                i64::from((slot / 2) * half),
            );
        }
    }

    for pixel in canvas.chunks_exact_mut(4) {
        let luma =
            (0.299 * pixel[0] as f32 + 0.587 * pixel[1] as f32 + 0.114 * pixel[2] as f32) / 255.0;
        let factor = 0.3 + 0.85 * luma;
        pixel[0] = (tint[2] * factor).min(255.0) as u8;
        pixel[1] = (tint[1] * factor).min(255.0) as u8;
        pixel[2] = (tint[0] * factor).min(255.0) as u8;
        pixel[3] = 255;
    }

    Ok(Arc::new(RenderImage::new(vec![Frame::new(canvas)])))
}

fn decode_bytes(bytes: &[u8], target: Option<u32>) -> Result<Arc<RenderImage>, ImageCacheError> {
    let format = image::guess_format(bytes).map_err(image_err)?;
    let decoded = image::load_from_memory_with_format(bytes, format).map_err(image_err)?;

    let decoded = match target {
        Some(size) => {
            let max = size.saturating_mul(RENDER_SCALE);
            if decoded.width() > max || decoded.height() > max {
                decoded.resize(max, max, FilterType::Triangle)
            } else {
                decoded
            }
        }
        None => decoded,
    };

    let mut data = decoded.into_rgba8();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let frames: Vec<Frame> = vec![Frame::new(data)];
    Ok(Arc::new(RenderImage::new(frames)))
}

fn image_err(e: ImageError) -> ImageCacheError {
    ImageCacheError::Image(Arc::new(e))
}

fn parse_image_request(path: &str) -> Option<(ImageRequest, Option<u32>)> {
    let rest = path.strip_prefix("image://")?;
    let (before, query) = match rest.split_once('?') {
        Some((before, query)) => (before, Some(query)),
        None => (rest, None),
    };
    let before = before.trim_start_matches('/');
    let mut segments = before.split('/');
    let head = segments.next()?;
    if head.is_empty() {
        return None;
    }

    let request = match head {
        TRACK_NAMESPACE | ALBUM_NAMESPACE | GENRE_NAMESPACE => {
            let id = segments.next().filter(|id| !id.is_empty())?;
            let id = Cuid::from(id.to_string());
            match head {
                TRACK_NAMESPACE => ImageRequest::Track(id),
                ALBUM_NAMESPACE => ImageRequest::Album(id),
                _ => ImageRequest::Genre(id),
            }
        }
        id => ImageRequest::Stored(id.to_string()),
    };

    let size = query.and_then(|query| {
        query.split('&').find_map(|pair| {
            pair.strip_prefix("size=")
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|value| *value > 0)
        })
    });

    Some((request, size))
}

pub struct VleerAssetSource;

impl VleerAssetSource {
    pub fn new() -> Self {
        Self
    }
}

impl AssetSource for VleerAssetSource {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let normalized = path.strip_prefix('!').unwrap_or(path);

        let url = Url::parse(normalized)?;
        match url.scheme() {
            "bundled" => BundledAssets::load(url),
            "image" => Ok(None),
            scheme => Err(anyhow::anyhow!("invalid url scheme for resource: {scheme}")),
        }
    }

    fn list(&self, path: &str) -> GpuiResult<Vec<gpui::SharedString>> {
        BundledAssets.list(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(uri: &str) -> ImageRequest {
        parse_image_request(uri).unwrap().0
    }

    #[test]
    fn parses_id_without_size() {
        let (id, size) = parse_image_request("image://abc123").unwrap();
        assert_eq!(id, ImageRequest::Stored("abc123".into()));
        assert_eq!(size, None);
    }

    #[test]
    fn parses_size_query() {
        let (id, size) = parse_image_request("image://abc123?size=36").unwrap();
        assert_eq!(id, ImageRequest::Stored("abc123".into()));
        assert_eq!(size, Some(36));
    }

    #[test]
    fn ignores_zero_and_invalid_sizes() {
        assert_eq!(parse_image_request("image://a?size=0").unwrap().1, None);
        assert_eq!(parse_image_request("image://a?size=xx").unwrap().1, None);
    }

    #[test]
    fn parses_unresolved_requests() {
        assert_eq!(
            request("image://track/song1"),
            ImageRequest::Track(Cuid::from("song1".to_string()))
        );
        assert_eq!(
            request("image://album/album1"),
            ImageRequest::Album(Cuid::from("album1".to_string()))
        );
        assert_eq!(
            parse_image_request("image://track/song1?size=36")
                .unwrap()
                .1,
            Some(36)
        );
    }

    #[test]
    fn stored_digest_is_not_mistaken_for_a_namespace() {
        let digest = "a".repeat(64);
        assert_eq!(
            request(&format!("image://{digest}")),
            ImageRequest::Stored(digest)
        );
    }

    #[test]
    fn rejects_namespace_without_id() {
        assert_eq!(parse_image_request("image://track"), None);
        assert_eq!(parse_image_request("image://track/"), None);
        assert_eq!(parse_image_request("image://album/"), None);
    }

    #[test]
    fn cover_uri_prefers_the_stored_image() {
        let song = Cuid::from("song1".to_string());
        assert_eq!(
            cover_uri(Some("abc"), ImageRequest::Track(song.clone())),
            "!image://abc"
        );
        assert_eq!(
            cover_uri(None, ImageRequest::Track(song)),
            "!image://track/song1"
        );
    }

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let img = image::DynamicImage::new_rgb8(width, height);
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn decode_downscales_to_requested_size() {
        let bytes = jpeg(1024, 1024);
        let image = decode_bytes(&bytes, Some(36)).unwrap();
        let size = image.size(0);
        assert_eq!(
            (size.width.0 as u32, size.height.0 as u32),
            (36 * RENDER_SCALE, 36 * RENDER_SCALE),
            "a 1024px cover rendered at 36px must not occupy a full atlas page"
        );
    }

    #[test]
    fn decode_never_upscales() {
        let bytes = jpeg(64, 64);
        let image = decode_bytes(&bytes, Some(512)).unwrap();
        let size = image.size(0);
        assert_eq!((size.width.0 as u32, size.height.0 as u32), (64, 64));
    }

    #[test]
    fn decode_without_size_keeps_original() {
        let bytes = jpeg(200, 200);
        let image = decode_bytes(&bytes, None).unwrap();
        let size = image.size(0);
        assert_eq!((size.width.0 as u32, size.height.0 as u32), (200, 200));
    }
}
