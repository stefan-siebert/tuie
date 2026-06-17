//! wgpu backend for the GUI.

use crate::prelude::{UnderlineType, Vec2};

use super::font::FontCache;

mod atlas;
mod context;
mod decoration;
#[cfg(feature = "images")]
mod image;
mod pipeline;

pub(crate) use context::{build_instance_and_adapter, create_window_and_backend};
pub(crate) use decoration::DecoStyle;
#[cfg(feature = "images")]
pub(crate) use image::ImageInstance;

use atlas::{AtlasEntry, GlyphAtlas};
use pipeline::{bytes_of, slice_bytes, BgPipeline, BgUniforms, GlyphPipeline, GlyphUniforms};

#[derive(Clone, Copy)]
struct BgRun {
    y: u16,
    start_x: u16,
    end_x: u16,
    rgba: [u8; 4],
    merged: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct CellRender {
    pub fg_rgba: [u8; 4],
    pub bg_rgba: [u8; 4],
    pub body_clear_rgba: [u8; 4],
    pub underline_rgba: [u8; 4],
    pub wide: bool,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: UnderlineType,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BgInstance {
    rect_px: [i16; 4],
    bg_rgba: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GlyphInstance {
    cell_xy: [u16; 2],
    glyph_off_px: [i16; 2],
    glyph_size_px: [u16; 2],
    atlas_uv_px: [u16; 2],
    fg_rgba: [u8; 4],
}

struct FrameTarget {
    surface_texture: wgpu::SurfaceTexture,
    view: wgpu::TextureView,
}

pub(crate) struct Backend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,

    bg_pipeline: BgPipeline,
    glyph_pipeline: GlyphPipeline,
    atlas: GlyphAtlas,
    glyph_bind_group: wgpu::BindGroup,
    glyph_atlas_generation: u32,

    bg_buffer: wgpu::Buffer,
    bg_capacity: usize,
    glyph_buffer: wgpu::Buffer,
    glyph_capacity: usize,

    bg_instances: Vec<BgInstance>,
    bg_runs: Vec<BgRun>,
    glyph_instances: Vec<GlyphInstance>,
    pending_bg: Option<BgRun>,
    last_glyph_key: Option<(char, bool, bool)>,
    last_glyph_entry: Option<AtlasEntry>,

    #[cfg(feature = "images")]
    image_pipeline: image::ImagePipeline,

    pixel_size: Vec2<u32>,
    grid_origin: Vec2<u32>,
    ncols: u16,
    nrows: u16,
    extend_sides: bool,
    extend_header: bool,
    extend_footer: bool,

    body_clear_linear: wgpu::Color,
    frame: Option<FrameTarget>,

    #[cfg(target_os = "macos")]
    metal_layer: Option<context::MetalLayer>,
    #[cfg(target_os = "macos")]
    last_layer_bg: u32,
}

impl Backend {
    pub(crate) fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        pixel_size: Vec2<u32>,
        grid_origin: Vec2<u32>,
        #[cfg(target_os = "macos")] metal_layer: Option<context::MetalLayer>,
    ) -> std::io::Result<Self> {
        let surface_format = config.format;
        let bg_pipeline = BgPipeline::new(&device, surface_format);
        let glyph_pipeline = GlyphPipeline::new(&device, surface_format);
        let atlas = GlyphAtlas::new(&device, 512, 512)?;
        let glyph_bind_group = glyph_pipeline.make_bind_group(&device, atlas.get_view());
        let glyph_atlas_generation = atlas.get_generation();

        #[cfg(feature = "images")]
        let image_pipeline = image::ImagePipeline::new(&device, surface_format);

        let bg_capacity = 64usize;
        let bg_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bg instance buffer"),
            size: (bg_capacity * std::mem::size_of::<BgInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_capacity = 64usize;
        let glyph_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph instance buffer"),
            size: (glyph_capacity * std::mem::size_of::<GlyphInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            surface,
            config,
            bg_pipeline,
            glyph_pipeline,
            atlas,
            glyph_bind_group,
            glyph_atlas_generation,
            bg_buffer,
            bg_capacity,
            glyph_buffer,
            glyph_capacity,
            bg_instances: Vec::new(),
            bg_runs: Vec::new(),
            glyph_instances: Vec::new(),
            pending_bg: None,
            last_glyph_key: None,
            last_glyph_entry: None,
            #[cfg(feature = "images")]
            image_pipeline,
            pixel_size,
            grid_origin,
            ncols: 0,
            nrows: 0,
            extend_sides: true,
            extend_header: true,
            extend_footer: true,
            body_clear_linear: wgpu::Color::BLACK,
            frame: None,
            #[cfg(target_os = "macos")]
            metal_layer,
            #[cfg(target_os = "macos")]
            last_layer_bg: u32::MAX,
        })
    }

    pub(crate) fn clear_glyph_atlas(&mut self) {
        self.atlas.clear();
        self.last_glyph_key = None;
        self.last_glyph_entry = None;
    }

    pub(crate) fn resize(&mut self, pixel_size: Vec2<u32>, grid_origin: Vec2<u32>) {
        self.pixel_size = pixel_size;
        self.grid_origin = grid_origin;
        if pixel_size.x == 0 || pixel_size.y == 0 {
            return;
        }
        self.config.width = pixel_size.x;
        self.config.height = pixel_size.y;
        self.surface.configure(&self.device, &self.config);
    }

    pub(crate) fn begin_frame(
        &mut self,
        body_clear: u32,
        cells: Vec2<u16>,
        extend_sides: bool,
        extend_header: bool,
        extend_footer: bool,
    ) {
        self.reset_pass_state(cells, extend_sides, extend_header, extend_footer);
        let [r, g, b, _] = u32_to_rgba(body_clear);
        self.body_clear_linear = wgpu::Color {
            r: srgb_to_linear_byte(r),
            g: srgb_to_linear_byte(g),
            b: srgb_to_linear_byte(b),
            a: 1.0,
        };
        #[cfg(target_os = "macos")]
        if body_clear != self.last_layer_bg {
            self.last_layer_bg = body_clear;
            if let Some(layer) = &self.metal_layer {
                layer.set_background_rgb(r, g, b);
            }
        }
    }

    fn reset_pass_state(
        &mut self,
        cells: Vec2<u16>,
        extend_sides: bool,
        extend_header: bool,
        extend_footer: bool,
    ) {
        self.bg_instances.clear();
        self.bg_runs.clear();
        self.glyph_instances.clear();
        self.pending_bg = None;
        self.last_glyph_key = None;
        self.last_glyph_entry = None;
        self.ncols = cells.x;
        self.nrows = cells.y;
        self.extend_sides = extend_sides;
        self.extend_header = extend_header;
        self.extend_footer = extend_footer;
        #[cfg(feature = "images")]
        self.image_pipeline.begin_frame();
    }

    pub(crate) fn push_cell(
        &mut self,
        cell_x: u16,
        cell_y: u16,
        glyph: &str,
        cell: &CellRender,
        font: &mut FontCache,
    ) {
        let span_w: u16 = if cell.wide {
            2
        } else {
            1
        };
        if cell.bg_rgba != cell.body_clear_rgba {
            self.add_bg(cell_x, cell_y, span_w, cell.bg_rgba);
        } else {
            self.flush_bg();
        }
        self.push_glyph_chars(cell_x, cell_y, glyph, cell, font);
        if cell.underline != UnderlineType::None {
            let style = DecoStyle::Underline(cell.underline);
            for dx in 0..span_w {
                self.push_decoration(cell_x + dx, cell_y, style, cell.underline_rgba, font);
            }
        }
        if cell.strikethrough {
            for dx in 0..span_w {
                self.push_decoration(cell_x + dx, cell_y, DecoStyle::Strikethrough, cell.fg_rgba, font);
            }
        }
    }

    fn push_glyph_chars(
        &mut self,
        cell_x: u16,
        cell_y: u16,
        glyph: &str,
        cell: &CellRender,
        font: &mut FontCache,
    ) {
        if glyph.len() == 1 {
            let b = glyph.as_bytes()[0];
            if b != b' ' && b != 0 {
                self.push_glyph(cell_x, cell_y, b as char, cell.fg_rgba, cell.bold, cell.italic, font);
            }
            return;
        }
        let mut chars = glyph.chars();
        let Some(c1) = chars.next() else {
            return;
        };
        match chars.next() {
            Some(c2) => {
                self.push_glyph(cell_x, cell_y, c1, cell.fg_rgba, cell.bold, cell.italic, font);
                self.push_glyph(cell_x + 1, cell_y, c2, cell.fg_rgba, cell.bold, cell.italic, font);
            }
            None if c1 != ' ' && c1 != '\0' => {
                self.push_glyph(cell_x, cell_y, c1, cell.fg_rgba, cell.bold, cell.italic, font);
            }
            None => {}
        }
    }

    fn add_bg(&mut self, cell_x: u16, cell_y: u16, span: u16, rgba: [u8; 4]) {
        if let Some(run) = self.pending_bg.as_mut()
            && run.y == cell_y
            && run.end_x == cell_x
            && run.rgba == rgba
        {
            run.end_x = run.end_x.saturating_add(span);
            run.merged = true;
            return;
        }
        self.flush_bg();
        self.pending_bg = Some(BgRun {
            y: cell_y,
            start_x: cell_x,
            end_x: cell_x.saturating_add(span),
            rgba,
            merged: false,
        });
    }

    fn maybe_push_top_fill(&mut self, pad_y: i32) {
        if !self.extend_header || pad_y <= 0 {
            return;
        }
        let Some(rgba) = self.row_full_bleed_bg(0) else {
            return;
        };
        self.push_pad_rect(0, 0, self.pixel_size.x as i32, pad_y, rgba);
    }

    fn maybe_push_bottom_fill(&mut self, pad_y: i32, cell_h: i32, bottom_pad_h: u32) {
        if !self.extend_footer || bottom_pad_h == 0 || self.nrows == 0 {
            return;
        }
        let Some(rgba) = self.row_full_bleed_bg(self.nrows - 1) else {
            return;
        };
        let grid_bottom = pad_y + self.nrows as i32 * cell_h;
        self.push_pad_rect(0, grid_bottom, self.pixel_size.x as i32, bottom_pad_h as i32, rgba);
    }

    fn row_full_bleed_bg(&self, row: u16) -> Option<[u8; 4]> {
        let mut left: Option<[u8; 4]> = None;
        let mut right: Option<[u8; 4]> = None;
        for r in &self.bg_runs {
            if r.y != row || !r.merged {
                continue;
            }
            if r.start_x == 0 {
                left = Some(r.rgba);
            }
            if r.end_x == self.ncols {
                right = Some(r.rgba);
            }
        }
        match (left, right) {
            (Some(l), Some(r)) if l == r => Some(l),
            _ => None,
        }
    }

    fn flush_bg(&mut self) {
        if let Some(run) = self.pending_bg.take() {
            self.bg_runs.push(run);
        }
    }

    fn push_pad_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgba: [u8; 4]) {
        self.bg_instances.push(BgInstance {
            rect_px: [
                x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                w.clamp(0, i16::MAX as i32) as i16,
                h.clamp(0, i16::MAX as i32) as i16,
            ],
            bg_rgba: rgba,
        });
    }

    fn finalize_bg(&mut self, cell_w: i32, cell_h: i32, pad_x: i32, pad_y: i32) {
        let viewport_w = self.pixel_size.x as i32;
        let right_pad_w = viewport_w - pad_x - self.ncols as i32 * cell_w;
        let runs = std::mem::take(&mut self.bg_runs);
        for run in runs {
            let span = (run.end_x - run.start_x) as i32;
            let x = pad_x + run.start_x as i32 * cell_w;
            let y = pad_y + run.y as i32 * cell_h;
            let w = span * cell_w;
            self.push_pad_rect(x, y, w, cell_h, run.rgba);
            if self.extend_sides && run.merged {
                if run.start_x == 0 && pad_x > 0 {
                    self.push_pad_rect(0, y, pad_x, cell_h, run.rgba);
                }
                if run.end_x == self.ncols && right_pad_w > 0 {
                    self.push_pad_rect(pad_x + self.ncols as i32 * cell_w, y, right_pad_w, cell_h, run.rgba);
                }
            }
        }
    }

    pub(crate) fn push_decoration(
        &mut self,
        cell_x: u16,
        cell_y: u16,
        style: DecoStyle,
        rgba: [u8; 4],
        font: &mut FontCache,
    ) {
        let entry = match self
            .atlas
            .decoration_entry(&self.device, &self.queue, style, font)
        {
            Some(e) => e,
            None => return,
        };
        if entry.size_px[0] == 0 || entry.size_px[1] == 0 {
            return;
        }
        self.glyph_instances.push(GlyphInstance {
            cell_xy: [cell_x, cell_y],
            glyph_off_px: entry.off_px,
            glyph_size_px: entry.size_px,
            atlas_uv_px: entry.atlas_uv,
            fg_rgba: rgba,
        });
    }

    #[cfg(feature = "images")]
    pub(crate) fn ensure_image_texture(
        &mut self,
        source: &crate::render::image::ImageSource,
    ) -> Option<*const ()> {
        self.image_pipeline
            .ensure_texture(&self.device, &self.queue, source)
    }

    #[cfg(feature = "images")]
    pub(crate) fn push_image_instance(&mut self, instance: ImageInstance) {
        self.image_pipeline.push(instance);
    }

    fn push_glyph(
        &mut self,
        cell_x: u16,
        cell_y: u16,
        c: char,
        fg_rgba: [u8; 4],
        bold: bool,
        italic: bool,
        font: &mut FontCache,
    ) {
        let key = (c, bold, italic);
        let entry = if self.last_glyph_key == Some(key) {
            match self.last_glyph_entry {
                Some(e) => e,
                None => return,
            }
        } else {
            let resolved = self
                .atlas
                .entry(&self.device, &self.queue, c, bold, italic, font);
            self.last_glyph_key = Some(key);
            self.last_glyph_entry = resolved;
            match resolved {
                Some(e) => e,
                None => return,
            }
        };
        if entry.size_px[0] == 0 || entry.size_px[1] == 0 {
            return;
        }
        self.glyph_instances.push(GlyphInstance {
            cell_xy: [cell_x, cell_y],
            glyph_off_px: entry.off_px,
            glyph_size_px: entry.size_px,
            atlas_uv_px: entry.atlas_uv,
            fg_rgba,
        });
    }

    pub(crate) fn begin_offset_pass(&mut self, cells: Vec2<u16>) {
        self.reset_pass_state(cells, false, false, false);
    }

    pub(crate) fn set_pass_extend(&mut self, sides: bool, header: bool, footer: bool) {
        self.extend_sides = sides;
        self.extend_header = header;
        self.extend_footer = footer;
    }

    pub(crate) fn flush_passes(
        &mut self,
        font_cell_px: Vec2<u32>,
        pad_px: Vec2<i32>,
        include_pad_strips: bool,
        scissor: Option<(Vec2<i32>, Vec2<u32>)>,
        bg_alpha: f32,
    ) {
        self.flush_bg();
        let cell_w = font_cell_px.x as i32;
        let cell_h = font_cell_px.y as i32;
        if include_pad_strips {
            let bottom_pad_h = self
                .pixel_size
                .y
                .saturating_sub(self.grid_origin.y)
                .saturating_sub(self.nrows as u32 * font_cell_px.y);
            self.maybe_push_top_fill(pad_px.y);
            self.maybe_push_bottom_fill(pad_px.y, cell_h, bottom_pad_h);
        }
        self.finalize_bg(cell_w, cell_h, pad_px.x, pad_px.y);
        let cell = (font_cell_px.x as f32, font_cell_px.y as f32);
        let viewport = (
            self.pixel_size.x.max(1) as f32,
            self.pixel_size.y.max(1) as f32,
        );
        let pad = (pad_px.x as f32, pad_px.y as f32);
        let atlas_dims = self.atlas.get_size();
        let atlas_px = (atlas_dims.0 as f32, atlas_dims.1 as f32);

        let bg_u = BgUniforms {
            viewport_px: [viewport.0, viewport.1],
            bg_alpha,
            _pad: 0.0,
        };
        self.queue
            .write_buffer(&self.bg_pipeline.uniform_buffer, 0, bytes_of(&bg_u));

        let glyph_u = GlyphUniforms {
            viewport_px: [viewport.0, viewport.1],
            cell_px: [cell.0, cell.1],
            pad_px: [pad.0, pad.1],
            atlas_px: [atlas_px.0, atlas_px.1],
        };
        self.queue
            .write_buffer(&self.glyph_pipeline.uniform_buffer, 0, bytes_of(&glyph_u));

        self.upload_bg();
        self.upload_glyph();

        #[cfg(feature = "images")]
        self.image_pipeline
            .prepare(&self.device, &self.queue, viewport, cell, pad);

        self.refresh_glyph_bind_group();

        let load = if self.frame.is_none() {
            match self.acquire_frame() {
                Some(f) => self.frame = Some(f),
                None => return,
            }
            wgpu::LoadOp::Clear(self.body_clear_linear)
        } else {
            wgpu::LoadOp::Load
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tuie pass"),
            });
        let frame = self.frame.as_ref().unwrap();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("tuie pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &frame.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        if let Some((origin, size)) = scissor {
            let x = origin.x.clamp(0, self.pixel_size.x as i32) as u32;
            let y = origin.y.clamp(0, self.pixel_size.y as i32) as u32;
            let max_w = self.pixel_size.x.saturating_sub(x);
            let max_h = self.pixel_size.y.saturating_sub(y);
            let w = size.x.min(max_w);
            let h = size.y.min(max_h);
            pass.set_scissor_rect(x, y, w, h);
        }

        if !self.bg_instances.is_empty() {
            pass.set_pipeline(&self.bg_pipeline.pipeline);
            pass.set_bind_group(0, &self.bg_pipeline.bind_group, &[]);
            pass.set_vertex_buffer(0, self.bg_buffer.slice(..));
            pass.draw(0..4, 0..self.bg_instances.len() as u32);
        }

        #[cfg(feature = "images")]
        self.image_pipeline.record_draws(&mut pass);

        if !self.glyph_instances.is_empty() {
            pass.set_pipeline(&self.glyph_pipeline.pipeline);
            pass.set_bind_group(0, &self.glyph_bind_group, &[]);
            pass.set_vertex_buffer(0, self.glyph_buffer.slice(..));
            pass.draw(0..4, 0..self.glyph_instances.len() as u32);
        }
        drop(pass);
        self.queue.submit(std::iter::once(encoder.finish()));
    }

    pub(crate) fn end_frame(&mut self) {
        #[cfg(feature = "images")]
        self.image_pipeline.evict_dead();
        if let Some(frame) = self.frame.take() {
            frame.surface_texture.present();
        }
    }

    fn acquire_frame(&mut self) -> Option<FrameTarget> {
        let surface_texture = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.config);
                self.surface.get_current_texture().ok()?
            }
            Err(_) => return None,
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Some(FrameTarget {
            surface_texture,
            view,
        })
    }

    fn refresh_glyph_bind_group(&mut self) {
        let current = self.atlas.get_generation();
        if current != self.glyph_atlas_generation {
            self.glyph_bind_group = self
                .glyph_pipeline
                .make_bind_group(&self.device, self.atlas.get_view());
            self.glyph_atlas_generation = current;
        }
    }

    fn upload_bg(&mut self) {
        if self.bg_instances.is_empty() {
            return;
        }
        let n = self.bg_instances.len();
        if n > self.bg_capacity {
            self.bg_capacity = n.next_power_of_two().max(64);
            self.bg_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bg instance buffer"),
                size: (self.bg_capacity * std::mem::size_of::<BgInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue
            .write_buffer(&self.bg_buffer, 0, slice_bytes(&self.bg_instances));
    }

    fn upload_glyph(&mut self) {
        if self.glyph_instances.is_empty() {
            return;
        }
        let n = self.glyph_instances.len();
        if n > self.glyph_capacity {
            self.glyph_capacity = n.next_power_of_two().max(64);
            self.glyph_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("glyph instance buffer"),
                size: (self.glyph_capacity * std::mem::size_of::<GlyphInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue.write_buffer(
            &self.glyph_buffer,
            0,
            slice_bytes(&self.glyph_instances),
        );
    }
}

pub(crate) fn u32_to_rgba(c: u32) -> [u8; 4] {
    [
        ((c >> 16) & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
        (c & 0xFF) as u8,
        ((c >> 24) & 0xFF) as u8,
    ]
}

fn srgb_to_linear_byte(c: u8) -> f64 {
    let cs = c as f64 / 255.0;
    if cs <= 0.04045 {
        cs / 12.92
    } else {
        ((cs + 0.055) / 1.055).powf(2.4)
    }
}
