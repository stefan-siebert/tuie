//! Box-drawing border types and junction utilities.

use crate::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Arms {
    None = 0b00,
    Neg = 0b01,
    Pos = 0b10,
    Both = 0b11,
}

impl Arms {
    const fn from_bools(neg: bool, pos: bool) -> Self {
        match (neg, pos) {
            (false, false) => Self::None,
            (true, false) => Self::Neg,
            (false, true) => Self::Pos,
            (true, true) => Self::Both,
        }
    }
}

/// Box-drawing character set indexed by which arms meet at each cell.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Border([char; 16]);

impl Border {
    /// Creates a border from its fifteen junction glyphs.
    pub const fn new(
        horizontal: char,
        vertical: char,
        top_left: char,
        top_right: char,
        bottom_left: char,
        bottom_right: char,
        t_down: char,
        t_up: char,
        t_right: char,
        t_left: char,
        cross: char,
        left_stub: char,
        right_stub: char,
        up_stub: char,
        down_stub: char,
    ) -> Self {
        Border([
            ' ',
            left_stub,
            right_stub,
            horizontal,
            up_stub,
            bottom_right,
            bottom_left,
            t_up,
            down_stub,
            top_right,
            top_left,
            t_down,
            vertical,
            t_left,
            t_right,
            cross,
        ])
    }

    #[inline]
    fn junction(&self, arms: Vec2<Arms>) -> char {
        self.0[(arms.y as usize) << 2 | arms.x as usize]
    }

    /// Returns the glyph for a straight edge running along the axis perpendicular to `side_axis`.
    #[inline]
    pub fn get_edge(&self, side_axis: Axis2D) -> char {
        let mut arms = Vec2::of(Arms::None);
        arms[side_axis.flip()] = Arms::Both;
        self.junction(arms)
    }

    /// Returns the glyph for a corner where each component of `end` selects the positive arm on that axis.
    #[inline]
    pub fn get_corner(&self, end: Vec2<bool>) -> char {
        self.junction(end.map(|e| Arms::from_bools(e, !e)))
    }

    /// Returns the glyph for the junction with the given arms present.
    #[inline]
    pub fn get_arms(&self, left: bool, right: bool, up: bool, down: bool) -> char {
        self.junction(Vec2::new(
            Arms::from_bools(left, right),
            Arms::from_bools(up, down),
        ))
    }

    /// Returns true when this border has non-blank single-arm stubs at both ends along `axis`.
    pub fn has_stubs(&self, axis: Axis2D) -> bool {
        let (leading, trailing) = match axis {
            Axis2D::Y => (self.get_arms(false, false, false, true), self.get_arms(false, false, true, false)),
            Axis2D::X => (self.get_arms(false, true, false, false), self.get_arms(true, false, false, false)),
        };
        leading != ' ' && trailing != ' '
    }

    /// Single-line box-drawing border with sharp corners.
    pub const SINGLE: &'static Border = &Border::new(
        '─', '│', '┌', '┐', '└', '┘', '┬', '┴', '├', '┤', '┼', '╴', '╶', '╵', '╷',
    );

    /// Double-line box-drawing border.
    pub const DOUBLE: &'static Border = &Border::new(
        '═', '║', '╔', '╗', '╚', '╝', '╦', '╩', '╠', '╣', '╬', ' ', ' ', ' ', ' ',
    );

    /// Heavy single-line box-drawing border.
    pub const THICK: &'static Border = &Border::new(
        '━', '┃', '┏', '┓', '┗', '┛', '┳', '┻', '┣', '┫', '╋', '╸', '╺', '╹', '╻',
    );

    /// Light single-line border with rounded corners.
    pub const ROUND: &'static Border = &Border::new(
        '─', '│', '╭', '╮', '╰', '╯', '┬', '┴', '├', '┤', '┼', '╴', '╶', '╵', '╷',
    );

    /// Light dashed border with sharp corners.
    pub const DASHED: &'static Border = &Border::new(
        '╌', '┊', '┌', '┐', '└', '┘', '┬', '┴', '├', '┤', '┼', '╴', '╶', '╵', '╷',
    );

    /// Heavy dashed border.
    pub const THICK_DASHED: &'static Border = &Border::new(
        '╍', '┋', '┏', '┓', '┗', '┛', '┳', '┻', '┣', '┫', '╋', '╸', '╺', '╹', '╻',
    );

    /// Plain ASCII border using `-`, `|`, and `+`. Use when Unicode is unavailable.
    pub const ASCII: &'static Border = &Border::new(
        '-', '|', '+', '+', '+', '+', '+', '+', '+', '+', '+', ' ', ' ', ' ', ' ',
    );

    /// Invisible border. Every glyph is a space.
    pub const HIDDEN: &'static Border = &Border::new(
        ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' ', ' ',
    );
}

/// Default border glyph sets and styles applied when widgets render frames.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct BorderConfig {
    /// Glyph set used for unselected widget borders.
    pub border: &'static Border,
    /// Glyph set used for selected widget borders.
    pub selected_border: &'static Border,
    /// Style applied to unselected borders.
    pub style: Style,
    /// Style applied to selected borders.
    pub selected_style: Style,
}

crate::config_module!(BorderConfig {
    border: Border::SINGLE,
    selected_border: Border::THICK,
    style: Style::new(),
    selected_style: Style::new().bold(),
});

#[repr(u8)]
enum BorderWeight {
    None = 0,
    Light = 1,
    Heavy = 2,
    Double = 3,
}

use BorderWeight::*;

const ENTRIES: &[(char, u8)] = &[
    ('─', key(Light, Light, None, None)),
    ('━', key(Heavy, Heavy, None, None)),
    ('═', key(Double, Double, None, None)),
    ('│', key(None, None, Light, Light)),
    ('┃', key(None, None, Heavy, Heavy)),
    ('║', key(None, None, Double, Double)),
    ('┌', key(None, Light, None, Light)),
    ('┍', key(None, Heavy, None, Light)),
    ('┎', key(None, Light, None, Heavy)),
    ('┏', key(None, Heavy, None, Heavy)),
    ('╒', key(None, Double, None, Light)),
    ('╓', key(None, Light, None, Double)),
    ('╔', key(None, Double, None, Double)),
    ('┐', key(Light, None, None, Light)),
    ('┑', key(Heavy, None, None, Light)),
    ('┒', key(Light, None, None, Heavy)),
    ('┓', key(Heavy, None, None, Heavy)),
    ('╕', key(Double, None, None, Light)),
    ('╖', key(Light, None, None, Double)),
    ('╗', key(Double, None, None, Double)),
    ('└', key(None, Light, Light, None)),
    ('┕', key(None, Heavy, Light, None)),
    ('┖', key(None, Light, Heavy, None)),
    ('┗', key(None, Heavy, Heavy, None)),
    ('╘', key(None, Double, Light, None)),
    ('╙', key(None, Light, Double, None)),
    ('╚', key(None, Double, Double, None)),
    ('┘', key(Light, None, Light, None)),
    ('┙', key(Heavy, None, Light, None)),
    ('┚', key(Light, None, Heavy, None)),
    ('┛', key(Heavy, None, Heavy, None)),
    ('╛', key(Double, None, Light, None)),
    ('╜', key(Light, None, Double, None)),
    ('╝', key(Double, None, Double, None)),
    ('╭', key(None, Light, None, Light)),
    ('╮', key(Light, None, None, Light)),
    ('╯', key(Light, None, Light, None)),
    ('╰', key(None, Light, Light, None)),
    ('├', key(None, Light, Light, Light)),
    ('┝', key(None, Heavy, Light, Light)),
    ('┞', key(None, Light, Heavy, Light)),
    ('┟', key(None, Light, Light, Heavy)),
    ('┠', key(None, Light, Heavy, Heavy)),
    ('┡', key(None, Heavy, Heavy, Light)),
    ('┢', key(None, Heavy, Light, Heavy)),
    ('┣', key(None, Heavy, Heavy, Heavy)),
    ('╞', key(None, Double, Light, Light)),
    ('╟', key(None, Light, Double, Double)),
    ('╠', key(None, Double, Double, Double)),
    ('┤', key(Light, None, Light, Light)),
    ('┥', key(Heavy, None, Light, Light)),
    ('┦', key(Light, None, Heavy, Light)),
    ('┧', key(Light, None, Light, Heavy)),
    ('┨', key(Light, None, Heavy, Heavy)),
    ('┩', key(Heavy, None, Heavy, Light)),
    ('┪', key(Heavy, None, Light, Heavy)),
    ('┫', key(Heavy, None, Heavy, Heavy)),
    ('╡', key(Double, None, Light, Light)),
    ('╢', key(Light, None, Double, Double)),
    ('╣', key(Double, None, Double, Double)),
    ('┬', key(Light, Light, None, Light)),
    ('┭', key(Heavy, Light, None, Light)),
    ('┮', key(Light, Heavy, None, Light)),
    ('┯', key(Heavy, Heavy, None, Light)),
    ('┰', key(Light, Light, None, Heavy)),
    ('┱', key(Heavy, Light, None, Heavy)),
    ('┲', key(Light, Heavy, None, Heavy)),
    ('┳', key(Heavy, Heavy, None, Heavy)),
    ('╤', key(Double, Double, None, Light)),
    ('╥', key(Light, Light, None, Double)),
    ('╦', key(Double, Double, None, Double)),
    ('┴', key(Light, Light, Light, None)),
    ('┵', key(Heavy, Light, Light, None)),
    ('┶', key(Light, Heavy, Light, None)),
    ('┷', key(Heavy, Heavy, Light, None)),
    ('┸', key(Light, Light, Heavy, None)),
    ('┹', key(Heavy, Light, Heavy, None)),
    ('┺', key(Light, Heavy, Heavy, None)),
    ('┻', key(Heavy, Heavy, Heavy, None)),
    ('╧', key(Double, Double, Light, None)),
    ('╨', key(Light, Light, Double, None)),
    ('╩', key(Double, Double, Double, None)),
    ('┼', key(Light, Light, Light, Light)),
    ('┽', key(Heavy, Light, Light, Light)),
    ('┾', key(Light, Heavy, Light, Light)),
    ('┿', key(Heavy, Heavy, Light, Light)),
    ('╀', key(Light, Light, Heavy, Light)),
    ('╁', key(Light, Light, Light, Heavy)),
    ('╂', key(Light, Light, Heavy, Heavy)),
    ('╃', key(Heavy, Light, Heavy, Light)),
    ('╄', key(Light, Heavy, Heavy, Light)),
    ('╅', key(Heavy, Light, Light, Heavy)),
    ('╆', key(Light, Heavy, Light, Heavy)),
    ('╇', key(Heavy, Heavy, Heavy, Light)),
    ('╈', key(Heavy, Heavy, Light, Heavy)),
    ('╉', key(Heavy, Light, Heavy, Heavy)),
    ('╊', key(Light, Heavy, Heavy, Heavy)),
    ('╋', key(Heavy, Heavy, Heavy, Heavy)),
    ('╪', key(Double, Double, Light, Light)),
    ('╫', key(Light, Light, Double, Double)),
    ('╬', key(Double, Double, Double, Double)),
    ('╴', key(Light, None, None, None)),
    ('╵', key(None, None, Light, None)),
    ('╶', key(None, Light, None, None)),
    ('╷', key(None, None, None, Light)),
    ('╸', key(Heavy, None, None, None)),
    ('╹', key(None, None, Heavy, None)),
    ('╺', key(None, Heavy, None, None)),
    ('╻', key(None, None, None, Heavy)),
    ('╼', key(Light, Heavy, None, None)),
    ('╽', key(None, None, Light, Heavy)),
    ('╾', key(Heavy, Light, None, None)),
    ('╿', key(None, None, Heavy, Light)),
];

const EMPTY: u8 = 0xFF;

const fn key(
    left: BorderWeight,
    right: BorderWeight,
    up: BorderWeight,
    down: BorderWeight
) -> u8 {
    ((left as u8) << 6)
        | ((right as u8) << 4)
        | ((up as u8) << 2)
        | (down as u8)
}

const fn build_char_to_key() -> [u8; 128] {
    let mut t = [EMPTY; 128];
    let mut i = 0;
    while i < ENTRIES.len() {
        let (c, k) = ENTRIES[i];
        t[c as usize - 0x2500] = k;
        i += 1;
    }
    t
}

const fn build_key_to_char() -> [Option<char>; 256] {
    let mut t = [Option::None; 256];
    let mut i = 0;
    while i < ENTRIES.len() {
        let (c, k) = ENTRIES[i];
        if t[k as usize].is_none() {
            t[k as usize] = Some(c);
        }
        i += 1;
    }
    t
}

static CHAR_TO_KEY: [u8; 128] = build_char_to_key();
static KEY_TO_CHAR: [Option<char>; 256] = build_key_to_char();

fn decode(c: char) -> Option<u8> {
    match c {
        ' ' => return Some(0),
        '-' => return Some(key(Light, Light, None, None)),
        '|' => return Some(key(None, None, Light, Light)),
        '+' => return Some(key(Light, Light, Light, Light)),
        _ => {}
    }
    let i = (c as u32).checked_sub(0x2500)? as usize;
    if i >= 128 {
        return Option::None;
    }
    let k = CHAR_TO_KEY[i];
    if k == EMPTY {
        Option::None
    } else {
        Some(k)
    }
}

fn is_ascii_border(c: char) -> bool {
    matches!(c, ' ' | '-' | '|' | '+')
}

fn ascii_glyph(k: u8) -> char {
    let h = (k >> 6) & 0b11 != 0 || (k >> 4) & 0b11 != 0;
    let v = (k >> 2) & 0b11 != 0 || k & 0b11 != 0;
    match (h, v) {
        (false, false) => ' ',
        (true, false) => '-',
        (false, true) => '|',
        (true, true) => '+',
    }
}

/// Returns the glyph for a junction with arms reaching toward each given neighbour border.
pub fn junction(
    left: &Border,
    right: &Border,
    up: &Border,
    down: &Border,
) -> Option<char> {
    let arms = [
        (left, left.get_edge(Axis2D::Y)),
        (right, right.get_edge(Axis2D::Y)),
        (up, up.get_edge(Axis2D::X)),
        (down, down.get_edge(Axis2D::X)),
    ];

    let mut shared: Option<&Border> = Option::None;
    let mut mixed = false;
    for (b, e) in arms {
        if e == ' ' {
            continue;
        }
        match shared {
            Some(s) if !std::ptr::eq(s, b) => {
                mixed = true;
                break;
            }
            _ => shared = Some(b),
        }
    }

    if !mixed {
        let b = shared.unwrap_or(Border::HIDDEN);
        let [(_, l), (_, r), (_, u), (_, d)] = arms;
        return Some(b.get_arms(l != ' ', r != ' ', u != ' ', d != ' '));
    }

    let mut k = 0u8;
    for (i, (_, e)) in arms.iter().enumerate() {
        k |= weight_of_edge(*e)? << (6 - i * 2);
    }
    KEY_TO_CHAR[k as usize]
}

fn weight_of_edge(c: char) -> Option<u8> {
    match c {
        ' ' => Some(None as u8),
        '─' | '│' | '╌' | '┊' | '-' | '|' => Some(Light as u8),
        '━' | '┃' | '╍' | '┋' => Some(Heavy as u8),
        '═' | '║' => Some(Double as u8),
        _ => Option::None,
    }
}

/// Returns the box-drawing glyph that combines the arms of `a` and `b`.
pub fn merge(a: char, b: char) -> char {
    let a_ascii = is_ascii_border(a);
    let b_ascii = is_ascii_border(b);
    match (a_ascii, b_ascii) {
        (true, false) => return b,
        (false, true) => return a,
        _ => {}
    }
    let (ka, kb) = match (decode(a), decode(b)) {
        (Some(ka), Some(kb)) => (ka, kb),
        _ => return b,
    };
    let mut out = 0u8;
    let mut shift = 0u8;
    while shift < 8 {
        let na = (ka >> shift) & 0b11;
        let nb = (kb >> shift) & 0b11;
        let m = if na > nb {
            na
        } else {
            nb
        };
        out |= m << shift;
        shift += 2;
    }
    if a_ascii && b_ascii {
        return ascii_glyph(out);
    }
    KEY_TO_CHAR[out as usize].unwrap_or(b)
}
