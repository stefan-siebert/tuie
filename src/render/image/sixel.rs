//! Sixel image protocol renderer.

use std::cell::RefCell;

use crate::prelude::*;

use super::cover::decode_rgba;
use super::sixel_encode::{build_palette_bytes, emit_sixel_dcs, quantize_rgba_dither};
use super::source::{await_async_entry, prepare_async, SixelQuantized, SourceData};
use super::ImageSource;

thread_local! {
    static PAYLOAD_SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

struct SixelLayout {
    px_dims: Vec2<u32>,
    cell_px: Vec2<u16>,
    image_cells: Vec2<u16>,
    cell_off: Vec2<u16>,
}

fn quantize_pipeline(data: &SourceData, px_w: u32, px_h: u32) -> Option<SixelQuantized> {
    let rgba = decode_rgba(data, px_w, px_h, px_w, px_h)?;
    let (indexed, used) = quantize_rgba_dither(&rgba, px_w, px_h);
    let palette_bytes = build_palette_bytes(&used);
    Some(SixelQuantized { indexed, palette_bytes })
}

fn layout(
    placement_size: Vec2<u16>,
    source_px: Vec2<u32>,
    fill: bool,
) -> Option<SixelLayout> {
    let cell_px = crate::get_runtime_info().cell_size?;
    if placement_size.x == 0 || placement_size.y == 0 || cell_px.x == 0 || cell_px.y == 0 {
        return None;
    }
    let image_cells = if fill {
        placement_size
    } else {
        letterbox_cells(placement_size, source_px, cell_px)
    };
    if image_cells.x == 0 || image_cells.y == 0 {
        return None;
    }
    let cell_off = Vec2::new(
        (placement_size.x - image_cells.x) / 2,
        (placement_size.y - image_cells.y) / 2,
    );
    let px_w = image_cells.x as u32 * cell_px.x as u32;
    let px_h = image_cells.y as u32 * cell_px.y as u32;
    if px_w == 0 || px_h == 0 {
        return None;
    }
    Some(SixelLayout { px_dims: Vec2::new(px_w, px_h), cell_px, image_cells, cell_off })
}

fn letterbox_cells(placement: Vec2<u16>, source_px: Vec2<u32>, cell_px: Vec2<u16>) -> Vec2<u16> {
    if source_px.x == 0 || source_px.y == 0 {
        return placement;
    }
    let widget_px_w = placement.x as u64 * cell_px.x as u64;
    let widget_px_h = placement.y as u64 * cell_px.y as u64;
    let src_w = source_px.x as u64;
    let src_h = source_px.y as u64;
    let by_w = widget_px_w * src_h;
    let by_h = widget_px_h * src_w;
    let (img_px_w, img_px_h) = if by_w <= by_h {
        (widget_px_w, by_w / src_w)
    } else {
        (by_h / src_h, widget_px_h)
    };
    let cx = cell_px.x as u64;
    let cy = cell_px.y as u64;
    let img_cells_x = ((img_px_w + cx / 2) / cx).max(1).min(placement.x as u64) as u16;
    let img_cells_y = ((img_px_h + cy / 2) / cy).max(1).min(placement.y as u64) as u16;
    Vec2::new(img_cells_x, img_cells_y)
}

pub(crate) fn prepare(source: &ImageSource, placement_size: Vec2<u16>, fill: bool) {
    let Some(lay) = layout(placement_size, source.inner.get_pixel_dims(), fill) else {
        return;
    };
    let px_dims = lay.px_dims;
    let px_w = px_dims.x;
    let px_h = px_dims.y;

    source.inner.with_sixel_cache(|cache| {
        prepare_async(cache, px_dims, &source.inner.data, move |data| {
            quantize_pipeline(data, px_w, px_h)
        });
    });
}

pub(crate) fn dispatch(
    ctx: &mut RenderContext,
    source: &ImageSource,
    placement_size: Vec2<u16>,
    fill: bool,
) {
    let Some(lay) = layout(placement_size, source.inner.get_pixel_dims(), fill) else {
        return;
    };
    let px_dims = lay.px_dims;
    let cell_px = lay.cell_px;
    let px_w = px_dims.x;
    let px_h = px_dims.y;

    let img_tl = ctx.anchor + Vec2::new(lay.cell_off.x as i32, lay.cell_off.y as i32);
    let img_br = img_tl + Vec2::new(lay.image_cells.x as i32, lay.image_cells.y as i32);

    let clip_tl = Vec2::new(ctx.pos.x as i32, ctx.pos.y as i32);
    let clip_br = clip_tl + Vec2::new(ctx.physical_size.x as i32, ctx.physical_size.y as i32);

    let vis_tl = Vec2::new(img_tl.x.max(clip_tl.x), img_tl.y.max(clip_tl.y));
    let mut vis_br = Vec2::new(img_br.x.min(clip_br.x), img_br.y.min(clip_br.y));
    if vis_br.x <= vis_tl.x || vis_br.y <= vis_tl.y {
        return;
    }

    let max_br_y = (ctx.get_screen_size().y as i32 - 1).max(0);
    vis_br.y = vis_br.y.min(max_br_y);
    if vis_br.y <= vis_tl.y {
        return;
    }

    let img_off_cells_x = (vis_tl.x - img_tl.x) as u32;
    let img_off_cells_y = (vis_tl.y - img_tl.y) as u32;
    let vis_w = (vis_br.x - vis_tl.x) as u32;
    let vis_h = (vis_br.y - vis_tl.y) as u32;

    let Some(quant) = source.inner.with_sixel_cache(|cache| {
        await_async_entry(cache, px_dims, || {
            quantize_pipeline(&source.inner.data, px_w, px_h)
        })
    }) else {
        return;
    };

    let scale_x = cell_px.x as f64;
    let scale_y = cell_px.y as f64;
    let crop_x = ((img_off_cells_x as f64 * scale_x).round() as u32).min(px_w);
    let crop_y = ((img_off_cells_y as f64 * scale_y).round() as u32).min(px_h);
    let crop_w = ((vis_w as f64 * scale_x).round() as u32).min(px_w - crop_x);
    let crop_h = ((vis_h as f64 * scale_y).round() as u32).min(px_h - crop_y);

    if crop_w == 0 || crop_h == 0 {
        return;
    }

    PAYLOAD_SCRATCH.with_borrow_mut(|payload| {
        payload.clear();
        payload.extend_from_slice(b"\x1b[0m");
        for row in 0..vis_h {
            let y = vis_tl.y + row as i32;
            payload.extend_from_slice(
                format!("\x1b[{};{}H\x1b[{}X", y + 1, vis_tl.x + 1, vis_w).as_bytes(),
            );
        }
        payload.extend_from_slice(
            format!("\x1b[{};{}H", vis_tl.y + 1, vis_tl.x + 1).as_bytes(),
        );
        emit_sixel_dcs(
            payload,
            &quant.indexed,
            px_w,
            crop_x,
            crop_y,
            crop_w,
            crop_h,
            &quant.palette_bytes,
        );
        ctx.queue_raw(payload);
    });

    let image_cell_rows = crop_h.div_ceil(cell_px.y as u32).min(vis_h) as u16;
    let widget_vis_tl_x = (vis_tl.x - ctx.anchor.x).max(0);
    let widget_vis_tl_y = (vis_tl.y - ctx.anchor.y).max(0);
    ctx.invalidate(
        Vec2::new(widget_vis_tl_x, widget_vis_tl_y),
        Vec2::new(vis_w as u16, image_cell_rows),
    );
}
