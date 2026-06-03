//! Kitty graphics protocol renderer.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use crate::prelude::*;

use super::escape;
use super::source::{await_async_entry, prepare_async, PixelFormat, SourceData, SourceInner};
use super::ImageSource;

pub(super) struct PixelSlot {
    id: Cell<Option<u32>>,
    source: Rc<SourceInner>,
    transmitted: Cell<bool>,
    placement_counter: Cell<u32>,
}

impl PixelSlot {
    fn set_id(&self, id: Option<u32>) {
        self.id.set(id);
        self.transmitted.set(false);
    }
}

impl Drop for PixelSlot {
    fn drop(&mut self) {
        let _ = IMAGE_SYSTEM.try_with(|system| {
            if let Ok(mut sys) = system.try_borrow_mut() {
                if let Some(id) = self.id.get() {
                    sys.free(id);
                }
            }
        });
    }
}

pub(super) struct PlacementInner {
    pixel_slot: Rc<PixelSlot>,
    placement_id: u32,
    last_emit: Cell<Option<u32>>,
}

impl Drop for PlacementInner {
    fn drop(&mut self) {
        if Rc::strong_count(&self.pixel_slot) == 1 {
            return;
        }
        let Some(id) = self.last_emit.get() else { return };
        let _ = IMAGE_SYSTEM.try_with(|system| {
            if let Ok(mut sys) = system.try_borrow_mut() {
                sys.pending_placement_frees.push((id, self.placement_id));
            }
        });
    }
}

struct Slot {
    weak: Weak<PixelSlot>,
    last_used: u64,
}

struct ImageSystem {
    pid_high: u32,
    slots: Vec<Option<Slot>>,
    pending_frees: Vec<u32>,
    pending_placement_frees: Vec<(u32, u32)>,
    counter: u64,
}

impl ImageSystem {
    const SLOT_BITS: u32 = 10;
    const MAX_SLOTS: u16 = 1 << Self::SLOT_BITS;
    const SLOT_MASK: u32 = (1 << Self::SLOT_BITS) - 1;
    const PID_MASK: u32 = (1u32 << (32 - Self::SLOT_BITS)) - 1;

    fn new() -> Self {
        let pid = std::process::id();
        let pid_high = (pid & Self::PID_MASK) << Self::SLOT_BITS;
        crate::runtime::on_quit(emit_free_escapes_for_live_slots);
        Self {
            pid_high,
            slots: Vec::new(),
            pending_frees: Vec::new(),
            pending_placement_frees: Vec::new(),
            counter: 0,
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    fn alloc(&mut self, weak: Weak<PixelSlot>) -> u32 {
        let last_used = self.next_tick();
        let new_slot = Slot { weak, last_used };
        for i in 0..self.slots.len() {
            let reusable = match &self.slots[i] {
                None => true,
                Some(s) => s.weak.strong_count() == 0,
            };
            if reusable {
                if self.slots[i].is_some() {
                    self.pending_frees.push(self.pid_high | i as u32);
                }
                self.slots[i] = Some(new_slot);
                return self.pid_high | i as u32;
            }
        }
        if (self.slots.len() as u16) < Self::MAX_SLOTS {
            self.slots.push(Some(new_slot));
            return self.pid_high | (self.slots.len() - 1) as u32;
        }
        let (idx, _) = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i, s.last_used)))
            .min_by_key(|(_, t)| *t)
            .expect("eviction with no live slots, invariant violated");
        if let Some(old) = &self.slots[idx] {
            if let Some(rc) = old.weak.upgrade() {
                rc.set_id(None);
            }
        }
        self.slots[idx] = Some(new_slot);
        self.pid_high | idx as u32
    }

    fn touch(&mut self, image_id: u32) {
        let idx = (image_id & Self::SLOT_MASK) as usize;
        let tick = self.next_tick();
        if let Some(slot) = &mut self.slots[idx] {
            slot.last_used = tick;
        }
    }

    fn free(&mut self, image_id: u32) {
        let idx = (image_id & Self::SLOT_MASK) as usize;
        self.slots[idx] = None;
        self.pending_frees.push(image_id);
    }

    fn drain_pending_frees(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.pending_frees)
    }

    fn drain_pending_placement_frees(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.pending_placement_frees)
    }
}

thread_local! {
    static IMAGE_SYSTEM: RefCell<ImageSystem> = RefCell::new(ImageSystem::new());
    static SCRATCH: RefCell<String> = RefCell::new(String::with_capacity(512));
}

// Only the GUI present pass scans cells back to a source/placement (see
// `gui::image_scan`); the terminal backends never reverse-map.
#[cfg(feature = "gui")]
pub(crate) fn lookup_source(image_id: u32) -> Option<Rc<SourceInner>> {
    IMAGE_SYSTEM.with_borrow(|sys| {
        if image_id & !ImageSystem::SLOT_MASK != sys.pid_high {
            return None;
        }
        let idx = (image_id & ImageSystem::SLOT_MASK) as usize;
        let slot = sys.slots.get(idx)?.as_ref()?;
        let pixel_slot = slot.weak.upgrade()?;
        if pixel_slot.id.get() != Some(image_id) {
            return None;
        }
        Some(pixel_slot.source.clone())
    })
}

#[cfg(feature = "gui")]
pub(crate) fn lookup_placement_size(source: &SourceInner, placement_id: u32) -> Option<Vec2<u16>> {
    source.with_kitty_cache(|cache| {
        cache
            .placements
            .iter()
            .find(|(_, p)| p.placement_id == placement_id)
            .map(|(size, _)| *size)
    })
}

fn pixel_slot_for(source: &Rc<SourceInner>) -> Rc<PixelSlot> {
    source.with_kitty_cache(|cache| {
        if let Some(rc) = &cache.pixel_slot {
            return rc.clone();
        }
        let slot = Rc::new(PixelSlot {
            id: Cell::new(None),
            source: source.clone(),
            transmitted: Cell::new(false),
            placement_counter: Cell::new(1),
        });
        cache.pixel_slot = Some(slot.clone());
        slot
    })
}

fn placement_for(
    source: &Rc<SourceInner>,
    pixel_slot: &Rc<PixelSlot>,
    cell_size: Vec2<u16>,
) -> Rc<PlacementInner> {
    source.with_kitty_cache(|cache| {
        if let Some((_, rc)) = cache.placements.iter().find(|(sz, _)| *sz == cell_size) {
            return rc.clone();
        }
        let placement_id = pixel_slot.placement_counter.get();
        pixel_slot.placement_counter.set(placement_id + 1);
        let placement = Rc::new(PlacementInner {
            pixel_slot: pixel_slot.clone(),
            placement_id,
            last_emit: Cell::new(None),
        });
        cache.placements.push((cell_size, placement.clone()));
        placement
    })
}

fn is_tmux() -> bool {
    crate::runtime::get_terminal_info()
        .and_then(|i| i.xtversion)
        .is_some_and(|v| v.starts_with("tmux "))
}

fn emit_free_escapes_for_live_slots(out: &mut dyn std::io::Write) {
    let tmux = is_tmux();
    IMAGE_SYSTEM.with_borrow(|sys| {
        let mut buf = String::with_capacity(64);
        for (i, slot) in sys.slots.iter().enumerate() {
            if slot.is_some() {
                let id = sys.pid_high | i as u32;
                buf.clear();
                escape::free(&mut buf, id, tmux);
                let _ = out.write_all(buf.as_bytes());
            }
        }
    });
}

fn build_transmit(
    out: &mut String,
    image_id: u32,
    source: &SourceInner,
    decoded: Option<&[u8]>,
    tmux: bool,
) -> bool {
    #[cfg(unix)]
    let use_shm = crate::runtime::get_image_caps().supports_kitty_shm;
    match &*source.data {
        SourceData::Raw { width, height, pixels, format } => {
            let bits_per_pixel = match format {
                PixelFormat::Rgb => 24,
                PixelFormat::Rgba => 32,
            };
            #[cfg(unix)]
            if use_shm {
                if let Some(name) = super::shm::write_image(image_id, pixels) {
                    escape::transmit_raw_shm(
                        out, image_id, bits_per_pixel, &name, *width, *height, tmux,
                    );
                    return true;
                }
            }
            escape::transmit_raw(out, image_id, bits_per_pixel, pixels, *width, *height, tmux);
            true
        }
        SourceData::Encoded { bytes, dims, format } => {
            if *format == image::ImageFormat::Png {
                #[cfg(unix)]
                if use_shm {
                    if let Some(name) = super::shm::write_image(image_id, bytes) {
                        escape::transmit_png_shm(out, image_id, &name, tmux);
                        return true;
                    }
                }
                escape::transmit_png(out, image_id, bytes, tmux);
                true
            } else if let Some(rgba) = decoded {
                #[cfg(unix)]
                if use_shm {
                    if let Some(name) = super::shm::write_image(image_id, rgba) {
                        escape::transmit_raw_shm(
                            out, image_id, 32, &name, dims.x, dims.y, tmux,
                        );
                        return true;
                    }
                }
                escape::transmit_raw(out, image_id, 32, rgba, dims.x, dims.y, tmux);
                true
            } else {
                false
            }
        }
    }
}

fn needs_async_decode(data: &SourceData) -> bool {
    matches!(data, SourceData::Encoded { format, .. } if *format != image::ImageFormat::Png)
}

fn decode_encoded_to_rgba(data: &SourceData) -> Option<Vec<u8>> {
    let SourceData::Encoded { bytes, .. } = data else { return None };
    let img = image::load_from_memory(bytes).ok()?;
    Some(img.to_rgba8().into_raw())
}

fn cover_placement(
    widget: Vec2<u16>,
    source: &SourceInner,
    max_cells: u16,
) -> (Vec2<u16>, Vec2<u16>) {
    let widget_clamped = Vec2::new(widget.x.min(max_cells), widget.y.min(max_cells));
    let src_px = source.get_pixel_dims();
    let Some(cell_px) = crate::runtime::get_terminal_info().and_then(|i| i.cell_px) else {
        return (widget_clamped, Vec2::of(0u16));
    };
    if src_px.x == 0
        || src_px.y == 0
        || cell_px.x == 0
        || cell_px.y == 0
        || widget.x == 0
        || widget.y == 0
    {
        return (widget_clamped, Vec2::of(0u16));
    }
    let w_px_x = widget.x as u64 * cell_px.x as u64;
    let w_px_y = widget.y as u64 * cell_px.y as u64;
    let widget_wider = w_px_x * src_px.y as u64 > w_px_y * src_px.x as u64;
    let (vp, off) = if widget_wider {
        let image_h = (src_px.y as u64 * w_px_x + src_px.x as u64 / 2) / src_px.x as u64;
        let mut r = ((image_h + cell_px.y as u64 - 1) / cell_px.y as u64).max(widget.y as u64);
        if (r - widget.y as u64) % 2 != 0 {
            r += 1;
        }
        let row_off = (r - widget.y as u64) / 2;
        (Vec2::new(widget.x as u64, r), Vec2::new(0u64, row_off))
    } else {
        let image_w = (src_px.x as u64 * w_px_y + src_px.y as u64 / 2) / src_px.y as u64;
        let mut c = ((image_w + cell_px.x as u64 - 1) / cell_px.x as u64).max(widget.x as u64);
        if (c - widget.x as u64) % 2 != 0 {
            c += 1;
        }
        let col_off = (c - widget.x as u64) / 2;
        (Vec2::new(c, widget.y as u64), Vec2::new(col_off, 0u64))
    };
    let max = max_cells as u64;
    let c = vp.x.min(max) as u16;
    let r = vp.y.min(max) as u16;
    let ox = off.x.min(max.saturating_sub(widget.x as u64)) as u16;
    let oy = off.y.min(max.saturating_sub(widget.y as u64)) as u16;
    (Vec2::new(c, r), Vec2::new(ox, oy))
}

struct RenderTick {
    pending_image_frees: Vec<u32>,
    pending_placement_frees: Vec<(u32, u32)>,
    image_id: u32,
}

fn begin_render(pixel_slot: &Rc<PixelSlot>) -> RenderTick {
    IMAGE_SYSTEM.with_borrow_mut(|sys| {
        let pending_image_frees = sys.drain_pending_frees();
        let pending_placement_frees = sys.drain_pending_placement_frees();
        let image_id = match pixel_slot.id.get() {
            Some(id) => {
                sys.touch(id);
                id
            }
            None => {
                let new_id = sys.alloc(Rc::downgrade(pixel_slot));
                pixel_slot.set_id(Some(new_id));
                new_id
            }
        };
        RenderTick { pending_image_frees, pending_placement_frees, image_id }
    })
}

fn mark_transmitted(pixel_slot: &PixelSlot) {
    pixel_slot.transmitted.set(true);
    pixel_slot.source.with_kitty_cache(|cache| {
        for (_, rc) in &cache.placements {
            rc.last_emit.set(None);
        }
    });
}

#[inline]
fn get_rgb_from_id(image_id: u32) -> (u8, u8, u8) {
    let r = ((image_id >> 16) & 0xFF) as u8;
    let g = ((image_id >> 8) & 0xFF) as u8;
    let b = (image_id & 0xFF) as u8;
    (r, g, b)
}

#[inline]
fn get_high_byte_diacritic(image_id: u32) -> Option<char> {
    let high_byte = ((image_id >> 24) & 0xFF) as usize;
    if high_byte == 0 {
        None
    } else {
        Some(DIACRITICS[high_byte])
    }
}

// Placeholder glyph + diacritic→number decode are used only by the GUI
// cell-scan path (`gui::image_scan`); the encode side (`DIACRITICS`) stays
// always-compiled because the transmit path uses it.
#[cfg(feature = "gui")]
pub(crate) const PLACEHOLDER_CHAR: char = '\u{10EEEE}';

#[cfg(feature = "gui")]
pub(crate) fn diacritic_to_num(c: char) -> Option<u32> {
    DIACRITICS.iter().position(|&d| d == c).map(|i| (i + 1) as u32)
}

const DIACRITICS: &[char] = &[
    '\u{0305}', '\u{030D}', '\u{030E}', '\u{0310}', '\u{0312}', '\u{033D}', '\u{033E}', '\u{033F}',
    '\u{0346}', '\u{034A}', '\u{034B}', '\u{034C}', '\u{0350}', '\u{0351}', '\u{0352}', '\u{0357}',
    '\u{035B}', '\u{0363}', '\u{0364}', '\u{0365}', '\u{0366}', '\u{0367}', '\u{0368}', '\u{0369}',
    '\u{036A}', '\u{036B}', '\u{036C}', '\u{036D}', '\u{036E}', '\u{036F}', '\u{0483}', '\u{0484}',
    '\u{0485}', '\u{0486}', '\u{0487}', '\u{0592}', '\u{0593}', '\u{0594}', '\u{0595}', '\u{0597}',
    '\u{0598}', '\u{0599}', '\u{059C}', '\u{059D}', '\u{059E}', '\u{059F}', '\u{05A0}', '\u{05A1}',
    '\u{05A8}', '\u{05A9}', '\u{05AB}', '\u{05AC}', '\u{05AF}', '\u{05C4}', '\u{0610}', '\u{0611}',
    '\u{0612}', '\u{0613}', '\u{0614}', '\u{0615}', '\u{0616}', '\u{0617}', '\u{0657}', '\u{0658}',
    '\u{0659}', '\u{065A}', '\u{065B}', '\u{065D}', '\u{065E}', '\u{06D6}', '\u{06D7}', '\u{06D8}',
    '\u{06D9}', '\u{06DA}', '\u{06DB}', '\u{06DC}', '\u{06DF}', '\u{06E0}', '\u{06E1}', '\u{06E2}',
    '\u{06E4}', '\u{06E7}', '\u{06E8}', '\u{06EB}', '\u{06EC}', '\u{0730}', '\u{0732}', '\u{0733}',
    '\u{0735}', '\u{0736}', '\u{073A}', '\u{073D}', '\u{073F}', '\u{0740}', '\u{0741}', '\u{0743}',
    '\u{0745}', '\u{0747}', '\u{0749}', '\u{074A}', '\u{07EB}', '\u{07EC}', '\u{07ED}', '\u{07EE}',
    '\u{07EF}', '\u{07F0}', '\u{07F1}', '\u{07F3}', '\u{0816}', '\u{0817}', '\u{0818}', '\u{0819}',
    '\u{081B}', '\u{081C}', '\u{081D}', '\u{081E}', '\u{081F}', '\u{0820}', '\u{0821}', '\u{0822}',
    '\u{0823}', '\u{0825}', '\u{0826}', '\u{0827}', '\u{0829}', '\u{082A}', '\u{082B}', '\u{082C}',
    '\u{082D}', '\u{0951}', '\u{0953}', '\u{0954}', '\u{0F82}', '\u{0F83}', '\u{0F86}', '\u{0F87}',
    '\u{135D}', '\u{135E}', '\u{135F}', '\u{17DD}', '\u{193A}', '\u{1A17}', '\u{1A75}', '\u{1A76}',
    '\u{1A77}', '\u{1A78}', '\u{1A79}', '\u{1A7A}', '\u{1A7B}', '\u{1A7C}', '\u{1B6B}', '\u{1B6D}',
    '\u{1B6E}', '\u{1B6F}', '\u{1B70}', '\u{1B71}', '\u{1B72}', '\u{1B73}', '\u{1CD0}', '\u{1CD1}',
    '\u{1CD2}', '\u{1CDA}', '\u{1CDB}', '\u{1CE0}', '\u{1DC0}', '\u{1DC1}', '\u{1DC3}', '\u{1DC4}',
    '\u{1DC5}', '\u{1DC6}', '\u{1DC7}', '\u{1DC8}', '\u{1DC9}', '\u{1DCB}', '\u{1DCC}', '\u{1DD1}',
    '\u{1DD2}', '\u{1DD3}', '\u{1DD4}', '\u{1DD5}', '\u{1DD6}', '\u{1DD7}', '\u{1DD8}', '\u{1DD9}',
    '\u{1DDA}', '\u{1DDB}', '\u{1DDC}', '\u{1DDD}', '\u{1DDE}', '\u{1DDF}', '\u{1DE0}', '\u{1DE1}',
    '\u{1DE2}', '\u{1DE3}', '\u{1DE4}', '\u{1DE5}', '\u{1DE6}', '\u{1DFE}', '\u{20D0}', '\u{20D1}',
    '\u{20D4}', '\u{20D5}', '\u{20D6}', '\u{20D7}', '\u{20DB}', '\u{20DC}', '\u{20E1}', '\u{20E7}',
    '\u{20E9}', '\u{20F0}', '\u{2CEF}', '\u{2CF0}', '\u{2CF1}', '\u{2DE0}', '\u{2DE1}', '\u{2DE2}',
    '\u{2DE3}', '\u{2DE4}', '\u{2DE5}', '\u{2DE6}', '\u{2DE7}', '\u{2DE8}', '\u{2DE9}', '\u{2DEA}',
    '\u{2DEB}', '\u{2DEC}', '\u{2DED}', '\u{2DEE}', '\u{2DEF}', '\u{2DF0}', '\u{2DF1}', '\u{2DF2}',
    '\u{2DF3}', '\u{2DF4}', '\u{2DF5}', '\u{2DF6}', '\u{2DF7}', '\u{2DF8}', '\u{2DF9}', '\u{2DFA}',
    '\u{2DFB}', '\u{2DFC}', '\u{2DFD}', '\u{2DFE}', '\u{2DFF}', '\u{A66F}', '\u{A67C}', '\u{A67D}',
    '\u{A6F0}', '\u{A6F1}', '\u{A8E0}', '\u{A8E1}', '\u{A8E2}', '\u{A8E3}', '\u{A8E4}', '\u{A8E5}',
    '\u{A8E6}', '\u{A8E7}', '\u{A8E8}', '\u{A8E9}', '\u{A8EA}', '\u{A8EB}', '\u{A8EC}', '\u{A8ED}',
    '\u{A8EE}', '\u{A8EF}', '\u{A8F0}', '\u{A8F1}', '\u{AAB0}', '\u{AAB2}', '\u{AAB3}', '\u{AAB7}',
    '\u{AAB8}', '\u{AABE}', '\u{AABF}', '\u{AAC1}', '\u{FE20}', '\u{FE21}', '\u{FE22}', '\u{FE23}',
    '\u{FE24}', '\u{FE25}', '\u{FE26}',
    '\u{10A0F}', '\u{10A38}', '\u{1D185}', '\u{1D186}', '\u{1D187}',
    '\u{1D188}', '\u{1D189}', '\u{1D1AA}', '\u{1D1AB}', '\u{1D1AC}',
    '\u{1D1AD}', '\u{1D242}', '\u{1D243}', '\u{1D244}',
];

pub(crate) fn prepare(source: &ImageSource, _placement_size: Vec2<u16>, _fill: bool) {
    if !needs_async_decode(&source.inner.data) {
        return;
    }
    let dims = source.inner.get_pixel_dims();
    source.inner.with_kitty_cache(|cache| {
        prepare_async(
            &mut cache.decoded,
            dims,
            &source.inner.data,
            decode_encoded_to_rgba,
        );
    });
}

pub(crate) fn dispatch(
    ctx: &mut RenderContext,
    source: &ImageSource,
    placement_size: Vec2<u16>,
    fill: bool,
) {
    let max_d = DIACRITICS.len().min(u16::MAX as usize) as u16;
    let (vp, diac_off) = if fill {
        cover_placement(placement_size, &source.inner, max_d)
    } else {
        (
            Vec2::new(placement_size.x.min(max_d), placement_size.y.min(max_d)),
            Vec2::of(0u16),
        )
    };

    let pixel_slot = pixel_slot_for(&source.inner);
    let placement = placement_for(&source.inner, &pixel_slot, vp);

    let tmux = is_tmux();
    let in_gui = crate::runtime::is_gui();
    let tick = begin_render(&pixel_slot);

    if !in_gui {
        let must_transmit = !pixel_slot.transmitted.get();

        let decoded = if must_transmit && needs_async_decode(&pixel_slot.source.data) {
            let dims = pixel_slot.source.get_pixel_dims();
            pixel_slot.source.with_kitty_cache(|cache| {
                await_async_entry(&mut cache.decoded, dims, || {
                    decode_encoded_to_rgba(&pixel_slot.source.data)
                })
            })
        } else {
            None
        };

        let must_emit_placement = placement.last_emit.get() != Some(tick.image_id);

        let did_transmit = SCRATCH.with_borrow_mut(|buf| {
            buf.clear();
            for id in tick.pending_image_frees {
                escape::free(buf, id, tmux);
            }
            for (id, pid) in tick.pending_placement_frees {
                escape::free_placement(buf, id, pid, tmux);
            }
            let transmitted = if must_transmit {
                build_transmit(
                    buf,
                    tick.image_id,
                    &pixel_slot.source,
                    decoded.as_deref().map(|v| v.as_slice()),
                    tmux,
                )
            } else {
                false
            };
            if must_emit_placement {
                escape::placement(
                    buf,
                    tick.image_id,
                    placement.placement_id,
                    vp.x,
                    vp.y,
                    tmux,
                );
            }
            if !buf.is_empty() {
                ctx.queue_raw(buf.as_bytes());
            }
            transmitted
        });

        if did_transmit {
            mark_transmitted(&pixel_slot);
        }
        if must_emit_placement {
            placement.last_emit.set(Some(tick.image_id));
        }
    }

    let (r, g, b) = get_rgb_from_id(tick.image_id);
    let high_diacritic = get_high_byte_diacritic(tick.image_id);
    let (ur, ug, ub) = get_rgb_from_id(placement.placement_id);

    let fg = Color::Rgb(r, g, b);
    let underline_color = Color::Rgb(ur, ug, ub);

    let max = DIACRITICS.len();
    let rows = (placement_size.y as usize).min(max);
    let cols = (placement_size.x as usize).min(max);
    let row_off = (diac_off.y as usize).min(max.saturating_sub(rows));
    let col_off = (diac_off.x as usize).min(max.saturating_sub(cols));

    const PLACEHOLDER_BYTES: &[u8] = "\u{10EEEE}".as_bytes();

    let mut high_buf = [0u8; 4];
    let high_bytes: &[u8] = match high_diacritic {
        Some(c) => c.encode_utf8(&mut high_buf).as_bytes(),
        None => &[],
    };

    for row in 0..rows {
        ctx.move_to(Vec2::new(0, row as i32));
        let mut writer = ctx.row_writer();
        let range = writer.get_range();
        let end = cols.min(range.end);
        if range.start >= end {
            continue;
        }

        let mut prefix_buf = [0u8; 8];
        prefix_buf[..4].copy_from_slice(PLACEHOLDER_BYTES);
        let mut row_buf = [0u8; 4];
        let row_str = DIACRITICS[row + row_off].encode_utf8(&mut row_buf);
        prefix_buf[4..4 + row_str.len()].copy_from_slice(row_str.as_bytes());
        let prefix_len = 4 + row_str.len();

        for col in range.start..end {
            let mut col_buf = [0u8; 4];
            let col_str = DIACRITICS[col + col_off].encode_utf8(&mut col_buf);
            let col_len = col_str.len();

            let mut scratch = [0u8; 16];
            scratch[..prefix_len].copy_from_slice(&prefix_buf[..prefix_len]);
            scratch[prefix_len..prefix_len + col_len].copy_from_slice(col_str.as_bytes());
            let mut total = prefix_len + col_len;
            scratch[total..total + high_bytes.len()].copy_from_slice(high_bytes);
            total += high_bytes.len();

            // SAFETY: scratch is assembled from the placeholder char bytes plus
            // `encode_utf8` output, so the prefix is always valid UTF-8.
            unsafe {
                let s = std::str::from_utf8_unchecked(&scratch[..total]);
                writer
                    .cell(col)
                    .grapheme_unchecked(false, s)
                    .style(&Style::new().fg(fg).underline_color(underline_color));
            }
        }
    }
}
