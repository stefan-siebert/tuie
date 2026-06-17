//! Cell-aligned glyph rasterizer for Unicode block and drawing characters.

use crate::prelude::*;
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stroke {
    Light,
    Heavy,
}

fn line_coeffs(a: (f64, f64), b: (f64, f64)) -> (f64, f64, f64) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        return (0.0, 0.0, 0.0);
    }
    let nx = -dy / len;
    let ny = dx / len;
    let c = -(nx * a.0 + ny * a.1);
    (nx, ny, c)
}

pub(crate) fn is_box_codepoint(ch: char) -> bool {
    matches!(
        ch,
        '\u{2190}'..='\u{2193}'
            | '\u{21E0}'..='\u{21E3}'
            | '\u{2500}'..='\u{259F}'
            | '\u{25C9}'
            | '\u{25CB}'
            | '\u{25CF}'
            | '\u{25D6}'..='\u{25D7}'
            | '\u{25DC}'..='\u{25E1}'
            | '\u{25E2}'..='\u{25E5}'
            | '\u{2800}'..='\u{28FF}'
            | '\u{E0B0}'..='\u{E0BF}'
            | '\u{E0D6}'..='\u{E0D7}'
            | '\u{1CD00}'..='\u{1CDE5}'
            | '\u{1FB00}'..='\u{1FB3B}'
            | '\u{1FB70}'..='\u{1FB81}'
            | '\u{1FB87}'..='\u{1FB8B}'
            | '\u{1FBE6}'
            | '\u{1FBE7}'
    )
}

pub(crate) fn rasterize(ch: char, cell_size: Vec2<u32>) -> Option<Vec<u8>> {
    if cell_size.x == 0 || cell_size.y == 0 {
        return None;
    }
    let mut glyph = Glyph::new(cell_size);
    if glyph.draw(ch) {
        Some(glyph.pixels)
    } else {
        None
    }
}

struct Glyph {
    pixels: Vec<u8>,
    size: Vec2<u32>,
}

impl Glyph {
    fn new(size: Vec2<u32>) -> Self {
        Self {
            pixels: vec![0; (size.x * size.y) as usize],
            size,
        }
    }

    fn stroke_thickness(&self, weight: Stroke) -> u32 {
        let base = self.size.y as f64 / 18.0;
        let scaled = match weight {
            Stroke::Light => base,
            Stroke::Heavy => base * 2.0,
        };
        (scaled.round() as u32).max(1)
    }

    fn band(&self, axis: Axis2D, center: u32, weight: Stroke) -> Range<u32> {
        let extent = self.axis_extent(axis);
        let thickness = self.stroke_thickness(weight);
        let lo = center.saturating_sub(thickness / 2);
        let hi = (lo + thickness).min(extent);
        lo..hi
    }

    fn fill_row(&mut self, y: u32, x_min: u32, x_max: u32, alpha: u8) {
        if y >= self.size.y {
            return;
        }
        let row = (y * self.size.x) as usize;
        let min = x_min.min(self.size.x) as usize;
        let max = x_max.min(self.size.x) as usize;
        if max > min {
            self.pixels[row + min..row + max].fill(alpha);
        }
    }

    fn fill_rect(&mut self, rect: Rect<u32>) {
        let x_min = rect.pos.x.min(self.size.x);
        let x_max = (rect.pos.x + rect.size.x).min(self.size.x);
        if x_min >= x_max {
            return;
        }
        let y_max = (rect.pos.y + rect.size.y).min(self.size.y);
        for y in rect.pos.y..y_max {
            self.fill_row(y, x_min, x_max, 255);
        }
    }

    fn put_max(&mut self, x: u32, y: u32, alpha: u8) {
        if x >= self.size.x || y >= self.size.y {
            return;
        }
        let index = (y * self.size.x + x) as usize;
        if alpha > self.pixels[index] {
            self.pixels[index] = alpha;
        }
    }

    fn stroke_segment(
        &mut self,
        parallel: Axis2D,
        range: Range<u32>,
        perp_center: u32,
        weight: Stroke,
    ) {
        let perp = self.band(parallel.flip(), perp_center, weight);
        let par_extent = self.axis_extent(parallel);
        let par_lo = range.start.min(par_extent);
        let par_hi = range.end.min(par_extent);
        if perp.start >= perp.end || par_lo >= par_hi {
            return;
        }
        let (pos, size) = match parallel {
            Axis2D::X => (
                Vec2::new(par_lo, perp.start),
                Vec2::new(par_hi - par_lo, perp.end - perp.start),
            ),
            Axis2D::Y => (
                Vec2::new(perp.start, par_lo),
                Vec2::new(perp.end - perp.start, par_hi - par_lo),
            ),
        };
        self.fill_rect(Rect { pos, size });
    }

    fn stroke_along(&mut self, axis: Axis2D, range: Range<u32>, weight: Stroke) {
        let perp_center = self.axis_extent(axis.flip()) / 2;
        self.stroke_segment(axis, range, perp_center, weight);
    }

    fn axis_extent(&self, axis: Axis2D) -> u32 {
        match axis {
            Axis2D::X => self.size.x,
            Axis2D::Y => self.size.y,
        }
    }

    fn full_axis(&self, axis: Axis2D) -> Range<u32> {
        0..self.axis_extent(axis)
    }

    fn near_half(&self, axis: Axis2D) -> Range<u32> {
        0..self.axis_extent(axis) / 2
    }

    fn far_half(&self, axis: Axis2D) -> Range<u32> {
        let n = self.axis_extent(axis);
        n / 2..n
    }

    fn draw(&mut self, ch: char) -> bool {
        let codepoint = ch as u32;
        match codepoint {
            0x2500 => self.stroke_along(Axis2D::X, self.full_axis(Axis2D::X), Stroke::Light),
            0x2501 => self.stroke_along(Axis2D::X, self.full_axis(Axis2D::X), Stroke::Heavy),
            0x2502 => self.stroke_along(Axis2D::Y, self.full_axis(Axis2D::Y), Stroke::Light),
            0x2503 => self.stroke_along(Axis2D::Y, self.full_axis(Axis2D::Y), Stroke::Heavy),

            0x2504 => dashes::horizontal(self, Stroke::Light, 2),
            0x2505 => dashes::horizontal(self, Stroke::Heavy, 2),
            0x2506 => dashes::vertical(self, Stroke::Light, 2),
            0x2507 => dashes::vertical(self, Stroke::Heavy, 2),
            0x2508 => dashes::horizontal(self, Stroke::Light, 3),
            0x2509 => dashes::horizontal(self, Stroke::Heavy, 3),
            0x250A => dashes::vertical(self, Stroke::Light, 3),
            0x250B => dashes::vertical(self, Stroke::Heavy, 3),

            0x250C..=0x250F => {
                let (h, v) = corner::weights(codepoint - 0x250C);
                corner::sharp(self, h, v, Edge2D::Right as u32 | Edge2D::Bottom as u32);
            }
            0x2510..=0x2513 => {
                let (h, v) = corner::weights(codepoint - 0x2510);
                corner::sharp(self, h, v, Edge2D::Left as u32 | Edge2D::Bottom as u32);
            }
            0x2514..=0x2517 => {
                let (h, v) = corner::weights(codepoint - 0x2514);
                corner::sharp(self, h, v, Edge2D::Right as u32 | Edge2D::Top as u32);
            }
            0x2518..=0x251B => {
                let (h, v) = corner::weights(codepoint - 0x2518);
                corner::sharp(self, h, v, Edge2D::Left as u32 | Edge2D::Top as u32);
            }

            0x251C..=0x2523 => tees::vertical_spine(self, codepoint - 0x251C, true),
            0x2524..=0x252B => tees::vertical_spine(self, codepoint - 0x2524, false),
            0x252C..=0x2533 => tees::horizontal_spine(self, codepoint - 0x252C, true),
            0x2534..=0x253B => tees::horizontal_spine(self, codepoint - 0x2534, false),

            0x253C..=0x254B => tees::cross(self, codepoint - 0x253C),

            0x254C => dashes::horizontal(self, Stroke::Light, 1),
            0x254D => dashes::horizontal(self, Stroke::Heavy, 1),
            0x254E => dashes::vertical(self, Stroke::Light, 1),
            0x254F => dashes::vertical(self, Stroke::Heavy, 1),

            0x2550 => double::horizontal(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Bottom as u32),
            0x2551 => double::vertical(self, Stroke::Light, Edge2D::Left as u32 | Edge2D::Right as u32),
            0x2552 => double::horizontal_corner(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Left as u32),
            0x2553 => double::vertical_corner(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Left as u32),
            0x2554 => double::corner(self, Edge2D::Right as u32 | Edge2D::Bottom as u32),
            0x2555 => double::horizontal_corner(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Right as u32),
            0x2556 => double::vertical_corner(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Right as u32),
            0x2557 => double::corner(self, Edge2D::Left as u32 | Edge2D::Bottom as u32),
            0x2558 => double::horizontal_corner(self, Stroke::Light, Edge2D::Bottom as u32 | Edge2D::Left as u32),
            0x2559 => double::vertical_corner(self, Stroke::Light, Edge2D::Bottom as u32 | Edge2D::Left as u32),
            0x255A => double::corner(self, Edge2D::Right as u32 | Edge2D::Top as u32),
            0x255B => double::horizontal_corner(self, Stroke::Light, Edge2D::Bottom as u32 | Edge2D::Right as u32),
            0x255C => double::vertical_corner(self, Stroke::Light, Edge2D::Bottom as u32 | Edge2D::Right as u32),
            0x255D => double::corner(self, Edge2D::Left as u32 | Edge2D::Top as u32),
            0x255E => {
                self.stroke_along(Axis2D::Y, self.full_axis(Axis2D::Y), Stroke::Light);
                double::half_horizontal(self, Stroke::Light, true, Edge2D::Top as u32 | Edge2D::Bottom as u32);
            }
            0x255F => double::with_arm(self, Stroke::Light, Direction2D::Right),
            0x2561 => {
                self.stroke_along(Axis2D::Y, self.full_axis(Axis2D::Y), Stroke::Light);
                double::half_horizontal(self, Stroke::Light, false, Edge2D::Top as u32 | Edge2D::Bottom as u32);
            }
            0x2562 => double::with_arm(self, Stroke::Light, Direction2D::Left),
            0x2564 => double::with_arm(self, Stroke::Light, Direction2D::Down),
            0x2565 => {
                self.stroke_along(Axis2D::X, self.full_axis(Axis2D::X), Stroke::Light);
                double::half_vertical(self, Stroke::Light, true, Edge2D::Left as u32 | Edge2D::Right as u32);
            }
            0x2567 => double::with_arm(self, Stroke::Light, Direction2D::Up),
            0x2568 => {
                self.stroke_along(Axis2D::X, self.full_axis(Axis2D::X), Stroke::Light);
                double::half_vertical(self, Stroke::Light, false, Edge2D::Left as u32 | Edge2D::Right as u32);
            }
            0x256A => {
                self.stroke_along(Axis2D::Y, self.full_axis(Axis2D::Y), Stroke::Light);
                double::horizontal(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Bottom as u32);
            }
            0x256B => {
                self.stroke_along(Axis2D::X, self.full_axis(Axis2D::X), Stroke::Light);
                double::vertical(self, Stroke::Light, Edge2D::Left as u32 | Edge2D::Right as u32);
            }
            0x2560 => {
                double::inner_corner(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Right as u32);
                double::inner_corner(self, Stroke::Light, Edge2D::Bottom as u32 | Edge2D::Right as u32);
                double::vertical(self, Stroke::Light, Edge2D::Left as u32);
            }
            0x2563 => {
                double::inner_corner(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Left as u32);
                double::inner_corner(self, Stroke::Light, Edge2D::Bottom as u32 | Edge2D::Left as u32);
                double::vertical(self, Stroke::Light, Edge2D::Right as u32);
            }
            0x2566 => {
                double::inner_corner(self, Stroke::Light, Edge2D::Bottom as u32 | Edge2D::Left as u32);
                double::inner_corner(self, Stroke::Light, Edge2D::Bottom as u32 | Edge2D::Right as u32);
                double::horizontal(self, Stroke::Light, Edge2D::Top as u32);
            }
            0x2569 => {
                double::inner_corner(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Left as u32);
                double::inner_corner(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Right as u32);
                double::horizontal(self, Stroke::Light, Edge2D::Bottom as u32);
            }
            0x256C => {
                double::inner_corner(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Left as u32);
                double::inner_corner(self, Stroke::Light, Edge2D::Top as u32 | Edge2D::Right as u32);
                double::inner_corner(self, Stroke::Light, Edge2D::Bottom as u32 | Edge2D::Left as u32);
                double::inner_corner(self, Stroke::Light, Edge2D::Bottom as u32 | Edge2D::Right as u32);
            }

            0x256D => corner::rounded(self, Stroke::Light, Edge2D::Right as u32 | Edge2D::Bottom as u32),
            0x256E => corner::rounded(self, Stroke::Light, Edge2D::Left as u32 | Edge2D::Bottom as u32),
            0x256F => corner::rounded(self, Stroke::Light, Edge2D::Left as u32 | Edge2D::Top as u32),
            0x2570 => corner::rounded(self, Stroke::Light, Edge2D::Right as u32 | Edge2D::Top as u32),

            0x2571 => diagonal::draw(self, true),
            0x2572 => diagonal::draw(self, false),
            0x2573 => {
                diagonal::draw(self, true);
                diagonal::draw(self, false);
            }

            0x2574 => self.stroke_along(Axis2D::X, self.near_half(Axis2D::X), Stroke::Light),
            0x2575 => self.stroke_along(Axis2D::Y, self.near_half(Axis2D::Y), Stroke::Light),
            0x2576 => self.stroke_along(Axis2D::X, self.far_half(Axis2D::X), Stroke::Light),
            0x2577 => self.stroke_along(Axis2D::Y, self.far_half(Axis2D::Y), Stroke::Light),
            0x2578 => self.stroke_along(Axis2D::X, self.near_half(Axis2D::X), Stroke::Heavy),
            0x2579 => self.stroke_along(Axis2D::Y, self.near_half(Axis2D::Y), Stroke::Heavy),
            0x257A => self.stroke_along(Axis2D::X, self.far_half(Axis2D::X), Stroke::Heavy),
            0x257B => self.stroke_along(Axis2D::Y, self.far_half(Axis2D::Y), Stroke::Heavy),

            0x257C => {
                self.stroke_along(Axis2D::X, self.near_half(Axis2D::X), Stroke::Light);
                self.stroke_along(Axis2D::X, self.far_half(Axis2D::X), Stroke::Heavy);
            }
            0x257D => {
                self.stroke_along(Axis2D::Y, self.near_half(Axis2D::Y), Stroke::Light);
                self.stroke_along(Axis2D::Y, self.far_half(Axis2D::Y), Stroke::Heavy);
            }
            0x257E => {
                self.stroke_along(Axis2D::X, self.near_half(Axis2D::X), Stroke::Heavy);
                self.stroke_along(Axis2D::X, self.far_half(Axis2D::X), Stroke::Light);
            }
            0x257F => {
                self.stroke_along(Axis2D::Y, self.near_half(Axis2D::Y), Stroke::Heavy);
                self.stroke_along(Axis2D::Y, self.far_half(Axis2D::Y), Stroke::Light);
            }

            0x2580 => self.fill_rect(Rect::xywh(0, 0, self.size.x, self.size.y / 2)),
            0x2581 => eighths::lower(self, 1),
            0x2582 => eighths::lower(self, 2),
            0x2583 => eighths::lower(self, 3),
            0x2584 => eighths::lower(self, 4),
            0x2585 => eighths::lower(self, 5),
            0x2586 => eighths::lower(self, 6),
            0x2587 => eighths::lower(self, 7),
            0x2588 => self.pixels.fill(255),
            0x2589 => eighths::left(self, 7),
            0x258A => eighths::left(self, 6),
            0x258B => eighths::left(self, 5),
            0x258C => eighths::left(self, 4),
            0x258D => eighths::left(self, 3),
            0x258E => eighths::left(self, 2),
            0x258F => eighths::left(self, 1),
            0x2590 => {
                let mid = self.size.x / 2;
                self.fill_rect(Rect::xywh(mid, 0, self.size.x - mid, self.size.y));
            }
            0x2591 => self.pixels.fill(64),
            0x2592 => self.pixels.fill(128),
            0x2593 => self.pixels.fill(192),
            0x2594 => {
                let h = eighths::boundary(self.size.y, 1, 8);
                self.fill_rect(Rect::xywh(0, 0, self.size.x, h));
            }
            0x2595 => {
                let l = eighths::boundary(self.size.x, 7, 8);
                self.fill_rect(Rect::xywh(l, 0, self.size.x - l, self.size.y));
            }
            0x2596 => quadrants::draw(self, false, false, true, false),
            0x2597 => quadrants::draw(self, false, false, false, true),
            0x2598 => quadrants::draw(self, true, false, false, false),
            0x2599 => quadrants::draw(self, true, false, true, true),
            0x259A => quadrants::draw(self, true, false, false, true),
            0x259B => quadrants::draw(self, true, true, true, false),
            0x259C => quadrants::draw(self, true, true, false, true),
            0x259D => quadrants::draw(self, false, true, false, false),
            0x259E => quadrants::draw(self, false, true, true, false),
            0x259F => quadrants::draw(self, false, true, true, true),

            0x25C9 => disk::fish_eye(self),
            0x25CB => disk::outline(self, disk::full),
            0x25CF => disk::filled(self, disk::full),
            0x25D6 => powerline::filled_d(self, false),
            0x25D7 => powerline::filled_d(self, true),
            0x25DC => disk::outline(self, disk::upper_left),
            0x25DD => disk::outline(self, disk::upper_right),
            0x25DE => disk::outline(self, disk::lower_right),
            0x25DF => disk::outline(self, disk::lower_left),
            0x25E0 => disk::outline(self, disk::upper),
            0x25E1 => disk::outline(self, disk::lower),

            0x25E2 => powerline::corner_triangle(self, Edge2D::Bottom as u32 | Edge2D::Right as u32),
            0x25E3 => powerline::corner_triangle(self, Edge2D::Bottom as u32 | Edge2D::Left as u32),
            0x25E4 => powerline::corner_triangle(self, Edge2D::Top as u32 | Edge2D::Left as u32),
            0x25E5 => powerline::corner_triangle(self, Edge2D::Top as u32 | Edge2D::Right as u32),

            0x2800..=0x28FF => braille::draw(self, ch),

            0x1FB00..=0x1FB3B => sextant::draw(self, codepoint - 0x1FB00),

            0x1FB70..=0x1FB75 => eighths::bar(self, codepoint - 0x1FB6F, false),
            0x1FB76..=0x1FB7B => eighths::bar(self, codepoint - 0x1FB75, true),

            0x1FB7C => {
                eighths::left(self, 1);
                eighths::lower(self, 1);
            }
            0x1FB7D => {
                eighths::left(self, 1);
                eighths::upper(self, 1);
            }
            0x1FB7E => {
                eighths::right(self, 1);
                eighths::upper(self, 1);
            }
            0x1FB7F => {
                eighths::right(self, 1);
                eighths::lower(self, 1);
            }
            0x1FB80 => {
                eighths::upper(self, 1);
                eighths::lower(self, 1);
            }
            0x1FB81 => {
                eighths::bar(self, 0, true);
                eighths::bar(self, 2, true);
                eighths::bar(self, 4, true);
                eighths::bar(self, 7, true);
            }

            0x1FB87 => eighths::right(self, 2),
            0x1FB88 => eighths::right(self, 3),
            0x1FB89 => eighths::right(self, 5),
            0x1FB8A => eighths::right(self, 6),
            0x1FB8B => eighths::right(self, 7),

            0x1FBE6 => octant::draw(self, 0xE6),
            0x1FBE7 => octant::draw(self, 0xE7),
            0x1CD00..=0x1CDE5 => octant::draw(self, (codepoint - 0x1CD00) as u8),

            0xE0B0 => powerline::triangle(self, true, false),
            0xE0B1 => {
                powerline::chevron(self, true);
            }
            0xE0B2 => powerline::triangle(self, false, false),
            0xE0B3 => {
                powerline::chevron(self, false);
            }
            0xE0B4 => powerline::filled_d(self, true),
            0xE0B5 => powerline::rounded_separator(self, true),
            0xE0B6 => powerline::filled_d(self, false),
            0xE0B7 => powerline::rounded_separator(self, false),
            0xE0B8 => powerline::corner_triangle(self, Edge2D::Bottom as u32 | Edge2D::Left as u32),
            0xE0B9 => diagonal::draw(self, false),
            0xE0BA => powerline::corner_triangle(self, Edge2D::Bottom as u32 | Edge2D::Right as u32),
            0xE0BB => diagonal::draw(self, true),
            0xE0BC => powerline::corner_triangle(self, Edge2D::Top as u32 | Edge2D::Left as u32),
            0xE0BD => diagonal::draw(self, true),
            0xE0BE => powerline::corner_triangle(self, Edge2D::Top as u32 | Edge2D::Right as u32),
            0xE0BF => diagonal::draw(self, false),
            0xE0D6 => powerline::triangle(self, false, true),
            0xE0D7 => powerline::triangle(self, true, true),

            0x2190 => arrow::single(self, Direction2D::Left, false),
            0x2191 => arrow::single(self, Direction2D::Up, false),
            0x2192 => arrow::single(self, Direction2D::Right, false),
            0x2193 => arrow::single(self, Direction2D::Down, false),
            0x21E0 => arrow::single(self, Direction2D::Left, true),
            0x21E1 => arrow::single(self, Direction2D::Up, true),
            0x21E2 => arrow::single(self, Direction2D::Right, true),
            0x21E3 => arrow::single(self, Direction2D::Down, true),

            _ => return false,
        }
        true
    }
}

mod corner {
    use super::*;

    pub(super) fn weights(index: u32) -> (Stroke, Stroke) {
        let h = if index & 1 != 0 { Stroke::Heavy } else { Stroke::Light };
        let v = if index & 2 != 0 { Stroke::Heavy } else { Stroke::Light };
        (h, v)
    }

    pub(super) fn sharp(g: &mut Glyph, h_weight: Stroke, v_weight: Stroke, opens: u32) {
        let vert_x = g.band(Axis2D::X, g.size.x / 2, v_weight);
        let horiz_y = g.band(Axis2D::Y, g.size.y / 2, h_weight);

        let (h_start, h_end) = if (opens & Edge2D::Right as u32) != 0 {
            (vert_x.start, g.size.x)
        } else {
            (0, vert_x.end)
        };
        let (v_start, v_end) = if (opens & Edge2D::Bottom as u32) != 0 {
            (horiz_y.start, g.size.y)
        } else {
            (0, horiz_y.end)
        };

        for y in horiz_y.start..horiz_y.end {
            g.fill_row(y, h_start, h_end, 255);
        }
        for y in v_start..v_end {
            g.fill_row(y, vert_x.start, vert_x.end, 255);
        }
    }

    pub(super) fn rounded(g: &mut Glyph, weight: Stroke, opens: u32) {
        let half = g.stroke_thickness(weight) as f64 / 2.0;

        let h_y = g.band(Axis2D::Y, g.size.y / 2, weight);
        let v_x = g.band(Axis2D::X, g.size.x / 2, weight);
        let centerline = Vec2::new(
            (v_x.start + v_x.end) as f64 / 2.0,
            (h_y.start + h_y.end) as f64 / 2.0,
        );

        let sign_x = if (opens & Edge2D::Right as u32) != 0 { Sign::Positive } else { Sign::Negative };
        let sign_y = if (opens & Edge2D::Bottom as u32) != 0 { Sign::Positive } else { Sign::Negative };
        let dx = sign_x.delta() as f64;
        let dy = sign_y.delta() as f64;

        let r_x = if sign_x.is_positive() { g.size.x as f64 - centerline.x } else { centerline.x };
        let r_y = if sign_y.is_positive() { g.size.y as f64 - centerline.y } else { centerline.y };
        let radius = r_x.min(r_y);
        let arc_center = Vec2::new(centerline.x + dx * radius, centerline.y + dy * radius);

        for y in 0..g.size.y {
            for x in 0..g.size.x {
                let p = Vec2::new(x as f64 + 0.5, y as f64 + 0.5);
                let off = p - arc_center;
                let past_x = dx * off.x > 0.0;
                let past_y = dy * off.y > 0.0;

                if past_x && past_y {
                    continue;
                } else if past_x {
                    if y >= h_y.start && y < h_y.end {
                        g.put_max(x, y, 255);
                    }
                } else if past_y {
                    if x >= v_x.start && x < v_x.end {
                        g.put_max(x, y, 255);
                    }
                } else {
                    let dist = ((off.x * off.x + off.y * off.y).sqrt() - radius).abs();
                    let coverage = (half + 0.5 - dist).clamp(0.0, 1.0);
                    if coverage > 0.0 {
                        g.put_max(x, y, (coverage * 255.0).round() as u8);
                    }
                }
            }
        }
    }
}

mod quadrants {
    use super::*;

    pub(super) fn draw(g: &mut Glyph, top_left: bool, top_right: bool, bottom_left: bool, bottom_right: bool) {
        let mid_x = g.size.x / 2;
        let mid_y = g.size.y / 2;
        let right_w = g.size.x - mid_x;
        let bottom_h = g.size.y - mid_y;
        if top_left {
            g.fill_rect(Rect::xywh(0, 0, mid_x, mid_y));
        }
        if top_right {
            g.fill_rect(Rect::xywh(mid_x, 0, right_w, mid_y));
        }
        if bottom_left {
            g.fill_rect(Rect::xywh(0, mid_y, mid_x, bottom_h));
        }
        if bottom_right {
            g.fill_rect(Rect::xywh(mid_x, mid_y, right_w, bottom_h));
        }
    }
}

mod disk {
    //! Disk and arc drawing primitives.
    use super::*;

    pub(super) type Sector = fn(f64, f64) -> bool;

    pub(super) fn full(_dx: f64, _dy: f64) -> bool {
        true
    }
    pub(super) fn upper(_dx: f64, dy: f64) -> bool {
        dy <= 0.0
    }
    pub(super) fn lower(_dx: f64, dy: f64) -> bool {
        dy >= 0.0
    }
    pub(super) fn upper_left(dx: f64, dy: f64) -> bool {
        dx <= 0.0 && dy <= 0.0
    }
    pub(super) fn upper_right(dx: f64, dy: f64) -> bool {
        dx >= 0.0 && dy <= 0.0
    }
    pub(super) fn lower_right(dx: f64, dy: f64) -> bool {
        dx >= 0.0 && dy >= 0.0
    }
    pub(super) fn lower_left(dx: f64, dy: f64) -> bool {
        dx <= 0.0 && dy >= 0.0
    }

    fn geometry(g: &Glyph) -> (f64, f64, f64) {
        let cx = g.size.x as f64 / 2.0;
        let cy = g.size.y as f64 / 2.0;
        let radius = g.size.x.min(g.size.y) as f64 / 2.0;
        (cx, cy, radius)
    }

    pub(super) fn filled(g: &mut Glyph, sector: Sector) {
        let (cx, cy, radius) = geometry(g);
        fill_at(g, cx, cy, radius, sector);
    }

    pub(super) fn outline(g: &mut Glyph, sector: Sector) {
        let (cx, cy, radius) = geometry(g);
        let thick = g.stroke_thickness(Stroke::Light) as f64;
        ring_at(g, cx, cy, radius - thick / 2.0, thick / 2.0, sector);
    }

    pub(super) fn fish_eye(g: &mut Glyph) {
        let (cx, cy, radius) = geometry(g);
        let thick = g.stroke_thickness(Stroke::Light) as f64;
        ring_at(g, cx, cy, radius - thick / 2.0, thick / 2.0, full);
        let inner = (radius - thick) * 0.55;
        if inner > 0.5 {
            fill_at(g, cx, cy, inner, full);
        }
    }

    fn fill_at(g: &mut Glyph, cx: f64, cy: f64, radius: f64, sector: Sector) {
        if radius <= 0.0 {
            return;
        }
        let outer_sq = (radius + 0.5).powi(2);
        let inner_sq = (radius - 0.5).max(0.0).powi(2);
        for y in 0..g.size.y {
            let dy = (y as f64 + 0.5) - cy;
            let dy_sq = dy * dy;
            if dy_sq >= outer_sq {
                continue;
            }
            let outer_chord = (outer_sq - dy_sq).sqrt();
            let x_lo = (cx - outer_chord).floor().max(0.0) as u32;
            let x_hi = ((cx + outer_chord).ceil() as u32).min(g.size.x);
            for x in x_lo..x_hi {
                let dx = (x as f64 + 0.5) - cx;
                if !sector(dx, dy) {
                    continue;
                }
                let dist_sq = dx * dx + dy_sq;
                if dist_sq <= inner_sq {
                    g.put_max(x, y, 255);
                } else {
                    let dist = dist_sq.sqrt();
                    let coverage = (radius + 0.5 - dist).clamp(0.0, 1.0);
                    if coverage > 0.0 {
                        g.put_max(x, y, (coverage * 255.0).round() as u8);
                    }
                }
            }
        }
    }

    fn ring_at(g: &mut Glyph, cx: f64, cy: f64, radius: f64, half_thick: f64, sector: Sector) {
        let outer_sq = (radius + half_thick + 0.5).powi(2);
        for y in 0..g.size.y {
            let dy = (y as f64 + 0.5) - cy;
            let dy_sq = dy * dy;
            if dy_sq >= outer_sq {
                continue;
            }
            let chord = (outer_sq - dy_sq).sqrt();
            let x_lo = (cx - chord).floor().max(0.0) as u32;
            let x_hi = ((cx + chord).ceil() as u32).min(g.size.x);
            for x in x_lo..x_hi {
                let dx = (x as f64 + 0.5) - cx;
                if !sector(dx, dy) {
                    continue;
                }
                let dist = (dx * dx + dy_sq).sqrt();
                let edge_distance = (dist - radius).abs() - half_thick;
                let coverage = (0.5 - edge_distance).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    g.put_max(x, y, (coverage * 255.0).round() as u8);
                }
            }
        }
    }
}

mod tees {
    //! Tee and cross glyph drawing.
    use super::*;

    fn weight(heavy_mask: u16, glyph_index: u32) -> Stroke {
        if (heavy_mask >> glyph_index) & 1 != 0 {
            Stroke::Heavy
        } else {
            Stroke::Light
        }
    }

    pub(super) fn vertical_spine(g: &mut Glyph, glyph_index: u32, branch_right: bool) {
        const HEAVY_TOP: u16 = 0xB4;
        const HEAVY_BOT: u16 = 0xD8;
        const HEAVY_BRANCH: u16 = 0xE2;
        g.stroke_along(Axis2D::Y, g.near_half(Axis2D::Y), weight(HEAVY_TOP, glyph_index));
        let h_range = if branch_right {
            g.far_half(Axis2D::X)
        } else {
            g.near_half(Axis2D::X)
        };
        g.stroke_along(Axis2D::X, h_range, weight(HEAVY_BRANCH, glyph_index));
        g.stroke_along(Axis2D::Y, g.far_half(Axis2D::Y), weight(HEAVY_BOT, glyph_index));
    }

    pub(super) fn horizontal_spine(g: &mut Glyph, glyph_index: u32, branch_down: bool) {
        let left_heavy = (glyph_index & 1) != 0;
        let right_heavy = (glyph_index & 2) != 0;
        let branch_heavy = (glyph_index & 4) != 0;
        let stroke = |heavy: bool| if heavy { Stroke::Heavy } else { Stroke::Light };
        g.stroke_along(Axis2D::X, g.near_half(Axis2D::X), stroke(left_heavy));
        g.stroke_along(Axis2D::X, g.far_half(Axis2D::X), stroke(right_heavy));
        let v_range = if branch_down {
            g.far_half(Axis2D::Y)
        } else {
            g.near_half(Axis2D::Y)
        };
        g.stroke_along(Axis2D::Y, v_range, stroke(branch_heavy));
    }

    pub(super) fn cross(g: &mut Glyph, glyph_index: u32) {
        const HEAVY_TOP: u16 = 0xE9D0;
        const HEAVY_LEFT: u16 = 0xBA8A;
        const HEAVY_RIGHT: u16 = 0xDD0C;
        const HEAVY_BOT: u16 = 0xF660;
        g.stroke_along(Axis2D::Y, g.near_half(Axis2D::Y), weight(HEAVY_TOP, glyph_index));
        g.stroke_along(Axis2D::X, g.near_half(Axis2D::X), weight(HEAVY_LEFT, glyph_index));
        g.stroke_along(Axis2D::X, g.far_half(Axis2D::X), weight(HEAVY_RIGHT, glyph_index));
        g.stroke_along(Axis2D::Y, g.far_half(Axis2D::Y), weight(HEAVY_BOT, glyph_index));
    }
}

mod dashes {
    use super::*;

    const DASH_GAP_RATIO: (u32, u32) = (3, 1);

    fn dash_bounds(span: u32, dash_count: u32, k: u32) -> (u32, u32) {
        let (dash, gap) = DASH_GAP_RATIO;
        let gap_width = ((span * gap) / (dash_count * (dash + gap))).max(1);
        let trail = gap_width / 2;
        let lead = gap_width - trail;
        let start = ((k * span) / dash_count).saturating_add(lead).min(span);
        let end = (((k + 1) * span) / dash_count).saturating_sub(trail);
        (start, end.min(span))
    }

    pub(super) fn horizontal(g: &mut Glyph, weight: Stroke, hole_count: u32) {
        let dash_count = hole_count + 1;
        let thickness = g.stroke_thickness(weight);
        let y0 = (g.size.y / 2).saturating_sub(thickness / 2);
        let y1 = (y0 + thickness).min(g.size.y);
        for k in 0..dash_count {
            let (xs, xe) = dash_bounds(g.size.x, dash_count, k);
            if xe <= xs {
                continue;
            }
            for y in y0..y1 {
                g.fill_row(y, xs, xe, 255);
            }
        }
    }

    pub(super) fn vertical(g: &mut Glyph, weight: Stroke, hole_count: u32) {
        let dash_count = hole_count + 1;
        let thickness = g.stroke_thickness(weight);
        let x0 = (g.size.x / 2).saturating_sub(thickness / 2);
        let x1 = (x0 + thickness).min(g.size.x);
        for k in 0..dash_count {
            let (ys, ye) = dash_bounds(g.size.y, dash_count, k);
            if ye <= ys {
                continue;
            }
            for y in ys..ye {
                g.fill_row(y, x0, x1, 255);
            }
        }
    }
}

mod double {
    use super::*;

    pub(super) fn half_horizontal(g: &mut Glyph, weight: Stroke, right: bool, sides: u32) {
        let off = g.stroke_thickness(weight);
        let mid_x = g.size.x / 2;
        let mid_y = g.size.y / 2;
        let (xs, xe) = if right { (mid_x, g.size.x) } else { (0, mid_x) };
        for edge in [Edge2D::Top, Edge2D::Bottom] {
            if sides & edge as u32 == 0 {
                continue;
            }
            let y = match edge {
                Edge2D::Top => mid_y.saturating_sub(off),
                _ => mid_y + off,
            };
            g.stroke_segment(Axis2D::X, xs..xe, y, weight);
        }
    }

    pub(super) fn half_vertical(g: &mut Glyph, weight: Stroke, bottom: bool, sides: u32) {
        let off = g.stroke_thickness(weight);
        let mid_x = g.size.x / 2;
        let mid_y = g.size.y / 2;
        let (ys, ye) = if bottom { (mid_y, g.size.y) } else { (0, mid_y) };
        for edge in [Edge2D::Left, Edge2D::Right] {
            if sides & edge as u32 == 0 {
                continue;
            }
            let x = match edge {
                Edge2D::Left => mid_x.saturating_sub(off),
                _ => mid_x + off,
            };
            g.stroke_segment(Axis2D::Y, ys..ye, x, weight);
        }
    }

    pub(super) fn horizontal(g: &mut Glyph, weight: Stroke, sides: u32) {
        for right in [false, true] {
            half_horizontal(g, weight, right, sides);
        }
    }

    pub(super) fn vertical(g: &mut Glyph, weight: Stroke, sides: u32) {
        for bottom in [false, true] {
            half_vertical(g, weight, bottom, sides);
        }
    }

    pub(super) fn horizontal_corner(g: &mut Glyph, weight: Stroke, corner_at: u32) {
        let face_left = corner_at & Edge2D::Left as u32 != 0;
        let face_down = corner_at & Edge2D::Bottom as u32 != 0;
        let off = g.stroke_thickness(weight);
        let half = off / 2;
        let mid_x = g.size.x / 2;
        let mid_y = g.size.y / 2;
        let north = mid_y.saturating_sub(off);
        let south = mid_y + off;
        let (xs, xe) = if face_left { (mid_x, g.size.x) } else { (0, mid_x) };

        for rail in [north, south] {
            g.stroke_segment(Axis2D::X, xs..xe, rail, weight);
        }

        let (arm_lo, arm_hi) = if face_down {
            (0, (south + half).min(g.size.y))
        } else {
            (north.saturating_sub(half), g.size.y)
        };
        g.stroke_segment(Axis2D::Y, arm_lo..arm_hi, mid_x, weight);
    }

    pub(super) fn vertical_corner(g: &mut Glyph, weight: Stroke, corner_at: u32) {
        let face_up = corner_at & Edge2D::Top as u32 != 0;
        let face_right = corner_at & Edge2D::Right as u32 != 0;
        let off = g.stroke_thickness(weight);
        let half = off / 2;
        let mid_x = g.size.x / 2;
        let mid_y = g.size.y / 2;
        let west = mid_x.saturating_sub(off);
        let east = mid_x + off;
        let (ys, ye) = if face_up { (mid_y, g.size.y) } else { (0, mid_y) };

        for rail in [west, east] {
            g.stroke_segment(Axis2D::Y, ys..ye, rail, weight);
        }

        let (arm_lo, arm_hi) = if face_right {
            (0, (east + half).min(g.size.x))
        } else {
            (west.saturating_sub(half), g.size.x)
        };
        g.stroke_segment(Axis2D::X, arm_lo..arm_hi, mid_y, weight);
    }

    pub(super) fn with_arm(g: &mut Glyph, weight: Stroke, arm_dir: Direction2D) {
        let off = g.stroke_thickness(weight);
        let arm_is_horizontal = matches!(arm_dir, Direction2D::Left | Direction2D::Right);
        if arm_is_horizontal {
            let cx = g.size.x / 2;
            let west = cx.saturating_sub(off);
            let east = cx + off;
            g.stroke_segment(Axis2D::Y, 0..g.size.y, west, weight);
            g.stroke_segment(Axis2D::Y, 0..g.size.y, east, weight);
            let arm_span = if matches!(arm_dir, Direction2D::Left) {
                (0, west)
            } else {
                (east, g.size.x)
            };
            g.stroke_segment(Axis2D::X, arm_span.0..arm_span.1, g.size.y / 2, weight);
        } else {
            let cy = g.size.y / 2;
            let north = cy.saturating_sub(off);
            let south = cy + off;
            g.stroke_segment(Axis2D::X, 0..g.size.x, north, weight);
            g.stroke_segment(Axis2D::X, 0..g.size.x, south, weight);
            let arm_span = if matches!(arm_dir, Direction2D::Up) {
                (0, north)
            } else {
                (south, g.size.y)
            };
            g.stroke_segment(Axis2D::Y, arm_span.0..arm_span.1, g.size.x / 2, weight);
        }
    }

    pub(super) fn corner(g: &mut Glyph, opens: u32) {
        let stroke = g.stroke_thickness(Stroke::Light);
        let trail = stroke / 2;
        let lead = stroke.div_ceil(2);
        let cx = g.size.x / 2;
        let cy = g.size.y / 2;
        let max_x = g.size.x.saturating_sub(1);
        let max_y = g.size.y.saturating_sub(1);
        let opens_east = opens & Edge2D::Right as u32 != 0;
        let opens_south = opens & Edge2D::Bottom as u32 != 0;

        for near_open in [false, true] {
            let px = if near_open == opens_east {
                (cx + stroke).min(max_x)
            } else {
                cx.saturating_sub(stroke)
            };
            let py = if near_open == opens_south {
                (cy + stroke).min(max_y)
            } else {
                cy.saturating_sub(stroke)
            };
            let (hx_lo, hx_hi) = if opens_east {
                (px.saturating_sub(trail), g.size.x)
            } else {
                (0, px + lead)
            };
            let (vy_lo, vy_hi) = if opens_south {
                (py.saturating_sub(trail), g.size.y)
            } else {
                (0, py + lead)
            };
            g.stroke_segment(Axis2D::X, hx_lo..hx_hi, py, Stroke::Light);
            g.stroke_segment(Axis2D::Y, vy_lo..vy_hi, px, Stroke::Light);
        }
    }

    pub(super) fn inner_corner(g: &mut Glyph, weight: Stroke, corner_at: u32) {
        let thickness = g.stroke_thickness(weight);
        let half = thickness / 2;
        let cx = g.size.x / 2;
        let cy = g.size.y / 2;

        let sx = if corner_at & Edge2D::Left as u32 != 0 { Sign::Negative } else { Sign::Positive };
        let sy = if corner_at & Edge2D::Top as u32 != 0 { Sign::Negative } else { Sign::Positive };

        let v_center = (cx as i32 + sx.delta() * thickness as i32).max(0) as u32;
        let h_center = (cy as i32 + sy.delta() * thickness as i32).max(0) as u32;
        let v_lo = v_center.saturating_sub(half);
        let v_hi = (v_lo + thickness).min(g.size.x);
        let h_lo = h_center.saturating_sub(half);
        let h_hi = (h_lo + thickness).min(g.size.y);

        let (hx_lo, hx_hi) = if sx.is_negative() { (0, v_hi) } else { (v_lo, g.size.x) };
        let (vy_lo, vy_hi) = if sy.is_negative() { (0, h_hi) } else { (h_lo, g.size.y) };
        g.fill_rect(Rect::xywh(hx_lo, h_lo, hx_hi - hx_lo, h_hi - h_lo));
        g.fill_rect(Rect::xywh(v_lo, vy_lo, v_hi - v_lo, vy_hi - vy_lo));
    }
}

mod eighths {
    use super::*;

    pub(super) fn boundary(total: u32, index: u32, count: u32) -> u32 {
        let count = count.max(1);
        let extra = total % count;
        let thick_lo = count / 2 - extra.div_ceil(2);
        let added = index.saturating_sub(thick_lo).min(extra);
        index * (total / count) + added
    }

    pub(super) fn band(total: u32, index: u32, count: u32) -> (u32, u32) {
        (boundary(total, index, count), boundary(total, index + 1, count))
    }

    pub(super) fn lower(g: &mut Glyph, count: u32) {
        let top = boundary(g.size.y, 8 - count, 8);
        g.fill_rect(Rect::xywh(0, top, g.size.x, g.size.y - top));
    }

    pub(super) fn upper(g: &mut Glyph, count: u32) {
        let bot = boundary(g.size.y, count, 8);
        g.fill_rect(Rect::xywh(0, 0, g.size.x, bot));
    }

    pub(super) fn left(g: &mut Glyph, count: u32) {
        let right = boundary(g.size.x, count, 8);
        g.fill_rect(Rect::xywh(0, 0, right, g.size.y));
    }

    pub(super) fn right(g: &mut Glyph, count: u32) {
        let left = boundary(g.size.x, 8 - count, 8);
        g.fill_rect(Rect::xywh(left, 0, g.size.x - left, g.size.y));
    }

    pub(super) fn bar(g: &mut Glyph, index: u32, horizontal: bool) {
        if horizontal {
            let (start, end) = band(g.size.y, index, 8);
            g.fill_rect(Rect::xywh(0, start, g.size.x, end - start));
        } else {
            let (start, end) = band(g.size.x, index, 8);
            g.fill_rect(Rect::xywh(start, 0, end - start, g.size.y));
        }
    }
}

mod octant {
    use super::*;

    pub(super) fn draw(g: &mut Glyph, index: u8) {
        let segments = SEGMENTS[index as usize];
        let mid = g.size.x / 2;
        for k in 0..8u32 {
            if (segments >> k) & 1 == 0 {
                continue;
            }
            let (x_min, x_max) = match k & 1 {
                0 => (0, mid),
                _ => (mid, g.size.x),
            };
            let (y_min, y_max) = eighths::band(g.size.y, k >> 1, 4);
            g.fill_rect(Rect::xywh(x_min, y_min, x_max - x_min, y_max - y_min));
        }
    }

    const SEGMENTS: [u8; 232] = {
        let mut table = [0u8; 232];
        let mut bm: u16 = 0;
        let mut idx = 0;
        while bm < 256 {
            if !is_excluded(bm as u8) {
                table[idx] = bm as u8;
                idx += 1;
            }
            bm += 1;
        }
        table[230] = 0x14;
        table[231] = 0x28;
        table
    };

    const fn is_excluded(bm: u8) -> bool {
        if ((bm & 0x33) << 2) == (bm & 0xCC) {
            return true;
        }
        if bm == 0x03 || bm == 0xC0 || bm == 0x3F || bm == 0xFC {
            return true;
        }
        if bm == 0x01 || bm == 0x02 || bm == 0x40 || bm == 0x80 {
            return true;
        }
        bm == 0x14 || bm == 0x28
    }
}

mod sextant {
    use super::*;

    pub(super) fn draw(g: &mut Glyph, index: u32) {
        let segments = SEGMENTS[index as usize];
        let third = g.size.y / 3;
        let two_thirds = 2 * g.size.y / 3;
        let mid = g.size.x / 2;
        for k in 0..6u32 {
            if (segments >> k) & 1 == 0 {
                continue;
            }
            let (y_min, y_max) = match k >> 1 {
                0 => (0, third),
                1 => (third, two_thirds),
                _ => (two_thirds, g.size.y),
            };
            let (x_min, x_max) = match k & 1 {
                0 => (0, mid),
                _ => (mid, g.size.x),
            };
            g.fill_rect(Rect::xywh(x_min, y_min, x_max - x_min, y_max - y_min));
        }
    }

    const SEGMENTS: [u8; 60] = {
        let mut table = [0u8; 60];
        let mut bm: u8 = 1;
        let mut idx = 0;
        while bm < 0x3F {
            if bm != 0x15 && bm != 0x2A {
                table[idx] = bm;
                idx += 1;
            }
            bm += 1;
        }
        table
    };
}

mod braille {
    use super::*;

    const DOT_GRID: [(u32, u32); 8] = [
        (0, 0), (0, 1), (0, 2),
        (1, 0), (1, 1), (1, 2),
        (0, 3), (1, 3),
    ];

    pub(super) fn draw(g: &mut Glyph, ch: char) {
        let mask = (ch as u32) - 0x2800;
        for (bit, &(col, row)) in DOT_GRID.iter().enumerate() {
            if mask & (1 << bit) == 0 {
                continue;
            }
            dot(g, col, row);
        }
    }

    fn layout(track: u32, dots: u32) -> ([u32; 4], u32) {
        let size = (track / (2 * dots)).max(1);
        let mut starts = [0u32; 4];
        for (i, start) in starts.iter_mut().take(dots as usize).enumerate() {
            let center = ((2 * i as u32 + 1) * track) / (2 * dots);
            *start = center.saturating_sub(size / 2);
        }
        (starts, size)
    }

    fn dot(g: &mut Glyph, col: u32, row: u32) {
        let (xs, w) = layout(g.size.x, 2);
        let (ys, h) = layout(g.size.y, 4);
        let sx = xs[col as usize];
        let sy = ys[row as usize];
        if sx >= g.size.x || sy >= g.size.y {
            return;
        }
        let w = w.min(g.size.x - sx);
        let h = h.min(g.size.y - sy);
        g.fill_rect(Rect::xywh(sx, sy, w, h));
    }
}

mod powerline {
    //! Powerline and related separator glyph drawing.
    use super::*;

    pub(super) fn triangle(g: &mut Glyph, apex_right: bool, inverted: bool) {
        let w = g.size.x;
        let h = g.size.y;
        if w == 0 || h == 0 {
            return;
        }
        let last_x = (w - 1) as f64;
        let last_y = (h - 1) as f64;
        let mid_y = last_y / 2.0;
        let (base_x, apex_x) = if apex_right {
            (0.0, last_x)
        } else {
            (last_x, 0.0)
        };

        let interior = (
            (2.0 * base_x + apex_x) / 3.0,
            mid_y,
        );
        let upper = oriented_line((base_x, 0.0), (apex_x, mid_y), interior);
        let lower = oriented_line((base_x, last_y), (apex_x, mid_y), interior);

        for y in 0..h {
            let py = y as f64 + 0.5;
            for x in 0..w {
                let px = x as f64 + 0.5;
                let cu = (upper.0 * px + upper.1 * py + upper.2 + 0.5).clamp(0.0, 1.0);
                let cl = (lower.0 * px + lower.1 * py + lower.2 + 0.5).clamp(0.0, 1.0);
                let cov = cu.min(cl);
                if inverted {
                    let alpha = ((1.0 - cov) * 255.0).round() as u8;
                    let idx = (y * g.size.x + x) as usize;
                    g.pixels[idx] = g.pixels[idx].max(alpha);
                } else if cov > 0.0 {
                    g.put_max(x, y, (cov * 255.0).round() as u8);
                }
            }
        }
    }

    fn oriented_line(a: (f64, f64), b: (f64, f64), interior: (f64, f64)) -> (f64, f64, f64) {
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-9 {
            return (0.0, 0.0, 0.0);
        }
        let nx = -dy / len;
        let ny = dx / len;
        let c = -(nx * a.0 + ny * a.1);
        if nx * interior.0 + ny * interior.1 + c < 0.0 {
            (-nx, -ny, -c)
        } else {
            (nx, ny, c)
        }
    }

    pub(super) fn chevron(g: &mut Glyph, right_pointing: bool) {
        let w = g.size.x;
        let h = g.size.y;
        if w == 0 || h == 0 {
            return;
        }
        let max_x = w as f64 - 0.5;
        let max_y = h as f64 - 0.5;
        let mid_y = ((h - 1) / 2) as f64 + 0.5;
        let half_thick = g.stroke_thickness(Stroke::Light) as f64 * 0.5;
        let edge = half_thick + 0.5;
        let half_h = mid_y - 0.5;
        let span_x = max_x - 0.5;
        let arm_len = (span_x * span_x + half_h * half_h).sqrt();
        let overshoot = if half_h > 1e-9 { edge * arm_len / half_h } else { 0.0 };
        let (apex_x, open_x) = if right_pointing {
            ((max_x - overshoot).max(0.5), 0.5)
        } else {
            ((0.5 + overshoot).min(max_x), max_x)
        };
        let top_a = (open_x, 0.5);
        let top_b = (apex_x, mid_y);
        let bot_a = (apex_x, mid_y);
        let bot_b = (open_x, max_y);
        let top = super::line_coeffs(top_a, top_b);
        let bot = super::line_coeffs(bot_a, bot_b);

        for y in 0..h {
            let py = y as f64 + 0.5;
            let line = if py <= mid_y { top } else { bot };
            for x in 0..w {
                let px = x as f64 + 0.5;
                let d = (line.0 * px + line.1 * py + line.2).abs();
                let cov = (edge - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    g.put_max(x, y, (cov * 255.0).round() as u8);
                }
            }
        }
    }

    pub(super) fn filled_d(g: &mut Glyph, bulge_right: bool) {
        let cy = g.size.y as f64 / 2.0;
        let cx = if bulge_right { 0.0 } else { g.size.x as f64 };
        let rx = g.size.x as f64;
        let ry = g.size.y as f64 / 2.0;
        if rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let inv_rx2 = 1.0 / (rx * rx);
        let inv_ry2 = 1.0 / (ry * ry);

        for y in 0..g.size.y {
            let dy = (y as f64 + 0.5) - cy;
            let dy_term = dy * dy * inv_ry2;
            for x in 0..g.size.x {
                let dx = (x as f64 + 0.5) - cx;
                let f = dx * dx * inv_rx2 + dy_term - 1.0;
                let gx = 2.0 * dx * inv_rx2;
                let gy = 2.0 * dy * inv_ry2;
                let g_mag = (gx * gx + gy * gy).sqrt();
                let signed = if g_mag > 1e-9 { f / g_mag } else { f };
                let coverage = (0.5 - signed).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    g.put_max(x, y, (coverage * 255.0).round() as u8);
                }
            }
        }
    }

    pub(super) fn rounded_separator(g: &mut Glyph, bulge_right: bool) {
        let cy = g.size.y as f64 / 2.0;
        let cx = if bulge_right { 0.0 } else { g.size.x as f64 };
        let thick = g.stroke_thickness(Stroke::Light) as f64;
        let rx = (g.size.x as f64 - thick * 0.5).max(0.0);
        let ry = (g.size.y as f64 / 2.0 - thick * 0.5).max(0.0);
        if rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let inv_rx2 = 1.0 / (rx * rx);
        let inv_ry2 = 1.0 / (ry * ry);
        let half_thick = thick * 0.5;

        for y in 0..g.size.y {
            let dy = (y as f64 + 0.5) - cy;
            let dy_term = dy * dy * inv_ry2;
            for x in 0..g.size.x {
                let dx = (x as f64 + 0.5) - cx;
                let f = dx * dx * inv_rx2 + dy_term - 1.0;
                let gx = 2.0 * dx * inv_rx2;
                let gy = 2.0 * dy * inv_ry2;
                let g_mag = (gx * gx + gy * gy).sqrt();
                if g_mag < 1e-9 {
                    continue;
                }
                let signed = f / g_mag;
                let edge_distance = signed.abs() - half_thick;
                let coverage = (0.5 - edge_distance).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    g.put_max(x, y, (coverage * 255.0).round() as u8);
                }
            }
        }
    }

    pub(super) fn corner_triangle(g: &mut Glyph, corner: u32) {
        let w = g.size.x;
        let h = g.size.y;
        if w == 0 || h == 0 {
            return;
        }
        let last_x = (w - 1) as f64;
        let last_y = (h - 1) as f64;
        let top = corner & Edge2D::Top as u32 != 0;
        let left = corner & Edge2D::Left as u32 != 0;
        let (a, b) = if top == left {
            ((last_x, 0.0), (0.0, last_y))
        } else {
            ((0.0, 0.0), (last_x, last_y))
        };
        let kept = (
            if left { 0.0 } else { last_x },
            if top { 0.0 } else { last_y },
        );
        let edge = oriented_line(a, b, kept);

        for y in 0..h {
            let py = y as f64 + 0.5;
            for x in 0..w {
                let px = x as f64 + 0.5;
                let cov = (edge.0 * px + edge.1 * py + edge.2 + 0.5).clamp(0.0, 1.0);
                if cov > 0.0 {
                    g.put_max(x, y, (cov * 255.0).round() as u8);
                }
            }
        }
    }
}

mod arrow {
    //! Cardinal and dashed arrow glyph drawing.
    use super::*;

    struct Layout {
        axis: Axis2D,
        lo: f64,
        hi: f64,
        perp: f64,
    }

    fn layout(g: &Glyph, axis: Axis2D) -> Layout {
        let w = g.size.x as f64;
        let h = g.size.y as f64;
        let s = w.min(h);
        let (along, across) = match axis {
            Axis2D::X => (w, h),
            Axis2D::Y => (h, w),
        };
        let lo = (along - s) * 0.5 + 0.5;
        let hi = (along + s) * 0.5 - 0.5;
        let perp = across * 0.5;
        Layout { axis, lo, hi, perp }
    }

    pub(super) fn single(g: &mut Glyph, dir: Direction2D, dashed: bool) {
        let axis = match dir {
            Direction2D::Left | Direction2D::Right => Axis2D::X,
            Direction2D::Up | Direction2D::Down => Axis2D::Y,
        };
        let lo = layout(g, axis);
        let pointing_far = matches!(dir, Direction2D::Right | Direction2D::Down);
        let mid = (lo.lo + lo.hi) * 0.5;
        let apex = if pointing_far { lo.hi } else { lo.lo };
        stake(g, &lo, apex, dashed);
        head(g, &lo, apex, mid);
    }

    fn stake(g: &mut Glyph, lo: &Layout, apex_along: f64, dashed: bool) {
        let half = g.stroke_thickness(Stroke::Light) as f64 * 0.5;
        let edge = half + 0.5;

        let mid = (lo.lo + lo.hi) * 0.5;
        let pointing_far = apex_along > mid;
        let tail = if pointing_far { lo.lo } else { lo.hi };
        let dir: f64 = if pointing_far { 1.0 } else { -1.0 };
        let apex_along = apex_along - dir * 2.0 * half;
        let total_t = (apex_along - tail).abs();
        if total_t < 0.5 {
            return;
        }
        let cap_t = (total_t - half).max(0.0);
        let cap_len = (total_t - cap_t).max(1e-6);

        let dot = g.stroke_thickness(Stroke::Light) as f64;
        let period = (dot * 2.0).max(1e-6);
        let in_dash = |t: f64| -> bool {
            if !dashed {
                return true;
            }
            let from_end = cap_t - t;
            let phase = from_end - (from_end / period).floor() * period;
            phase < dot
        };

        for y in 0..g.size.y {
            let py = y as f64 + 0.5;
            for x in 0..g.size.x {
                let px = x as f64 + 0.5;
                let (along, across) = match lo.axis {
                    Axis2D::X => (px, py),
                    Axis2D::Y => (py, px),
                };
                let t = (along - tail) * dir;
                if t < -0.5 || t > total_t {
                    continue;
                }
                let dp = (across - lo.perp).abs();
                let perp_half = if t <= cap_t {
                    half
                } else {
                    (half * (1.0 - (t - cap_t) / cap_len)).max(0.0)
                };
                let cov_perp = (perp_half + 0.5 - dp).clamp(0.0, 1.0);
                let cov_tail = if t < 0.0 {
                    (edge + t).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let dash_ok = if t <= cap_t {
                    in_dash(t.max(0.0))
                } else {
                    true
                };
                if !dash_ok {
                    continue;
                }
                let cov = cov_perp.min(cov_tail);
                if cov > 0.0 {
                    g.put_max(x, y, (cov * 255.0).round() as u8);
                }
            }
        }
    }

    fn head(g: &mut Glyph, lo: &Layout, apex_along: f64, base_along: f64) {
        let half_thick = g.stroke_thickness(Stroke::Light) as f64 * 0.5;
        let edge = half_thick + 0.5;

        let half_ext = (lo.hi - lo.lo) * 0.5;
        let base_to_apex = (apex_along - base_along).abs();
        if base_to_apex < 1e-6 {
            return;
        }
        let arm_len = (base_to_apex * base_to_apex + half_ext * half_ext).sqrt();
        let overshoot = edge * arm_len / half_ext;
        let dir_sign = if apex_along > base_along { -1.0 } else { 1.0 };
        let apex = apex_along + dir_sign * overshoot;

        let base_a = (base_along, lo.perp - half_ext);
        let base_b = (base_along, lo.perp + half_ext);
        let apex_pt = (apex, lo.perp);
        let line_a = super::line_coeffs(apex_pt, base_a);
        let line_b = super::line_coeffs(apex_pt, base_b);

        let tan_a = unit_tangent(apex_pt, base_a);
        let tan_b = unit_tangent(apex_pt, base_b);

        for y in 0..g.size.y {
            let py = y as f64 + 0.5;
            for x in 0..g.size.x {
                let px = x as f64 + 0.5;
                let (along, across) = match lo.axis {
                    Axis2D::X => (px, py),
                    Axis2D::Y => (py, px),
                };
                let (line, tan, base_pt) = if across <= lo.perp {
                    (line_a, tan_a, base_a)
                } else {
                    (line_b, tan_b, base_b)
                };
                let d = (line.0 * along + line.1 * across + line.2).abs();
                let cov_perp = (edge - d).clamp(0.0, 1.0);
                let proj = (along - base_pt.0) * tan.0 + (across - base_pt.1) * tan.1;
                let cov_base = (edge - proj).clamp(0.0, 1.0);
                let cov = cov_perp.min(cov_base);
                if cov > 0.0 {
                    g.put_max(x, y, (cov * 255.0).round() as u8);
                }
            }
        }
    }

    fn unit_tangent(from: (f64, f64), to: (f64, f64)) -> (f64, f64) {
        let dx = to.0 - from.0;
        let dy = to.1 - from.1;
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        (dx / len, dy / len)
    }
}

mod diagonal {
    use super::*;

    pub(super) fn draw(g: &mut Glyph, top_right: bool) {
        let w = g.size.x as f64;
        let h = g.size.y as f64;
        let half_thick = g.stroke_thickness(Stroke::Light) as f64 * 0.5;
        let edge = half_thick + 0.5;

        let (a, b) = if top_right {
            ((-0.5, h - 0.5), (w - 0.5, -0.5))
        } else {
            ((-0.5, -0.5), (w - 0.5, h - 0.5))
        };
        let (nx, ny, c) = super::line_coeffs(a, b);

        for y in 0..g.size.y {
            let py = y as f64 + 0.5;
            for x in 0..g.size.x {
                let px = x as f64 + 0.5;
                let d = (nx * px + ny * py + c).abs();
                if d > edge {
                    continue;
                }
                let alpha = ((edge - d).clamp(0.0, 1.0) * 255.0).round() as u8;
                g.put_max(x, y, alpha);
            }
        }
    }
}
