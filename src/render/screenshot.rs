//! The last painted frame as pixels, for an inspector.
//!
//! A terminal has no framebuffer to read back, so this paints the frame the
//! renderer last flushed itself: every cell's background, its glyph in the
//! foreground colour from the system's monospace font (with fallbacks for
//! symbols the font lacks, Nerd Font glyphs above all), underline and
//! strikethrough, and the images kitty placeholders stand for, from the
//! pixels the image system holds. Half-block images need nothing special:
//! they are cells.
//!
//! What it cannot show is what the terminal adds on its own: its font, its
//! palette for the 16 ANSI colours, its own cursor. Sixel images are not in
//! the cell grid and come out as the cells under them.

use std::cell::RefCell;
use std::rc::Rc;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use image::{Rgba, RgbaImage};

use super::GridCellStyle;
use super::color::Color;
use super::image::kitty;
use super::style::StyleAttribute;
use super::underline::UnderlineType;
use crate::prelude::Vec2;

/// Size of the painted text in pixels. Cells come out about 10 × 20.
const FONT_PX: f32 = 18.0;

/// Primary monospace families, tried in order before the generic one.
const MONOSPACE: &[&str] = &[
    "DejaVu Sans Mono",
    "Noto Sans Mono",
    "Liberation Mono",
    "Adwaita Mono",
    "Menlo",
    "SF Mono",
    "Consolas",
    "Cascadia Mono",
];

/// Families tried for a glyph the primary font lacks.
const FALLBACKS: &[&str] = &[
    "Symbols Nerd Font Mono",
    "Symbols Nerd Font",
    "Noto Sans Symbols 2",
    "Noto Sans Symbols",
    "DejaVu Sans",
    "Noto Sans Mono CJK SC",
    "Segoe UI Symbol",
    "Apple Symbols",
];

struct Fonts {
    regular: Vec<FontVec>,
    bold: Vec<FontVec>,
    cell: Vec2<u16>,
    ascent: f32,
}

thread_local! {
    static FONTS: RefCell<Option<Rc<Fonts>>> = const { RefCell::new(None) };
}

fn fonts() -> Result<Rc<Fonts>, String> {
    if let Some(fonts) = FONTS.with_borrow(Clone::clone) {
        return Ok(fonts);
    }
    let fonts = Rc::new(load_fonts()?);
    FONTS.with_borrow_mut(|slot| *slot = Some(fonts.clone()));
    Ok(fonts)
}

fn load_fonts() -> Result<Fonts, String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let load = |family: fontdb::Family<'_>, weight: fontdb::Weight| -> Option<FontVec> {
        let query = fontdb::Query {
            families: &[family],
            weight,
            ..fontdb::Query::default()
        };
        let id = db.query(&query)?;
        db.with_face_data(id, |data, index| {
            FontVec::try_from_vec_and_index(data.to_vec(), index).ok()
        })
        .flatten()
    };
    let chain = |weight: fontdb::Weight| -> Vec<FontVec> {
        let primary = MONOSPACE
            .iter()
            .find_map(|name| load(fontdb::Family::Name(name), weight))
            .or_else(|| load(fontdb::Family::Monospace, weight));
        primary
            .into_iter()
            .chain(
                FALLBACKS
                    .iter()
                    .filter_map(|name| load(fontdb::Family::Name(name), fontdb::Weight::NORMAL)),
            )
            .collect()
    };
    let regular = chain(fontdb::Weight::NORMAL);
    let Some(primary) = regular.first() else {
        return Err("no monospace font found on this system".into());
    };
    let scaled = primary.as_scaled(PxScale::from(FONT_PX));
    let width = scaled.h_advance(primary.glyph_id('M')).round().max(1.0);
    let height = (scaled.ascent() - scaled.descent()).ceil().max(1.0);
    let ascent = scaled.ascent();
    let cell = Vec2::new(width as u16, height as u16);
    let mut bold = chain(fontdb::Weight::BOLD);
    if bold.is_empty() {
        bold = chain(fontdb::Weight::NORMAL);
    }
    Ok(Fonts {
        regular,
        bold,
        cell,
        ascent,
    })
}

/// The pixel size of one cell in a screenshot.
pub(crate) fn cell_size() -> Result<Vec2<u16>, String> {
    Ok(fonts()?.cell)
}

/// Paints the last flushed frame of `renderer`. `fitted_cell` is the cell
/// size the runtime reported, which is what images were fitted with.
pub(crate) fn paint(
    renderer: &super::GridRenderer,
    fitted_cell: Vec2<u16>,
) -> Result<RgbaImage, String> {
    let fonts = fonts()?;
    let size = renderer.flushed_size();
    let cell = fonts.cell;
    let mut canvas = RgbaImage::from_pixel(
        u32::from(size.x) * u32::from(cell.x),
        u32::from(size.y) * u32::from(cell.y),
        rgba(resolve(Color::Background)),
    );
    // Two passes, as a terminal paints: every background first, then the
    // text. A glyph wider than its cell (Nerd Font icons often are) would
    // otherwise lose its right half to the next cell's background.
    let mut images = Vec::new();
    let mut texts = Vec::new();
    renderer.for_each_flushed_cell(|x, y, glyph, style, wide| {
        let (fg, bg) = colours(style);
        let span = if wide { 2 } else { 1 };
        let left = u32::from(x) * u32::from(cell.x);
        let top = u32::from(y) * u32::from(cell.y);
        fill(&mut canvas, left, top, u32::from(cell.x) * span, u32::from(cell.y), bg);
        let decorated = style.underline != UnderlineType::None
            || style.attrs & StyleAttribute::Strikethrough as u8 != 0;
        if let Some(piece) = placeholder(glyph, style, x, y) {
            images.push(piece);
        } else if decorated || !glyph.trim().is_empty() {
            texts.push((left, top, span, glyph.to_string(), *style, fg));
        }
    });
    for (left, top, span, glyph, style, fg) in texts {
        let bold = style.attrs & StyleAttribute::Bold as u8 != 0;
        let faces = if bold { &fonts.bold } else { &fonts.regular };
        for character in glyph.chars().filter(|character| !character.is_control()) {
            draw_char(&mut canvas, faces, fonts.ascent, character, left, top, fg);
        }
        let width = u32::from(cell.x) * span;
        let line = |canvas: &mut RgbaImage, row: u32| {
            fill(canvas, left, top + row.min(u32::from(cell.y) - 1), width, 1, fg);
        };
        if style.underline != UnderlineType::None {
            line(&mut canvas, fonts.ascent.round() as u32 + 2);
        }
        if style.attrs & StyleAttribute::Strikethrough as u8 != 0 {
            line(&mut canvas, u32::from(cell.y) / 2);
        }
    }
    for piece in images {
        draw_image(&mut canvas, &piece, cell, fitted_cell);
    }
    Ok(canvas)
}

/// One placeholder cell: which image, which cell of its placement.
struct Piece {
    image_id: u32,
    placement_id: u32,
    image_col: u32,
    image_row: u32,
    screen: Vec2<u16>,
}

fn placeholder(glyph: &str, style: &GridCellStyle, x: u16, y: u16) -> Option<Piece> {
    let mut chars = glyph.chars();
    if chars.next()? != kitty::PLACEHOLDER_CHAR {
        return None;
    }
    let row = chars.next().and_then(kitty::diacritic_to_num).unwrap_or(0);
    let col = chars.next().and_then(kitty::diacritic_to_num).unwrap_or(0);
    let high = chars.next().and_then(kitty::diacritic_to_num).unwrap_or(0).max(1);
    let id_of = |color: Color| match color {
        Color::Rgb(r, g, b) => Some((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)),
        _ => None,
    };
    Some(Piece {
        image_id: id_of(style.fg)? | ((high - 1) << 24),
        placement_id: id_of(style.underline_color).unwrap_or(0),
        image_col: col.saturating_sub(1),
        image_row: row.saturating_sub(1),
        screen: Vec2::new(x, y),
    })
}

fn draw_image(canvas: &mut RgbaImage, piece: &Piece, cell: Vec2<u16>, fitted_cell: Vec2<u16>) {
    let Some(source) = kitty::lookup_source(piece.image_id) else {
        return;
    };
    let Some(pixels) = source.get_rgba() else {
        return;
    };
    let dims = source.get_pixel_dims();
    if dims.x == 0 || dims.y == 0 {
        return;
    }
    let placement = kitty::lookup_placement_size(&source, piece.placement_id);
    let left = u32::from(piece.screen.x) * u32::from(cell.x);
    let top = u32::from(piece.screen.y) * u32::from(cell.y);
    let (cell_w, cell_h) = (u32::from(cell.x), u32::from(cell.y));
    // The fitted image covers only part of a cell at its edges: map each
    // pixel back into the image, and leave what falls outside it to the
    // background already painted.
    let (box_u0, box_v0, box_u1, box_v1) = cell_box(dims, fitted_cell, placement, piece);
    for py in 0..cell_h {
        for px in 0..cell_w {
            let bu = box_u0 + (box_u1 - box_u0) * (f64::from(px) + 0.5) / f64::from(cell_w);
            let bv = box_v0 + (box_v1 - box_v0) * (f64::from(py) + 0.5) / f64::from(cell_h);
            let Some((u, v)) = to_image(bu, bv, dims, fitted_cell, placement) else {
                continue;
            };
            let sx = ((u * f64::from(dims.x)) as u32).min(dims.x - 1);
            let sy = ((v * f64::from(dims.y)) as u32).min(dims.y - 1);
            let offset = ((sy * dims.x + sx) * 4) as usize;
            let Some(&[r, g, b, a]) = pixels.get(offset..offset + 4) else {
                continue;
            };
            if let Some(target) = canvas.get_pixel_mut_checked(left + px, top + py) {
                blend(target, [r, g, b], f32::from(a) / 255.0);
            }
        }
    }
}

/// The cell's extent in placement coordinates (0..1 across the placement).
fn cell_box(
    dims: Vec2<u32>,
    cell_px: Vec2<u16>,
    placement: Option<Vec2<u16>>,
    piece: &Piece,
) -> (f64, f64, f64, f64) {
    let (width, height) = placement_cells(dims, cell_px, placement);
    (
        f64::from(piece.image_col) / width,
        f64::from(piece.image_row) / height,
        f64::from(piece.image_col + 1) / width,
        f64::from(piece.image_row + 1) / height,
    )
}

fn placement_cells(dims: Vec2<u32>, cell_px: Vec2<u16>, placement: Option<Vec2<u16>>) -> (f64, f64) {
    match placement {
        Some(size) => (f64::from(size.x).max(1.0), f64::from(size.y).max(1.0)),
        None => (
            (f64::from(dims.x) / f64::from(cell_px.x.max(1))).round().max(1.0),
            (f64::from(dims.y) / f64::from(cell_px.y.max(1))).round().max(1.0),
        ),
    }
}

/// A point of the placement box in image coordinates, or `None` in the
/// letterbox the aspect fit leaves around the image.
fn to_image(
    box_u: f64,
    box_v: f64,
    dims: Vec2<u32>,
    cell_px: Vec2<u16>,
    placement: Option<Vec2<u16>>,
) -> Option<(f64, f64)> {
    let (width, height) = placement_cells(dims, cell_px, placement);
    let image_aspect = f64::from(dims.x) / f64::from(dims.y);
    let box_aspect = (width * f64::from(cell_px.x.max(1))) / (height * f64::from(cell_px.y.max(1)));
    let (visible_w, visible_h) = if image_aspect >= box_aspect {
        (1.0, box_aspect / image_aspect)
    } else {
        (image_aspect / box_aspect, 1.0)
    };
    let u = (box_u - (1.0 - visible_w) * 0.5) / visible_w;
    let v = (box_v - (1.0 - visible_h) * 0.5) / visible_h;
    ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some((u, v))
}

fn draw_char(
    canvas: &mut RgbaImage,
    faces: &[FontVec],
    ascent: f32,
    character: char,
    left: u32,
    top: u32,
    fg: [u8; 3],
) {
    if character == ' ' {
        return;
    }
    let Some(face) = faces
        .iter()
        .find(|face| face.glyph_id(character).0 != 0)
        .or_else(|| faces.first())
    else {
        return;
    };
    let glyph = face
        .glyph_id(character)
        .with_scale_and_position(PxScale::from(FONT_PX), point(left as f32, top as f32 + ascent));
    let Some(outline) = face.outline_glyph(glyph) else {
        return;
    };
    let bounds = outline.px_bounds();
    outline.draw(|gx, gy, coverage| {
        let x = bounds.min.x as i64 + i64::from(gx);
        let y = bounds.min.y as i64 + i64::from(gy);
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            return;
        };
        if let Some(target) = canvas.get_pixel_mut_checked(x, y) {
            blend(target, fg, coverage);
        }
    });
}

fn colours(style: &GridCellStyle) -> ([u8; 3], [u8; 3]) {
    let mut fg = resolve(style.fg);
    let mut bg = resolve(style.bg);
    if style.attrs & StyleAttribute::Dim as u8 != 0 {
        fg = [
            ((u16::from(fg[0]) + u16::from(bg[0])) / 2) as u8,
            ((u16::from(fg[1]) + u16::from(bg[1])) / 2) as u8,
            ((u16::from(fg[2]) + u16::from(bg[2])) / 2) as u8,
        ];
    }
    if style.has_reverse() {
        std::mem::swap(&mut fg, &mut bg);
    }
    (fg, bg)
}

fn resolve(color: Color) -> [u8; 3] {
    #[cfg(feature = "harmonious")]
    if let Some(rgb) = crate::theme::harmonious::resolve_rgb(color) {
        return [rgb.r, rgb.g, rgb.b];
    }
    match color {
        Color::Foreground => [0xD0, 0xD0, 0xD0],
        Color::Background => [0x10, 0x10, 0x10],
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Indexed(index) => ansi(index),
    }
}

/// xterm's 256-colour palette.
fn ansi(index: u8) -> [u8; 3] {
    const BASIC: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    match index {
        0..=15 => BASIC[usize::from(index)],
        16..=231 => {
            let cube = index - 16;
            let level = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            [level(cube / 36), level((cube % 36) / 6), level(cube % 6)]
        }
        232..=255 => {
            let value = 8 + (index - 232) * 10;
            [value, value, value]
        }
    }
}

fn rgba(rgb: [u8; 3]) -> Rgba<u8> {
    Rgba([rgb[0], rgb[1], rgb[2], 255])
}

fn fill(canvas: &mut RgbaImage, left: u32, top: u32, width: u32, height: u32, rgb: [u8; 3]) {
    for y in top..top.saturating_add(height) {
        for x in left..left.saturating_add(width) {
            if let Some(pixel) = canvas.get_pixel_mut_checked(x, y) {
                *pixel = rgba(rgb);
            }
        }
    }
}

fn blend(target: &mut Rgba<u8>, rgb: [u8; 3], alpha: f32) {
    let alpha = alpha.clamp(0.0, 1.0);
    for (channel, value) in target.0.iter_mut().zip(rgb) {
        *channel = (f32::from(*channel) * (1.0 - alpha) + f32::from(value) * alpha).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_256_colour_cube_and_ramp() {
        assert_eq!(ansi(16), [0, 0, 0]);
        assert_eq!(ansi(231), [255, 255, 255]);
        assert_eq!(ansi(196), [255, 0, 0]);
        assert_eq!(ansi(232), [8, 8, 8]);
        assert_eq!(ansi(255), [238, 238, 238]);
    }

    #[test]
    fn a_letterboxed_image_leaves_its_margins_alone() {
        // A 2:1 image in a square box: the top and bottom quarter are empty.
        let dims = Vec2::new(200, 100);
        let cell = Vec2::new(10, 10);
        let placement = Some(Vec2::new(4, 4));
        assert_eq!(to_image(0.5, 0.1, dims, cell, placement), None);
        let (u, v) = to_image(0.5, 0.5, dims, cell, placement).expect("inside");
        assert!((u - 0.5).abs() < 1e-9 && (v - 0.5).abs() < 1e-9);
    }
}
