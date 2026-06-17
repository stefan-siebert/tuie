//! Sixel palette quantization and DCS payload encoder.

use std::cell::RefCell;

const LEVELS: usize = 6;
const STEP: i32 = 255 / (LEVELS as i32 - 1);
const HALF_STEP: i32 = STEP / 2;
const MAX_LEVEL: u8 = (LEVELS - 1) as u8;

pub(super) const PALETTE_LEN: usize = LEVELS * LEVELS * LEVELS;

const TRANSPARENT: u8 = 0xFF;

const ALPHA_THRESHOLD: u8 = 128;

#[inline(always)]
fn get_palette_rgb(idx: u8) -> (u8, u8, u8) {
    let i = idx as usize;
    let r = (i / (LEVELS * LEVELS)) as u8;
    let g = ((i / LEVELS) % LEVELS) as u8;
    let b = (i % LEVELS) as u8;
    (r * STEP as u8, g * STEP as u8, b * STEP as u8)
}

#[inline(always)]
fn quantize_channel(value: i32) -> u8 {
    let clamped = value.clamp(0, 255);
    ((clamped + HALF_STEP) / STEP).min(MAX_LEVEL as i32) as u8
}

pub(super) fn quantize_rgba_dither(rgba: &[u8], width: u32, height: u32) -> (Vec<u8>, [bool; PALETTE_LEN]) {
    let width = width as usize;
    let height = height as usize;
    let mut indexed = vec![0u8; width * height];
    let mut used = [false; PALETTE_LEN];

    let mut err_cur = vec![0i16; (width + 2) * 3];
    let mut err_nxt = vec![0i16; (width + 2) * 3];

    for y in 0..height {
        for x in 0..width {
            let src_off = (y * width + x) * 4;
            if rgba[src_off + 3] < ALPHA_THRESHOLD {
                indexed[y * width + x] = TRANSPARENT;
                continue;
            }
            let r = rgba[src_off] as i32 + err_cur[(x + 1) * 3] as i32;
            let g = rgba[src_off + 1] as i32 + err_cur[(x + 1) * 3 + 1] as i32;
            let b = rgba[src_off + 2] as i32 + err_cur[(x + 1) * 3 + 2] as i32;

            let r_level = quantize_channel(r);
            let g_level = quantize_channel(g);
            let b_level = quantize_channel(b);
            let idx = r_level * (LEVELS * LEVELS) as u8 + g_level * LEVELS as u8 + b_level;
            indexed[y * width + x] = idx;
            used[idx as usize] = true;

            let pr = (r_level * STEP as u8) as i32;
            let pg = (g_level * STEP as u8) as i32;
            let pb = (b_level * STEP as u8) as i32;
            let err_r = r - pr;
            let err_g = g - pg;
            let err_b = b - pb;

            let spread = |error: i32, weight: i32| -> i16 { ((error * weight) / 16) as i16 };

            err_cur[(x + 2) * 3] = err_cur[(x + 2) * 3].saturating_add(spread(err_r, 7));
            err_cur[(x + 2) * 3 + 1] = err_cur[(x + 2) * 3 + 1].saturating_add(spread(err_g, 7));
            err_cur[(x + 2) * 3 + 2] = err_cur[(x + 2) * 3 + 2].saturating_add(spread(err_b, 7));

            err_nxt[x * 3] = err_nxt[x * 3].saturating_add(spread(err_r, 3));
            err_nxt[x * 3 + 1] = err_nxt[x * 3 + 1].saturating_add(spread(err_g, 3));
            err_nxt[x * 3 + 2] = err_nxt[x * 3 + 2].saturating_add(spread(err_b, 3));

            err_nxt[(x + 1) * 3] = err_nxt[(x + 1) * 3].saturating_add(spread(err_r, 5));
            err_nxt[(x + 1) * 3 + 1] = err_nxt[(x + 1) * 3 + 1].saturating_add(spread(err_g, 5));
            err_nxt[(x + 1) * 3 + 2] = err_nxt[(x + 1) * 3 + 2].saturating_add(spread(err_b, 5));

            err_nxt[(x + 2) * 3] = err_nxt[(x + 2) * 3].saturating_add(spread(err_r, 1));
            err_nxt[(x + 2) * 3 + 1] = err_nxt[(x + 2) * 3 + 1].saturating_add(spread(err_g, 1));
            err_nxt[(x + 2) * 3 + 2] = err_nxt[(x + 2) * 3 + 2].saturating_add(spread(err_b, 1));
        }
        std::mem::swap(&mut err_cur, &mut err_nxt);
        err_nxt.fill(0);
    }

    (indexed, used)
}

#[inline(always)]
fn push_num(out: &mut Vec<u8>, mut n: u32) {
    if n == 0 {
        out.push(b'0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut len = 0;
    while n > 0 {
        buf[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        out.push(buf[len]);
    }
}

#[inline(always)]
fn push_sixel_char(out: &mut Vec<u8>, ch: u8, run: u32) {
    if run == 0 {
        return;
    }
    if run >= 3 {
        out.push(b'!');
        push_num(out, run);
        out.push(ch);
    } else {
        for _ in 0..run {
            out.push(ch);
        }
    }
}

pub(super) fn build_palette_bytes(used: &[bool; PALETTE_LEN]) -> Vec<u8> {
    let mut out = Vec::with_capacity(used.iter().filter(|in_use| **in_use).count() * 16);
    for (idx, in_use) in used.iter().enumerate() {
        if !*in_use {
            continue;
        }
        let (r, g, b) = get_palette_rgb(idx as u8);
        out.push(b'#');
        push_num(&mut out, idx as u32);
        out.extend_from_slice(b";2;");
        push_num(&mut out, (r as u32 * 100 + 127) / 255);
        out.push(b';');
        push_num(&mut out, (g as u32 * 100 + 127) / 255);
        out.push(b';');
        push_num(&mut out, (b as u32 * 100 + 127) / 255);
    }
    out
}

pub(super) fn emit_sixel_dcs(
    out: &mut Vec<u8>,
    indexed: &[u8],
    stride: u32,
    crop_x: u32,
    crop_y: u32,
    crop_w: u32,
    crop_h: u32,
    palette_bytes: &[u8],
) {
    out.reserve((crop_w * crop_h / 2) as usize + palette_bytes.len() + 64);

    out.extend_from_slice(b"\x1bP0;1q");
    out.push(b'"');
    push_num(out, 1);
    out.push(b';');
    push_num(out, 1);
    out.push(b';');
    push_num(out, crop_w);
    out.push(b';');
    push_num(out, crop_h);

    out.extend_from_slice(palette_bytes);

    let crop_x = crop_x as usize;
    let crop_y = crop_y as usize;
    let crop_w = crop_w as usize;
    let crop_h = crop_h as usize;
    let stride = stride as usize;

    BITMAPS_SCRATCH.with_borrow_mut(|bitmaps| {
        bitmaps.resize(PALETTE_LEN * crop_w, 0);

        let mut global_to_local = [u8::MAX; PALETTE_LEN];
        let mut local_to_global = [0u8; PALETTE_LEN];

        let mut y = 0;
        while y < crop_h {
            let rows_in_band = (crop_h - y).min(6);
            let mut local_count: usize = 0;

            for ry in 0..rows_in_band {
                let row_off = (crop_y + y + ry) * stride + crop_x;
                let bit = 1u8 << ry;
                // SAFETY: indexed.len() >= stride * (crop_y + crop_h) and stride >= crop_x + crop_w,
                // so row_off + crop_w <= indexed.len(). quantize_rgba_dither yields indices in
                // 0..PALETTE_LEN for opaque pixels (TRANSPARENT is skipped), so global < PALETTE_LEN.
                // local_count only grows on a new global, capped at PALETTE_LEN, so
                // local * crop_w + cx < bitmaps.len().
                unsafe {
                    let row = indexed.get_unchecked(row_off..row_off + crop_w);
                    for (cx, &idx) in row.iter().enumerate() {
                        if idx == TRANSPARENT {
                            continue;
                        }
                        let global = idx as usize;
                        let slot = global_to_local.get_unchecked_mut(global);
                        let local = if *slot == u8::MAX {
                            let new_local = local_count as u8;
                            *slot = new_local;
                            *local_to_global.get_unchecked_mut(local_count) = idx;
                            local_count += 1;
                            new_local as usize
                        } else {
                            *slot as usize
                        };
                        *bitmaps.get_unchecked_mut(local * crop_w + cx) |= bit;
                    }
                }
            }

            for local in 0..local_count {
                if local > 0 {
                    out.push(b'$');
                }
                // SAFETY: local < local_count <= PALETTE_LEN.
                let global = unsafe { *local_to_global.get_unchecked(local) };
                out.push(b'#');
                push_num(out, global as u32);

                // SAFETY: local < local_count <= PALETTE_LEN, so
                // `(local + 1) * crop_w <= PALETTE_LEN * crop_w == bitmaps.len()`.
                let row = unsafe {
                    bitmaps.get_unchecked_mut(local * crop_w..(local + 1) * crop_w)
                };
                let mut run_char: u8 = b'?';
                let mut run_len: u32 = 0;
                for cell in row.iter_mut() {
                    let bits = *cell;
                    *cell = 0;
                    let ch = b'?' + bits;
                    if ch == run_char {
                        run_len += 1;
                    } else {
                        push_sixel_char(out, run_char, run_len);
                        run_char = ch;
                        run_len = 1;
                    }
                }
                push_sixel_char(out, run_char, run_len);

                // SAFETY: global < PALETTE_LEN (came from quantize output).
                unsafe {
                    *global_to_local.get_unchecked_mut(global as usize) = u8::MAX;
                }
            }

            y += rows_in_band;
            if y < crop_h {
                out.push(b'-');
            }
        }
    });

    out.extend_from_slice(b"\x1b\\");
}

thread_local! {
    static BITMAPS_SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}
