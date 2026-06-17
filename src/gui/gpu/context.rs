//! wgpu Instance, Adapter, Device, Queue, and Surface creation tied to a winit Window.

use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::OnceLock;

use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

use crate::prelude::Vec2;

use super::Backend;

pub(crate) fn create_window_and_backend(
    event_loop: &ActiveEventLoop,
    window_attrs: WindowAttributes,
    grid_origin: Vec2<u32>,
    prebuilt: Option<(wgpu::Instance, wgpu::Adapter)>,
) -> std::io::Result<(Arc<Window>, Backend)> {
    let (instance, adapter) = match prebuilt {
        Some(p) => p,
        None => build_instance_and_adapter()?,
    };

    let window = event_loop
        .create_window(window_attrs)
        .map_err(|e| io_err(format!("create_window: {e}")))?;
    let window = Arc::new(window);

    let surface = instance
        .create_surface(window.clone())
        .map_err(|e| io_err(format!("create_surface: {e}")))?;

    #[cfg(target_os = "macos")]
    let metal_layer = configure_metal_layer(&window);

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("tuie device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .map_err(|e| io_err(format!("request_device: {e}")))?;

    let caps = surface.get_capabilities(&adapter);
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| matches!(f, wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Rgba8UnormSrgb))
        .or_else(|| caps.formats.first().copied())
        .ok_or_else(|| io_err("no surface format"))?;
    let alpha_mode = caps
        .alpha_modes
        .iter()
        .copied()
        .find(|m| *m == wgpu::CompositeAlphaMode::Opaque)
        .unwrap_or(caps.alpha_modes[0]);

    let inner = window.inner_size();
    let pixel_size = Vec2::new(inner.width.max(1), inner.height.max(1));

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: pixel_size.x,
        height: pixel_size.y,
        present_mode: wgpu::PresentMode::AutoVsync,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: vec![],
    };
    surface.configure(&device, &config);

    let backend = Backend::new(
        device,
        queue,
        surface,
        config,
        pixel_size,
        grid_origin,
        #[cfg(target_os = "macos")]
        metal_layer,
    )?;
    Ok((window, backend))
}

pub(crate) fn build_instance_and_adapter() -> std::io::Result<(wgpu::Instance, wgpu::Adapter)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok_or_else(|| io_err("no GPU adapter"))?;
    Ok((instance, adapter))
}

fn io_err<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[cfg(target_os = "macos")]
pub(crate) struct MetalLayer(*mut objc2::runtime::AnyObject);

#[cfg(target_os = "macos")]
#[repr(C)]
struct CGColor {
    _private: [u8; 0],
}

#[cfg(target_os = "macos")]
unsafe impl objc2::RefEncode for CGColor {
    const ENCODING_REF: objc2::Encoding =
        objc2::Encoding::Pointer(&objc2::Encoding::Struct("CGColor", &[]));
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: *const std::ffi::c_void;
    fn CGColorSpaceCreateWithName(name: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CGColorCreate(space: *mut std::ffi::c_void, components: *const f64) -> *mut CGColor;
    fn CGColorRelease(c: *mut CGColor);
}

#[cfg(target_os = "macos")]
fn srgb_color_space() -> *mut std::ffi::c_void {
    static SPACE: OnceLock<usize> = OnceLock::new();
    *SPACE.get_or_init(|| unsafe {
        CGColorSpaceCreateWithName(kCGColorSpaceSRGB) as usize
    }) as *mut std::ffi::c_void
}

#[cfg(target_os = "macos")]
impl MetalLayer {
    pub(crate) fn set_background_rgb(&self, r: u8, g: u8, b: u8) {
        use objc2::msg_send;
        let components: [f64; 4] = [
            r as f64 / 255.0,
            g as f64 / 255.0,
            b as f64 / 255.0,
            1.0,
        ];
        unsafe {
            let color = CGColorCreate(srgb_color_space(), components.as_ptr());
            let _: () = msg_send![self.0, setBackgroundColor: color];
            CGColorRelease(color);
        }
    }
}

#[cfg(target_os = "macos")]
fn configure_metal_layer(window: &Window) -> Option<MetalLayer> {
    use objc2::{class, msg_send, runtime::{AnyObject, Bool}};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[link(name = "QuartzCore", kind = "framework")]
    unsafe extern "C" {
        static kCAGravityBottomLeft: *const AnyObject;
    }

    let RawWindowHandle::AppKit(h) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    let view = h.ns_view.as_ptr() as *mut AnyObject;
    let metal_class = class!(CAMetalLayer);
    unsafe {
        let root: *mut AnyObject = msg_send![view, layer];
        let subs: *mut AnyObject = msg_send![root, sublayers];
        let count: usize = msg_send![subs, count];
        for i in 0..count {
            let sub: *mut AnyObject = msg_send![subs, objectAtIndex: i];
            let is_metal: Bool = msg_send![sub, isKindOfClass: metal_class];
            if is_metal.as_bool() {
                let _: () = msg_send![sub, setContentsGravity: kCAGravityBottomLeft];
                return Some(MetalLayer(sub));
            }
        }
    }
    None
}
