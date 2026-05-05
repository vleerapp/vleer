use std::{
    collections::VecDeque,
    mem::take,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use futures::FutureExt;
use gpui::{
    App, AppContext, Asset, AssetLogger, ElementId, Entity, ImageAssetLoader, ImageCache,
    ImageCacheItem, ImageCacheProvider, ImageSource, Resource, hash,
};
use rustc_hash::{FxBuildHasher, FxHashMap};
use tracing::{error, trace};

use crate::ui::assets::{VleerImageLoader, is_vleer_image};

pub fn vleer_cache(id: impl Into<ElementId>, max_items: usize) -> VleerImageCacheProvider {
    VleerImageCacheProvider {
        id: id.into(),
        max_items,
    }
}

pub fn app_image_cache() -> VleerImageCacheProvider {
    vleer_cache("vleer-app-image-cache", 200)
}

pub struct VleerImageCacheProvider {
    id: ElementId,
    max_items: usize,
}

impl ImageCacheProvider for VleerImageCacheProvider {
    fn provide(&mut self, window: &mut gpui::Window, cx: &mut App) -> gpui::AnyImageCache {
        window
            .with_global_id(self.id.clone(), |id, window| {
                window.with_element_state(id, |cache: Option<Entity<VleerImageCache>>, _| {
                    let cache = if let Some(cache) = cache {
                        cache.update(cx, |cache, _| {
                            cache.ensure_capacity(self.max_items);
                        });
                        cache
                    } else {
                        VleerImageCache::new(self.max_items, cx)
                    };

                    (cache.clone(), cache)
                })
            })
            .into()
    }
}

pub struct VleerImageCache {
    max_items: usize,
    usage_list: VecDeque<u64>,
    cache: FxHashMap<u64, (ImageCacheItem, Resource)>,
    notify_pending: Arc<AtomicBool>,
}

impl VleerImageCache {
    pub fn new(max_items: usize, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            trace!("Creating VleerImageCache");
            cx.on_release(|this: &mut Self, cx| {
                for (idx, (mut image, resource)) in take(&mut this.cache) {
                    if let Some(Ok(image)) = image.get() {
                        trace!("Dropping image {idx}");
                        cx.drop_image(image, None);
                    }

                    ImageSource::Resource(resource).remove_asset(cx);
                }
            })
            .detach();

            VleerImageCache {
                max_items,
                usage_list: VecDeque::with_capacity(max_items),
                cache: FxHashMap::with_capacity_and_hasher(max_items, FxBuildHasher),
                notify_pending: Arc::new(AtomicBool::new(false)),
            }
        })
    }

    fn ensure_capacity(&mut self, max_items: usize) {
        if max_items <= self.max_items {
            return;
        }

        let additional = max_items - self.max_items;
        self.max_items = max_items;
        self.usage_list.reserve(additional);
        self.cache.reserve(additional);
    }
}

impl ImageCache for VleerImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Option<Result<std::sync::Arc<gpui::RenderImage>, gpui::ImageCacheError>> {
        let hash = hash(resource);

        if let Some(item) = self.cache.get_mut(&hash) {
            let current_idx = self
                .usage_list
                .iter()
                .position(|item| *item == hash)
                .expect("cache has an item usage_list doesn't");

            self.usage_list.remove(current_idx);
            self.usage_list.push_front(hash);

            return item.0.get();
        }

        let task = if is_vleer_image(resource) {
            let future = VleerImageLoader::load(resource.clone(), cx);
            cx.background_executor().spawn(future).shared()
        } else {
            let future = AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
            cx.background_executor().spawn(future).shared()
        };

        if self.usage_list.len() >= self.max_items {
            trace!("Image cache is full, evicting oldest item");

            let oldest = self.usage_list.pop_back().unwrap();
            let mut image = self
                .cache
                .remove(&oldest)
                .expect("usage_list has an item cache doesn't");

            if let Some(Ok(image)) = image.0.get() {
                trace!("requesting image to be dropped");
                cx.drop_image(image, Some(window));
            }

            ImageSource::Resource(image.1).remove_asset(cx);
        }

        self.cache.insert(
            hash,
            (
                gpui::ImageCacheItem::Loading(task.clone()),
                resource.clone(),
            ),
        );
        self.usage_list.push_front(hash);

        let entity = window.current_view();
        let notify_pending = self.notify_pending.clone();

        window
            .spawn(cx, async move |cx| {
                let result = task.await;

                if let Err(err) = result {
                    error!("error loading image into cache: {:?}", err);
                }

                if !notify_pending.swap(true, Ordering::AcqRel) {
                    let notify_pending = notify_pending.clone();
                    cx.update(move |_, cx| {
                        notify_pending.store(false, Ordering::Release);
                        cx.notify(entity);
                    })
                    .ok();
                }
            })
            .detach();

        None
    }
}
