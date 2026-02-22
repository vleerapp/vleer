pub mod bundled;
pub mod image_cache;

use crate::ui::assets::bundled::BundledAssets;
use dashmap::DashMap;
use gpui::{AssetSource, Result as GpuiResult};
use sqlx::SqlitePool;
use std::{borrow::Cow, sync::Arc, time::Duration};
use tracing::debug;
use url::Url;

pub struct VleerAssetSource {
    pool: Arc<SqlitePool>,
    cache: DashMap<String, Option<Vec<u8>>>,
}

impl VleerAssetSource {
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self {
            pool,
            cache: DashMap::new(),
        }
    }
}

impl AssetSource for VleerAssetSource {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let normalized = path.strip_prefix('!').unwrap_or(path);
        if let Some(image_id) = parse_image_id(normalized) {
            return get_image(&self.pool, &image_id, &self.cache);
        }

        let url = Url::parse(normalized)?;
        match url.scheme() {
            "bundled" => BundledAssets::load(url),
            _ => panic!("invalid url scheme for resource"),
        }
    }

    fn list(&self, path: &str) -> GpuiResult<Vec<gpui::SharedString>> {
        BundledAssets.list(path)
    }
}

pub fn get_image(
    pool: &SqlitePool,
    id: &str,
    cache: &DashMap<String, Option<Vec<u8>>>,
) -> gpui::Result<Option<Cow<'static, [u8]>>> {
    if let Some(cached) = cache.get(id) {
        return Ok(cached.as_ref().map(|v| Cow::Owned(v.clone())));
    }

    let query = "SELECT data FROM images WHERE id = ?";

    let row: Option<(Vec<u8>,)> = match crate::RUNTIME.block_on(async {
        tokio::time::timeout(
            Duration::from_millis(200),
            sqlx::query_as(query).bind(id).fetch_optional(pool),
        )
        .await
    }) {
        Ok(row) => row?,
        Err(_) => {
            debug!(image_id = %id, "image lookup timed out");
            return Ok(None);
        }
    };

    let result = row.map(|(image,)| image.clone());
    cache.insert(id.to_string(), result.clone());

    Ok(result.map(Cow::Owned))
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
