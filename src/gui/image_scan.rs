//! Kitty unicode-placeholder cell scanner for the GUI present pass.

use crate::prelude::*;
use crate::render::image;
use crate::render::image::kitty;

use super::gpu::{self, Backend};

pub(super) struct PlaceholderScanner {
    run: Option<Run>,
}

#[derive(Clone, Copy)]
struct Run {
    screen_x_start: u16,
    screen_y: u16,
    image_id_lo: u32,
    placement_id: u32,
    image_row_1based: u32,
    image_col_start_1based: u32,
    last_col_1based: u32,
    hi_1based: u32,
    length_cells: u16,
}

struct Parsed {
    image_id_lo: u32,
    placement_id: u32,
    row_1based: u32,
    col_1based: u32,
    hi_1based: u32,
}

impl PlaceholderScanner {
    fn flush(&mut self, backend: &mut Backend) {
        let Some(run) = self.run.take() else {
            return;
        };
        Self::emit_run(backend, run);
    }

    fn parse_placeholder(glyph: &str, fg: Color, underline: Color) -> Option<Parsed> {
        let mut chars = glyph.chars();
        if chars.next()? != kitty::PLACEHOLDER_CHAR {
            return None;
        }
        let row_1based = chars.next().and_then(kitty::diacritic_to_num).unwrap_or(0);
        let col_1based = chars.next().and_then(kitty::diacritic_to_num).unwrap_or(0);
        let hi_1based = chars.next().and_then(kitty::diacritic_to_num).unwrap_or(0);
        let image_id_lo = Self::rgb_to_id_field(fg)?;
        let placement_id = Self::rgb_to_id_field(underline).unwrap_or(0);
        Some(Parsed { image_id_lo, placement_id, row_1based, col_1based, hi_1based })
    }

    fn rgb_to_id_field(color: Color) -> Option<u32> {
        match color {
            Color::Rgb(r, g, b) => Some(((r as u32) << 16) | ((g as u32) << 8) | (b as u32)),
            _ => None,
        }
    }

    fn start_run(p: &Parsed, screen_x: u16, screen_y: u16) -> Run {
        let row = p.row_1based.max(1);
        let col = p.col_1based.max(1);
        let hi = p.hi_1based.max(1);
        Run {
            screen_x_start: screen_x,
            screen_y,
            image_id_lo: p.image_id_lo,
            placement_id: p.placement_id,
            image_row_1based: row,
            image_col_start_1based: col,
            last_col_1based: col,
            hi_1based: hi,
            length_cells: 1,
        }
    }

    fn can_continue(run: &Run, p: &Parsed, screen_x: u16) -> bool {
        let next_x = run.screen_x_start + run.length_cells;
        let next_col = run.last_col_1based + 1;
        screen_x == next_x
            && p.image_id_lo == run.image_id_lo
            && p.placement_id == run.placement_id
            && (p.row_1based == 0 || p.row_1based == run.image_row_1based)
            && (p.col_1based == 0 || p.col_1based == next_col)
            && (p.hi_1based == 0 || p.hi_1based == run.hi_1based)
    }

    fn emit_run(backend: &mut Backend, run: Run) {
        let image_id = run.image_id_lo | ((run.hi_1based - 1) << 24);
        let Some(source_inner) = kitty::lookup_source(image_id) else {
            return;
        };
        let source = image::ImageSource { inner: source_inner.clone() };
        let Some(key) = backend.ensure_image_texture(&source) else {
            return;
        };

        let placement_size = kitty::lookup_placement_size(&source_inner, run.placement_id);
        let img_dims = source_inner.get_pixel_dims();
        let cell_px = crate::get_runtime_info()
            .cell_size
            .unwrap_or(Vec2::new(1u16, 1u16));

        let uv_rect = Self::compute_uv_rect(
            img_dims,
            cell_px,
            placement_size,
            run.image_col_start_1based.saturating_sub(1),
            run.image_row_1based.saturating_sub(1),
            run.length_cells as u32,
            1,
        );

        backend.push_image_instance(gpu::ImageInstance {
            cell_xy: [run.screen_x_start, run.screen_y],
            span_wh: [run.length_cells, 1],
            uv_rect,
            source_key: key,
        });
    }

    fn compute_uv_rect(
        img: Vec2<u32>,
        cell_px: Vec2<u16>,
        placement: Option<Vec2<u16>>,
        img_col_start: u32,
        img_row_start: u32,
        cells_w: u32,
        cells_h: u32,
    ) -> [f32; 4] {
        if img.x == 0 || img.y == 0 {
            return [0.0, 0.0, 1.0, 1.0];
        }
        let cw = cell_px.x.max(1) as f64;
        let ch = cell_px.y.max(1) as f64;
        let pl = placement.unwrap_or_else(|| {
            let nx = ((img.x as f64 / cw).round().max(1.0)) as u16;
            let ny = ((img.y as f64 / ch).round().max(1.0)) as u16;
            Vec2::new(nx, ny)
        });
        let pl_w = (pl.x as f64).max(1.0);
        let pl_h = (pl.y as f64).max(1.0);

        let src_aspect = (img.x as f64) / (img.y as f64);
        let box_aspect = (pl_w * cw) / (pl_h * ch);
        let (vis_w, vis_h) = if src_aspect >= box_aspect {
            (1.0, box_aspect / src_aspect)
        } else {
            (src_aspect / box_aspect, 1.0)
        };
        let vis_x0 = (1.0 - vis_w) * 0.5;
        let vis_y0 = (1.0 - vis_h) * 0.5;

        let bx0 = (img_col_start as f64) / pl_w;
        let by0 = (img_row_start as f64) / pl_h;
        let bx1 = ((img_col_start + cells_w) as f64) / pl_w;
        let by1 = ((img_row_start + cells_h) as f64) / pl_h;

        let to_u = |bx: f64| ((bx - vis_x0) / vis_w).clamp(0.0, 1.0) as f32;
        let to_v = |by: f64| ((by - vis_y0) / vis_h).clamp(0.0, 1.0) as f32;
        [to_u(bx0), to_v(by0), to_u(bx1), to_v(by1)]
    }
}

impl PlaceholderScanner {
    /// Creates an empty [`PlaceholderScanner`].
    pub fn new() -> Self {
        Self { run: None }
    }

    /// Feeds one cell to the scanner, returning `true` if it was a placeholder cell.
    pub fn feed(
        &mut self,
        backend: &mut Backend,
        x: u16,
        y: u16,
        glyph: &str,
        fg: Color,
        underline_color: Color,
    ) -> bool {
        if let Some(run) = &self.run
            && run.screen_y != y
        {
            self.flush(backend);
        }

        let parsed = Self::parse_placeholder(glyph, fg, underline_color);
        let Some(parsed) = parsed else {
            self.flush(backend);
            return false;
        };

        match self.run {
            Some(mut run) if Self::can_continue(&run, &parsed, x) => {
                run.length_cells += 1;
                run.last_col_1based += 1;
                self.run = Some(run);
            }
            _ => {
                self.flush(backend);
                self.run = Some(Self::start_run(&parsed, x, y));
            }
        }
        true
    }

    /// Flushes any in-progress run to `backend`.
    pub fn end_frame(&mut self, backend: &mut Backend) {
        self.flush(backend);
    }
}
