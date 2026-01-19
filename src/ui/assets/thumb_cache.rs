use image::ImageReader;
use lru::LruCache;
use std::io::Cursor;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Mutex;

const JPG_QUALITY: u8 = 80;
const MAX_CACHE_ENTRIES: usize = 500;

lazy_static::lazy_static! {
    static ref MEM_CACHE: Mutex<LruCache<String, Vec<u8>>> =
        Mutex::new(LruCache::new(NonZeroUsize::new(MAX_CACHE_ENTRIES).unwrap()));
}

pub fn get_thumbnail_for_path(orig: &Path, size: u32) -> Option<Vec<u8>> {
    let size = size.max(1).min(512) as f32 * 1.5;
    let key = format!("{}_{}", orig.to_string_lossy(), size);

    {
        let mut cache = MEM_CACHE.lock().ok()?;
        if let Some(bytes) = cache.get(&key) {
            return Some(bytes.clone());
        }
    }

    let bytes = generate_thumbnail(orig, size)?;

    {
        let mut cache = MEM_CACHE.lock().ok()?;
        cache.put(key, bytes.clone());
    }

    Some(bytes)
}

fn generate_thumbnail(orig: &Path, size: f32) -> Option<Vec<u8>> {
    let img = ImageReader::open(orig)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;

    let thumb = img.resize_exact(size as u32, size as u32, image::imageops::FilterType::Lanczos3);
    let mut cursor = Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, JPG_QUALITY);
    encoder.encode_image(&thumb).ok()?;

    Some(cursor.into_inner())
}
