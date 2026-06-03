//! Shared image source data and per-protocol cache state.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::{mpsc, Arc};

use axis2d::Vec2;

use super::kitty::{PixelSlot, PlacementInner};

/// Error returned when constructing an [`ImageSource`](super::ImageSource) from encoded bytes fails.
#[derive(Debug)]
pub enum ImageSourceError {
    /// The byte signature did not match any known format.
    UnknownFormat,
    /// The format is recognized but not enabled in this build.
    UnsupportedFormat,
    /// The image header could not be parsed.
    Malformed,
}

impl fmt::Display for ImageSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat => write!(f, "could not detect image format"),
            Self::UnsupportedFormat => write!(f, "image format is not enabled in this build"),
            Self::Malformed => write!(f, "image header is malformed"),
        }
    }
}

impl std::error::Error for ImageSourceError {}

pub(crate) enum PixelFormat {
    Rgb,
    Rgba,
}

pub(crate) enum SourceData {
    Raw { width: u32, height: u32, pixels: Vec<u8>, format: PixelFormat },
    Encoded { bytes: Vec<u8>, dims: Vec2<u32>, format: image::ImageFormat },
}

impl SourceData {
    pub(crate) fn get_pixel_dims(&self) -> Vec2<u32> {
        match self {
            Self::Raw { width, height, .. } => Vec2::new(*width, *height),
            Self::Encoded { dims, .. } => *dims,
        }
    }
}

pub(super) struct SixelQuantized {
    pub indexed: Vec<u8>,
    pub palette_bytes: Vec<u8>,
}

pub(crate) struct KittyCache {
    // Visibility matches `PixelSlot` / `PlacementInner` (both `pub(super)`): the
    // cache is `pub(crate)` for `SourceInner`, but these slot/placement handles
    // never leave the image module, so the fields stay `pub(super)`.
    pub(super) pixel_slot: Option<Rc<PixelSlot>>,
    pub(super) placements: Vec<(Vec2<u16>, Rc<PlacementInner>)>,
    pub(super) decoded: Vec<(Vec2<u32>, AsyncEntry<Vec<u8>>)>,
}

impl KittyCache {
    fn new() -> Self {
        Self {
            pixel_slot: None,
            placements: Vec::new(),
            decoded: Vec::new(),
        }
    }
}

pub(crate) enum AsyncEntry<T> {
    Pending {
        #[allow(dead_code)]
        alive: Arc<()>,
        rx: mpsc::Receiver<Option<T>>,
    },
    Ready(Rc<T>),
}

pub(super) type HalfblockEntry = AsyncEntry<Vec<u8>>;
pub(super) type SixelEntry = AsyncEntry<SixelQuantized>;

pub(super) enum GraphicsCache {
    Kitty(KittyCache),
    Sixel(Vec<(Vec2<u32>, SixelEntry)>),
    Halfblock(Vec<(Vec2<u32>, HalfblockEntry)>),
}

pub(crate) struct SourceInner {
    pub data: Arc<SourceData>,
    // Private: the protocol caches are an implementation detail reached only
    // through the `with_*_cache` accessors below, so `GraphicsCache` never
    // escapes this module.
    cache: RefCell<Option<GraphicsCache>>,
}

impl SourceInner {
    pub(crate) fn get_pixel_dims(&self) -> Vec2<u32> {
        self.data.get_pixel_dims()
    }

    #[cfg(feature = "gui")]
    pub(crate) fn get_rgba(self: &Rc<Self>) -> Option<Rc<Vec<u8>>> {
        let dims = self.get_pixel_dims();
        self.with_kitty_cache(|cache| {
            await_async_entry(&mut cache.decoded, dims, || {
                decode_to_rgba(&self.data)
            })
        })
    }

    pub(crate) fn with_kitty_cache<R>(&self, f: impl FnOnce(&mut KittyCache) -> R) -> R {
        let mut slot = self.cache.borrow_mut();
        if !matches!(*slot, Some(GraphicsCache::Kitty(_))) {
            *slot = Some(GraphicsCache::Kitty(KittyCache::new()));
        }
        let Some(GraphicsCache::Kitty(kitty)) = slot.as_mut() else { unreachable!() };
        f(kitty)
    }

    pub(super) fn with_sixel_cache<R>(
        &self,
        f: impl FnOnce(&mut Vec<(Vec2<u32>, SixelEntry)>) -> R,
    ) -> R {
        let mut slot = self.cache.borrow_mut();
        if !matches!(*slot, Some(GraphicsCache::Sixel(_))) {
            *slot = Some(GraphicsCache::Sixel(Vec::new()));
        }
        let Some(GraphicsCache::Sixel(entries)) = slot.as_mut() else { unreachable!() };
        f(entries)
    }

    pub(super) fn with_halfblock_cache<R>(
        &self,
        f: impl FnOnce(&mut Vec<(Vec2<u32>, HalfblockEntry)>) -> R,
    ) -> R {
        let mut slot = self.cache.borrow_mut();
        if !matches!(*slot, Some(GraphicsCache::Halfblock(_))) {
            *slot = Some(GraphicsCache::Halfblock(Vec::new()));
        }
        let Some(GraphicsCache::Halfblock(entries)) = slot.as_mut() else { unreachable!() };
        f(entries)
    }
}

#[cfg(feature = "gui")]
fn decode_to_rgba(data: &SourceData) -> Option<Vec<u8>> {
    match data {
        SourceData::Raw { width, height, pixels, format } => match format {
            PixelFormat::Rgba => Some(pixels.clone()),
            PixelFormat::Rgb => {
                let n = (*width as usize) * (*height as usize);
                let mut out = Vec::with_capacity(n * 4);
                for i in 0..n {
                    let j = i * 3;
                    out.push(pixels[j]);
                    out.push(pixels[j + 1]);
                    out.push(pixels[j + 2]);
                    out.push(0xFF);
                }
                Some(out)
            }
        },
        SourceData::Encoded { bytes, .. } => {
            let img = image::load_from_memory(bytes).ok()?;
            Some(img.to_rgba8().into_raw())
        }
    }
}

pub(crate) fn prepare_async<T: Send + 'static>(
    entries: &mut Vec<(Vec2<u32>, AsyncEntry<T>)>,
    key: Vec2<u32>,
    data: &Arc<SourceData>,
    compute: impl FnOnce(&SourceData) -> Option<T> + Send + 'static,
) {
    if entries.iter().any(|(k, _)| *k == key) {
        return;
    }
    let alive = Arc::new(());
    let weak_for_worker = Arc::downgrade(&alive);
    let (tx, rx) = mpsc::channel();
    let data = data.clone();
    super::decode_pool::spawn(move || {
        if weak_for_worker.strong_count() == 0 {
            return;
        }
        let _ = tx.send(compute(&data));
    });
    entries.push((key, AsyncEntry::Pending { alive, rx }));
}

pub(crate) fn await_async_entry<T>(
    entries: &mut Vec<(Vec2<u32>, AsyncEntry<T>)>,
    key: Vec2<u32>,
    compute_sync: impl FnOnce() -> Option<T>,
) -> Option<Rc<T>> {
    if let Some(idx) = entries.iter().position(|(k, _)| *k == key) {
        if matches!(entries[idx].1, AsyncEntry::Ready(_)) {
            let AsyncEntry::Ready(rc) = &entries[idx].1 else { unreachable!() };
            return Some(rc.clone());
        }
        let (_, entry) = entries.remove(idx);
        let AsyncEntry::Pending { rx, .. } = entry else { unreachable!() };
        let value = rx.recv().ok().flatten()?;
        let rc = Rc::new(value);
        entries.push((key, AsyncEntry::Ready(rc.clone())));
        return Some(rc);
    }
    let rc = Rc::new(compute_sync()?);
    entries.push((key, AsyncEntry::Ready(rc.clone())));
    Some(rc)
}

pub(super) fn new_raw_source(
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    format: PixelFormat,
) -> Rc<SourceInner> {
    Rc::new(SourceInner {
        data: Arc::new(SourceData::Raw { width, height, pixels, format }),
        cache: RefCell::new(None),
    })
}

pub(super) fn new_encoded_source(bytes: Vec<u8>) -> Result<Rc<SourceInner>, ImageSourceError> {
    let (dims, format) = probe_encoded(&bytes)?;
    Ok(Rc::new(SourceInner {
        data: Arc::new(SourceData::Encoded { bytes, dims, format }),
        cache: RefCell::new(None),
    }))
}

fn probe_encoded(bytes: &[u8]) -> Result<(Vec2<u32>, image::ImageFormat), ImageSourceError> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ImageSourceError::Malformed)?;
    let format = reader.format().ok_or(ImageSourceError::UnknownFormat)?;
    let (width, height) = reader.into_dimensions().map_err(|e| match e {
        image::ImageError::Unsupported(_) => ImageSourceError::UnsupportedFormat,
        _ => ImageSourceError::Malformed,
    })?;
    Ok((Vec2::new(width, height), format))
}
