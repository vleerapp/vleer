pub mod bundled;
pub mod image_cache;

use crate::ui::assets::bundled::BundledAssets;
use gpui::{App, Asset, Global, ImageCacheError, RenderImage, Resource};
use gpui::{AssetSource, Result as GpuiResult};
use image::{Frame, ImageError};
use sqlx::SqlitePool;
use std::borrow::Cow;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::Semaphore;
use url::Url;

#[derive(Clone)]
pub struct ImagePool(pub Arc<SqlitePool>);

impl Global for ImagePool {}

static IMAGE_LOAD_SEMAPHORE: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(8)));

const IMAGE_QUERY_TIMEOUT: Duration = Duration::from_millis(500);

pub enum VleerImageLoader {}

impl Asset for VleerImageLoader {
    type Source = Resource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl std::future::Future<Output = Self::Output> + Send + 'static {
        let pool = cx.global::<ImagePool>().0.clone();
        let semaphore = IMAGE_LOAD_SEMAPHORE.clone();
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

            crate::RUNTIME
                .spawn(async move {
                    let _permit = semaphore
                        .acquire()
                        .await
                        .map_err(|e| ImageCacheError::Asset(format!("semaphore closed: {}", e).into()))?;

                    let row = tokio::time::timeout(
                        IMAGE_QUERY_TIMEOUT,
                        sqlx::query_as::<_, (Vec<u8>,)>("SELECT data FROM images WHERE id = ?")
                            .bind(&image_id)
                            .fetch_optional(&*pool),
                    )
                    .await
                    .map_err(|_| ImageCacheError::Asset(format!("image query timed out: {}", image_id).into()))?
                    .map_err(|e| ImageCacheError::Asset(format!("sqlx: {}", e).into()))?;

                    let bytes = row
                        .ok_or_else(|| ImageCacheError::Asset("image not found".into()))?
                        .0;

                    tokio::task::spawn_blocking(move || decode_bytes(&bytes))
                        .await
                        .map_err(|e| ImageCacheError::Asset(format!("decode join: {}", e).into()))?
                })
                .await
                .map_err(|e| ImageCacheError::Asset(format!("tokio join: {}", e).into()))?
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
