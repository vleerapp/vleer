pub mod bundled;
mod thumb_cache;

use crate::ui::assets::bundled::BundledAssets;
use gpui::{AssetSource, Result as GpuiResult};
use std::borrow::Cow;
use std::fs;
use std::path::PathBuf;
use url::Url;

pub struct VleerAssetSource;

impl VleerAssetSource {
    pub fn new() -> Self {
        Self {}
    }

    fn parse_path_with_size(&self, path: &str) -> (String, u32) {
        if let Some(idx) = path.find('?') {
            let (file_part, query_part) = path.split_at(idx);
            if let Some(size_str) = query_part.strip_prefix("?size=") {
                if let Ok(size) = size_str.parse::<u32>() {
                    return (file_part.to_string(), size);
                }
            }
        }
        (path.to_string(), 200)
    }

    fn read_file(path: &PathBuf) -> Option<Vec<u8>> {
        fs::read(path).ok()
    }

    fn load_thumbnail_sync(&self, file_path: &PathBuf, size: u32) -> Option<Vec<u8>> {
        // Now fully synchronous - safe to call from GPUI worker threads
        thumb_cache::get_thumbnail_for_path(file_path, size)
    }
}

impl AssetSource for VleerAssetSource {
    fn load(&self, path: &str) -> GpuiResult<Option<Cow<'static, [u8]>>> {
        // Local files with absolute paths
        if path.starts_with("file:///") {
            let (file_path_str, size) = self.parse_path_with_size(&path[7..]);
            let file_path = PathBuf::from(file_path_str);

            if let Some(bytes) = self.load_thumbnail_sync(&file_path, size) {
                tracing::debug!("Cover thumb loaded: {} ({}px)", file_path.display(), size);
                return Ok(Some(Cow::Owned(bytes)));
            }

            if let Some(data) = Self::read_file(&file_path) {
                tracing::debug!("Cover loaded fallback: {}", file_path.display());
                return Ok(Some(Cow::Owned(data)));
            }

            tracing::warn!("Cover load failed: {}", file_path.display());
            return Ok(None);
        }

        // Local files with home-relative paths
        if path.starts_with("file:/") && !path.starts_with("file:///") {
            let (file_path_str, size) = self.parse_path_with_size(&path[6..]);
            let file_path = dirs::home_dir().unwrap().join(file_path_str);

            if let Some(bytes) = self.load_thumbnail_sync(&file_path, size) {
                tracing::debug!("Cover thumb loaded: {} ({}px)", file_path.display(), size);
                return Ok(Some(Cow::Owned(bytes)));
            }

            if let Some(data) = Self::read_file(&file_path) {
                tracing::debug!("Cover loaded fallback: {}", file_path.display());
                return Ok(Some(Cow::Owned(data)));
            }

            tracing::warn!("Cover load failed: {}", file_path.display());
            return Ok(None);
        }

        // Bundled assets
        if path.starts_with("!bundled:") {
            let asset_path = &path[9..];
            return Ok(BundledAssets::get(asset_path).map(|f| f.data));
        }

        if let Ok(url) = Url::parse(path) {
            if url.scheme() == "bundled" {
                return BundledAssets::load(url);
            }
        }

        tracing::warn!("Unsupported asset path: {}", path);
        Ok(None)
    }

    fn list(&self, path: &str) -> GpuiResult<Vec<gpui::SharedString>> {
        BundledAssets.list(path)
    }
}
