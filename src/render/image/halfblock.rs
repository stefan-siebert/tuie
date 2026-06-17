//! Halfblock image protocol renderer.

use crate::prelude::*;
#[cfg(feature = "harmonious")]
use crate::theme::harmonious::resolve_rgb;
use crate::util::rgb::blend_over;
#[cfg(not(feature = "harmonious"))]
use crate::util::rgb::Rgb;

use super::cover::decode_rgba;
use super::source::{await_async_entry, prepare_async};
use super::ImageSource;

const LOWER_HALF: char = '\u{2584}';
const UPPER_HALF: char = '\u{2580}';
const ALPHA_THRESHOLD: u8 = 128;

struct HalfblockLayout {
    aspect: (u32, u32),
    dst: Vec2<u32>,
    image_cells: Vec2<u16>,
    cell_off: Vec2<u16>,
}

fn layout(placement_size: Vec2<u16>, source_px: Vec2<u32>, fill: bool) -> HalfblockLayout {
    let cell_px = crate::get_runtime_info()
        .cell_size
        .unwrap_or(Vec2::new(1, 2));
    let image_cells = if fill {
        placement_size
    } else {
        letterbox_cells(placement_size, source_px, cell_px)
    };
    let cell_off = Vec2::new(
        (placement_size.x - image_cells.x) / 2,
        (placement_size.y - image_cells.y) / 2,
    );
    let aspect_w = image_cells.x as u32 * cell_px.x as u32;
    let aspect_h = image_cells.y as u32 * cell_px.y as u32;
    let dst = Vec2::new(image_cells.x as u32, image_cells.y as u32 * 2);
    HalfblockLayout { aspect: (aspect_w, aspect_h), dst, image_cells, cell_off }
}

fn letterbox_cells(placement: Vec2<u16>, source_px: Vec2<u32>, cell_px: Vec2<u16>) -> Vec2<u16> {
    if source_px.x == 0 || source_px.y == 0 || cell_px.x == 0 || cell_px.y == 0 {
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
    let lay = layout(placement_size, source.inner.get_pixel_dims(), fill);
    if lay.image_cells.x == 0 || lay.image_cells.y == 0 {
        return;
    }
    let (aspect_w, aspect_h) = lay.aspect;
    let dst = lay.dst;
    source.inner.with_halfblock_cache(|cache| {
        prepare_async(cache, dst, &source.inner.data, move |data| {
            decode_rgba(data, aspect_w, aspect_h, dst.x, dst.y)
        });
    });
}

pub(crate) fn dispatch(
    ctx: &mut RenderContext,
    source: &ImageSource,
    placement_size: Vec2<u16>,
    fill: bool,
) {
    let lay = layout(placement_size, source.inner.get_pixel_dims(), fill);
    if lay.image_cells.x == 0 || lay.image_cells.y == 0 {
        return;
    }
    let (aspect_w, aspect_h) = lay.aspect;
    let dst = lay.dst;

    let Some(pixels) = source.inner.with_halfblock_cache(|cache| {
        await_async_entry(cache, dst, || {
            decode_rgba(&source.inner.data, aspect_w, aspect_h, dst.x, dst.y)
        })
    }) else {
        return;
    };

    #[cfg(feature = "harmonious")]
    let term_bg = resolve_rgb(Color::Background);
    #[cfg(not(feature = "harmonious"))]
    let term_bg: Option<Rgb> = None;
    let row_stride = lay.image_cells.x as usize * 4;
    let cols = lay.image_cells.x as usize;
    let col_off = lay.cell_off.x as usize;
    let row_off = lay.cell_off.y as i32;
    for row in 0..lay.image_cells.y as usize {
        ctx.move_to(Vec2::new(0, row as i32 + row_off));
        let mut writer = ctx.row_writer();
        let range = writer.get_range();
        let row_start = col_off;
        let row_end = (col_off + cols).min(range.end);
        let start = range.start.max(row_start);
        let top_row_start = row * 2 * row_stride;
        let bot_row_start = top_row_start + row_stride;

        for cell_col in start..row_end {
            let pixel_col = cell_col - col_off;
            let top_off = top_row_start + pixel_col * 4;
            let bot_off = bot_row_start + pixel_col * 4;
            let top = &pixels[top_off..top_off + 4];
            let bot = &pixels[bot_off..bot_off + 4];
            let (glyph, fg, bg) = match term_bg {
                Some(bg) => {
                    let t = blend_over(top, bg);
                    let b = blend_over(bot, bg);
                    (LOWER_HALF, Color::Rgb(b.r, b.g, b.b), Color::Rgb(t.r, t.g, t.b))
                }
                None => match (top[3] >= ALPHA_THRESHOLD, bot[3] >= ALPHA_THRESHOLD) {
                    (true, true) => (
                        LOWER_HALF,
                        Color::Rgb(bot[0], bot[1], bot[2]),
                        Color::Rgb(top[0], top[1], top[2]),
                    ),
                    (true, false) => (
                        UPPER_HALF,
                        Color::Rgb(top[0], top[1], top[2]),
                        Color::Background,
                    ),
                    (false, true) => (
                        LOWER_HALF,
                        Color::Rgb(bot[0], bot[1], bot[2]),
                        Color::Background,
                    ),
                    (false, false) => (' ', Color::Foreground, Color::Background),
                },
            };
            writer
                .cell(cell_col)
                .glyph(glyph)
                .style(&Style::new().fg(fg).bg(bg));
        }
    }
}
