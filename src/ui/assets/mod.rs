pub mod bundled;
pub mod image_cache;

use crate::data::db::repo::Database;
use crate::ui::assets::bundled::BundledAssets;
use gpui::{App, Asset, ImageCacheError, RenderImage, Resource};
use gpui::{AssetSource, Result as GpuiResult};
use image::imageops::FilterType;
use image::{Frame, ImageError};
use rusqlite::{OptionalExtension, params};
use std::borrow::Cow;
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;
use url::Url;

const RENDER_SCALE: u32 = 2;

fn decode_limit() -> &'static Semaphore {
    static LIMIT: OnceLock<Semaphore> = OnceLock::new();
    LIMIT.get_or_init(|| {
        let permits = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).max(2))
            .unwrap_or(2);
        Semaphore::new(permits)
    })
}

pub enum VleerImageLoader {}

impl Asset for VleerImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let image_conn = cx.global::<Database>().image_conn.clone();
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
            let (image_id, target) = parse_image_request(normalized).ok_or_else(|| {
                ImageCacheError::Asset(format!("invalid image uri: {}", path).into())
            })?;

            executor
                .spawn(async move {
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

fn parse_image_request(path: &str) -> Option<(String, Option<u32>)> {
    let rest = path.strip_prefix("image://")?;
    let (before, query) = match rest.split_once('?') {
        Some((before, query)) => (before, Some(query)),
        None => (rest, None),
    };
    let before = before.trim_start_matches('/');
    let id = before.split('/').next()?;
    if id.is_empty() {
        return None;
    }

    let size = query.and_then(|query| {
        query.split('&').find_map(|pair| {
            pair.strip_prefix("size=")
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|value| *value > 0)
        })
    });

    Some((id.to_string(), size))
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

    #[test]
    fn parses_id_without_size() {
        let (id, size) = parse_image_request("image://abc123").unwrap();
        assert_eq!(id, "abc123");
        assert_eq!(size, None);
    }

    #[test]
    fn parses_size_query() {
        let (id, size) = parse_image_request("image://abc123?size=36").unwrap();
        assert_eq!(id, "abc123");
        assert_eq!(size, Some(36));
    }

    #[test]
    fn ignores_zero_and_invalid_sizes() {
        assert_eq!(parse_image_request("image://a?size=0").unwrap().1, None);
        assert_eq!(parse_image_request("image://a?size=xx").unwrap().1, None);
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
