pub mod bundled;
pub mod image_cache;

use crate::ui::assets::bundled::BundledAssets;
use gpui::{AssetSource, Result as GpuiResult};
use sqlx::SqlitePool;
use std::{borrow::Cow, sync::Arc};
use url::Url;

pub struct VleerAssetSource {
    pool: Arc<SqlitePool>,
}

impl VleerAssetSource {
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }
}

impl AssetSource for VleerAssetSource {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let normalized = path.strip_prefix('!').unwrap_or(path);
        if let Some(image_id) = parse_image_id(normalized) {
            return get_image(&self.pool, &image_id);
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

pub fn get_image(pool: &SqlitePool, id: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
    let query = "SELECT data FROM images WHERE id = ?";

    let row: Option<(Vec<u8>,)> =
        crate::RUNTIME.block_on(sqlx::query_as(query).bind(id).fetch_optional(pool))?;

    Ok(row.map(|(image,)| Cow::Owned(image)))
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
