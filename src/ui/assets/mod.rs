pub mod bundled;

use gpui::AssetSource;
use std::borrow::Cow;
use std::fs;
use std::path::PathBuf;
use url::Url;

use crate::ui::assets::bundled::BundledAssets;

pub struct VleerAssetSource;

impl VleerAssetSource {
    pub fn new() -> Self {
        Self {}
    }
}

impl AssetSource for VleerAssetSource {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path.starts_with("file:///") {
            let file_path_str = &path[7..];
            let file_path = PathBuf::from(file_path_str);
            match fs::read(&file_path) {
                Ok(data) => {
                    tracing::debug!("Cover loaded: {}", file_path.display());
                    return Ok(Some(Cow::Owned(data)));
                }
                Err(e) => {
                    tracing::warn!("Cover load failed: {:?} - {}", file_path, e);
                    return Ok(None);
                }
            }
        } else if path.starts_with("file:/") && !path.starts_with("file:///") {
            let file_path = dirs::home_dir().unwrap().join(&path[6..]);
            match fs::read(&file_path) {
                Ok(data) => {
                    tracing::debug!("Cover loaded: {}", file_path.display());
                    return Ok(Some(Cow::Owned(data)));
                }
                Err(e) => {
                    tracing::warn!("Cover load failed: {:?} - {}", file_path, e);
                    return Ok(None);
                }
            }
        }

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

    fn list(&self, path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        BundledAssets.list(path)
    }
}
