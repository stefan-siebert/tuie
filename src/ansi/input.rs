//! Incremental VT input parser.

use crate::prelude::*;
use std::collections::VecDeque;

use super::{ColorEntry, ColorScheme, ColorType, ParsedEvent, MouseInput};

/// State of the incremental parser between bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ground,
    Esc,
    Csi,
    NormalMouse,
    Ss3,
    Osc,
    Dcs,
    Apc,
    Paste,
    Utf8,
}

/// Incremental VT input parser.
pub struct Parser {
    state: State,
    buf: Vec<u8>,
    paste: Vec<u8>,
    utf8_buf: Vec<u8>,
    utf8_needed: usize,
    utf8_alt: bool,
    out: VecDeque<ParsedEvent>,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    /// Creates an empty parser in the ground state.
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            buf: Vec::with_capacity(32),
            paste: Vec::new(),
            utf8_buf: Vec::with_capacity(4),
            utf8_needed: 0,
            utf8_alt: false,
            out: VecDeque::with_capacity(16),
        }
    }

    /// Pops the next decoded event, if any are ready.
    pub fn next(&mut self) -> Option<ParsedEvent> {
        self.out.pop_front()
    }

    /// Whether at least one decoded event is queued.
    pub fn has_event(&self) -> bool {
        !self.out.is_empty()
    }

    /// Enqueues a [`ParsedEvent`] directly into the output queue.
    pub fn push_event(&mut self, event: ParsedEvent) {
        self.out.push_back(event);
    }

    /// Resolves a pending lone `ESC` or bare sequence introducer as a keypress.
    pub fn flush_escape(&mut self) {
        let b = match self.state {
            State::Esc => {
                self.state = State::Ground;
                self.push_key(Key::Esc, Modifiers::new());
                return;
            }
            State::Ss3 => b'O',
            State::Csi if self.buf.is_empty() => b'[',
            State::Dcs if self.buf.is_empty() => b'P',
            State::Osc if self.buf.is_empty() => b']',
            State::Apc if self.buf.is_empty() => b'_',
            _ => return,
        };
        self.state = State::Ground;
        self.ground(b, true);
    }

    /// Feeds one input byte, advancing the state machine.
    pub fn feed(&mut self, b: u8) {
        match self.state {
            State::Ground => self.ground(b, false),
            State::Esc => self.after_esc(b),
            State::Csi => self.csi(b),
            State::NormalMouse => self.normal_mouse(b),
            State::Ss3 => self.ss3(b),
            State::Osc => self.osc(b),
            State::Dcs => self.dcs(b),
            State::Apc => self.apc(b),
            State::Paste => self.paste_byte(b),
            State::Utf8 => self.utf8(b),
        }
    }

    /// Feeds a slice of bytes in order.
    pub fn feed_all(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.feed(b);
        }
    }

    fn ground(&mut self, b: u8, alt: bool) {
        match b {
            0x1B => self.state = State::Esc,
            b'\r' => self.push_key(Key::Enter, mods(false, alt, false)),
            b'\t' => self.push_key(Key::Tab, mods(false, alt, false)),
            0x7F => self.push_key(Key::Backspace, mods(false, alt, false)),
            0x00 => self.push_key(Key::Char(' '), mods(true, alt, false)),
            0x01..=0x1A => {
                let c = (b - 0x01 + b'a') as char;
                self.push_key(Key::Char(c), mods(true, alt, false));
            }
            0x1C..=0x1F => {
                let c = (b - 0x1C + b'4') as char;
                self.push_key(Key::Char(c), mods(true, alt, false));
            }
            0x20..=0x7E => self.push_key(Key::Char(b as char), mods(false, alt, false)),
            _ => self.start_utf8(b, alt),
        }
    }

    fn after_esc(&mut self, b: u8) {
        match b {
            b'[' => {
                self.state = State::Csi;
                self.buf.clear();
            }
            b'O' => {
                self.state = State::Ss3;
                self.buf.clear();
            }
            b'P' => {
                self.state = State::Dcs;
                self.buf.clear();
            }
            b'_' => {
                self.state = State::Apc;
                self.buf.clear();
            }
            b']' => {
                self.state = State::Osc;
                self.buf.clear();
            }
            0x1B => {
                self.state = State::Ground;
                self.push_key(Key::Esc, Modifiers::new());
            }
            _ => {
                self.state = State::Ground;
                self.ground(b, true);
            }
        }
    }

    fn start_utf8(&mut self, b: u8, alt: bool) {
        let needed = match b {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => return,
        };
        self.utf8_buf.clear();
        self.utf8_buf.push(b);
        self.utf8_needed = needed;
        self.utf8_alt = alt;
        self.state = State::Utf8;
    }

    fn utf8(&mut self, b: u8) {
        if b & 0b1100_0000 != 0b1000_0000 {
            self.state = State::Ground;
            self.utf8_buf.clear();
            self.feed(b);
            return;
        }
        self.utf8_buf.push(b);
        if self.utf8_buf.len() < self.utf8_needed {
            return;
        }
        let alt = self.utf8_alt;
        self.state = State::Ground;
        if let Ok(s) = std::str::from_utf8(&self.utf8_buf)
            && let Some(c) = s.chars().next()
        {
            self.push_key(Key::Char(c), mods(false, alt, false));
        }
        self.utf8_buf.clear();
    }

    fn ss3(&mut self, b: u8) {
        self.state = State::Ground;
        let key = match b {
            b'D' => Key::Arrow(Direction2D::Left),
            b'C' => Key::Arrow(Direction2D::Right),
            b'A' => Key::Arrow(Direction2D::Up),
            b'B' => Key::Arrow(Direction2D::Down),
            b'H' => Key::Home,
            b'F' => Key::End,
            b'P'..=b'S' => Key::F(1 + (b - b'P')),
            _ => return,
        };
        self.push_key(key, Modifiers::new());
    }

    fn csi(&mut self, b: u8) {
        if (0x20..=0x3F).contains(&b) {
            self.buf.push(b);
            return;
        }
        if (0x40..=0x7E).contains(&b) {
            if b == b'M' && self.buf.is_empty() {
                self.state = State::NormalMouse;
                self.buf.clear();
                return;
            }
            let body = std::mem::take(&mut self.buf);
            self.state = State::Ground;
            self.dispatch_csi(&body, b);
            return;
        }
        self.state = State::Ground;
        self.buf.clear();
    }

    fn dispatch_csi(&mut self, body: &[u8], last: u8) {
        if body.is_empty() {
            match last {
                b'A' => self.push_key(Key::Arrow(Direction2D::Up), Modifiers::new()),
                b'B' => self.push_key(Key::Arrow(Direction2D::Down), Modifiers::new()),
                b'C' => self.push_key(Key::Arrow(Direction2D::Right), Modifiers::new()),
                b'D' => self.push_key(Key::Arrow(Direction2D::Left), Modifiers::new()),
                b'H' => self.push_key(Key::Home, Modifiers::new()),
                b'F' => self.push_key(Key::End, Modifiers::new()),
                b'Z' => self.push_key(Key::Tab, Modifiers::new().with(Modifier::Shift)),
                b'I' => self.out.push_back(ParsedEvent::Focus(true)),
                b'O' => self.out.push_back(ParsedEvent::Focus(false)),
                b'P' => self.push_key(Key::F(1), Modifiers::new()),
                b'Q' => self.push_key(Key::F(2), Modifiers::new()),
                b'S' => self.push_key(Key::F(4), Modifiers::new()),
                _ => {}
            }
            return;
        }
        match body[0] {
            b'<' => self.parse_sgr_mouse(&body[1..], last),
            b'?' => self.parse_private(&body[1..], last),
            b'0'..=b'9' | b';' | b':' => match last {
                b'~' => self.parse_special_or_paste(body),
                b'u' => self.parse_csi_u(body),
                b'M' => self.parse_rxvt_mouse(body),
                b't' => self.parse_window_size(body),
                b'R' => {}
                b'A' | b'B' | b'C' | b'D' | b'F' | b'H' | b'P' | b'Q' | b'S' => {
                    self.parse_modifier_key(body, last)
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn parse_special_or_paste(&mut self, body: &[u8]) {
        if body == b"200" {
            self.state = State::Paste;
            self.paste.clear();
            return;
        }
        let mut split = body.split(|&c| c == b';');
        let first = match split.next().and_then(parse_u16) {
            Some(v) => v,
            None => return,
        };
        let modifiers = split
            .next()
            .map(modifier_field)
            .unwrap_or_else(Modifiers::new);
        let key = match first {
            1 | 7 => Key::Home,
            2 => Key::Insert,
            3 => Key::Delete,
            4 | 8 => Key::End,
            5 => Key::PageUp,
            6 => Key::PageDown,
            11..=15 => Key::F((first - 10) as u8),
            17..=21 => Key::F((first - 11) as u8),
            23..=26 => Key::F((first - 12) as u8),
            28..=29 => Key::F((first - 13) as u8),
            31..=34 => Key::F((first - 14) as u8),
            _ => return,
        };
        self.push_key(key, modifiers);
    }

    fn parse_csi_u(&mut self, body: &[u8]) {
        let mut split = body.split(|&c| c == b';');
        let mut code_parts = match split.next() {
            Some(part) => part.split(|&c| c == b':'),
            None => return,
        };
        let codepoint = match code_parts.next().and_then(parse_u32) {
            Some(v) => v,
            None => return,
        };
        let mut modifiers = split
            .next()
            .map(modifier_field)
            .unwrap_or_else(Modifiers::new);

        let mut key = if codepoint >= 57344 {
            match functional_key(codepoint) {
                Some(k) => k,
                None => return,
            }
        } else {
            match char::from_u32(codepoint) {
                Some('\x1B') => Key::Esc,
                Some('\r') => Key::Enter,
                Some('\t') => Key::Tab,
                Some('\x7F') => Key::Backspace,
                Some(c) => Key::Char(c),
                None => return,
            }
        };

        if modifiers.has(Modifier::Shift)
            && let Some(shifted) = code_parts
                .next()
                .and_then(parse_u32)
                .and_then(char::from_u32)
        {
            key = Key::Char(shifted);
            modifiers.set(Modifier::Shift, false);
        }
        self.push_key(key, modifiers);
    }

    fn parse_modifier_key(&mut self, body: &[u8], last: u8) {
        let mut split = body.split(|&c| c == b';');
        split.next();
        let modifiers = split
            .next()
            .map(modifier_field)
            .unwrap_or_else(Modifiers::new);
        let key = match last {
            b'A' => Key::Arrow(Direction2D::Up),
            b'B' => Key::Arrow(Direction2D::Down),
            b'C' => Key::Arrow(Direction2D::Right),
            b'D' => Key::Arrow(Direction2D::Left),
            b'F' => Key::End,
            b'H' => Key::Home,
            b'P' => Key::F(1),
            b'Q' => Key::F(2),
            b'S' => Key::F(4),
            _ => return,
        };
        self.push_key(key, modifiers);
    }

    fn parse_sgr_mouse(&mut self, body: &[u8], last: u8) {
        let mut split = body.split(|&c| c == b';');
        let cb = match split.next().and_then(parse_u16) {
            Some(v) => v as u8,
            None => return,
        };
        let cx = match split.next().and_then(parse_u16) {
            Some(v) => v.saturating_sub(1),
            None => return,
        };
        let cy = match split.next().and_then(parse_u16) {
            Some(v) => v.saturating_sub(1),
            None => return,
        };
        let (mut trigger, modifiers) = match parse_cb(cb) {
            Some(v) => v,
            None => return,
        };
        if last == b'm'
            && let Trigger::MouseDown(btn) = trigger
        {
            trigger = Trigger::MouseUp(btn);
        }
        self.out.push_back(ParsedEvent::Mouse(MouseInput {
            trigger,
            column: cx,
            row: cy,
            modifiers,
        }));
    }

    fn parse_rxvt_mouse(&mut self, body: &[u8]) {
        let mut split = body.split(|&c| c == b';');
        let cb = match split.next().and_then(parse_u16) {
            Some(v) => match (v as u8).checked_sub(32) {
                Some(v) => v,
                None => return,
            },
            None => return,
        };
        let cx = match split.next().and_then(parse_u16) {
            Some(v) => v.saturating_sub(1),
            None => return,
        };
        let cy = match split.next().and_then(parse_u16) {
            Some(v) => v.saturating_sub(1),
            None => return,
        };
        let (trigger, modifiers) = match parse_cb(cb) {
            Some(v) => v,
            None => return,
        };
        self.out.push_back(ParsedEvent::Mouse(MouseInput {
            trigger,
            column: cx,
            row: cy,
            modifiers,
        }));
    }

    fn normal_mouse(&mut self, b: u8) {
        self.buf.push(b);
        if self.buf.len() < 3 {
            return;
        }
        let bytes = std::mem::take(&mut self.buf);
        self.state = State::Ground;
        let cb = match bytes[0].checked_sub(32) {
            Some(v) => v,
            None => return,
        };
        let cx = (bytes[1].saturating_sub(32) as u16).saturating_sub(1);
        let cy = (bytes[2].saturating_sub(32) as u16).saturating_sub(1);
        let (trigger, modifiers) = match parse_cb(cb) {
            Some(v) => v,
            None => return,
        };
        self.out.push_back(ParsedEvent::Mouse(MouseInput {
            trigger,
            column: cx,
            row: cy,
            modifiers,
        }));
    }

    fn parse_window_size(&mut self, body: &[u8]) {
        let mut split = body.split(|&c| c == b';');
        let kind = split.next().and_then(parse_u16);
        let height = split.next().and_then(parse_u16);
        let width = split.next().and_then(parse_u16);
        match (kind, height, width) {
            (Some(4), Some(height), Some(width)) => {
                self.out.push_back(ParsedEvent::WindowPixelSize { width, height })
            }
            (Some(6), Some(height), Some(width)) => {
                self.out.push_back(ParsedEvent::CellPixelSize { width, height })
            }
            _ => {}
        }
    }

    fn parse_private(&mut self, body: &[u8], last: u8) {
        match last {
            b'c' => {
                let params: Vec<u16> = body
                    .split(|&c| c == b';')
                    .filter(|p| !p.is_empty())
                    .filter_map(parse_u16)
                    .collect();
                self.out.push_back(ParsedEvent::PrimaryDeviceAttributes(params));
            }
            b'n' => {
                let mut split = body.split(|&c| c == b';');
                if split.next().and_then(parse_u16) != Some(997) {
                    return;
                }
                match split.next().and_then(parse_u16) {
                    Some(1) => self.out.push_back(ParsedEvent::ColorScheme(ColorScheme::Dark)),
                    Some(2) => self.out.push_back(ParsedEvent::ColorScheme(ColorScheme::Light)),
                    _ => {}
                }
            }
            b'y' => {
                let trimmed = match body.strip_suffix(b"$") {
                    Some(t) => t,
                    None => return,
                };
                let mut split = trimmed.split(|&c| c == b';');
                let mode = split.next().and_then(parse_u16);
                let status = split.next().and_then(parse_u16);
                if let (Some(mode), Some(status)) = (mode, status) {
                    self.out
                        .push_back(ParsedEvent::DecModeReport { mode, status: status as u8 });
                }
            }
            _ => {}
        }
    }

    fn osc(&mut self, b: u8) {
        if b == 0x07 {
            let body = std::mem::take(&mut self.buf);
            self.state = State::Ground;
            self.parse_osc(&body);
            return;
        }
        self.buf.push(b);
        if self.buf.ends_with(b"\x1b\\") {
            let len = self.buf.len();
            self.buf.truncate(len - 2);
            let body = std::mem::take(&mut self.buf);
            self.state = State::Ground;
            self.parse_osc(&body);
        }
    }

    fn parse_osc(&mut self, body: &[u8]) {
        let payload = match std::str::from_utf8(body) {
            Ok(s) => s,
            Err(_) => return,
        };
        let (num_str, rest) = match payload.split_once(';') {
            Some(v) => v,
            None => return,
        };
        let osc_num: u8 = match num_str.parse() {
            Ok(v) => v,
            Err(_) => return,
        };
        if osc_num == 4 {
            let (index_str, rgb_str) = match rest.split_once(';') {
                Some(v) => v,
                None => return,
            };
            let index: u8 = match index_str.parse() {
                Ok(v) => v,
                Err(_) => return,
            };
            if let Some((r, g, b)) = parse_rgb_spec(rgb_str) {
                self.out.push_back(ParsedEvent::Color(ColorEntry {
                    color_type: ColorType::Indexed(index),
                    r,
                    g,
                    b,
                }));
            }
        } else if let Some(color_type) = ColorType::from_osc_number(osc_num)
            && let Some((r, g, b)) = parse_rgb_spec(rest)
        {
            self.out
                .push_back(ParsedEvent::Color(ColorEntry { color_type, r, g, b }));
        }
    }

    fn dcs(&mut self, b: u8) {
        self.buf.push(b);
        if self.buf.ends_with(b"\x1b\\") {
            let len = self.buf.len();
            self.buf.truncate(len - 2);
            let body = std::mem::take(&mut self.buf);
            self.state = State::Ground;
            if let Some(version) = body.strip_prefix(b">|")
                && let Ok(s) = std::str::from_utf8(version)
            {
                self.out.push_back(ParsedEvent::XtVersion(s.to_owned()));
            }
        }
    }

    fn apc(&mut self, b: u8) {
        self.buf.push(b);
        if self.buf.ends_with(b"\x1b\\") {
            let len = self.buf.len();
            self.buf.truncate(len - 2);
            let body = std::mem::take(&mut self.buf);
            self.state = State::Ground;
            if let Some(rest) = body.strip_prefix(b"G")
                && let Ok(s) = std::str::from_utf8(rest)
            {
                let (keys, status) = s.split_once(';').unwrap_or((s, ""));
                let id = keys
                    .split(',')
                    .find_map(|kv| kv.strip_prefix("i="))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let ok = status.starts_with("OK");
                self.out.push_back(ParsedEvent::KittyGraphicsReply { id, ok });
            }
        }
    }

    fn paste_byte(&mut self, b: u8) {
        self.paste.push(b);
        if self.paste.ends_with(b"\x1b[201~") {
            let len = self.paste.len();
            self.paste.truncate(len - 6);
            let text = String::from_utf8_lossy(&self.paste).into_owned();
            self.paste.clear();
            self.state = State::Ground;
            self.out.push_back(ParsedEvent::Paste(text));
        }
    }

    fn push_key(&mut self, key: Key, mut modifiers: Modifiers) {
        if let Key::Char(_) = key {
            modifiers.set(Modifier::Shift, false);
        }
        self.out.push_back(ParsedEvent::Key(Chord {
            trigger: Trigger::Key(key),
            modifiers,
        }));
    }
}

fn mods(ctrl: bool, alt: bool, shift: bool) -> Modifiers {
    Modifiers::new()
        .with_if(Modifier::Ctrl, ctrl)
        .with_if(Modifier::Alt, alt)
        .with_if(Modifier::Shift, shift)
}

fn parse_u16(bytes: &[u8]) -> Option<u16> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn parse_u32(bytes: &[u8]) -> Option<u32> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Parses a CSI `modifiers` field into [`Modifiers`].
fn modifier_field(field: &[u8]) -> Modifiers {
    let mask = field
        .split(|&c| c == b':')
        .next()
        .and_then(parse_u16)
        .unwrap_or(0);
    parse_modifiers(mask)
}

/// Maps a 1-based xterm modifier mask to [`Modifiers`].
fn parse_modifiers(mask: u16) -> Modifiers {
    let bits = mask.saturating_sub(1);
    Modifiers::new()
        .with_if(Modifier::Shift, bits & 1 != 0)
        .with_if(Modifier::Alt, bits & 2 != 0)
        .with_if(Modifier::Ctrl, bits & 4 != 0)
        .with_if(Modifier::Super, bits & 8 != 0)
        .with_if(Modifier::Hyper, bits & 16 != 0)
        .with_if(Modifier::Meta, bits & 32 != 0)
}

/// Decodes a mouse `cb` byte into a [`Trigger`] and `Modifiers`.
fn parse_cb(cb: u8) -> Option<(Trigger, Modifiers)> {
    let button = (cb & 0b0000_0011) | ((cb & 0b1100_0000) >> 4);
    let dragging = cb & 0b0010_0000 != 0;
    let trigger = match (button, dragging) {
        (0, false) => Trigger::MouseDown(MouseButton::Left),
        (1, false) => Trigger::MouseDown(MouseButton::Middle),
        (2, false) => Trigger::MouseDown(MouseButton::Right),
        (0, true) => Trigger::MouseDrag(MouseButton::Left),
        (1, true) => Trigger::MouseDrag(MouseButton::Middle),
        (2, true) => Trigger::MouseDrag(MouseButton::Right),
        (3, false) => Trigger::MouseUp(MouseButton::Left),
        (3, true) | (4, true) | (5, true) => Trigger::MouseHover,
        (4, false) => Trigger::MouseScroll(Direction2D::Up),
        (5, false) => Trigger::MouseScroll(Direction2D::Down),
        (6, false) => Trigger::MouseScroll(Direction2D::Left),
        (7, false) => Trigger::MouseScroll(Direction2D::Right),
        _ => return None,
    };
    let modifiers = Modifiers::new()
        .with_if(Modifier::Shift, cb & 0b0000_0100 != 0)
        .with_if(Modifier::Alt, cb & 0b0000_1000 != 0)
        .with_if(Modifier::Ctrl, cb & 0b0001_0000 != 0);
    Some((trigger, modifiers))
}

/// Maps a Kitty functional-key codepoint to a [`Key`].
fn functional_key(codepoint: u32) -> Option<Key> {
    Some(match codepoint {
        57399..=57408 => Key::Char((b'0' + (codepoint - 57399) as u8) as char),
        57409 => Key::Char('.'),
        57410 => Key::Char('/'),
        57411 => Key::Char('*'),
        57412 => Key::Char('-'),
        57413 => Key::Char('+'),
        57414 => Key::Enter,
        57415 => Key::Char('='),
        57416 => Key::Char(','),
        57417 => Key::Arrow(Direction2D::Left),
        57418 => Key::Arrow(Direction2D::Right),
        57419 => Key::Arrow(Direction2D::Up),
        57420 => Key::Arrow(Direction2D::Down),
        57421 => Key::PageUp,
        57422 => Key::PageDown,
        57423 => Key::Home,
        57424 => Key::End,
        57425 => Key::Insert,
        57426 => Key::Delete,
        57376..=57398 => Key::F(13 + (codepoint - 57376) as u8),
        _ => return None,
    })
}

fn parse_color_channel(s: &str) -> Option<u8> {
    let val = u16::from_str_radix(s, 16).ok()?;
    Some(if s.len() <= 2 { val as u8 } else { (val >> 8) as u8 })
}

fn parse_rgb_spec(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.strip_prefix("rgb:")?;
    let mut parts = s.split('/');
    let r = parse_color_channel(parts.next()?)?;
    let g = parse_color_channel(parts.next()?)?;
    let b = parse_color_channel(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((r, g, b))
}
