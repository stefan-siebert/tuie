//! Application runtime: event loop, widget tree, popups, and clipboard.

pub mod clipboard;
pub mod event;
pub mod popup;
mod signals;
pub(crate) mod tree;

use crate::prelude::*;
use crate::runtime::popup::*;
use crate::widget::{clear_path_cache, walk_path_mut};

use crate::ansi::{self, output};
use crate::ansi::query::{
    QueryBatch, QueryCellPixelSize, QueryKittyGraphicsSupport, QueryMousePixelMode,
    QuerySixelSupport, QueryWindowPixelSize, QueryXtVersion,
};
use self::tree::*;
use std::{
    cell::RefCell,
    future::Future,
    io::Write,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex, atomic::{AtomicI16, AtomicU16, Ordering}, mpsc as sync_mpsc},
    time::Duration,
};

/// Kitty keyboard enhancement flags.
const KBD_FLAGS: u8 = 0b0000_0101;

#[cfg(unix)]
static WAKER: std::sync::OnceLock<(std::os::unix::net::UnixStream, std::os::unix::net::UnixStream)> =
    std::sync::OnceLock::new();

/// Windows runtime waker: a single auto-reset event signalled by [`poke_waker`]
/// and waited on by the reader, mirroring the Unix self-pipe.
#[cfg(windows)]
struct WindowsWaker {
    event: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
static WAKER: std::sync::OnceLock<WindowsWaker> = std::sync::OnceLock::new();

/// Cross-thread / signal-handler control flags drained on each runtime tick.
struct ControlFlags {
    /// Set by [`wake`]; produces [`RuntimeEvent::Wake`].
    wake_pending: std::sync::atomic::AtomicBool,
    /// Set by the `SIGTSTP` handler; produces [`RuntimeEvent::Suspend`].
    suspend_pending: std::sync::atomic::AtomicBool,
    /// Set by [`quit`]. `-1` = no pending quit, `0..=255` = exit code.
    quit_code: AtomicI16,
    /// Set by the SIGINT/SIGTERM/SIGHUP/SIGQUIT handler; `128 + signo`.
    quit_signal: std::sync::atomic::AtomicU8,
}

impl ControlFlags {
    const fn new() -> Self {
        Self {
            wake_pending: std::sync::atomic::AtomicBool::new(false),
            suspend_pending: std::sync::atomic::AtomicBool::new(false),
            quit_code: AtomicI16::new(-1),
            quit_signal: std::sync::atomic::AtomicU8::new(0),
        }
    }
}

static CONTROL: ControlFlags = ControlFlags::new();

/// Initializes the wake pipe and returns its read-end fd.
#[cfg(unix)]
fn init_waker() -> std::io::Result<crate::ansi::WakeHandle> {
    use std::os::unix::io::AsRawFd;
    let waker = WAKER.get_or_init(|| {
        let (read, write) = std::os::unix::net::UnixStream::pair().expect("wake pipe");
        let _ = read.set_nonblocking(true);
        let _ = write.set_nonblocking(true);
        (read, write)
    });
    Ok(waker.0.as_raw_fd())
}

/// Returns the write end of the wake pipe.
#[cfg(unix)]
fn waker_write() -> Option<&'static std::os::unix::net::UnixStream> {
    WAKER.get().map(|(_, write)| write)
}

/// Initializes the wake event and returns its handle for the reader's wait set.
#[cfg(windows)]
fn init_waker() -> std::io::Result<crate::ansi::WakeHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    let waker = WAKER.get_or_init(|| {
        // SAFETY: CreateEventW returns a fresh owned auto-reset event handle.
        let handle = unsafe {
            windows_sys::Win32::System::Threading::CreateEventW(
                std::ptr::null(), // default security attributes
                0,                // bManualReset = FALSE (auto-reset)
                0,                // bInitialState = FALSE
                std::ptr::null(), // unnamed
            )
        };
        assert!(!handle.is_null(), "CreateEventW failed for runtime waker");
        // SAFETY: we own the freshly created handle.
        let event = unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) };
        WindowsWaker { event }
    });
    Ok(waker.event.as_raw_handle())
}

/// Returns the wake event handle wrapper.
#[cfg(windows)]
fn waker_write() -> Option<&'static std::os::windows::io::OwnedHandle> {
    WAKER.get().map(|w| &w.event)
}

/// Signals the runtime waker.
fn poke_waker() {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        if let Some(write) = waker_write() {
            let _ = (&mut &*write).write(&[0u8]);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        if let Some(event) = waker_write() {
            // SAFETY: signalling a valid, owned auto-reset event.
            unsafe {
                windows_sys::Win32::System::Threading::SetEvent(event.as_raw_handle() as _);
            }
        }
    }
}

/// Drains pending control-state atomics into [`RuntimeEvent`]s.
fn drain_control() -> Vec<RuntimeEvent> {
    let mut out = Vec::new();
    if CONTROL.suspend_pending.swap(false, Ordering::AcqRel) {
        out.push(RuntimeEvent::Suspend);
    }
    let signal = CONTROL.quit_signal.swap(0, Ordering::AcqRel);
    if signal != 0 {
        out.push(RuntimeEvent::Quit(signal));
    }
    let code = CONTROL.quit_code.swap(-1, Ordering::AcqRel);
    if code >= 0 {
        out.push(RuntimeEvent::Quit(code as u8));
    }
    if CONTROL.wake_pending.swap(false, Ordering::AcqRel) {
        out.push(RuntimeEvent::Wake);
    }
    out
}

thread_local! {
    static EVENT_READER: RefCell<Option<crate::runtime::event::RuntimeEventReader>> =
        const { RefCell::new(None) };
}

/// Runtime configuration.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct TuiConfig {
    /// The minimum number of rows or columns between the cursor and the scroll edge.
    pub scrolloff: u16,
    /// The width of a tab character in cells.
    pub tabstop: u8,
    /// Whether inserted tabs are expanded to spaces.
    pub expand_tabs: bool,
    /// Whether the cursor blinks.
    pub cursor_blink: bool,
    /// Whether the terminal reports mouse hover events.
    pub hover_events: bool,
    /// Whether the runtime always keeps some widget selected.
    pub always_selected: bool,
}

crate::config_module!(TuiConfig {
    scrolloff: 1,
    tabstop: 8,
    expand_tabs: true,
    cursor_blink: true,
    hover_events: true,
    always_selected: false,
});

/// Host capabilities and dimensions.
#[derive(Clone)]
#[non_exhaustive]
pub struct RuntimeInfo {
    /// The surface size in cells.
    pub size: Vec2<u16>,
    /// The pixel size of a single cell, when reported by the host.
    pub cell_size: Option<Vec2<u16>>,
    /// Whether [`InputEvent::pos`](crate::widget::input::InputEvent::pos) carries real sub-cell precision.
    pub subcell_events: bool,
    /// The detected color scheme, when reported by the host.
    pub color_scheme: Option<ColorScheme>,
    /// The raw `XTVERSION` response, if the terminal replied.
    pub xtversion: Option<String>,
}

impl Default for RuntimeInfo {
    fn default() -> Self {
        Self {
            size: Vec2::of(0u16),
            cell_size: None,
            subcell_events: false,
            color_scheme: None,
            xtversion: None,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ImageCaps {
    pub supports_kitty_graphics: bool,
    pub supports_kitty_shm: bool,
    pub supports_sixel: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Tui,
    Gui,
}

#[derive(Clone, Copy)]
enum FrameRender {
    Skipped,
    Painted(Option<(CursorShape, Vec2<i32>)>),
}

impl FrameRender {
    fn painted(&self) -> bool {
        matches!(self, FrameRender::Painted(_))
    }
}

/// Position and size of the focused widget.
#[derive(Clone)]
#[non_exhaustive]
pub struct FocusedMeasure {
    /// The position of the focused widget in window coordinates.
    pub pos: Vec2<i32>,
    /// The size of the focused widget in cells.
    pub size: Vec2<u16>,
    /// The position of the visible portion of the focused widget after scroll clipping.
    pub visible_pos: Vec2<u16>,
    /// The size of the visible portion of the focused widget after scroll clipping.
    pub visible_size: Vec2<u16>,
}

struct EmittedEvent {
    source_id: WidgetId,
    payload: Box<dyn std::any::Any>,
}

struct IdVec<T> {
    items: Vec<(u64, T)>,
    next_id: u64,
}

impl<T> IdVec<T> {
    fn new() -> Self {
        Self { items: Vec::new(), next_id: 0 }
    }

    fn push(&mut self, value: T) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.items.push((id, value));
        id
    }

    fn take(&mut self, id: u64) -> Option<T> {
        let pos = self.items.iter().position(|(i, _)| *i == id)?;
        Some(self.items.swap_remove(pos).1)
    }

    fn reinsert(&mut self, id: u64, value: T) {
        self.items.push((id, value));
    }

    fn remove(&mut self, id: u64) {
        if let Some(pos) = self.items.iter().position(|(i, _)| *i == id) {
            self.items.swap_remove(pos);
        }
    }
}

type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type Spawner = Rc<dyn Fn(BoxFuture)>;
type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static>;

enum TaskKind {
    Timer(std::time::Instant, Box<dyn FnOnce(&mut dyn Widget)>),
    SpawnOnce(Box<dyn FnOnce(&mut dyn Widget, Box<dyn std::any::Any>)>),
    SpawnStream(Box<dyn FnMut(&mut dyn Widget, Box<dyn std::any::Any>) -> bool>),
    Quit(Box<dyn FnMut(&mut dyn Write)>),
}

enum NavigateOp {
    TabOrder(Sign),
    Directional(Direction2D),
}

struct RuntimeContext {
    mode: Mode,
    dirty: DirtyImpact,
    reveal_request: Option<WidgetId>,
    reveal_align: Vec2<Option<Align>>,
    focused_measure: Option<FocusedMeasure>,
    emitted: Vec<EmittedEvent>,
    tasks: IdVec<TaskKind>,
    spawn_tx: sync_mpsc::Sender<(u64, Box<dyn std::any::Any + Send>)>,
    spawn_rx: sync_mpsc::Receiver<(u64, Box<dyn std::any::Any + Send>)>,
    spawner: Option<Spawner>,
    focus_request: Option<WidgetId>,
    navigate_request: Option<NavigateOp>,
    focus_chain: Vec<WidgetId>,
    runtime_info: Option<RuntimeInfo>,
    image_caps: ImageCaps,
    pending_cell_px: Option<Vec2<u16>>,
    buffer: std::io::BufWriter<Box<dyn Write>>,
    panic_hook: Arc<Mutex<Option<PanicHook>>>,
    renderer: GridRenderer,
    terminal_initialized: bool,
    cursor_visible: bool,
    emulator_cursor: Option<(CursorShape, Vec2<i32>)>,
    /// Last DECSCUSR (shape, blink) emitted to the terminal. Re-emitting it on
    /// every frame makes terminals restart the cursor blink phase, so we only
    /// send it when it actually changes. `None` means "unknown / reset" — the
    /// next paint will (re-)emit it.
    last_cursor_style: Option<(CursorShape, bool)>,
    /// Last mouse pointer shape emitted via `OSC 22`. Diffed so we only re-emit
    /// on change. `None` means "unknown" — the next paint (re-)emits.
    last_mouse_pointer_shape: Option<MousePointerShape>,
    buf: String,
    mouse_pixel_dpr: Option<Vec2<u8>>,
}

impl RuntimeContext {
    fn new() -> Self {
        let (spawn_tx, spawn_rx) = sync_mpsc::channel();
        let buffer: std::io::BufWriter<Box<dyn Write>> =
            std::io::BufWriter::with_capacity(65536, Box::new(std::io::stdout()));
        RuntimeContext {
            mode: Mode::Tui,
            reveal_request: None,
            dirty: DirtyImpact::None,
            reveal_align: Vec2::of(None),
            focused_measure: None,
            emitted: Vec::new(),
            tasks: IdVec::new(),
            spawn_tx,
            spawn_rx,
            spawner: None,
            focus_request: None,
            navigate_request: None,
            focus_chain: Vec::new(),
            runtime_info: None,
            image_caps: ImageCaps::default(),
            pending_cell_px: None,
            buffer,
            panic_hook: Arc::new(Mutex::new(None)),
            renderer: GridRenderer::new(),
            terminal_initialized: false,
            cursor_visible: true,
            emulator_cursor: None,
            last_cursor_style: None,
            last_mouse_pointer_shape: None,
            buf: String::new(),
            mouse_pixel_dpr: None,
        }
    }
}

thread_local! {
    static RUNTIME_CTX: RefCell<RuntimeContext> = RefCell::new(RuntimeContext::new());
}

#[cfg(feature = "gui")]
static EVENT_LOOP_PROXY: std::sync::OnceLock<winit::event_loop::EventLoopProxy<()>> =
    std::sync::OnceLock::new();

static MOUSE_PIXEL_DPR: AtomicU16 = AtomicU16::new(0);

pub(crate) fn mouse_pixel_dpr() -> Option<Vec2<u8>> {
    let v = MOUSE_PIXEL_DPR.load(Ordering::Relaxed);
    if v == 0 {
        None
    } else {
        Some(Vec2::new((v >> 8) as u8, v as u8))
    }
}

fn set_mouse_pixel_dpr(dpr: Option<Vec2<u8>>) {
    let packed = dpr
        .map(|d| (u16::from(d.x.max(1)) << 8) | u16::from(d.y.max(1)))
        .unwrap_or(0);
    MOUSE_PIXEL_DPR.store(packed, Ordering::Relaxed);
}

static INBOX: Mutex<Vec<(WidgetId, Box<dyn std::any::Any + Send>)>> =
    Mutex::new(Vec::new());

/// Sends `msg` to the widget identified by `id` from any thread.
pub fn send<W: Widget + ?Sized, M: std::any::Any + Send>(id: WidgetId<W>, msg: M) {
    INBOX.lock().unwrap().push((id.untyped(), Box::new(msg)));
    wake();
}

fn drain_inbox(root: &mut dyn Widget) {
    loop {
        let messages = std::mem::take(&mut *INBOX.lock().unwrap());
        if messages.is_empty() {
            return;
        }
        for (target, payload) in messages {
            let payload: Box<dyn std::any::Any> = payload;
            if let Some(widget) = root.find_mut(target) {
                let mut event = WidgetEvent::new(target, payload);
                widget.on_event(&mut event);
            } else {
                // Not in the main tree — the target may live in an active popup
                // (e.g. a host widget forwarding messages into a popup dialog it
                // opened). Mirror `handle_state_change`'s root-then-popups
                // search. Safe to dispatch while borrowing the runtime: widget
                // callbacks reach the runtime only through the separate
                // `RUNTIME_CTX` / popup / inbox queues, never `with_runtime_mut`.
                with_runtime_mut(|rt| {
                    let index =
                        rt.popups.iter_mut().position(|p| p.content.find_mut(target).is_some());
                    if let Some(i) = index {
                        if let Some(widget) = rt.popups[i].content.find_mut(target) {
                            let mut event = WidgetEvent::new(target, payload);
                            widget.on_event(&mut event);
                        }
                    }
                });
            }
        }
    }
}

fn wake() {
    #[cfg(feature = "gui")]
    if let Some(p) = EVENT_LOOP_PROXY.get() {
        let _ = p.send_event(());
        return;
    }
    CONTROL.wake_pending.store(true, Ordering::Release);
    poke_waker();
}

/// Requests the runtime to exit with `code`.
pub fn quit(code: u8) {
    #[cfg(feature = "gui")]
    if is_gui() {
        try_with_gui_state(|s| s.pending_events.push(RuntimeEvent::Quit(code)));
        wake();
        return;
    }
    CONTROL.quit_code.store(code as i16, Ordering::Release);
    poke_waker();
}

/// Returns true when the runtime is in GUI mode.
pub fn is_gui() -> bool {
    with_ctx(|c| c.mode == Mode::Gui)
}

/// Returns the current [`RuntimeInfo`]. Before startup completes, returns the [`Default`]
/// (`size: 0`, `cell_size: None`, etc.) rather than panicking.
pub fn get_runtime_info() -> RuntimeInfo {
    with_ctx(|ctx| ctx.runtime_info.clone().unwrap_or_default())
}

#[cfg(feature = "images")]
pub(crate) fn get_image_caps() -> ImageCaps {
    with_ctx(|ctx| ctx.image_caps)
}

pub(crate) fn update_runtime_info(f: impl FnOnce(&mut RuntimeInfo)) {
    with_ctx_mut(|ctx| {
        if let Some(info) = ctx.runtime_info.as_mut() {
            f(info);
        }
    });
}

#[cfg(feature = "gui")]
pub(crate) fn sync_gui_grid_size(cells: Vec2<u16>, cell_px: Vec2<u16>) {
    with_ctx_mut(|ctx| {
        ctx.pending_cell_px = Some(cell_px);
        if let Some(info) = ctx.runtime_info.as_mut() {
            info.size = cells;
            info.cell_size = Some(cell_px);
        }
    });
}

#[cfg(unix)]
fn physical_cell_px() -> Option<Vec2<u16>> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc != 0 || ws.ws_col == 0 || ws.ws_row == 0 || ws.ws_xpixel == 0 || ws.ws_ypixel == 0 {
        return None;
    }
    Some(Vec2::new(ws.ws_xpixel / ws.ws_col, ws.ws_ypixel / ws.ws_row))
}

/// The Windows console exposes no per-cell pixel size via a local API; pixel
/// dimensions instead come from the VT `QueryCellPixelSize` / `QueryWindowPixelSize`
/// replies (in terminals that answer them), handled by the capability queries.
#[cfg(windows)]
fn physical_cell_px() -> Option<Vec2<u16>> {
    None
}

/// Returns the [`FocusedMeasure`] for the focused widget.
pub fn get_focused_measure() -> Option<FocusedMeasure> {
    with_ctx(|ctx| ctx.focused_measure.clone())
}

/// Marks layout dirty.
pub fn dirty_layout() {
    with_ctx_mut(|ctx| {
        ctx.dirty |= DirtyImpact::Layout;
    });
}

/// Marks paint dirty.
pub fn dirty_paint() {
    with_ctx_mut(|ctx| {
        ctx.dirty |= DirtyImpact::Paint;
    });
}

thread_local! {
    /// The widget the pointer is hovering for a resize affordance, plus the
    /// pointer position in that widget's local frame. Set by `paint_terminal`
    /// each frame and read by widgets during render (e.g. `Split` to highlight
    /// the hovered divider).
    static RESIZE_HOVER: std::cell::Cell<Option<(WidgetId, Vec2<i32>)>> =
        const { std::cell::Cell::new(None) };
}

pub(crate) fn set_resize_hover(value: Option<(WidgetId, Vec2<i32>)>) {
    RESIZE_HOVER.with(|c| c.set(value));
}

/// The local pointer position when `id` is the resize-hovered widget this frame.
pub(crate) fn resize_hover_for(id: WidgetId) -> Option<Vec2<i32>> {
    RESIZE_HOVER.with(|c| c.get()).and_then(|(hover_id, local)| {
        (hover_id == id).then_some(local)
    })
}

/// Scrolls to bring the widget with `id` into view, pinned by `align`.
pub fn reveal(id: WidgetId<impl ?Sized>, align: Vec2<Option<Align>>) {
    let id = id.untyped();
    with_ctx_mut(|ctx| {
        ctx.reveal_request = Some(id);
        ctx.reveal_align = align;
    });
}

fn set_dirty(dirty: DirtyImpact) {
    with_ctx_mut(|ctx| {
        ctx.dirty = dirty;
    });
}

fn get_dirty() -> DirtyImpact {
    with_ctx(|ctx| ctx.dirty)
}

fn needs_layout(w: &mut dyn Widget) -> bool {
    get_dirty() == DirtyImpact::Layout || check_dirty(w) == DirtyImpact::Layout
}

/// Focuses the first focusable widget if nothing is currently focused.
pub fn ensure_focused() {
    if get_focus_chain().is_empty() {
        focus_next_tab_order(Sign::Positive);
    }
}

/// Moves focus to the next focusable widget in tab order along `direction`.
pub fn focus_next_tab_order(direction: Sign) {
    with_ctx_mut(|ctx| ctx.navigate_request = Some(NavigateOp::TabOrder(direction)));
}

/// Moves focus to the nearest focusable widget in `direction`.
pub fn focus_next_directionally(direction: Direction2D) {
    with_ctx_mut(|ctx| ctx.navigate_request = Some(NavigateOp::Directional(direction)));
}

/// Queues `payload` for delivery up the ancestor chain of `source_id`.
pub fn emit<P: std::any::Any + Send + 'static>(source_id: WidgetId<impl ?Sized>, payload: P) {
    let source_id = source_id.untyped();
    with_ctx_mut(|ctx| {
        ctx.emitted.push(EmittedEvent {
            source_id,
            payload: Box::new(payload),
        });
    });
}

/// A handle to a scheduled timer or spawned job.
#[derive(PartialEq)]
pub struct TaskHandle(u64);

impl TaskHandle {
    /// Cancels the task.
    pub fn cancel(&self) {
        if self.0 == 0 {
            return;
        }
        with_ctx_mut(|ctx| ctx.tasks.remove(self.0));
    }

    /// A sentinel handle that refers to no task.
    pub const EMPTY: Self = Self(0);
}

/// Schedules `callback` to run on the widget after `duration`.
pub fn schedule<W: Widget + 'static>(
    id: WidgetId<W>,
    duration: std::time::Duration,
    callback: impl FnOnce(&mut W) + 'static,
) -> TaskHandle {
    with_ctx_mut(|ctx| {
        let task_id = ctx.tasks.push(TaskKind::Timer(
            std::time::Instant::now() + duration,
            Box::new(move |root: &mut dyn Widget| {
                if let Some(w) = root.get_widget_mut(id) {
                    callback(w);
                }
            }),
        ));
        TaskHandle(task_id)
    })
}

/// Installs the executor used by [`spawn`] and [`spawn_stream`].
///
/// # Example
///
/// ```no_run
/// tuie::set_spawner(|fut| {
///     std::thread::spawn(move || {
///         futures::executor::block_on(fut);
///     });
/// });
/// ```
pub fn set_spawner(spawner: impl Fn(BoxFuture) + Send + 'static) {
    with_ctx_mut(|ctx| {
        ctx.spawner = Some(Rc::new(spawner));
    });
}

/// Spawns `work` and delivers its result to the widget on completion.
///
/// # Panics
///
/// Panics if no spawner is registered.
pub fn spawn<W: Widget + 'static, T: Send + 'static>(
    id: WidgetId<W>,
    work: impl Future<Output = T> + Send + 'static,
    on_done: impl FnOnce(&mut W, T) + 'static,
) -> TaskHandle {
    let (spawner, tx, job_id) = with_ctx_mut(|ctx| {
        let spawner = ctx.spawner.as_ref().expect("no spawner registered; call set_spawner first").clone();
        let job_id = ctx.tasks.push(TaskKind::SpawnOnce(Box::new(move |root, val| {
            if let Some(w) = root.get_widget_mut(id) {
                on_done(w, *val.downcast::<T>().unwrap());
            }
        })));
        let tx = ctx.spawn_tx.clone();
        (spawner, tx, job_id)
    });
    spawner(Box::pin(async move {
        let result = work.await;
        let _ = tx.send((job_id, Box::new(result)));
        wake();
    }));
    TaskHandle(job_id)
}

/// Spawns `work` and delivers each produced value to the widget via `on_value`.
///
/// # Panics
///
/// Panics if no spawner is registered.
pub fn spawn_stream<W: Widget + 'static, T: Send + 'static, Fut: Future<Output = ()> + Send + 'static>(
    id: WidgetId<W>,
    work: impl FnOnce(Box<dyn Fn(T) + Send>) -> Fut + Send + 'static,
    mut on_value: impl FnMut(&mut W, Option<T>) + 'static,
) -> TaskHandle {
    let (spawner, tx, job_id) = with_ctx_mut(|ctx| {
        let spawner = ctx.spawner.as_ref().expect("no spawner registered; call set_spawner first").clone();
        let job_id = ctx.tasks.push(TaskKind::SpawnStream(Box::new(move |root: &mut dyn Widget, val: Box<dyn std::any::Any>| {
            let opt = *val.downcast::<Option<T>>().unwrap();
            let done = opt.is_none();
            if let Some(w) = root.get_widget_mut(id) {
                on_value(w, opt);
            }
            done
        })));
        let tx = ctx.spawn_tx.clone();
        (spawner, tx, job_id)
    });
    let send_tx = tx.clone();
    let send = Box::new(move |value: T| {
        let _ = send_tx.send((job_id, Box::new(Some(value))));
        wake();
    });
    spawner(Box::pin(async move {
        work(send).await;
        let _ = tx.send((job_id, Box::new(None::<T>)));
        wake();
    }));
    TaskHandle(job_id)
}

fn drain_spawn_completions(root: &mut dyn Widget) {
    let messages: Vec<_> = with_ctx_mut(|ctx| {
        ctx.spawn_rx.try_iter().collect()
    });
    for (id, result) in messages {
        let Some(kind) = with_ctx_mut(|ctx| ctx.tasks.take(id)) else {
            continue;
        };
        match kind {
            TaskKind::SpawnOnce(cb) => {
                cb(root, result);
            }
            TaskKind::SpawnStream(mut cb) => {
                if !cb(root, result) {
                    with_ctx_mut(|ctx| {
                        ctx.tasks.reinsert(id, TaskKind::SpawnStream(cb));
                    });
                }
            }
            TaskKind::Timer(..) | TaskKind::Quit(..) => unreachable!("spawn channel id mapped to non-spawn task"),
        }
    }
}

/// Requests focus to move to the widget with `id`.
pub fn focus_widget(id: WidgetId<impl ?Sized>) {
    with_ctx_mut(|ctx| ctx.focus_request = Some(id.untyped()));
}

fn take_focus_request() -> Option<WidgetId> {
    with_ctx_mut(|ctx| ctx.focus_request.take())
}

fn set_focus_chain(path: &[WidgetId]) {
    with_ctx_mut(|ctx| {
        ctx.focus_chain.clear();
        ctx.focus_chain.extend_from_slice(path);
    });
}

/// Returns the focus chain from root down to the focused leaf.
pub fn get_focus_chain() -> Vec<WidgetId> {
    with_ctx(|ctx| ctx.focus_chain.clone())
}

/// Returns the id of the focused leaf.
pub fn get_focused_widget() -> Option<WidgetId> {
    with_ctx(|ctx| ctx.focus_chain.last().copied())
}

/// Returns true when `id` is the focused leaf widget.
pub fn is_focused(id: WidgetId<impl ?Sized>) -> bool {
    let id = id.untyped();
    with_ctx(|ctx| ctx.focus_chain.last() == Some(&id))
}

/// Returns true when `id` lies anywhere on the focus chain.
pub fn in_focus_chain(id: WidgetId<impl ?Sized>) -> bool {
    let id = id.untyped();
    with_ctx(|ctx| ctx.focus_chain.contains(&id))
}

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::new());
}

#[cfg(feature = "gui")]
thread_local! {
    static GUI: RefCell<Option<crate::gui::Gui>> = const { RefCell::new(None) };
}

fn with_runtime_mut<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    RUNTIME.with_borrow_mut(f)
}

fn with_ctx<R>(f: impl FnOnce(&RuntimeContext) -> R) -> R {
    RUNTIME_CTX.with_borrow(f)
}

fn with_ctx_mut<R>(f: impl FnOnce(&mut RuntimeContext) -> R) -> R {
    RUNTIME_CTX.with_borrow_mut(f)
}

fn with_render_io<R>(
    f: impl FnOnce(&mut GridRenderer, &mut std::io::BufWriter<Box<dyn Write>>, &mut String) -> R,
) -> R {
    let (mut renderer, mut buffer, mut buf) = with_ctx_mut(|ctx| {
        (
            std::mem::take(&mut ctx.renderer),
            std::mem::replace(
                &mut ctx.buffer,
                std::io::BufWriter::with_capacity(0, Box::new(std::io::sink())),
            ),
            std::mem::take(&mut ctx.buf),
        )
    });
    let out = f(&mut renderer, &mut buffer, &mut buf);
    with_ctx_mut(|ctx| {
        ctx.renderer = renderer;
        ctx.buffer = buffer;
        ctx.buf = buf;
    });
    out
}

/// Registers a callback invoked just before the runtime tears down.
pub fn on_quit(cb: impl FnMut(&mut dyn Write) + 'static) -> TaskHandle {
    with_ctx_mut(|ctx| {
        let id = ctx.tasks.push(TaskKind::Quit(Box::new(cb)));
        TaskHandle(id)
    })
}

fn run_quit_handlers() {
    let mut out = Vec::new();
    for cb in take_quit_handlers().iter_mut() {
        cb(&mut out);
    }
    with_ctx_mut(|ctx| {
        let _ = ctx.buffer.write_all(&out);
    });
}

fn take_quit_handlers() -> Vec<Box<dyn FnMut(&mut dyn Write)>> {
    with_ctx_mut(|ctx| {
        let mut out = Vec::new();
        let mut i = ctx.tasks.items.len();
        while i > 0 {
            i -= 1;
            if matches!(ctx.tasks.items[i].1, TaskKind::Quit(_))
                && let (_, TaskKind::Quit(cb)) = ctx.tasks.items.swap_remove(i)
            {
                out.push(cb);
            }
        }
        out.reverse();
        out
    })
}

fn make_queue(events: &[InputEvent], flushing: bool, unhandled: bool) -> InputQueue<'_> {
    if flushing {
        InputQueue::new_flushing(events, unhandled)
    } else {
        InputQueue::new(events, unhandled)
    }
}

fn dispatch_input(target: &mut dyn Widget, key_queue: &mut [InputEvent], flushing: bool, unhandled: bool) -> (InputResult, usize) {
    let content_pos = target.get_pos().map(|v| v as f32);
    for event in key_queue.iter_mut() {
        event.pos -= content_pos;
    }
    let mut queue = make_queue(key_queue, flushing, unhandled);
    let result = target.on_input(&mut queue);
    (result, queue.get_consumed_count())
}

fn process_keys_interleaved(root: &mut dyn Widget) {
    loop {
        let (path, key_queue_empty) = with_runtime_mut(|rt| {
            (rt.focus_chain.clone(), rt.key_queue.is_empty())
        });
        if key_queue_empty {
            with_runtime_mut(|rt| rt.key_queue_flushing = false);
            break;
        }

        let mut handled = false;
        let mut pending = false;
        let mut total_consumed = 0;

        let passes: Vec<(usize, bool)> = if path.is_empty() {
            vec![(0, false), (0, true)]
        } else {
            (0..path.len()).map(|i| (i, false))
                .chain((0..path.len()).rev().map(|i| (i, true)))
                .collect()
        };

        with_runtime_mut(|rt| {
            for (depth, unhandled) in passes {
                if rt.key_queue.is_empty() {
                    break;
                }
                let mut key_queue = rt.key_queue.clone();
                let flushing = rt.key_queue_flushing;
                let (result, consumed) = {
                    let active_root = rt.get_active_root_mut(root);
                    let target = if path.is_empty() {
                        active_root
                    } else {
                        match walk_path_mut(active_root, &path[..=depth]) {
                            Some(target) => target,
                            None => continue,
                        }
                    };
                    dispatch_input(target, &mut key_queue, flushing, unhandled)
                };
                rt.flush_events(root);
                match result {
                    InputResult::Handled if consumed > 0 => {
                        handled = true;
                        total_consumed = consumed;
                        break;
                    }
                    InputResult::Pending => {
                        pending = true;
                        break;
                    }
                    _ => {}
                }
            }
        });

        let cont = with_runtime_mut(|rt| {
            if handled {
                let drain = total_consumed.min(rt.key_queue.len());
                rt.key_queue.drain(..drain);
                return true;
            }
            if pending {
                rt.handle_pending_queue();
                return false;
            }

            if rt.key_queue.is_empty() {
                return false;
            }
            if let Some(idx) = rt.get_active_popup_index() {
                let chord = &rt.key_queue[0].chord;
                let is_dismiss_key = matches!(chord.trigger, Trigger::Key(Key::Enter) | Trigger::Key(Key::Esc))
                    || (chord.trigger == Trigger::Key(Key::Char('c')) && chord.modifiers == Modifiers::new().with(Modifier::Ctrl));
                if is_dismiss_key {
                    rt.key_queue.drain(..1);
                    if rt.popups[idx].dismissible {
                        rt.close_popup_at(root, idx);
                    } else {
                        rt.request_dismiss_popup(root, idx);
                    }
                    return true;
                }
            }
            rt.key_queue.drain(..1);
            true
        });
        if !cont {
            break;
        }
    }
}

/// Replaces the writer used to flush rendered frames.
pub fn set_output(output: impl Write + 'static) {
    with_ctx_mut(|ctx| {
        ctx.buffer = std::io::BufWriter::with_capacity(65536, Box::new(output));
    });
}

/// Enters the alternate screen and enables raw mode.
pub fn enable() -> std::io::Result<()> {
    with_ctx_mut(|ctx| ctx.enable())
}

/// Restores the terminal state changed by [`enable`].
pub fn disable() -> std::io::Result<()> {
    with_ctx_mut(|ctx| ctx.disable())
}

/// Requests a suspend: the next runtime tick disables the terminal, raises `SIGSTOP`, and
/// re-enables on resume. Deferred so it is safe to call from within widget callbacks.
pub fn suspend() {
    CONTROL.suspend_pending.store(true, Ordering::Release);
    poke_waker();
}

pub(crate) fn update(
    root: &mut dyn Widget,
    events: &[RuntimeEvent],
) -> std::io::Result<Option<std::time::Duration>> {
    let inactive = with_ctx(|ctx| ctx.panic_hook.lock().unwrap().is_none());
    if inactive {
        with_ctx_mut(|ctx| ctx.enable())?;
    }

    for event in events {
        match event {
            RuntimeEvent::Suspend => {
                log::debug!("tui: suspend");
                with_ctx_mut(|ctx| ctx.suspend())?;
                dirty_layout();
            }
            RuntimeEvent::Resize(size) => {
                let cell_px = match with_ctx(|c| c.mode) {
                    Mode::Tui => physical_cell_px(),
                    #[cfg(feature = "gui")]
                    Mode::Gui => try_with_gui_state(|s| s.font_cell_px()),
                    #[cfg(not(feature = "gui"))]
                    Mode::Gui => unreachable!(),
                };
                with_ctx_mut(|ctx| {
                    if let Some(px) = cell_px {
                        ctx.pending_cell_px = Some(px);
                    }
                    if let Some(info) = ctx.runtime_info.as_mut() {
                        info.size = *size;
                        if let Some(new_physical) = cell_px {
                            info.cell_size = Some(new_physical);
                        }
                    }
                });
            }
            _ => {}
        }
    }

    drain_spawn_completions(root);
    drain_inbox(root);

    let need_flush = with_runtime_mut(|rt| {
        rt.drain_task_queue(root);
        if rt.key_queue_deadline.map(|d| d <= std::time::Instant::now()).unwrap_or(false) {
            rt.key_queue_deadline = None;
            rt.key_queue_flushing = true;
            true
        } else {
            false
        }
    });
    if need_flush {
        process_keys_interleaved(root);
    }
    for event in events {
        let keys_enqueued = with_runtime_mut(|rt| rt.handle_event_inner(root, event.clone()))?;
        if keys_enqueued {
            process_keys_interleaved(root);
        }
    }
    let next_timeout = with_runtime_mut(|rt| rt.handle_events_finish(root));

    #[cfg(feature = "gui")]
    if let Some(Some(deadline)) = try_with_gui_state(|s| s.next_blink_wake)
        && std::time::Instant::now() >= deadline
    {
        dirty_paint();
    }

    let outcome = with_runtime_mut(|rt| rt.layout_and_render(root))?;

    if is_gui()
        && let FrameRender::Painted(cursor) = outcome
    {
        #[cfg(feature = "gui")]
        with_runtime_mut(|rt| rt.present_gui(root, cursor));
        #[cfg(not(feature = "gui"))]
        let _ = cursor;
        }

    #[cfg(feature = "gui")]
    let next_timeout = {
        let blink_wake = try_with_gui_state(|s| s.next_blink_wake).flatten();
        match blink_wake {
            Some(deadline) => {
                let blink_dur = deadline.saturating_duration_since(std::time::Instant::now());
                Some(next_timeout.unwrap_or(std::time::Duration::MAX).min(blink_dur))
            }
            None => next_timeout,
        }
    };

    if outcome.painted() {
        let pending = with_runtime_mut(|rt| rt.take_pending_events());
        if !pending.is_empty() {
            for event in pending {
                let keys_enqueued = with_runtime_mut(|rt| rt.handle_event_inner(root, event))?;
                if keys_enqueued {
                    process_keys_interleaved(root);
                }
            }
        }
    }

    Ok(next_timeout)
}

/// Runs the runtime in terminal mode and returns the exit code.
pub fn start_tui(
    root: Box<dyn Widget>,
) -> std::io::Result<std::process::ExitCode> {
    with_ctx_mut(|c| c.mode = Mode::Tui);
    let mut root = root;
    run_terminal(&mut *root).map(std::process::ExitCode::from)
}

/// Runs the runtime in GUI mode and returns the exit code.
#[cfg(feature = "gui")]
pub fn start_gui(
    root: Box<dyn Widget>,
) -> std::io::Result<std::process::ExitCode> {
    with_ctx_mut(|c| c.mode = Mode::Gui);
    with_ctx_mut(|ctx| {
        ctx.buffer =
            std::io::BufWriter::with_capacity(4096, Box::new(std::io::sink()));
    });
    let result = (|| -> std::io::Result<u8> {
        with_ctx_mut(|ctx| ctx.enable())?;
        let event_loop = GUI
            .with_borrow_mut(|g| {
                g.as_mut()
                    .expect("tuie: gui not initialized")
                    .event_loop
                    .take()
            })
            .ok_or_else(|| std::io::Error::other("tuie: event loop already consumed"))?;
        let _ = EVENT_LOOP_PROXY.set(event_loop.create_proxy());
        let mut handler = crate::gui::RunHandler::new(root);
        event_loop
            .run_app(&mut handler)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let code = try_with_gui_state(|s| s.exit_code).flatten().unwrap_or(0);
        run_quit_handlers();
        Ok(code)
    })();
    let _ = with_ctx_mut(|ctx| ctx.disable());
    result.map(std::process::ExitCode::from)
}

fn run_terminal(root: &mut dyn Widget) -> std::io::Result<u8> {
    let result = (|| -> std::io::Result<u8> {
        let mut events: Vec<RuntimeEvent> = Vec::new();
        loop {
            if let Some(code) = events.iter().find_map(|e| match e {
                RuntimeEvent::Quit(c) => Some(*c),
                _ => None,
            }) {
                run_quit_handlers();
                return Ok(code);
            }
            let timeout = update(root, &events)?;
            events = read(timeout)?;
        }
    })();
    let _ = with_ctx_mut(|ctx| ctx.disable());
    result
}

/// Resets the runtime to a clean state for an in-process emulator at `size`.
pub(crate) fn init_emulator(size: Vec2<u16>) {
    RUNTIME.with_borrow_mut(|rt| *rt = Runtime::new());
    with_ctx_mut(|ctx| *ctx = RuntimeContext::new());
    with_ctx_mut(|ctx| {
        *ctx.panic_hook.lock().unwrap() = Some(Box::new(|_| {}));
    });
    set_output(std::io::sink());
    with_ctx_mut(|ctx| {
        ctx.runtime_info = Some(RuntimeInfo { size, ..Default::default() });
        ctx.image_caps = ImageCaps::default();
    });
}

/// Returns the most recently rendered frame as a [`crate::render::style::StyledString`] snapshot.
pub(crate) fn get_emulator_snapshot() -> crate::render::style::StyledString {
    with_ctx(|ctx| ctx.renderer.get_snapshot())
}

/// Returns the cursor from the most recently rendered emulator frame.
pub(crate) fn get_emulator_cursor() -> Option<(CursorShape, Vec2<i32>)> {
    with_ctx(|ctx| ctx.emulator_cursor)
}

fn read(timeout: Option<Duration>) -> std::io::Result<Vec<RuntimeEvent>> {
    let mut out = drain_control();
    let t = if out.is_empty() { timeout } else { Some(Duration::ZERO) };
    let (input, _woken) = EVENT_READER.with_borrow_mut(|r| {
        r.as_mut()
            .expect("tuie: event reader not initialized")
            .read_batch(t)
    })?;
    out.extend(input);
    out.append(&mut drain_control());
    Ok(out)
}

#[cfg(feature = "gui")]
pub(crate) fn with_gui_state<R>(f: impl FnOnce(&mut crate::gui::GuiState) -> R) -> R {
    GUI.with_borrow_mut(|g| {
        let gui = g.as_mut().expect("gui not initialized");
        f(&mut gui.state)
    })
}

#[cfg(feature = "gui")]
pub(crate) fn try_with_gui_state<R>(f: impl FnOnce(&mut crate::gui::GuiState) -> R) -> Option<R> {
    GUI.with_borrow_mut(|g| g.as_mut().map(|gui| f(&mut gui.state)))
}

#[cfg(feature = "gui")]
pub(crate) fn take_pending_gui_events() -> Vec<RuntimeEvent> {
    try_with_gui_state(|s| std::mem::take(&mut s.pending_events)).unwrap_or_default()
}

struct Runtime {
    last_scroll: std::time::Instant,
    scroll_held: bool,
    scroll_accum: Vec2<f32>,
    scroll_pos: Vec2<f32>,
    mouse_pos: Vec2<f32>,
    dragging: bool,
    scroll_path: Vec<WidgetId>,
    mouse_path: Vec<WidgetId>,
    focus_chain: Vec<WidgetId>,
    curswant: Vec2<i32>,
    key_queue: Vec<InputEvent>,
    key_queue_flushing: bool,
    key_queue_deadline: Option<std::time::Instant>,
    has_rendered: bool,
    pending_events: Vec<RuntimeEvent>,
    popups: Vec<crate::runtime::popup::ActivePopup>,
}

fn find_visible_focusable(
    widget: &dyn Widget,
    clip: Vec2<(i32, i32)>,
    direction: Sign,
) -> Option<WidgetId> {
    let clip = Axis2D::map(|a| match widget.get_scroll_clip_range()[a] {
        Some((start, end)) => (clip[a].0.max(start), clip[a].1.min(end)),
        None => clip[a],
    });
    if clip.x.0 >= clip.x.1 || clip.y.0 >= clip.y.1 {
        return None;
    }
    let mut result = None;
    widget.each_child(
        &mut |child| {
            if result.is_some() {
                return;
            }
            if child.get_layer() > Layer::Bottom {
                return;
            }
            let child_rect = child.get_rect();
            if child_rect.pos.x + child_rect.size.x as i32 <= clip.x.0 || child_rect.pos.x >= clip.x.1
                || child_rect.pos.y + child_rect.size.y as i32 <= clip.y.0 || child_rect.pos.y >= clip.y.1
            {
                return;
            }
            if child.is_focusable() || child.get_focus_target().is_some() {
                result = Some(child.get_id());
                return;
            }
            result = find_visible_focusable(child, clip, direction);
        },
        direction,
    );
    result
}

enum PopupHitResult {
    Hit(usize),
    Blocked,
    Miss,
}

impl Runtime {
    fn popup_hit_test(&self, pos: Vec2<i32>) -> PopupHitResult {
        if let Some(i) = self.popups.len().checked_sub(1) {
            if self.popups[i].content.get_rect().contains_point(pos) {
                return PopupHitResult::Hit(i);
            }
            return PopupHitResult::Blocked;
        }
        PopupHitResult::Miss
    }

    fn new() -> Self {
        Self {
            scroll_path: Vec::new(),
            dragging: false,
            mouse_path: Vec::new(),
            last_scroll: std::time::Instant::now(),
            scroll_held: false,
            scroll_accum: Vec2::new(0.0, 0.0),
            scroll_pos: Vec2::of(-1.0),
            mouse_pos: Vec2::of(-1.0),

            focus_chain: Vec::new(),
            curswant: Vec2::of(0i32),
            key_queue: Vec::new(),
            key_queue_flushing: false,
            key_queue_deadline: None,
            has_rendered: false,
            pending_events: Vec::new(),
            popups: Vec::new(),
        }
    }
}

impl RuntimeContext {
    fn enable(&mut self) -> std::io::Result<()> {
        match self.mode {
            Mode::Tui => self.enable_terminal(),
            #[cfg(feature = "gui")]
            Mode::Gui => self.enable_gui(),
            #[cfg(not(feature = "gui"))]
            Mode::Gui => unreachable!(),
        }
    }

    fn enable_terminal(&mut self) -> std::io::Result<()> {
        let newly_installed = {
            let mut guard = self.panic_hook.lock().unwrap();
            if guard.is_none() {
                *guard = Some(std::panic::take_hook());
                true
            } else {
                false
            }
        };
        if newly_installed {
            let panic_hook_mutex = self.panic_hook.clone();
            std::panic::set_hook(Box::new(move |info| {
                let mut guard = match panic_hook_mutex.lock() {
                    Ok(ok) => ok,
                    Err(err) => err.into_inner(),
                };
                if let Some(panic_hook) = guard.take() {
                    let _ = ansi::disable_raw_mode();
                    #[cfg(feature = "harmonious")]
                    crate::theme::harmonious::clear_palette();
                    let mut buf = String::new();
                    output::reset_cursor_style(&mut buf);
                    output::show_cursor(&mut buf);
                    output::set_mouse_pointer_shape(&mut buf, MousePointerShape::Default);
                    output::leave_alternate_screen(&mut buf);
                    output::pop_keyboard_enhancement_flags(&mut buf);
                    output::disable_mouse_pixel_capture(&mut buf);
                    output::disable_mouse_capture(&mut buf);
                    output::disable_focus_change(&mut buf);
                    output::disable_bracketed_paste(&mut buf);
                    #[cfg(feature = "harmonious")]
                    output::disable_color_scheme_detection(&mut buf);
                    let mut stderr = std::io::stderr();
                    let _ = stderr.write_all(buf.as_bytes());
                    let _ = stderr.flush();
                    panic_hook(info);
                }
            }));
        }

        match self.enable_after_hook_terminal() {
            Ok(()) => Ok(()),
            Err(e) => {
                if newly_installed {
                    let _ = ansi::disable_raw_mode();
                    if let Some(old) = self.panic_hook.lock().unwrap().take() {
                        std::panic::set_hook(old);
                    }
                }
                Err(e)
            }
        }
    }

    #[cfg(feature = "gui")]
    fn enable_gui(&mut self) -> std::io::Result<()> {
        match self.enable_after_hook_gui() {
            Ok(()) => {
                let mut guard = self.panic_hook.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(Box::new(|_| {}));
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    #[cfg(feature = "gui")]
    fn enable_after_hook_gui(&mut self) -> std::io::Result<()> {
        let needs_init = GUI.with_borrow(|g| g.is_none());
        if needs_init {
            let gui = crate::gui::Gui::new()?;
            GUI.with_borrow_mut(|g| *g = Some(gui));
            #[cfg(feature = "harmonious")]
            crate::theme::harmonious::apply_palette(
                crate::theme::harmonious::Palette::from_theme(
                    crate::gui::config::get().dark_theme,
                ),
            );
        }
        let (cell_size, cell_px) = with_gui_state(|s| (s.cell_size, s.font_cell_px()));
        self.pending_cell_px = Some(cell_px);
        self.runtime_info = Some(RuntimeInfo {
            size: cell_size,
            cell_size: Some(cell_px),
            subcell_events: true,
            ..Default::default()
        });
        self.image_caps = ImageCaps::default();
        self.renderer.clear();
        self.cursor_visible = true;
        self.dirty |= DirtyImpact::Layout;
        Ok(())
    }

    fn enable_after_hook_terminal(&mut self) -> std::io::Result<()> {
        log::debug!("tui: entering terminal (raw mode, alt screen)");
        ansi::enable_raw_mode()?;
        // (Re-)entering the terminal leaves the cursor at its default style;
        // force the next paint to emit DECSCUSR rather than trusting the cache.
        self.last_cursor_style = None;
        let initial_size: Vec2<u16> = ansi::size()?.into();

        if !std::mem::replace(&mut self.terminal_initialized, true) {
            {
                let mut batch = QueryBatch::new().timeout(std::time::Duration::from_millis(100));
                let kitty_h = batch.add(QueryKittyGraphicsSupport);
                let sixel_h = batch.add(QuerySixelSupport);
                let xtver = batch.add(QueryXtVersion);
                let mouse_px_h = batch.add(QueryMousePixelMode);
                let win_px_h = batch.add(QueryWindowPixelSize);
                let cell_px_h = batch.add(QueryCellPixelSize);
                #[cfg(feature = "harmonious")]
                let color_handles = crate::theme::harmonious::add_color_queries(&mut batch);

                let physical = physical_cell_px();
                let mut info = RuntimeInfo {
                    size: initial_size,
                    cell_size: physical,
                    ..Default::default()
                };
                let mut caps = ImageCaps::default();

                if let Ok(results) = batch.execute() {
                    let kitty_reply = results.get(&kitty_h).unwrap_or(None);
                    caps.supports_kitty_graphics = kitty_reply.is_some();
                    caps.supports_kitty_shm = kitty_reply == Some(true);
                    caps.supports_sixel = results.get(&sixel_h).unwrap_or(false);
                    if let Some(px) = results.get(&cell_px_h).unwrap_or(None) {
                        info.cell_size = Some(Vec2::new(px.width, px.height));
                    }
                    if let (Some(win), Some(cell)) = (
                        results.get(&win_px_h).unwrap_or(None),
                        info.cell_size,
                    ) {
                        let round_div = |num: u32, den: u32| {
                            let den = den.max(1);
                            ((num + den / 2) / den).clamp(1, 4) as u8
                        };
                        self.mouse_pixel_dpr = Some(Vec2::new(
                            round_div(
                                cell.x as u32 * initial_size.x as u32,
                                win.width as u32,
                            ),
                            round_div(
                                cell.y as u32 * initial_size.y as u32,
                                win.height as u32,
                            ),
                        ));
                    }
                    info.subcell_events =
                        results.get(&mouse_px_h).unwrap_or(None).unwrap_or(false)
                            && self.mouse_pixel_dpr.is_some();
                    if let Some(ver) = results.get(&xtver).unwrap_or(None) {
                        if ver.starts_with("WezTerm ") {
                            caps.supports_kitty_graphics = false;
                            caps.supports_kitty_shm = false;
                        }
                        info.xtversion = Some(ver);
                    }
                    #[cfg(feature = "harmonious")]
                    if let Ok(palette) = crate::theme::harmonious::build_palette_from_batch(color_handles, &results) {
                        crate::theme::harmonious::apply_palette(palette);
                    }
                }

                log::debug!(
                    "tui ready: size={:?} cell_size={:?} subcell_events={} xtversion={:?}",
                    (info.size.x, info.size.y),
                    info.cell_size,
                    info.subcell_events,
                    info.xtversion,
                );

                #[cfg(all(unix, feature = "images"))]
                crate::render::image::shm::unlink_probe();

                self.pending_cell_px = info.cell_size;
                self.runtime_info = Some(info);
                self.image_caps = caps;
            }

            let wake_fd = init_waker()?;
            let mut reader = crate::runtime::event::RuntimeEventReader::new()?;
            reader.set_wake_fd(wake_fd);
            EVENT_READER.with_borrow_mut(|r| *r = Some(reader));
            let _ = signals::install(waker_write());
        } else if let Some(info) = self.runtime_info.as_mut() {
            info.size = initial_size;
        }

        self.buf.clear();
        output::begin_synchronized_update(&mut self.buf);
        output::enter_alternate_screen(&mut self.buf);
        output::enable_mouse_click_events(&mut self.buf);
        output::enable_mouse_drag_events(&mut self.buf);
        if config::get().hover_events {
            output::enable_mouse_hover_events(&mut self.buf);
        }
        output::enable_sgr_mouse(&mut self.buf);
        let pixel_mouse = self
            .runtime_info
            .as_ref()
            .map(|i| i.subcell_events)
            .unwrap_or(false);
        if pixel_mouse {
            output::enable_mouse_pixel_capture(&mut self.buf);
            set_mouse_pixel_dpr(self.mouse_pixel_dpr);
        } else {
            set_mouse_pixel_dpr(None);
        }
        output::enable_focus_change(&mut self.buf);
        output::enable_bracketed_paste(&mut self.buf);
        #[cfg(feature = "harmonious")]
        output::enable_color_scheme_detection(&mut self.buf);
        output::push_keyboard_enhancement_flags(&mut self.buf, KBD_FLAGS);

        self.renderer.clear();
        output::clear_screen(&mut self.buf);

        self.cursor_visible = true;

        self.dirty |= DirtyImpact::Layout;

        self.buffer.write_all(self.buf.as_bytes())?;
        self.buffer.flush()?;
        Ok(())
    }

    fn disable(&mut self) -> std::io::Result<()> {
        match self.mode {
            Mode::Tui => self.disable_terminal(),
            #[cfg(feature = "gui")]
            Mode::Gui => {
                if let Some(old) = self.panic_hook.lock().unwrap().take() {
                    std::panic::set_hook(old);
                }
                let _ = GUI.try_with(|g| drop(g.borrow_mut().take()));
                Ok(())
            }
            #[cfg(not(feature = "gui"))]
            Mode::Gui => unreachable!(),
        }
    }

    fn disable_terminal(&mut self) -> std::io::Result<()> {
        log::debug!("tui: leaving terminal (restoring cursor / screen)");
        if let Some(old) = self.panic_hook.lock().unwrap().take() {
            std::panic::set_hook(old);
        }
        self.buf.clear();
        output::end_synchronized_update(&mut self.buf);
        #[cfg(feature = "harmonious")]
        crate::theme::harmonious::clear_palette();
        output::reset_cursor_style(&mut self.buf);
        output::show_cursor(&mut self.buf);
        output::set_mouse_pointer_shape(&mut self.buf, MousePointerShape::Default);
        self.cursor_visible = true;
        self.last_cursor_style = None;
        self.last_mouse_pointer_shape = None;
        output::leave_alternate_screen(&mut self.buf);
        output::pop_keyboard_enhancement_flags(&mut self.buf);
        let pixel_mouse = self
            .runtime_info
            .as_ref()
            .map(|i| i.subcell_events)
            .unwrap_or(false);
        if pixel_mouse {
            output::disable_mouse_pixel_capture(&mut self.buf);
        }
        set_mouse_pixel_dpr(None);
        output::disable_mouse_capture(&mut self.buf);
        output::disable_focus_change(&mut self.buf);
        output::disable_bracketed_paste(&mut self.buf);
        #[cfg(feature = "harmonious")]
        output::disable_color_scheme_detection(&mut self.buf);
        self.buffer.write_all(self.buf.as_bytes())?;
        self.buffer.flush()?;
        ansi::disable_raw_mode()?;
        Ok(())
    }

    fn suspend(&mut self) -> std::io::Result<()> {
        // Job-control suspend (Ctrl-Z / SIGTSTP) has no Windows equivalent; the
        // `#[cfg]` attribute (not the `cfg!` macro) keeps the libc call out of
        // the Windows build entirely.
        #[cfg(not(windows))]
        {
            self.disable()?;
            // SAFETY: sending SIGSTOP to our own process group has no preconditions.
            unsafe {
                libc::kill(0, libc::SIGSTOP);
            }
            self.enable()?;
        }
        Ok(())
    }
}

impl Runtime {
    fn focused_widget_id(&self) -> Option<WidgetId> {
        self.focus_chain.last().copied()
    }

    fn get_active_root<'a>(&'a self, root: &'a dyn Widget) -> &'a dyn Widget {
        match self.popups.last() {
            Some(p) => &*p.content,
            None => root,
        }
    }

    fn get_active_root_mut<'a>(&'a mut self, root: &'a mut dyn Widget) -> &'a mut dyn Widget {
        match self.popups.last_mut() {
            Some(p) => &mut *p.content,
            None => root,
        }
    }

    fn get_active_popup_index(&self) -> Option<usize> {
        let &root_id = self.focus_chain.first()?;
        self.popups.iter().position(|p| p.content.get_id() == root_id)
    }

    fn request_dismiss_popup(&mut self, _root: &mut dyn Widget, index: usize) {
        // Deliver the dismiss request to the popup's own content. `focus_chain`
        // here is the SAVED pre-popup app focus chain (used to restore focus on
        // close), rooted at the app root — emitting along it would notify the
        // app, not the popup. Address the content directly so the dialog that
        // owns the popup can react (e.g. cancel itself), mirroring `Esc`.
        let popup = &mut self.popups[index];
        let popup_id = popup.content.get_id();
        WidgetPath::from_ids(vec![popup_id]).emit_event(
            &mut *popup.content,
            &mut WidgetEvent::new(popup_id, Box::new(PopupDismissRequested)),
        );
    }

    fn close_popup_at(&mut self, root: &mut dyn Widget, index: usize) {
        log::debug!("popup closed: index={}", index);
        let removed = self.popups.remove(index);
        let popup_id = removed.content.get_id();
        WidgetPath::from_ids(removed.focus_chain.clone())
            .emit_event(root, &mut WidgetEvent::new(popup_id, Box::new(PopupClosed)));
        self.update_focus_chain(root, removed.focus_chain);
        dirty_paint();
    }

    fn set_widget_hover_by_id(&mut self, root: &mut dyn Widget, widget_id: WidgetId, hover: bool) {
        let selected = self.focus_chain.contains(&widget_id);
        let state = match (selected, hover) {
            (true, true) => WidgetState::FocusedHover,
            (true, false) => WidgetState::Focused,
            (false, true) => WidgetState::Hover,
            (false, false) => WidgetState::None,
        };

        if let Some(w) = root.find_mut(widget_id) {
            w.on_state_change(state);
            return;
        }
        for popup in &mut self.popups {
            if let Some(w) = popup.content.find_mut(widget_id) {
                w.on_state_change(state);
                return;
            }
        }
    }

    fn handle_hover(&mut self, root: &mut dyn Widget, pos: Vec2<f32>, release: bool) {
        let mut new_path: Vec<WidgetId> = vec![];
        let new_id = if pos.x < 0.0 {
            None
        } else {
            let cell = pos.map(|v| v.floor() as i32);
            match self.popup_hit_test(cell) {
                PopupHitResult::Hit(i) => {
                    let id = self.popups[i].content.descendant_at_pos(pos, Some(&mut new_path));
                    if id.is_some() {
                        new_path.push(self.popups[i].content.get_id());
                        new_path.reverse();
                    }
                    id
                }
                PopupHitResult::Blocked => None,
                PopupHitResult::Miss => {
                    let hit = hit_test_z(root, pos, &mut new_path, &[]);
                    if hit.is_some() {
                        new_path.push(root.get_id());
                        new_path.reverse();
                    }
                    hit.map(|(id, _z)| id)
                }
            }
        };

        let old_id = self.mouse_path.last().copied();
        if old_id == new_id {
            if release && let Some(wid) = old_id {
                self.set_widget_hover_by_id(root, wid, true);
            }
            return;
        }

        if let Some(wid) = old_id {
            self.set_widget_hover_by_id(root, wid, false);
        }
        if let Some(wid) = new_id {
            self.set_widget_hover_by_id(root, wid, true);
            self.mouse_path = new_path;
        } else {
            self.mouse_path.clear();
        }
    }

    fn get_valid_prefix_len(&self, root: &dyn Widget, path: &[WidgetId]) -> usize {
        if path.is_empty() {
            return 0;
        }
        let found_root: &dyn Widget = self.find_root_for_path(root, path);
        if found_root.get_id() != path[0] {
            return 0;
        }
        let mut len = 1;
        crate::widget::valid_prefix_len_recursive(found_root, &path[1..], &mut len);
        len
    }

    fn is_focus_chain_valid(&self, root: &dyn Widget) -> bool {
        let path = &self.focus_chain;
        let Some(&last) = path.last() else {
            return false;
        };
        self.get_valid_prefix_len(root, path) == path.len()
            && self.find_root_for_path(root, path)
                .find(last).is_some_and(|w| w.is_focusable())
    }

    #[cfg(debug_assertions)]
    fn assert_valid_path(&self, root: &dyn Widget, path: &[WidgetId]) {
        for i in 0..path.len() {
            for j in i + 1..path.len() {
                assert!(
                    path[i] != path[j],
                    "invalid ui path: {} and {} are duplicated",
                    i, j,
                );
            }
        }

        assert!(
            self.get_valid_prefix_len(root, path) == path.len(),
            "invalid ui path: chain broken",
        );
    }

    fn repair_focus_chain(&mut self, root: &mut dyn Widget) {
        let valid_len = self.get_valid_prefix_len(&*root, &self.focus_chain);
        if valid_len < self.focus_chain.len() {
            while self.focus_chain.len() > valid_len {
                let wid = self.focus_chain.pop().unwrap();
                if let Some(w) = root.find_mut(wid) {
                    w.on_state_change(WidgetState::None);
                }
            }
            set_focus_chain(&self.focus_chain);
        }
    }

    fn deselect_current(&mut self, root: &mut dyn Widget) -> Vec<WidgetId> {
        let old = std::mem::take(&mut self.focus_chain);
        set_focus_chain(&[]);
        let old_root = self.find_root_for_path_mut(root, &old);
        WidgetPath::from_ids(old.clone())
            .for_each_mut(old_root, .., |w| w.on_state_change(WidgetState::None));
        self.clear_key_queue();
        old
    }

    fn find_root_for_path<'a>(&'a self, root: &'a dyn Widget, path: &[WidgetId]) -> &'a dyn Widget {
        if let Some(&first) = path.first() {
            for popup in &self.popups {
                if popup.content.get_id() == first {
                    return &*popup.content;
                }
            }
        }
        root
    }

    fn find_root_for_path_mut<'a>(
        &'a mut self,
        root: &'a mut dyn Widget,
        path: &[WidgetId],
    ) -> &'a mut dyn Widget {
        if let Some(&first) = path.first()
            && let Some(popup) = self.popups.iter_mut().find(|p| p.content.get_id() == first)
        {
            return &mut *popup.content;
        }
        root
    }

    fn process_focus_request(&mut self, root: &mut dyn Widget, select_id: WidgetId, active: bool) {
        let active_root = self.get_active_root(&*root);
        let path = match active_root.find_path(select_id) {
            Some(p) => p,
            None => return,
        };
        let (can_sel, has_resolve) = match active_root.find(select_id) {
            Some(w) => (w.is_focusable(), w.get_focus_target()),
            None => return,
        };
        if can_sel {
            self.update_focus_chain(root, path);
            if active {
                let active_root = self.get_active_root_mut(root);
                if let Some(w) = active_root.find_mut(select_id) {
                    w.on_state_change(WidgetState::Active);
                }
            }
            dirty_paint();
        } else {
            if let Some(resolved_id) = has_resolve {
                self.process_focus_request(root, resolved_id, active);
                return;
            }
            for &wid in path.iter().rev().skip(1) {
                let active_root = self.get_active_root(&*root);
                if let Some(resolved_id) = active_root.find(wid).and_then(|w| w.get_focus_target()) {
                    self.process_focus_request(root, resolved_id, active);
                    return;
                }
            }
        }
    }

    fn clear_key_queue(&mut self) {
        self.key_queue.clear();
        self.key_queue_flushing = false;
        self.key_queue_deadline = None;
    }

    fn update_focus_chain(&mut self, root: &mut dyn Widget, new_path: Vec<WidgetId>) {
        if self.focus_chain == new_path {
            return;
        }
        self.clear_key_queue();
        #[cfg(debug_assertions)]
        self.assert_valid_path(&*root, &new_path);
        let old_path = std::mem::replace(&mut self.focus_chain, new_path.clone());
        set_focus_chain(&new_path);
        let old_root = self.find_root_for_path_mut(root, &old_path);
        WidgetPath::from_ids(old_path)
            .for_each_mut(old_root, .., |w| w.on_state_change(WidgetState::None));
        let new_root = self.find_root_for_path_mut(root, &new_path);
        WidgetPath::from_ids(new_path.clone())
            .for_each_mut(new_root, .., |w| w.on_state_change(WidgetState::Focused));
    }

    fn notify_before_focus_move(&mut self, root: &mut dyn Widget, path: &[WidgetId], axis: Option<Axis2D>, direction: Sign) {
        let active_root = self.get_active_root_mut(root);
        WidgetPath::from_ids(path.to_vec())
            .for_each_edge_mut(active_root, |w, child_id| {
                w.before_focus_move(child_id, axis, direction);
            });
    }

    fn focus_first_widget(&mut self, root: &mut dyn Widget) {
        let terminal_size = get_runtime_info().size;
        let clip = Axis2D::map(|a| (0, terminal_size[a] as i32));

        let mut id = {
            let active_root = self.get_active_root(&*root);
            find_visible_focusable(active_root, clip, Sign::Positive)
        };
        if id.is_none() {
            let active_root = self.get_active_root(&*root);
            id = find_visible_focusable(active_root, clip, Sign::Negative);
        }
        if let Some(id) = id {
            let active_root = self.get_active_root(&*root);
            let path = active_root.find_path(id);
            self.finish_focus_move(root, None, path);
        }
    }

    fn focus_next_1d(&mut self, root: &mut dyn Widget, direction: Sign) {
        self.repair_focus_chain(root);
        if self.focus_chain.is_empty() {
            self.focus_first_widget(root);
            return;
        }
        let path = self.focus_chain.clone();
        self.notify_before_focus_move(root, &path, None, direction);
        let root_id = self.get_active_root(&*root).get_id();
        let selected_chain = &path[1..];
        let mut out_path = vec![root_id];
        let mut found = {
            let active_root = self.get_active_root(&*root);
            find_focusable_1d(
                active_root,
                selected_chain,
                direction,
                &mut out_path,
            )
        };
        if !found {
            out_path.clear();
            out_path.push(root_id);
            let active_root = self.get_active_root(&*root);
            found =
                find_focusable_1d(active_root, &[], direction, &mut out_path);
        }
        self.finish_focus_move(root, None, found.then_some(out_path));
    }

    fn focus_next_2d(&mut self, root: &mut dyn Widget, direction: Direction2D) {
        self.repair_focus_chain(root);
        if self.focus_chain.is_empty() {
            self.focus_first_widget(root);
            return;
        }
        let selected_id = self.focused_widget_id().unwrap();
        let path = self.focus_chain.clone();
        self.notify_before_focus_move(root, &path, Some(direction.axis()), direction.screen_sign());
        let rect = widget_rect_or_zero(self.get_active_root(&*root), selected_id);
        let mut desired = rect_center(rect);
        desired[direction.axis().flip()] =
            self.curswant[direction.axis().flip()];

        let selected_chain = &path[1..];
        let result = {
            let active_root = self.get_active_root(&*root);
            find_focusable_2d(active_root, selected_chain, desired, direction)
        };
        self.finish_focus_move(root, Some(direction.axis()), result);
    }

    fn finish_focus_move(
        &mut self,
        root: &mut dyn Widget,
        axis: Option<Axis2D>,
        result: Option<Vec<WidgetId>>,
    ) {
        if let Some(path) = result {
            let Some(&widget_id) = path.last() else {
                return;
            };
            let active_root = self.get_active_root(&*root);
            let (rect, resolved) = active_root.find(widget_id)
                .map(|w| (w.get_rect(), w.get_focus_target()))
                .unwrap_or((Rect::new(Vec2::of(0i32), Vec2::of(0u16)), None));
            let center = rect_center(rect);
            if let Some(axis) = axis {
                self.curswant[axis] = center[axis];
            } else {
                self.curswant = center;
            }
            if let Some(resolved_id) = resolved
                && resolved_id != widget_id
            {
                self.process_focus_request(root, resolved_id, false);
                reveal(resolved_id, Vec2 { x: None, y: None });
                return;
            }
            self.update_focus_chain(root, path);
            dirty_paint();
            reveal(widget_id, Vec2 { x: None, y: None });
        } else if let Some(&id) = self.focus_chain.last() {
            reveal(id, Vec2 { x: None, y: None });
        }
    }

    fn handle_event_inner(
        &mut self,
        root: &mut dyn Widget,
        event: RuntimeEvent,
    ) -> std::io::Result<bool> {
        if !self.has_rendered {
            self.pending_events.push(event);
            return Ok(false);
        }
        match event {
            RuntimeEvent::Focus(focused) => {
                if !focused {
                    self.clear_key_queue();
                    self.handle_hover(root, Vec2::of(-1.0), false);
                }
            }
            RuntimeEvent::Input(mut event) => {
                self.mouse_pos = event.pos;
                if event.is_mouse_event() {
                    let mut early_exit = false;
                    match &event.chord.trigger {
                        Trigger::MouseDown(_) => {
                            if self.handle_popup_click(root, &mut event) {
                                early_exit = true;
                            } else {
                                self.dispatch_click(root, None, &mut event, true);
                            }
                        }
                        Trigger::MouseDrag(_) => {
                            if !self.mouse_path.is_empty() {
                                let path = self.mouse_path.clone();
                                self.dispatch_mouse(root, &path, &mut event);
                            }
                        }
                        Trigger::MouseUp(_) => {
                            let was_dragging = self.dragging;
                            self.dragging = false;

                            if was_dragging
                                && !self.mouse_path.is_empty() {
                                    let path = self.mouse_path.clone();
                                    self.dispatch_mouse(root, &path, &mut event);
                                }

                            if let Some(select_id) = take_focus_request() {
                                self.process_focus_request(root, select_id, false);
                            }

                            self.handle_hover(root, event.pos, true);
                        }
                        Trigger::MouseHover => {
                            self.handle_hover(root, event.pos, false);
                            if !self.mouse_path.is_empty() {
                                let path = self.mouse_path.clone();
                                self.dispatch_mouse(root, &path, &mut event);
                            }
                            // Repaint so the resize affordance (OSC 22 cursor +
                            // divider highlight) tracks the pointer even when no
                            // widget dirtied. A non-default shape repaints on
                            // every motion so the cursor is re-asserted against
                            // terminals that reset the pointer on movement;
                            // default only repaints on the transition.
                            let shape = self
                                .compute_resize_hover(root)
                                .map(|(_, _, s)| s)
                                .unwrap_or(MousePointerShape::Default);
                            if shape != MousePointerShape::Default
                                || with_ctx(|ctx| ctx.last_mouse_pointer_shape) != Some(shape)
                            {
                                dirty_paint();
                            }
                        }
                        Trigger::MouseScroll(direction) => {
                            let direction = *direction;
                            if self.handle_popup_scroll(root, &mut event, direction).is_none() {
                                self.handle_scroll(root, &mut event, direction);
                            }
                        }
                        Trigger::MouseSmoothScroll(direction, delta) => {
                            let direction = *direction;
                            let delta = *delta;
                            let axis = direction.axis();
                            let result = match self.handle_popup_scroll(root, &mut event, direction) {
                                Some(r) => r,
                                None => self.handle_scroll(root, &mut event, direction),
                            };
                            let consumed = matches!(result, InputResult::Handled | InputResult::Pending);
                            if consumed {
                                self.scroll_accum[axis] = 0.0;
                            } else {
                                let signed = if direction.screen_sign() == Sign::Positive { delta } else { -delta };
                                self.scroll_accum[axis] += signed;
                                while self.scroll_accum[axis] >= 1.0 || self.scroll_accum[axis] <= -1.0 {
                                    let positive = self.scroll_accum[axis] >= 1.0;
                                    let sign = if positive { 1.0_f32 } else { -1.0_f32 };
                                    let synth_dir = Direction2D::from_screen_parts(
                                        axis,
                                        if positive { Sign::Positive } else { Sign::Negative },
                                    );
                                    let mut synth = InputEvent {
                                        chord: Chord::new(Trigger::MouseScroll(synth_dir), event.chord.modifiers),
                                        pos: event.pos,
                                        count: 1,
                                    };
                                    if self.handle_popup_scroll(root, &mut synth, synth_dir).is_none() {
                                        self.handle_scroll(root, &mut synth, synth_dir);
                                    }
                                    self.scroll_accum[axis] -= sign;
                                }
                            }
                        }
                        _ => {},
                    }
                    if early_exit {
                        return Ok(false);
                    }
                } else {
                    self.enqueue_keyboard(event);
                    return Ok(true);
                }
            }
            RuntimeEvent::ColorSchemeChange(scheme) => {
                update_runtime_info(|info| info.color_scheme = Some(scheme));
                #[cfg(feature = "harmonious")]
                {
                    if let Ok(p) = crate::theme::harmonious::query_palette() { crate::theme::harmonious::apply_palette(p) }
                }
                dirty_paint();
            }
            RuntimeEvent::Paste(s) => {
                let event = InputEvent {
                    chord: Chord::new(Trigger::Paste(s), Modifiers::new()),
                    pos: self.mouse_pos,
                    count: 1,
                };
                self.enqueue_keyboard(event);
                return Ok(true);
            }
            RuntimeEvent::DragHold(held) => {
                let elapsed = self.last_scroll.elapsed();
                if held {
                    if elapsed < std::time::Duration::from_millis(250) {
                        self.scroll_held = true;
                    }
                } else {
                    self.scroll_held = false;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn click_inside_widget_border(click_root: &dyn Widget, widget_id: WidgetId, window_pos: Vec2<i32>) -> bool {
        click_root.find(widget_id)
            .map(|w| {
                let border_rect = w.get_rect();
                Axis2D::all(|a| {
                    window_pos[a] >= border_rect.pos[a]
                        && window_pos[a] < border_rect.pos[a] + border_rect.size[a] as i32
                })
            })
            .unwrap_or(true)
    }


    fn dispatch_click(&mut self, root: &mut dyn Widget, popup_index: Option<usize>, event: &mut InputEvent, selectable: bool) {
        self.dragging = true;
        self.scroll_path.clear();
        let window_pos = event.pos;
        let mut click_path: Vec<WidgetId> = vec![];
        let mut consumed_idx: Option<usize> = None;
        let mut excluded: Vec<WidgetId> = vec![];

        loop {
            let mut found_hit = false;
            let mut hit_z: Option<Layer> = None;
            click_path.clear();
            {
                let click_root: &dyn Widget = match popup_index {
                    Some(i) => &*self.popups[i].content,
                    None => root as &dyn Widget,
                };
                match popup_index {
                    Some(_) => {
                        if click_root.descendant_at_pos(
                            window_pos,
                            Some(&mut click_path),
                        ).is_some() {
                            found_hit = true;
                        }
                    }
                    None => {
                        if let Some((_, z)) = hit_test_z(click_root, window_pos, &mut click_path, &excluded) {
                            hit_z = Some(z);
                            found_hit = true;
                        }
                    }
                }
                click_path.push(click_root.get_id());
            }
            click_path.reverse();

            for i in (0..click_path.len()).rev() {
                let click_root: &mut dyn Widget = match popup_index {
                    Some(pi) => &mut *self.popups[pi].content,
                    None => root,
                };
                let leaf_pos = crate::runtime::tree::window_to_leaf(
                    &*click_root, &click_path[..=i], window_pos,
                );
                let result = if let Some(target) = walk_path_mut(click_root, &click_path[..=i]) {
                    event.pos = leaf_pos - target.get_pos().map(|v| v as f32);
                    let mut queue = InputQueue::new(std::slice::from_ref(&*event), false);
                    target.on_input(&mut queue)
                } else {
                    InputResult::Rejected
                };
                self.flush_events(root);
                if result == InputResult::Handled {
                    consumed_idx = Some(i);
                    break;
                }
            }
            event.pos = window_pos;

            if consumed_idx.is_some() {
                break;
            }
            if popup_index.is_some() {
                break;
            }
            if !found_hit {
                break;
            }
            if hit_z.is_some_and(|z| z > Layer::Bottom) {
                break;
            }

            for &wid in click_path.iter().skip(1) {
                excluded.push(wid);
            }
        }

        if let Some(idx) = consumed_idx {
            self.mouse_path = click_path[..=idx].to_vec();
        } else {
            self.mouse_path.clear();
        }

        if !selectable {
            take_focus_request();
            return;
        }

        if let Some(select_id) = take_focus_request() {
            let active = {
                let click_root: &dyn Widget = match popup_index {
                    Some(i) => &*self.popups[i].content,
                    None => root as &dyn Widget,
                };
                Self::click_inside_widget_border(click_root, select_id, window_pos.map(|v| v.floor() as i32))
            };
            self.process_focus_request(root, select_id, active);
        } else {
            let click_root: &dyn Widget = match popup_index {
                Some(i) => &*self.popups[i].content,
                None => root as &dyn Widget,
            };
            let mut found = false;
            for &wid in click_path.iter().rev() {
                if let Some(resolved_id) = click_root.find(wid).and_then(|w| w.get_focus_target()) {
                    let active = Self::click_inside_widget_border(click_root, resolved_id, window_pos.map(|v| v.floor() as i32));
                    self.process_focus_request(root, resolved_id, active);
                    found = true;
                    break;
                }
            }
            if !found {
                self.deselect_current(root);
                dirty_paint();
            }
        }
    }

    fn handle_scroll(
        &mut self,
        root: &mut dyn Widget,
        event: &mut InputEvent,
        direction: Direction2D,
    ) -> InputResult {
        let (edge_window, alive_window) = match with_ctx(|c| c.mode) {
            Mode::Gui => (
                std::time::Duration::from_millis(100),
                std::time::Duration::from_millis(250),
            ),
            Mode::Tui => (
                std::time::Duration::from_millis(200),
                std::time::Duration::from_millis(1000),
            ),
        };

        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last_scroll);
        let near_prev = event
            .pos
            .diff(self.scroll_pos)
            .all_le(Vec2::new(2.0_f32, 1.0_f32));

        if near_prev && let Some(&scroll_wid) = self.scroll_path.last() {
            let can_scroll = root
                .find(scroll_wid)
                .is_some_and(|w| w.can_scroll(direction));
            let alive =
                self.scroll_held || elapsed < edge_window || (can_scroll && elapsed < alive_window);
                if alive {
                    let result = if can_scroll {
                        let path = self.scroll_path.clone();
                        self.dispatch_mouse(root, &path, event)
                    } else {
                        InputResult::Rejected
                    };
                    self.last_scroll = now;
                    return result;
                }
            }

        self.scroll_pos = event.pos;
        self.scroll_path = build_scroll_path(root, event.pos, direction);
        if !self.scroll_path.is_empty() {
            let path = self.scroll_path.clone();
            let result = self.dispatch_mouse(root, &path, event);
            self.last_scroll = now;
            result
        } else {
            InputResult::Rejected
        }
    }

    fn dispatch_mouse(&mut self, root: &mut dyn Widget, path: &[WidgetId], event: &mut InputEvent) -> InputResult {
        if path.is_empty() {
            return InputResult::Rejected;
        }
        let window_pos = event.pos;
        let dispatch_root = self.find_root_for_path_mut(root, path);
        let leaf_pos = crate::runtime::tree::window_to_leaf(&*dispatch_root, path, window_pos);
        let result = match walk_path_mut(dispatch_root, path) {
            Some(target) => {
                event.pos = leaf_pos - target.get_pos().map(|v| v as f32);
                log::trace!(
                    target: "tuie::input",
                    "dispatch_mouse: chord={} path_len={} leaf_pos={:?}",
                    event.chord,
                    path.len(),
                    (leaf_pos.x, leaf_pos.y),
                );
                let mut queue = InputQueue::new(std::slice::from_ref(&*event), false);
                target.on_input(&mut queue)
            }
            None => InputResult::Rejected,
        };
        event.pos = window_pos;
        self.flush_events(root);
        result
    }

    fn flush_events(&mut self, root: &mut dyn Widget) {
        loop {
            if let Some(op) = with_ctx_mut(|ctx| ctx.navigate_request.take()) {
                match op {
                    NavigateOp::TabOrder(d) => self.focus_next_1d(root, d),
                    NavigateOp::Directional(d) => self.focus_next_2d(root, d),
                }
            }
            let events: Vec<EmittedEvent> = with_ctx_mut(|ctx| std::mem::take(&mut ctx.emitted));
            if events.is_empty() {
                return;
            }
            for emitted in events {
                let source_id = emitted.source_id;

                let ancestors_ids = [&self.focus_chain, &self.mouse_path, &self.scroll_path]
                    .iter()
                    .find_map(|p| p.iter().position(|&wid| wid == source_id).map(|i| p[..i].to_vec()))
                    .or_else(|| root.find_path(source_id).map(|f| f[..f.len() - 1].to_vec()))
                    .or_else(|| {
                        self.popups.iter()
                            .find_map(|p| p.content.find_path(source_id))
                            .map(|f| f[..f.len() - 1].to_vec())
                    })
                    .unwrap_or_default();
                let widget_path = WidgetPath::from_ids(ancestors_ids);
                let mut event = WidgetEvent::new(source_id, emitted.payload);

                if let Some(&first) = widget_path.as_slice().first()
                    && let Some(popup) = self.popups.iter_mut().find(|p| p.content.get_id() == first)
                {
                    widget_path.emit_event(&mut *popup.content, &mut event);
                    continue;
                }
                widget_path.emit_event(root, &mut event);
            }
        }
    }

    fn enqueue_keyboard(&mut self, event: InputEvent) {
        self.key_queue_deadline = None;
        self.key_queue_flushing = false;
        self.key_queue.push(event);
    }

    fn handle_pending_queue(&mut self) {
        if !self.key_queue_flushing {
            self.key_queue_deadline = Some(
                std::time::Instant::now() + std::time::Duration::from_millis(1000),
            );
        } else if !self.key_queue.is_empty() {
            self.key_queue.drain(..1);
        }
    }

    fn drain_task_queue(&mut self, root: &mut dyn Widget) -> Option<std::time::Duration> {
        let mut ready: Vec<Box<dyn FnOnce(&mut dyn Widget)>> = Vec::new();
        let next = loop {
            let now = std::time::Instant::now();
            let mut next: Option<std::time::Instant> = None;
            with_ctx_mut(|ctx| {
                let items = &mut ctx.tasks.items;
                let mut i = items.len();
                while i > 0 {
                    i -= 1;
                    let TaskKind::Timer(schedule, _) = &items[i].1 else {
                        continue;
                    };
                    if *schedule <= now {
                        let (_, kind) = items.swap_remove(i);
                        if let TaskKind::Timer(_, callback) = kind {
                            ready.push(callback);
                        }
                    } else {
                        let schedule = *schedule;
                        if next.map(|earliest| schedule < earliest).unwrap_or(true) {
                            next = Some(schedule);
                        }
                    }
                }
            });
            if ready.is_empty() {
                break next;
            }
            for cb in ready.drain(..) {
                cb(root);
            }
        };
        next.map(|instant| {
            instant.saturating_duration_since(std::time::Instant::now())
        })
    }

    fn handle_events_finish(
        &mut self,
        root: &mut dyn Widget,
    ) -> Option<std::time::Duration> {
        self.flush_events(root);
        self.drain_popup_queues(root);
        let task_timeout = self.drain_task_queue(root);
        let deadline_timeout = self.key_queue_deadline
            .map(|d| d.saturating_duration_since(std::time::Instant::now()));
        match (task_timeout, deadline_timeout) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn repair_selection(&mut self, root: &mut dyn Widget) {
        if let Some(select_id) = take_focus_request() {
            self.process_focus_request(root, select_id, false);
        }

        if let Some(op) = with_ctx_mut(|ctx| ctx.navigate_request.take()) {
            match op {
                NavigateOp::TabOrder(d) => self.focus_next_1d(root, d),
                NavigateOp::Directional(d) => self.focus_next_2d(root, d),
            }
        }

        if self.focus_chain.is_empty() {
            return;
        }

        if !self.is_focus_chain_valid(&*root) {
            let target = if self.mouse_pos.x >= 0.0 {
                self.mouse_pos.map(|v| v.floor() as i32)
            } else if let Some(selected_id) = self.focused_widget_id() {
                rect_center(widget_rect_or_zero(self.get_active_root(&*root), selected_id))
            } else {
                Vec2::of(0i32)
            };
            let mut best = (u8::MAX, f64::MAX, None);
            let search_root: &dyn Widget = match self.popups.last() {
                Some(p) => &*p.content,
                None => self.get_active_root(&*root),
            };
            find_nearest_focusable(search_root, target, &mut best);
            if let Some(winner_id) = best.2 {
                if let Some(path) = search_root.find_path(winner_id) {
                    self.update_focus_chain(root, path);
                }
                dirty_paint();
                reveal(winner_id, Vec2 { x: None, y: None });
            } else {
                self.focus_next_1d(root, Sign::Positive);
            }
        }
    }

    fn handle_reveal_request(&mut self, root: &mut dyn Widget) {
        if let Some(focus_id) = with_ctx_mut(|ctx| ctx.reveal_request.take()) {
            let align = with_ctx_mut(|ctx| {
                std::mem::replace(&mut ctx.reveal_align, Vec2 { x: None, y: None })
            });
            let in_selected = self.focus_chain.contains(&focus_id);
            let path = if in_selected {
                self.focus_chain.clone()
            } else {
                root.find_path(focus_id).unwrap_or_else(|| self.focus_chain.clone())
            };
            let focus_root = self.find_root_for_path_mut(root, &path);
            focus_along_path(focus_root, &path, align);
        }
    }

    fn take_pending_events(&mut self) -> Vec<RuntimeEvent> {
        if !self.has_rendered {
            self.has_rendered = true;
            std::mem::take(&mut self.pending_events)
        } else {
            Vec::new()
        }
    }

    fn layout_and_render(&mut self, root: &mut dyn Widget) -> std::io::Result<FrameRender> {
        let terminal_size = get_runtime_info().size;
        if terminal_size != root.get_rect_size() {
            dirty_layout();
        }
        let global_dirty = get_dirty();
        let mut widget_dirty = check_dirty(root);
        for popup in &mut self.popups {
            widget_dirty |= check_dirty(&mut *popup.content);
        }
        if global_dirty == DirtyImpact::None && widget_dirty == DirtyImpact::None {
            return Ok(FrameRender::Skipped);
        }

        if global_dirty == DirtyImpact::Layout || widget_dirty == DirtyImpact::Layout {
            clear_path_cache();
        }

        with_ctx_mut(|ctx| ctx.renderer.resize(terminal_size));

        self.flush_events(root);

        self.repair_selection(root);

        compute_overflows(root);
        for popup in &mut self.popups {
            compute_overflows(&mut *popup.content);
        }

        self.layout(root, terminal_size);

        let old_center = if let Some(selected_id) = self.focused_widget_id() {
            rect_center(widget_rect_or_zero(self.get_active_root(&*root), selected_id))
        } else {
            Vec2::of(0i32)
        };

        self.handle_reveal_request(root);
        root.layout_position();
        for i in 0..self.popups.len() {
            position_popup(&mut self.popups[i], terminal_size);
        }

        if config::get().always_selected && self.focus_chain.is_empty() {
            self.focus_first_widget(root);
        }

        {
            let path = &self.focus_chain;
            let focus_root = self.find_root_for_path(&*root, path);
            let measure = compute_focused_measure(focus_root, path);
            with_ctx_mut(|ctx| ctx.focused_measure = measure);
        }

        if let Some(selected_id) = self.focused_widget_id() {
            let new_center = rect_center(widget_rect_or_zero(self.get_active_root(&*root), selected_id));
            self.curswant = self.curswant + new_center - old_center;
        }

        set_dirty(DirtyImpact::None);
        clear_dirty(root);
        for popup in &mut self.popups {
            clear_dirty(&mut *popup.content);
        }

        if !self.dragging {
            self.handle_hover(root, self.mouse_pos, false);
        }

        let cursor = self.compute_cursor(root, terminal_size);
        self.paint(root, cursor)?;
        Ok(FrameRender::Painted(cursor))
    }

    #[cfg(feature = "gui")]
    fn present_gui(&mut self, root: &dyn Widget, cursor: Option<(CursorShape, Vec2<i32>)>) {
        let cursor_subpixel = match cursor {
            Some(_) => path_subcell_offset(root, &self.focus_chain),
            None => Vec2::of(0i32),
        };
        let focus_chain = &self.focus_chain;
        with_render_io(|renderer, buffer, _buf| {
            try_with_gui_state(|s| s.present(renderer, cursor, cursor_subpixel, focus_chain, buffer));
        });
    }

    fn layout(&mut self, root: &mut dyn Widget, viewport_size: Vec2<u16>) {
        for _ in 0..3 {
            if !needs_layout(root) {
                break;
            }
            for _ in 0..2 {
                set_dirty(DirtyImpact::None);
                perform_layout(root, viewport_size, false);

                root.set_pos(root.get_layout().get_margin_before().map(|v| v as i32));
                root.layout_position();
                if !needs_layout(root) {
                    break;
                }
            }
            for i in 0..self.popups.len() {
                for _ in 0..2 {
                    set_dirty(DirtyImpact::None);
                    perform_layout(&mut *self.popups[i].content, viewport_size, true);

                    if !needs_layout(&mut *self.popups[i].content) {
                        break;
                    }
                    position_popup(&mut self.popups[i], viewport_size);
                }
            }
            self.flush_events(root);
        }
    }

    fn compute_cursor(
        &self,
        root: &dyn Widget,
        terminal_size: Vec2<u16>,
    ) -> Option<(CursorShape, Vec2<i32>)> {
        let active_root = self.get_active_root(root);
        let selected = self
            .focused_widget_id()
            .filter(|&id| id != active_root.get_id());
        let (shape, cursor_pos) = active_root.get_cursor(selected)?;
        let pos = cursor_pos + active_root.get_pos();
        let in_bounds = Axis2D::all(|a| {
            pos[a] >= -1 && pos[a] <= terminal_size[a] as i32
        });
        if in_bounds {
            Some((shape, pos))
        } else {
            None
        }
    }

    /// Walks the hovered path (deepest widget first) and returns the first
    /// widget that wants a non-default mouse pointer, with the pointer position
    /// in that widget's local frame and the requested shape.
    ///
    /// Walking the whole path — not just the leaf — lets an ancestor like
    /// `Split` claim the resize cursor for a divider cell that hit-testing
    /// attributes to a child pane (a merged pane border).
    fn compute_resize_hover(
        &self,
        root: &dyn Widget,
    ) -> Option<(WidgetId, Vec2<i32>, MousePointerShape)> {
        if self.mouse_path.is_empty() {
            return None;
        }
        let active_root = self.get_active_root(root);
        let abs = self.mouse_pos.map(|v| v.floor() as i32);
        for &id in self.mouse_path.iter().rev() {
            if let Some(widget) = active_root.find(id) {
                let local = abs - widget.get_pos();
                if let Some(shape) = widget.get_mouse_pointer_shape(local) {
                    return Some((id, local, shape));
                }
            }
        }
        None
    }

    fn paint(
        &mut self,
        root: &dyn Widget,
        cursor: Option<(CursorShape, Vec2<i32>)>,
    ) -> std::io::Result<()> {
        match with_ctx(|c| c.mode) {
            Mode::Tui => self.paint_terminal(root, cursor),
            #[cfg(feature = "gui")]
            Mode::Gui => self.paint_gui(root, cursor),
            #[cfg(not(feature = "gui"))]
            Mode::Gui => unreachable!(),
        }
    }

    #[cfg(feature = "gui")]
    fn paint_gui(
        &mut self,
        root: &dyn Widget,
        _cursor: Option<(CursorShape, Vec2<i32>)>,
    ) -> std::io::Result<()> {
        let (grid_origin_px, window_px) = try_with_gui_state(|s| {
            let o = s.grid_origin();
            (Vec2::new(o.x as i32, o.y as i32), s.pixel_size)
        })
        .unwrap_or((Vec2::of(0i32), Vec2::of(0u32)));
        let cell_size = get_runtime_info().cell_size;
        let popups = &self.popups;
        with_render_io(|renderer, buffer, _buf| {
            let _ = renderer.take_full_dirty();
            renderer.set_root_screen_pos_px(grid_origin_px);
            if let Some(cell_px) = cell_size {
                let size = renderer.gui_size();
                let grid_size_px = Vec2::new(
                    size.x as u32 * cell_px.x as u32,
                    size.y as u32 * cell_px.y as u32,
                );
                let root_clip_size = Vec2::new(
                    grid_size_px
                        .x
                        .max(window_px.x.saturating_sub(grid_origin_px.x.max(0) as u32)),
                    grid_size_px
                        .y
                        .max(window_px.y.saturating_sub(grid_origin_px.y.max(0) as u32)),
                );
                renderer.set_root_clip_screen_px((grid_origin_px, root_clip_size));
            }
            renderer.clear_defer_queue();
            renderer.render_to_queue(root, Vec2::of(0i32), buffer);
            seed_popup_queue(renderer, buffer, popups);
        });
        Ok(())
    }

    fn paint_terminal(
        &mut self,
        root: &dyn Widget,
        cursor: Option<(CursorShape, Vec2<i32>)>,
    ) -> std::io::Result<()> {
        let term_size = get_runtime_info().size;
        let resize_hover = self.compute_resize_hover(root);
        // Publish the resize-hover so widgets (e.g. Split) can highlight the
        // hovered divider during render; derive the OSC 22 cursor from it.
        set_resize_hover(resize_hover.map(|(id, local, _)| (id, local)));
        let mouse_pointer = resize_hover
            .map(|(_, _, shape)| shape)
            .unwrap_or(MousePointerShape::Default);
        let popups = &self.popups;
        with_render_io(|renderer, buffer, buf| {
            buf.clear();
            output::begin_synchronized_update(buf);
            if renderer.take_full_dirty() {
                output::clear_screen(buf);
            }
            buffer.write_all(buf.as_bytes())?;
            renderer.clear_defer_queue();
            renderer.render_to_queue(root, Vec2::of(0i32), buffer);
            seed_popup_queue(renderer, buffer, popups);
            renderer.drain_queue(buffer);
            let wrote = renderer.flush(buffer)?;

            buf.clear();
            output::end_synchronized_update(buf);

            let in_grid = |pos: Vec2<i32>| {
                pos.x >= 0
                    && pos.y >= 0
                    && pos.x < term_size.x as i32
                    && pos.y < term_size.y as i32
            };
            let visible_cursor = cursor.filter(|(_, p)| in_grid(*p));
            let prev_cursor = with_ctx(|ctx| ctx.emulator_cursor);

            // Only touch the terminal cursor when the grid actually changed
            // (which moved the hardware cursor as a side effect of writing
            // cells) or when the cursor itself moved / appeared / vanished. On a
            // no-op repaint — e.g. the app's periodic tick re-rendering identical
            // content — we leave the cursor exactly where it is so the terminal's
            // hardware blink keeps its cadence instead of restarting every frame.
            let cursor_touched = wrote || visible_cursor != prev_cursor;
            log::trace!(
                target: "tuie::render",
                "paint: wrote={wrote} cursor_touched={cursor_touched} cursor={:?}",
                cursor.map(|(_, p)| (p.x, p.y)),
            );
            if cursor_touched {
                let (mut was_visible, last_style) =
                    with_ctx(|ctx| (ctx.cursor_visible, ctx.last_cursor_style));
                let mut cursor_visible = false;
                let mut new_style = last_style;
                if let Some((shape, pos)) = visible_cursor {
                    output::move_to(buf, pos.x as u16, pos.y as u16);
                    cursor_visible = true;
                    if !was_visible {
                        was_visible = true;
                        output::show_cursor(buf);
                    }
                    // Only (re-)emit DECSCUSR when shape/blink actually changes;
                    // many terminals restart the blink phase on every set.
                    let style = (shape, config::get().cursor_blink);
                    if last_style != Some(style) {
                        output::set_cursor_style(buf, style.0, style.1);
                        new_style = Some(style);
                    }
                }

                if !cursor_visible {
                    output::move_to(buf, 0, 0);
                    if was_visible {
                        was_visible = false;
                        output::hide_cursor(buf);
                    }
                }
                with_ctx_mut(|ctx| {
                    ctx.cursor_visible = was_visible;
                    ctx.emulator_cursor = visible_cursor;
                    ctx.last_cursor_style = new_style;
                });
            }

            // Mouse pointer shape (OSC 22), emitted independently of the text
            // cursor. Re-assert any non-default shape on every paint: some
            // terminals (e.g. Ghostty) reset the pointer to default on mouse
            // motion, so a set-once-on-change emit would be reverted and never
            // restored. The idle default is still emitted only on change.
            let prev_pointer = with_ctx(|ctx| ctx.last_mouse_pointer_shape);
            if prev_pointer != Some(mouse_pointer) || mouse_pointer != MousePointerShape::Default {
                output::set_mouse_pointer_shape(buf, mouse_pointer);
                with_ctx_mut(|ctx| ctx.last_mouse_pointer_shape = Some(mouse_pointer));
            }

            buffer.write_all(buf.as_bytes())?;
            buffer.flush()?;
            Ok(())
        })
    }

    fn drain_popup_queues(&mut self, root: &mut dyn Widget) {
        for popup in crate::runtime::popup::drain_open_requests() {
            let saved = self.deselect_current(root);
            let content_id = popup.content.get_id();
            log::debug!("popup opened: id={:?}", content_id);
            let active = crate::runtime::popup::ActivePopup::from_popup(popup, saved);
            let mut path = vec![content_id];
            find_focusable_1d(&*active.content, &[], Sign::Positive, &mut path);
            self.popups.push(active);
            self.finish_focus_move(root, None, Some(path));
            dirty_layout();
        }
        for id in crate::runtime::popup::drain_dismiss_requests() {
            if let Some(pos) = self.popups.iter().position(|p| p.content.get_id() == id) {
                self.request_dismiss_popup(root, pos);
            }
        }
        for id in crate::runtime::popup::drain_close_requests() {
            if let Some(pos) = self.popups.iter().position(|p| p.content.get_id() == id) {
                self.close_popup_at(root, pos);
            }
        }
    }

    fn handle_popup_scroll(&mut self, root: &mut dyn Widget, event: &mut InputEvent, direction: Direction2D) -> Option<InputResult> {
        if self.popups.is_empty() {
            return None;
        }
        match self.popup_hit_test(event.cell()) {
            PopupHitResult::Hit(i) => {
                let path = build_scroll_path(&*self.popups[i].content, event.pos, direction);
                log::trace!(
                    target: "tuie::scroll",
                    "popup scroll: hit=i{i} can_scroll(dir)={} path_len={}",
                    self.popups[i].content.can_scroll(direction),
                    path.len(),
                );
                let result = if !path.is_empty() {
                    self.dispatch_mouse(root, &path, event)
                } else {
                    InputResult::Rejected
                };
                Some(result)
            }
            PopupHitResult::Blocked => {
                log::trace!(target: "tuie::scroll", "popup scroll: blocked cell={:?}", (event.cell().x, event.cell().y));
                Some(InputResult::Handled)
            }
            PopupHitResult::Miss => {
                log::trace!(target: "tuie::scroll", "popup scroll: miss cell={:?}", (event.cell().x, event.cell().y));
                None
            }
        }
    }

    fn handle_popup_click(&mut self, root: &mut dyn Widget, event: &mut InputEvent) -> bool {
        if self.popups.is_empty() {
            return false;
        }

        if let PopupHitResult::Hit(i) = self.popup_hit_test(event.cell()) {
            self.dispatch_click(root, Some(i), event, true);
            self.drain_popup_queues(root);
            return true;
        }

        let topmost = self.popups.len() - 1;
        if self.popups[topmost].dismissible {
            self.close_popup_at(root, topmost);
            self.flush_events(root);
            self.drain_popup_queues(root);
            return true;
        }
        self.request_dismiss_popup(root, topmost);
        true
    }
}

fn seed_popup_queue(
    renderer: &mut crate::render::GridRenderer,
    buffer: &mut dyn Write,
    popups: &[crate::runtime::popup::ActivePopup],
) {
    let mut ctx = renderer.context(buffer);
    for popup in popups {
        let pos = popup.content.get_pos();
        let size = popup.content.get_rect_size();
        if size.x == 0 || size.y == 0 {
            continue;
        }
        ctx.queue_popup(&*popup.content, pos);
    }
}
