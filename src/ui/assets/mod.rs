pub mod bundled;
pub mod image_cache;

use crate::data::db::repo::Database;
use crate::ui::assets::bundled::BundledAssets;
use gpui::{App, Asset, ImageCacheError, RenderImage, Resource};
use gpui::{AssetSource, Result as GpuiResult};
use image::{Frame, ImageError};
use rusqlite::{OptionalExtension, params};
use std::borrow::Cow;
use std::sync::Arc;
use url::Url;

pub enum VleerImageLoader {}

impl Asset for VleerImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl std::future::Future<Output = Self::Output> + Send + 'static {
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
            let image_id = parse_image_id(normalized).ok_or_else(|| {
                ImageCacheError::Asset(format!("invalid image uri: {}", path).into())
            })?;

            executor
                .spawn(async move {
                    let conn = image_conn.lock();
                    let bytes: Option<Vec<u8>> = conn
                        .query_row(
                            "SELECT data FROM images WHERE id = ?1",
                            params![image_id],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|e| ImageCacheError::Asset(format!("rusqlite: {}", e).into()))?;
                    let bytes =
                        bytes.ok_or_else(|| ImageCacheError::Asset("image not found".into()))?;
                    decode_bytes(&bytes)
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

fn decode_bytes(bytes: &[u8]) -> Result<Arc<RenderImage>, ImageCacheError> {
    let format = image::guess_format(bytes).map_err(image_err)?;
    let mut data = image::load_from_memory_with_format(bytes, format)
        .map_err(image_err)?
        .into_rgba8();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let frames: Vec<Frame> = vec![Frame::new(data)];
    Ok(Arc::new(RenderImage::new(frames)))
}

fn image_err(e: ImageError) -> ImageCacheError {
    ImageCacheError::Image(Arc::new(e))
}

fn parse_image_id(path: &str) -> Option<String> {
    let rest = path.strip_prefix("image://")?;
    let rest = rest.split('?').next()?;
    let rest = rest.trim_start_matches('/');
    let id = rest.split('/').next()?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
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
            _ => panic!("invalid url scheme for resource"),
        }
    }

    fn list(&self, path: &str) -> GpuiResult<Vec<gpui::SharedString>> {
        BundledAssets.list(path)
    }
}
