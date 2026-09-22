use std::{
    cell::RefCell,
    mem::take,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use gpui::{
    App, AppContext, Asset, AssetLogger, ElementId, Entity, ImageAssetLoader, ImageCache,
    ImageCacheError, ImageCacheProvider, ImageSource, RenderImage, Resource, hash,
};
use rustc_hash::{FxBuildHasher, FxHashMap};
use tracing::{error, trace};

use crate::data::images::is_missing_image;
use crate::ui::assets::{VleerImageLoader, is_vleer_image};

const BUFFER: usize = 10;
const FRAME_WINDOW: Duration = Duration::from_millis(50);

pub fn vleer_cache(id: impl Into<ElementId>) -> VleerImageCacheProvider {
    VleerImageCacheProvider { id: id.into() }
}

pub fn chrome_image_cache() -> VleerImageCacheProvider {
    vleer_cache("vleer-chrome-image-cache")
}

pub fn view_image_cache(view: crate::ui::views::AppView) -> VleerImageCacheProvider {
    vleer_cache(ElementId::Name(
        format!("vleer-view-image-cache-{}", view.title()).into(),
    ))
}

pub struct VleerImageCacheProvider {
    id: ElementId,
}

#[derive(Default)]
struct CacheRegistry(FxHashMap<ElementId, Entity<VleerImageCache>>);

impl gpui::Global for CacheRegistry {}

impl ImageCacheProvider for VleerImageCacheProvider {
    fn provide(&mut self, _window: &mut gpui::Window, cx: &mut App) -> gpui::AnyImageCache {
        let existing = cx
            .default_global::<CacheRegistry>()
            .0
            .get(&self.id)
            .cloned();
        let cache = existing.unwrap_or_else(|| {
            let cache = VleerImageCache::new(cx);
            cx.default_global::<CacheRegistry>()
                .0
                .insert(self.id.clone(), cache.clone());
            cache
        });
        cache.into()
    }
}

type ImageResult = Result<Arc<RenderImage>, ImageCacheError>;

struct CacheItem(
    Rc<RefCell<Option<ImageResult>>>,
    #[allow(dead_code)] gpui::Task<()>,
);

impl CacheItem {
    fn get(&self) -> Option<ImageResult> {
        self.0.borrow().clone()
    }
}

pub struct VleerImageCache {
    cache: FxHashMap<u64, (CacheItem, Resource, Instant)>,
    newest: Instant,
    notify_pending: Arc<AtomicBool>,
}

impl VleerImageCache {
    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            trace!("Creating VleerImageCache");
            cx.on_release(|this: &mut Self, cx| {
                for (idx, (image, resource, _)) in take(&mut this.cache) {
                    if let Some(Ok(image)) = image.get() {
                        trace!("Dropping image {idx}");
                        cx.drop_image(image, None);
                    }

                    ImageSource::Resource(resource).remove_asset(cx);
                }
            })
            .detach();

            VleerImageCache {
                cache: FxHashMap::with_hasher(FxBuildHasher),
                newest: Instant::now(),
                notify_pending: Arc::new(AtomicBool::new(false)),
            }
        })
    }

    fn evict_stale(&mut self, window: &mut gpui::Window, cx: &mut App) {
        let cutoff = self.newest.checked_sub(FRAME_WINDOW);
        let is_visible = |used: &Instant| cutoff.is_none_or(|cutoff| *used >= cutoff);

        let mut stale: Vec<(u64, Instant)> = self
            .cache
            .iter()
            .filter(|(_, (_, _, u))| !is_visible(u))
            .map(|(h, (_, _, u))| (*h, *u))
            .collect();

        if stale.len() <= BUFFER {
            return;
        }

        stale.sort_by_key(|(_, used)| *used);
        let evict = stale.len() - BUFFER;
        for (hash, _) in stale.into_iter().take(evict) {
            let Some((item, resource, _)) = self.cache.remove(&hash) else {
                continue;
            };

            if let Some(Ok(image)) = item.get() {
                cx.drop_image(image, Some(window));
            }

            ImageSource::Resource(resource).remove_asset(cx);
        }
    }
}

impl ImageCache for VleerImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut gpui::Window,
        cx: &mut App,
    ) -> Option<Result<Arc<gpui::RenderImage>, gpui::ImageCacheError>> {
        let hash = hash(resource);
        let now = Instant::now();

        if let Some(item) = self.cache.get_mut(&hash) {
            item.2 = now;
            let result = item.0.get();
            self.newest = now;
            return result;
        }

        self.evict_stale(window, cx);

        let task = if is_vleer_image(resource) {
            let future = VleerImageLoader::load(resource.clone(), cx);
            cx.background_executor().spawn(future)
        } else {
            let future = AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
            cx.background_executor().spawn(future)
        };

        let slot = Rc::new(RefCell::new(None));
        let slot_for_item = slot.clone();

        let entity = window.current_view();
        let notify_pending = self.notify_pending.clone();

        let load_task = window.spawn(cx, async move |cx| {
            let result = task.await;
            *slot.borrow_mut() = Some(result.clone());

            match result {
                Err(gpui::ImageCacheError::Asset(message))
                    if is_missing_image(message.as_ref()) =>
                {
                    trace!("no cover image for {message}");
                }
                Err(err) => error!("error loading image into cache: {:?}", err),
                Ok(_) => {}
            }

            if !notify_pending.swap(true, Ordering::AcqRel) {
                let notify_pending = notify_pending.clone();
                cx.update(move |_, cx| {
                    notify_pending.store(false, Ordering::Release);
                    cx.notify(entity);
                })
                .ok();
            }
        });

        self.cache.insert(
            hash,
            (CacheItem(slot_for_item, load_task), resource.clone(), now),
        );
        self.newest = now;

        None
    }
}
