//! Image rendering support.

pub(crate) mod cover;
pub(crate) mod decode_pool;
pub(crate) mod escape;
pub(crate) mod halfblock;
pub(crate) mod kitty;
#[cfg(unix)]
pub(crate) mod shm;
pub(crate) mod sixel;
pub(crate) mod sixel_encode;
pub(crate) mod source;

pub use source::ImageSourceError;

use std::rc::Rc;

use crate::prelude::*;
use source::{PixelFormat, SourceInner};

/// Cloneable handle to pixel data shared between [`Image`](crate::widget::widgets::image::Image) instances.
pub struct ImageSource {
    pub(crate) inner: Rc<SourceInner>,
}

impl Clone for ImageSource {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl ImageSource {
    /// Creates a source from tightly packed RGB bytes.
    ///
    /// # Panics
    ///
    /// Panics when `pixels.len()` does not equal `width * height * 3`.
    pub fn from_rgb(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        let expected = width as usize * height as usize * 3;
        assert_eq!(
            pixels.len(), expected,
            "ImageSource::from_rgb: pixels.len()={} but width*height*3={} ({}x{})",
            pixels.len(), expected, width, height,
        );
        Self { inner: source::new_raw_source(width, height, pixels, PixelFormat::Rgb) }
    }

    /// Creates a source from tightly packed RGBA bytes.
    ///
    /// # Panics
    ///
    /// Panics when `pixels.len()` does not equal `width * height * 4`.
    pub fn from_rgba(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        let expected = width as usize * height as usize * 4;
        assert_eq!(
            pixels.len(), expected,
            "ImageSource::from_rgba: pixels.len()={} but width*height*4={} ({}x{})",
            pixels.len(), expected, width, height,
        );
        Self { inner: source::new_raw_source(width, height, pixels, PixelFormat::Rgba) }
    }

    /// Creates a source from encoded image bytes.
    pub fn from_encoded(bytes: Vec<u8>) -> Result<Self, ImageSourceError> {
        Ok(Self { inner: source::new_encoded_source(bytes)? })
    }

    pub(crate) fn get_pixel_dims(&self) -> Vec2<u32> {
        self.inner.get_pixel_dims()
    }

    #[cfg(feature = "gui")]
    pub(crate) fn identity(&self) -> *const () {
        std::rc::Rc::as_ptr(&self.inner) as *const ()
    }

    #[cfg(feature = "gui")]
    pub(crate) fn get_rgba(&self) -> Option<std::rc::Rc<Vec<u8>>> {
        self.inner.get_rgba()
    }
}

/// Image rendering backend selection.
#[derive(Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImageProtocol {
    /// Sixel DCS payload backend.
    Sixel,
    /// Kitty graphics protocol backend.
    Kitty,
    /// Halfblock character fallback backend.
    HalfBlock,
}

#[cfg(feature = "images")]
impl std::fmt::Display for ImageProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sixel => write!(f, "Sixel"),
            Self::Kitty => write!(f, "Kitty"),
            Self::HalfBlock => write!(f, "HalfBlock"),
        }
    }
}

/// Default configuration for all [`Image`](crate::widget::widgets::image::Image) widgets.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct ImageConfig {
    /// The forced backend, or `None` to auto-select.
    pub protocol: Option<ImageProtocol>,
}

crate::config_module!(ImageConfig {
    protocol: None,
});

/// The image protocol that would be auto-selected for the current terminal
/// (honoring any forced override). Exposed for diagnostics and settings UIs.
pub fn selected_protocol() -> ImageProtocol {
    pick_protocol()
}

pub(crate) fn pick_protocol() -> ImageProtocol {
    if let Some(forced) = config::get().protocol {
        return forced;
    }
    let caps = crate::runtime::get_image_caps();
    if crate::is_gui() {
        ImageProtocol::Kitty
    } else if caps.supports_sixel {
        ImageProtocol::Sixel
    } else if caps.supports_kitty_graphics {
        ImageProtocol::Kitty
    } else {
        ImageProtocol::HalfBlock
    }
}

pub(crate) fn prepare(source: &ImageSource, placement_size: Vec2<u16>, fill: bool) {
    if placement_size.x == 0 || placement_size.y == 0 {
        return;
    }
    match pick_protocol() {
        ImageProtocol::Sixel => sixel::prepare(source, placement_size, fill),
        ImageProtocol::Kitty => kitty::prepare(source, placement_size, fill),
        ImageProtocol::HalfBlock => halfblock::prepare(source, placement_size, fill),
    }
}

pub(crate) fn dispatch(ctx: &mut RenderContext, source: &ImageSource, fill: bool) {
    let placement_size = ctx.size;
    if placement_size.x == 0 || placement_size.y == 0 {
        return;
    }
    match pick_protocol() {
        ImageProtocol::Sixel => sixel::dispatch(ctx, source, placement_size, fill),
        ImageProtocol::Kitty => kitty::dispatch(ctx, source, placement_size, fill),
        ImageProtocol::HalfBlock => halfblock::dispatch(ctx, source, placement_size, fill),
    }
}
