pub mod bundled;

use std::borrow::Cow;
use std::fs;
use gpui::AssetSource;
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
        let url = Url::parse(&path[1..])?;

        match url.scheme() {
            "bundled" => BundledAssets::load(url),
            "file" => {
                let file_path = url.to_file_path().map_err(|_| {
                    anyhow::anyhow!("Invalid file path: {}", path)
                })?;
                match fs::read(&file_path) {
                    Ok(data) => Ok(Some(Cow::Owned(data))),
                    Err(_) => Ok(None),
                }
            }
            _ => panic!("invalid url scheme for resource: {}", url.scheme()),
        }
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        BundledAssets.list(path)
    }
}
