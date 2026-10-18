//! Hooks for reading and driving a running app from outside its terminal.
//!
//! Built for an in-app inspector (an MCP server's app side) but free of any
//! protocol: no sockets, no serialization. Everything here is either a read of
//! what the runtime already holds or a door into the path real input takes, so
//! an inspector built on it tests what a user would hit:
//!
//! * [`inject`] queues events *as the terminal parser would have decoded
//!   them*. They go through the same translation as terminal bytes — click
//!   counting, key-repeat counting, and repeat inference where the terminal
//!   does not report repeats — and then through the normal dispatch. A
//!   widget cannot tell an injected key from a typed one.
//! * [`after_update`] runs a callback at the end of the runtime tick, after
//!   the frame was laid out and painted. Answering from there is answering
//!   from what is on screen.
//! * [`is_settled`] says whether the runtime still owes work for input it has
//!   seen (queued injections, a pending multi-key chord, unpainted changes).
//! * [`screen`] and [`cursor`] read the last painted frame.
//!
//! Terminal mode only: the GUI backend has its own event loop and does not
//! drain the injection queue. In tests, [`Emulator::pump_injected`]
//! (crate::emulator::Emulator::pump_injected) stands in for the reader.

use crate::ansi::{MouseInput, ParsedEvent};
use crate::prelude::*;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Mutex;

/// An event to inject, in the shape the terminal parser produces.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Injected {
    /// A key press. Where the terminal does not report repeats, the same key
    /// injected again within [`INFERRED_REPEAT_WINDOW`]
    /// (crate::runtime::INFERRED_REPEAT_WINDOW) is inferred as one — exactly
    /// as it would be from the keyboard.
    Key(Chord),
    /// A key the terminal reports as held (kitty keyboard protocol, flag 2).
    /// Only meaningful where [`RuntimeInfo::key_repeats_inferred`] is false;
    /// a terminal without the protocol never sends this.
    KeyRepeat(Chord),
    /// A mouse event at a cell, 0-indexed.
    Mouse {
        trigger: Trigger,
        column: u16,
        row: u16,
        modifiers: Modifiers,
    },
    /// A bracketed paste.
    Paste(String),
    /// The terminal changed size, in cells. Only meaningful headless
    /// ([`crate::runtime::start_headless`]); a real terminal reports its own.
    Resize(Vec2<u16>),
}

impl Injected {
    fn into_parsed(self) -> ParsedEvent {
        match self {
            Injected::Key(chord) => ParsedEvent::Key(chord),
            Injected::KeyRepeat(chord) => ParsedEvent::KeyRepeat(chord),
            Injected::Mouse {
                trigger,
                column,
                row,
                modifiers,
            } => ParsedEvent::Mouse(MouseInput {
                trigger,
                column,
                row,
                modifiers,
            }),
            Injected::Paste(text) => ParsedEvent::Paste(text),
            Injected::Resize(size) => ParsedEvent::Resize(size),
        }
    }
}

static INJECTED: Mutex<VecDeque<Injected>> = Mutex::new(VecDeque::new());

/// Queues `events` for the next read and wakes the runtime. Callable from any
/// thread. Events injected by one call arrive in one batch, in order; to make
/// time pass between two keys (a held key, a deliberate double press), inject
/// them in separate calls with the delay in between.
pub fn inject(events: impl IntoIterator<Item = Injected>) {
    match INJECTED.lock() {
        Ok(mut queue) => queue.extend(events),
        Err(poisoned) => poisoned.into_inner().extend(events),
    }
    crate::runtime::wake();
}

/// Whether injected events are still waiting to be read.
pub fn has_pending_injection() -> bool {
    match INJECTED.lock() {
        Ok(queue) => !queue.is_empty(),
        Err(poisoned) => !poisoned.into_inner().is_empty(),
    }
}

pub(crate) fn take_injected() -> Vec<ParsedEvent> {
    let drained: Vec<Injected> = match INJECTED.lock() {
        Ok(mut queue) => queue.drain(..).collect(),
        Err(poisoned) => poisoned.into_inner().drain(..).collect(),
    };
    drained.into_iter().map(Injected::into_parsed).collect()
}

type AfterUpdate = Box<dyn FnOnce(&mut dyn Widget)>;

thread_local! {
    static AFTER_UPDATE: RefCell<Vec<AfterUpdate>> = const { RefCell::new(Vec::new()) };
}

/// Runs `callback` at the end of the current runtime tick — or the next one,
/// when called outside a tick — after layout and paint, with the root widget.
/// Runtime thread only.
///
/// A callback that registers another one defers it to the following tick, so
/// "check again later" is a callback re-registering itself. The following
/// tick comes only when something wakes the runtime: input, a [`send`]
/// (crate::send), a due [`schedule`](crate::schedule) timer. A callback
/// waiting for a deadline should schedule a timer for it. A callback that
/// changes state gets its frame painted right away.
pub fn after_update(callback: impl FnOnce(&mut dyn Widget) + 'static) {
    AFTER_UPDATE.with_borrow_mut(|callbacks| callbacks.push(Box::new(callback)));
}

/// Asks for another runtime tick soon, even if nothing else happens. For the
/// first check of a callback registered through [`after_update`] from inside
/// another one, which would otherwise wait for an unrelated wake. Not for a
/// re-check loop: a callback that requests a tick every time it re-registers
/// spins the runtime.
pub fn request_tick() {
    crate::runtime::wake();
}

pub(crate) fn run_after_update(root: &mut dyn Widget) {
    let callbacks = AFTER_UPDATE.with_borrow_mut(std::mem::take);
    for callback in callbacks {
        callback(root);
    }
}

/// Whether the runtime owes no work for input it has already seen: nothing
/// injected is waiting, no [`send`](crate::send) message is queued, no
/// multi-key chord is pending, and no change is waiting to be painted.
///
/// Only the runtime's own work: a timer an app scheduled in answer to a key,
/// or an async task it started, is the app's to account for. False while
/// called from inside a tick (a widget handler), where the answer would be
/// premature by definition.
pub fn is_settled() -> bool {
    !has_pending_injection()
        && crate::runtime::inbox_is_empty()
        && crate::runtime::key_queue_pending() == Some(false)
        && crate::runtime::get_dirty() == DirtyImpact::None
}

/// The last painted frame, row by row, with styles.
pub fn screen() -> StyledString {
    crate::runtime::get_emulator_snapshot()
}

/// The last painted frame as a PNG, as [`screenshot`] returns it.
#[cfg(feature = "screenshot")]
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Screenshot {
    /// The encoded image.
    pub png: Vec<u8>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixels per cell before any downscaling.
    pub cell_px: Vec2<u16>,
    /// Set when the image was downscaled to fit `max_width`.
    pub scale: Option<f32>,
}

/// The last painted frame in pixels: cell backgrounds, glyphs from the
/// system's monospace font (Nerd Font symbols from a fallback), underlines,
/// and the images kitty placeholders stand for. `rect` crops it to cells;
/// `max_width` downscales until the image is at most that wide (0: never).
///
/// Not what a terminal shows to the pixel: the terminal's own font, its
/// palette for the 16 ANSI colours and Sixel images are not known here.
#[cfg(feature = "screenshot")]
pub fn screenshot(rect: Option<Rect<i32, u16>>, max_width: u32) -> Result<Screenshot, String> {
    let fitted_cell = crate::get_runtime_info().cell_size.unwrap_or(Vec2::new(1, 2));
    let mut image = crate::runtime::paint_screenshot(fitted_cell)?;
    let cell = crate::render::screenshot::cell_size()?;
    if let Some(rect) = rect {
        let cell_w = u32::from(cell.x);
        let cell_h = u32::from(cell.y);
        let x = u32::try_from(rect.pos.x.max(0)).unwrap_or(0) * cell_w;
        let y = u32::try_from(rect.pos.y.max(0)).unwrap_or(0) * cell_h;
        let width = (u32::from(rect.size.x) * cell_w).min(image.width().saturating_sub(x));
        let height = (u32::from(rect.size.y) * cell_h).min(image.height().saturating_sub(y));
        if width == 0 || height == 0 {
            return Err("the element is not on screen".into());
        }
        image = image::imageops::crop_imm(&image, x, y, width, height).to_image();
    }
    let mut scale = None;
    if max_width > 0 && image.width() > max_width {
        let factor = max_width as f32 / image.width() as f32;
        let height = ((image.height() as f32 * factor).round() as u32).max(1);
        image = image::imageops::resize(
            &image,
            max_width,
            height,
            image::imageops::FilterType::Triangle,
        );
        scale = Some(factor);
    }
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| format!("encoding the screenshot failed: {error}"))?;
    Ok(Screenshot { width: image.width(), height: image.height(), png, cell_px: cell, scale })
}

/// The terminal cursor of the last painted frame, if it was shown.
pub fn cursor() -> Option<(CursorShape, Vec2<i32>)> {
    crate::runtime::get_emulator_cursor()
}

/// Forgets queued injections and after-update callbacks, for a fresh
/// emulator session.
pub(crate) fn reset() {
    match INJECTED.lock() {
        Ok(mut queue) => queue.clear(),
        Err(poisoned) => poisoned.into_inner().clear(),
    }
    AFTER_UPDATE.with_borrow_mut(Vec::clear);
}

/// How many popups are open (dialogs, menus, tooltips). `None` in the middle
/// of a tick.
pub fn open_popups() -> Option<usize> {
    crate::runtime::with_popup_contents(|contents| contents.len())
}

/// Where the focus is: the focus chain from the outermost widget to the
/// focused one, as each widget's [`Widget::get_name`], and whether it runs
/// through an open popup — a popup that has focus is the active root, and
/// takes every key before the main tree sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusChain {
    pub in_popup: bool,
    pub names: Vec<&'static str>,
}

/// The focus chain, looked up in `root` and then in the open popups. Empty
/// when nothing is focused, or in the middle of a tick.
pub fn focus_chain(root: &dyn Widget) -> FocusChain {
    let chain = crate::get_focus_chain();
    let names_in = |tree: &dyn Widget| -> Option<Vec<&'static str>> {
        let names: Vec<&'static str> = chain
            .iter()
            .filter_map(|id| tree.find(*id).map(|widget| widget.get_name()))
            .collect();
        (!names.is_empty()).then_some(names)
    };
    if let Some(names) = names_in(root) {
        return FocusChain {
            in_popup: false,
            names,
        };
    }
    crate::runtime::with_popup_contents(|contents| {
        contents.iter().find_map(|content| names_in(*content))
    })
    .flatten()
    .map(|names| FocusChain {
        in_popup: true,
        names,
    })
    .unwrap_or_default()
}

/// The text of the last painted frame inside `rect` (window cells), one
/// string per row, cell-exact.
pub fn text_in(rect: Rect<i32, u16>) -> Vec<String> {
    crate::runtime::frame_text_in(rect)
}

/// One widget as an inspector sees it.
#[derive(Clone, Debug)]
pub struct ElementInfo {
    /// The widget id: unique within the process, not stable across runs.
    pub id: u64,
    /// [`Widget::get_name`].
    pub name: &'static str,
    /// [`WidgetMethods::test_id`]: stable, set by the app.
    pub test_id: Option<String>,
    /// Outer rect in window cells.
    pub rect: Rect<i32, u16>,
    pub focusable: bool,
    /// On the focus chain; the last widget on it is the focused one.
    pub in_focus_chain: bool,
    pub focused: bool,
    /// Index of the popup this widget lives in, bottom first; `None` for the
    /// main tree. A higher index is painted over a lower one.
    pub popup: Option<usize>,
    pub children: Vec<ElementInfo>,
}

/// The widget tree as laid out in the last frame: the main tree, then every
/// open popup's content, bottom first. Widgets with an empty rect are left
/// out with everything below them — nothing of them is on screen. Empty in
/// the middle of a tick, where the popups are borrowed.
pub fn element_tree(root: &dyn Widget) -> Vec<ElementInfo> {
    let chain = crate::get_focus_chain();
    let focused = chain.last().copied();
    let describe =
        |widget: &dyn Widget, popup: Option<usize>| describe_widget(widget, popup, &chain, focused);
    let mut roots: Vec<ElementInfo> = describe(root, None).into_iter().collect();
    if let Some(popups) = crate::runtime::with_popup_contents(|contents| {
        contents
            .iter()
            .enumerate()
            .filter_map(|(index, content)| describe(*content, Some(index)))
            .collect::<Vec<_>>()
    }) {
        roots.extend(popups);
    }
    roots
}

fn describe_widget(
    widget: &dyn Widget,
    popup: Option<usize>,
    chain: &[WidgetId],
    focused: Option<WidgetId>,
) -> Option<ElementInfo> {
    let rect = widget.get_rect();
    if rect.size.x == 0 || rect.size.y == 0 {
        return None;
    }
    let id = widget.get_identity();
    let mut children = Vec::new();
    widget.each_child(
        &mut |child| {
            if let Some(info) = describe_widget(child, popup, chain, focused) {
                children.push(info);
            }
        },
        Sign::Positive,
    );
    Some(ElementInfo {
        id: id.as_u64(),
        name: widget.get_name(),
        test_id: widget.get_layout().get_test_id().map(str::to_string),
        rect,
        focusable: widget.is_focusable(),
        in_focus_chain: chain.contains(&id),
        focused: focused == Some(id),
        popup,
        children,
    })
}

/// Whether the runtime runs without a terminal
/// ([`crate::runtime::start_headless`]).
pub fn is_headless() -> bool {
    crate::runtime::is_headless()
}
