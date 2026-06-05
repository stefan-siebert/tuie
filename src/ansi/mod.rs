//! Native ANSI/VT terminal backend.

pub mod input;
pub mod output;
pub mod query;

pub use output::*;

use crate::prelude::*;

/// Terminal light/dark color-scheme preference.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorScheme {
    /// Dark background.
    Dark,
    /// Light background.
    Light,
}

impl std::fmt::Display for ColorScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dark => write!(f, "Dark"),
            Self::Light => write!(f, "Light"),
        }
    }
}

/// A queryable terminal color slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ColorType {
    /// 256-color palette entry `n` (OSC 4).
    Palette(u8),
    /// Default foreground (OSC 10).
    Foreground,
    /// Default background (OSC 11).
    Background,
    /// Text cursor color (OSC 12).
    Cursor,
    /// Mouse pointer foreground (OSC 13).
    PointerForeground,
    /// Mouse pointer background (OSC 14).
    PointerBackground,
    /// Tektronix foreground (OSC 15).
    TektronixForeground,
    /// Tektronix background (OSC 16).
    TektronixBackground,
    /// Highlight background (OSC 17).
    HighlightBackground,
    /// Tektronix cursor (OSC 18).
    TektronixCursor,
    /// Highlight foreground (OSC 19).
    HighlightForeground,
}

impl ColorType {
    /// Maps an OSC number (10..=19) to its `ColorType`.
    pub fn from_osc_number(n: u8) -> Option<Self> {
        Some(match n {
            10 => Self::Foreground,
            11 => Self::Background,
            12 => Self::Cursor,
            13 => Self::PointerForeground,
            14 => Self::PointerBackground,
            15 => Self::TektronixForeground,
            16 => Self::TektronixBackground,
            17 => Self::HighlightBackground,
            18 => Self::TektronixCursor,
            19 => Self::HighlightForeground,
            _ => return None,
        })
    }

    /// Returns the OSC number for this color slot.
    pub fn get_osc_number(&self) -> u8 {
        match self {
            Self::Palette(_) => 4,
            Self::Foreground => 10,
            Self::Background => 11,
            Self::Cursor => 12,
            Self::PointerForeground => 13,
            Self::PointerBackground => 14,
            Self::TektronixForeground => 15,
            Self::TektronixBackground => 16,
            Self::HighlightBackground => 17,
            Self::TektronixCursor => 18,
            Self::HighlightForeground => 19,
        }
    }
}

/// A parsed color reported by the terminal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ColorEntry {
    /// The color slot.
    pub color_type: ColorType,
    /// The red channel.
    pub r: u8,
    /// The green channel.
    pub g: u8,
    /// The blue channel.
    pub b: u8,
}

/// A decoded mouse event.
#[derive(Clone, PartialEq, Debug)]
pub struct MouseInput {
    /// The mouse action, as a native [`Trigger`].
    pub trigger: Trigger,
    /// The column position, 0-indexed.
    pub column: u16,
    /// The row position, 0-indexed.
    pub row: u16,
    /// The modifier keys held during the event.
    pub modifiers: Modifiers,
}

/// A single decoded terminal event.
#[derive(Clone, PartialEq, Debug)]
pub enum ParsedEvent {
    /// A decoded keypress as a [`Chord`].
    Key(Chord),
    /// A decoded mouse event.
    Mouse(MouseInput),
    /// Terminal resized to `(columns, rows)`.
    Resize(u16, u16),
    /// Focus gained (`true`) or lost (`false`).
    Focus(bool),
    /// A bracketed-paste payload.
    Paste(String),
    /// A color-scheme change report.
    ColorScheme(ColorScheme),
    /// A primary device attributes reply.
    PrimaryDeviceAttributes(Vec<u16>),
    /// A terminal name/version reply.
    XtVersion(String),
    /// A Kitty graphics protocol reply.
    KittyGraphicsReply { id: u32, ok: bool },
    /// The cell size in pixels.
    CellPixelSize { width: u16, height: u16 },
    /// The window size in pixels.
    WindowPixelSize { width: u16, height: u16 },
    /// A terminal color query reply.
    Color(ColorEntry),
    /// A DEC mode state report.
    DecModeReport { mode: u16, status: u8 },
}

/// Platform handle used to wake a blocked [`Reader`] from another thread.
///
/// On Unix this is the read end of a self-pipe (`RawFd`); on Windows it is an
/// auto-reset event object (`RawHandle`). The runtime's waker (`init_waker` in
/// `runtime/mod.rs`) produces one of these and hands it to [`Reader::set_wake_fd`].
#[cfg(unix)]
pub type WakeHandle = std::os::unix::io::RawFd;
/// See [`WakeHandle`] (Unix variant).
#[cfg(windows)]
pub type WakeHandle = std::os::windows::io::RawHandle;

#[cfg(unix)]
pub use unix::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled, size, write_query, Reader};

#[cfg(windows)]
pub use windows::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled, size, write_query, Reader};

#[cfg(unix)]
mod unix {
    use super::input::Parser;
    use super::ParsedEvent;
    use std::io::{self, Read, Write};
    use std::os::unix::io::{AsRawFd, RawFd};
    use std::os::unix::net::UnixStream;
    use std::sync::Mutex;
    use std::time::Duration;

    const BUFFER_SIZE: usize = 1024;

    struct RawMode {
        termios: libc::termios,
        fd: RawFd,
        _tty: Option<std::fs::File>,
    }

    static PRIOR_TERMIOS: Mutex<Option<RawMode>> = Mutex::new(None);

    /// Whether raw mode is currently enabled.
    pub fn is_raw_mode_enabled() -> bool {
        PRIOR_TERMIOS.lock().unwrap().is_some()
    }

    /// Enables terminal raw mode (idempotent).
    pub fn enable_raw_mode() -> io::Result<()> {
        let mut prior = PRIOR_TERMIOS.lock().unwrap();
        if prior.is_some() {
            return Ok(());
        }
        let (fd, tty) = open_tty()?;
        let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let original = termios;
        unsafe { libc::cfmakeraw(&mut termios) };
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } != 0 {
            return Err(io::Error::last_os_error());
        }
        *prior = Some(RawMode { termios: original, fd, _tty: tty });
        Ok(())
    }

    /// Restores the terminal mode saved by [`enable_raw_mode`].
    pub fn disable_raw_mode() -> io::Result<()> {
        let mut prior = PRIOR_TERMIOS.lock().unwrap();
        if let Some(state) = prior.as_ref() {
            let rc = unsafe { libc::tcsetattr(state.fd, libc::TCSANOW, &state.termios) };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            *prior = None;
        }
        Ok(())
    }

    /// Returns the terminal size as `(columns, rows)`.
    pub fn size() -> io::Result<(u16, u16)> {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
        if rc != 0 || ws.ws_col == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((ws.ws_col, ws.ws_row))
    }

    /// Writes terminal query bytes to the tty.
    pub fn write_query(bytes: &[u8]) -> io::Result<()> {
        if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
            tty.write_all(bytes)?;
            tty.flush()
        } else {
            let mut out = io::stdout();
            out.write_all(bytes)?;
            out.flush()
        }
    }

    fn open_tty() -> io::Result<(RawFd, Option<std::fs::File>)> {
        match std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty") {
            Ok(file) => {
                let fd = file.as_raw_fd();
                Ok((fd, Some(file)))
            }
            Err(_) => Ok((libc::STDIN_FILENO, None)),
        }
    }

    fn set_nonblocking(fd: RawFd) -> io::Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Reads and parses input from the terminal.
    pub struct Reader {
        parser: Parser,
        fd: RawFd,
        _tty: Option<std::fs::File>,
        winch: Option<UnixStream>,
        wake: Option<RawFd>,
        buf: [u8; BUFFER_SIZE],
    }

    impl Reader {
        /// Creates an input reader with `SIGWINCH` resize tracking.
        pub fn new() -> io::Result<Self> {
            let (fd, tty) = open_tty()?;
            set_nonblocking(fd)?;
            let (receiver, sender) = UnixStream::pair()?;
            receiver.set_nonblocking(true)?;
            sender.set_nonblocking(true)?;
            signal_hook::low_level::pipe::register(signal_hook::consts::SIGWINCH, sender)?;
            Ok(Self {
                parser: Parser::new(),
                fd,
                _tty: tty,
                winch: Some(receiver),
                wake: None,
                buf: [0u8; BUFFER_SIZE],
            })
        }

        /// Creates a reader without `SIGWINCH` tracking, for capability queries.
        pub fn for_query() -> io::Result<Self> {
            let (fd, tty) = open_tty()?;
            set_nonblocking(fd)?;
            Ok(Self {
                parser: Parser::new(),
                fd,
                _tty: tty,
                winch: None,
                wake: None,
                buf: [0u8; BUFFER_SIZE],
            })
        }

        /// Sets a wake pipe file descriptor to include in the poll set.
        pub fn set_wake_fd(&mut self, fd: RawFd) {
            self.wake = Some(fd);
        }

        /// Returns whether a decoded event is available within `timeout`.
        pub fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            if self.parser.has_event() {
                return Ok(true);
            }
            self.wait(Some(timeout))?;
            Ok(self.parser.has_event())
        }

        /// Pops a queued event without blocking.
        pub fn try_read(&mut self) -> Option<ParsedEvent> {
            self.parser.next()
        }

        /// Waits up to `timeout` (or blocks if `None`) for input, returning whether the wake pipe fired.
        pub fn wait(&mut self, timeout: Option<Duration>) -> io::Result<bool> {
            let winch_fd = self.winch.as_ref().map(|s| s.as_raw_fd());
            let mut read_set: libc::fd_set = unsafe { std::mem::zeroed() };
            unsafe { libc::FD_ZERO(&mut read_set) };
            let mut max_fd = self.fd;
            unsafe { libc::FD_SET(self.fd, &mut read_set) };
            if let Some(w) = winch_fd {
                unsafe { libc::FD_SET(w, &mut read_set) };
                max_fd = max_fd.max(w);
            }
            if let Some(w) = self.wake {
                unsafe { libc::FD_SET(w, &mut read_set) };
                max_fd = max_fd.max(w);
            }
            let mut tv = timeout.map(|d| libc::timeval {
                tv_sec: d.as_secs() as libc::time_t,
                tv_usec: d.subsec_micros() as libc::suseconds_t,
            });
            let tv_ptr = tv
                .as_mut()
                .map_or(std::ptr::null_mut(), |t| t as *mut libc::timeval);
            let rc = unsafe {
                libc::select(
                    max_fd + 1,
                    &mut read_set,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    tv_ptr,
                )
            };
            if rc < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    return Ok(false);
                }
                return Err(err);
            }
            if rc == 0 {
                return Ok(false);
            }
            if unsafe { libc::FD_ISSET(self.fd, &read_set) } {
                self.drain_tty()?;
            }
            if let Some(w) = winch_fd {
                if unsafe { libc::FD_ISSET(w, &read_set) } {
                    self.drain_winch()?;
                }
            }
            let woken = match self.wake {
                Some(w) => unsafe { libc::FD_ISSET(w, &read_set) },
                None => false,
            };
            if woken {
                self.drain_wake();
            }
            Ok(woken)
        }

        fn drain_wake(&mut self) {
            let Some(fd) = self.wake else {
                return;
            };
            let mut scratch = [0u8; 64];
            loop {
                let n = unsafe {
                    libc::read(fd, scratch.as_mut_ptr() as *mut libc::c_void, scratch.len())
                };
                if n <= 0 {
                    break;
                }
            }
        }

        fn drain_tty(&mut self) -> io::Result<()> {
            loop {
                let n = unsafe {
                    libc::read(
                        self.fd,
                        self.buf.as_mut_ptr() as *mut libc::c_void,
                        self.buf.len(),
                    )
                };
                if n > 0 {
                    let n = n as usize;
                    self.parser.feed_all(&self.buf[..n]);
                    if n == self.buf.len() {
                        continue;
                    }
                    break;
                } else if n == 0 {
                    break;
                } else {
                    let err = io::Error::last_os_error();
                    match err.kind() {
                        io::ErrorKind::WouldBlock => break,
                        io::ErrorKind::Interrupted => continue,
                        _ => return Err(err),
                    }
                }
            }
            self.parser.flush_escape();
            Ok(())
        }

        fn drain_winch(&mut self) -> io::Result<()> {
            if let Some(stream) = self.winch.as_mut() {
                let mut scratch = [0u8; 64];
                loop {
                    match stream.read(&mut scratch) {
                        Ok(0) => break,
                        Ok(_) => continue,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e),
                    }
                }
            }
            let (cols, rows) = size()?;
            self.parser.push_event(ParsedEvent::Resize(cols, rows));
            Ok(())
        }
    }

}

#[cfg(windows)]
mod windows {
    use super::input::Parser;
    use super::ParsedEvent;
    use std::io::{self, Write};
    use std::sync::Mutex;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0};
    use windows_sys::Win32::Storage::FileSystem::ReadFile;
    use windows_sys::Win32::System::Console::{
        GetConsoleCP, GetConsoleMode, GetConsoleOutputCP, GetConsoleScreenBufferInfo, GetStdHandle,
        PeekConsoleInputW, ReadConsoleInputW, SetConsoleCP, SetConsoleMode, SetConsoleOutputCP,
        CONSOLE_MODE, CONSOLE_SCREEN_BUFFER_INFO, DISABLE_NEWLINE_AUTO_RETURN, ENABLE_ECHO_INPUT,
        ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_PROCESSED_OUTPUT,
        ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        ENABLE_WINDOW_INPUT, INPUT_RECORD, KEY_EVENT, MOUSE_EVENT, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::Threading::{WaitForMultipleObjects, WaitForSingleObject};

    const BUFFER_SIZE: usize = 1024;
    /// UTF-8 code page, so the VT byte stream and parser agree on encoding.
    const CP_UTF8: u32 = 65001;

    /// Console state captured by [`enable_raw_mode`] for restoration. Holds only
    /// the prior mode flags and code pages (all `Copy`/`Send`); the console
    /// handles themselves are re-fetched via `GetStdHandle` on restore so this
    /// stays `Send` enough to live in a `static`.
    struct PriorMode {
        in_mode: CONSOLE_MODE,
        out_mode: CONSOLE_MODE,
        in_cp: u32,
        out_cp: u32,
    }

    static PRIOR: Mutex<Option<PriorMode>> = Mutex::new(None);

    fn std_handles() -> io::Result<(HANDLE, HANDLE)> {
        // SAFETY: GetStdHandle returns process-global standard handles.
        let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if input == INVALID_HANDLE_VALUE || output == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok((input, output))
    }

    /// Whether raw mode is currently enabled.
    pub fn is_raw_mode_enabled() -> bool {
        PRIOR.lock().unwrap().is_some()
    }

    /// Enables terminal raw mode (idempotent).
    ///
    /// Puts the input console into virtual-terminal mode so keyboard, mouse,
    /// focus, paste and terminal capability replies all arrive as a VT byte
    /// stream (read via [`Reader`]), and enables VT processing on the output so
    /// escape sequences are interpreted. Code pages are switched to UTF-8.
    pub fn enable_raw_mode() -> io::Result<()> {
        let mut prior = PRIOR.lock().unwrap();
        if prior.is_some() {
            return Ok(());
        }
        let (input, output) = std_handles()?;
        // SAFETY: console handles are valid; out-params are stack locals.
        unsafe {
            let mut in_mode: CONSOLE_MODE = 0;
            let mut out_mode: CONSOLE_MODE = 0;
            if GetConsoleMode(input, &mut in_mode) == 0 {
                return Err(io::Error::last_os_error());
            }
            if GetConsoleMode(output, &mut out_mode) == 0 {
                return Err(io::Error::last_os_error());
            }
            let in_cp = GetConsoleCP();
            let out_cp = GetConsoleOutputCP();

            // Clear cooked-input flags + window/quick-edit (so resizes don't
            // enqueue records that would leave the handle signalled with no
            // bytes for ReadFile), and request VT input.
            let new_in = (in_mode
                & !(ENABLE_LINE_INPUT
                    | ENABLE_ECHO_INPUT
                    | ENABLE_PROCESSED_INPUT
                    | ENABLE_QUICK_EDIT_MODE
                    | ENABLE_WINDOW_INPUT))
                | ENABLE_VIRTUAL_TERMINAL_INPUT
                | ENABLE_EXTENDED_FLAGS;
            let new_out = out_mode
                | ENABLE_PROCESSED_OUTPUT
                | ENABLE_VIRTUAL_TERMINAL_PROCESSING
                | DISABLE_NEWLINE_AUTO_RETURN;

            if SetConsoleMode(input, new_in) == 0 {
                return Err(io::Error::last_os_error());
            }
            if SetConsoleMode(output, new_out) == 0 {
                let err = io::Error::last_os_error();
                let _ = SetConsoleMode(input, in_mode);
                return Err(err);
            }
            SetConsoleCP(CP_UTF8);
            SetConsoleOutputCP(CP_UTF8);

            *prior = Some(PriorMode { in_mode, out_mode, in_cp, out_cp });
        }
        Ok(())
    }

    /// Restores the terminal mode saved by [`enable_raw_mode`].
    pub fn disable_raw_mode() -> io::Result<()> {
        let mut prior = PRIOR.lock().unwrap();
        if let Some(state) = prior.as_ref() {
            let (input, output) = std_handles()?;
            // SAFETY: restoring previously-read, valid mode flags / code pages.
            unsafe {
                SetConsoleMode(input, state.in_mode);
                SetConsoleMode(output, state.out_mode);
                SetConsoleCP(state.in_cp);
                SetConsoleOutputCP(state.out_cp);
            }
            *prior = None;
        }
        Ok(())
    }

    /// Returns the terminal size as `(columns, rows)`.
    pub fn size() -> io::Result<(u16, u16)> {
        let (_, output) = std_handles()?;
        // SAFETY: valid console output handle; info is a stack out-param.
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
        if unsafe { GetConsoleScreenBufferInfo(output, &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let cols = (info.srWindow.Right - info.srWindow.Left + 1).max(0) as u16;
        let rows = (info.srWindow.Bottom - info.srWindow.Top + 1).max(0) as u16;
        if cols == 0 {
            return Err(io::Error::new(io::ErrorKind::Other, "console reported zero width"));
        }
        Ok((cols, rows))
    }

    /// Writes terminal query bytes to the console output.
    pub fn write_query(bytes: &[u8]) -> io::Result<()> {
        if let Ok(mut conout) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
            conout.write_all(bytes)?;
            conout.flush()
        } else {
            let mut out = io::stdout();
            out.write_all(bytes)?;
            out.flush()
        }
    }

    /// How [`Reader::wait`] should clear a signalled console input handle without
    /// blocking the event loop. See [`Reader::classify_input`].
    enum InputAction {
        /// A byte-producing record is queued — `ReadFile` is safe.
        Read,
        /// `n` queued records are all noise — consume them, skip `ReadFile`.
        DrainNoise(u32),
        /// Nothing queued.
        Idle,
    }

    /// True for virtual-key codes that are pure modifiers or locks. Key-down
    /// events for these produce no VT bytes, so they must not trigger a
    /// (blocking) `ReadFile`; every other key-down yields a character or an
    /// escape sequence. Values are the stable `VK_*` constants (hard-coded to
    /// avoid pulling in the `Win32_UI_Input_KeyboardAndMouse` feature):
    /// SHIFT/CONTROL/MENU (incl. L/R variants), L/R WIN, CAPS/NUM/SCROLL lock.
    fn is_modifier_vk(vk: u16) -> bool {
        matches!(
            vk,
            0x10 | 0x11 | 0x12          // VK_SHIFT, VK_CONTROL, VK_MENU (Alt)
                | 0xA0..=0xA5           // VK_L/R SHIFT, CONTROL, MENU
                | 0x5B | 0x5C           // VK_LWIN, VK_RWIN
                | 0x14 | 0x90 | 0x91    // VK_CAPITAL, VK_NUMLOCK, VK_SCROLL
        )
    }

    /// Reads and parses input from the console.
    ///
    /// Handles are stored as `isize` (cast to `HANDLE` at the call site) so the
    /// reader stays `Send`, matching the Unix backend's `RawFd`-based reader.
    pub struct Reader {
        parser: Parser,
        input: isize,
        wake: Option<isize>,
        buf: [u8; BUFFER_SIZE],
        last_size: Option<(u16, u16)>,
    }

    impl Reader {
        /// Creates an input reader.
        pub fn new() -> io::Result<Self> {
            Self::open()
        }

        /// Creates a reader for capability queries (identical on Windows).
        pub fn for_query() -> io::Result<Self> {
            Self::open()
        }

        fn open() -> io::Result<Self> {
            let (input, _) = std_handles()?;
            Ok(Self {
                parser: Parser::new(),
                input: input as isize,
                wake: None,
                buf: [0u8; BUFFER_SIZE],
                last_size: size().ok(),
            })
        }

        /// Adds an auto-reset event handle to the wait set (see [`super::WakeHandle`]).
        pub fn set_wake_fd(&mut self, handle: super::WakeHandle) {
            self.wake = Some(handle as isize);
        }

        /// Returns whether a decoded event is available within `timeout`.
        pub fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            if self.parser.has_event() {
                return Ok(true);
            }
            self.wait(Some(timeout))?;
            Ok(self.parser.has_event())
        }

        /// Pops a queued event without blocking.
        pub fn try_read(&mut self) -> Option<ParsedEvent> {
            self.parser.next()
        }

        /// Waits up to `timeout` (or indefinitely if `None`) for input, returning
        /// whether the wake event fired.
        pub fn wait(&mut self, timeout: Option<Duration>) -> io::Result<bool> {
            // Windows has no SIGWINCH; resizes are detected by polling `size()`,
            // so cap the wait to keep that poll responsive even while idle.
            const POLL_CAP: Duration = Duration::from_millis(100);
            let effective = match timeout {
                Some(t) => t.min(POLL_CAP),
                None => POLL_CAP,
            };
            let ms = effective.as_millis().min(u32::MAX as u128 - 1) as u32;

            let mut handles: [HANDLE; 2] = [self.input as HANDLE, std::ptr::null_mut()];
            let count = match self.wake {
                Some(w) => {
                    handles[1] = w as HANDLE;
                    2
                }
                None => 1,
            };

            // SAFETY: handles[..count] are valid; bWaitAll = FALSE.
            let rc = unsafe { WaitForMultipleObjects(count, handles.as_ptr(), 0, ms) };
            if rc == WAIT_FAILED {
                return Err(io::Error::last_os_error());
            }
            let mut woken = false;
            if rc == WAIT_OBJECT_0 {
                // The input handle is signalled whenever the console input buffer
                // is non-empty — INCLUDING by "noise" records (focus, menu, buffer
                // resize, key-up, and lone modifier key-down) that `ReadFile`
                // translates to ZERO VT bytes and then BLOCKS on, waiting for real
                // input. A blind `ReadFile` here would park the whole event loop
                // until the next keystroke, starving cross-thread/async wakes (the
                // auto-reset event below never gets a chance to be observed). So
                // only `ReadFile` when a byte-producing key/mouse record is queued;
                // otherwise consume the noise records out-of-band so the handle
                // stops signalling without ever blocking.
                match self.classify_input()? {
                    InputAction::Read => self.drain_input()?,
                    InputAction::DrainNoise(count) => self.drain_noise(count)?,
                    InputAction::Idle => {}
                }
            }
            // ALWAYS check the wake event independently of the input handle.
            // `WaitForMultipleObjects` reports only the lowest signalled index,
            // so a cross-thread wake that coincides with a signalled input
            // handle (index 0) would never surface as index 1 on its own — the
            // wake would be starved until the next keystroke. A zero-timeout
            // probe consumes (auto-resets) the event whenever it is set,
            // regardless of which index the wait reported.
            if let Some(w) = self.wake {
                let already = count == 2 && rc == WAIT_OBJECT_0 + 1;
                // SAFETY: `w` is the runtime's owned auto-reset event handle.
                if already || unsafe { WaitForSingleObject(w as HANDLE, 0) } == WAIT_OBJECT_0 {
                    woken = true;
                }
            }
            // WAIT_TIMEOUT (and every other branch) falls through to a resize poll.
            self.poll_resize();
            Ok(woken)
        }

        /// Inspects the queued console input records (without consuming them) and
        /// decides how to clear the signalled handle:
        ///
        /// * [`InputAction::Read`] — a byte-producing key/mouse record is queued,
        ///   so a following `ReadFile` is guaranteed to return ≥1 byte and won't
        ///   block. Also the fallback for a redirected / piped (non-console)
        ///   handle, where `PeekConsoleInput` is unavailable.
        /// * [`InputAction::DrainNoise(n)`] — the `n` peeked records are all noise
        ///   (focus/menu/resize/key-up/modifier) that yield no VT bytes; they must
        ///   be removed via `ReadConsoleInputW` so `wait` doesn't spin, but
        ///   `ReadFile` must be skipped or it would block.
        /// * [`InputAction::Idle`] — nothing queued (a spurious signal).
        fn classify_input(&self) -> io::Result<InputAction> {
            const PEEK: usize = 32;
            // SAFETY: INPUT_RECORD is a plain C struct; a zeroed buffer is a valid
            // (EventType == 0) placeholder overwritten by PeekConsoleInputW.
            let mut records: [INPUT_RECORD; PEEK] = unsafe { std::mem::zeroed() };
            let mut read: u32 = 0;
            // SAFETY: valid input handle; buffer + out-param are local.
            let ok = unsafe {
                PeekConsoleInputW(self.input as HANDLE, records.as_mut_ptr(), PEEK as u32, &mut read)
            };
            if ok == 0 {
                // Not a console (redirected / pipe): preserve the piped-input path
                // by reading directly — a pipe's `ReadFile` returns on available
                // data and has no noise records.
                return Ok(InputAction::Read);
            }
            if read == 0 {
                return Ok(InputAction::Idle);
            }
            let any_real = records[..read as usize]
                .iter()
                .any(Self::record_produces_bytes);
            Ok(if any_real {
                InputAction::Read
            } else {
                // Drain only the records we actually peeked-as-noise; a real
                // keystroke arriving after the peek sits behind them and is left
                // for the next `ReadFile`.
                InputAction::DrainNoise(read)
            })
        }

        /// Whether the console would translate `record` into ≥1 VT byte (so a
        /// following `ReadFile` won't block): true for non-modifier key-down
        /// events and mouse events (mouse VT tracking is enabled while raw mode is
        /// on). Focus/menu/buffer-resize records, key-up events, and lone modifier
        /// key-down events all yield no bytes.
        fn record_produces_bytes(record: &INPUT_RECORD) -> bool {
            let event_type = record.EventType as u32;
            if event_type == KEY_EVENT {
                // SAFETY: union field selected by EventType == KEY_EVENT.
                let key = unsafe { &record.Event.KeyEvent };
                key.bKeyDown != 0 && !is_modifier_vk(key.wVirtualKeyCode)
            } else {
                event_type == MOUSE_EVENT
            }
        }

        /// Removes exactly `count` queued records via `ReadConsoleInputW`. Called
        /// only when those records are all noise, so nothing readable is dropped;
        /// this clears the signalled state so `wait` stops returning immediately.
        fn drain_noise(&self, count: u32) -> io::Result<()> {
            const BATCH: usize = 32;
            let want = (count as usize).min(BATCH);
            if want == 0 {
                return Ok(());
            }
            // SAFETY: see `classify_input` — zeroed INPUT_RECORDs overwritten by
            // ReadConsoleInputW.
            let mut records: [INPUT_RECORD; BATCH] = unsafe { std::mem::zeroed() };
            let mut read: u32 = 0;
            // SAFETY: valid input handle; buffer + out-param are local.
            let ok = unsafe {
                ReadConsoleInputW(self.input as HANDLE, records.as_mut_ptr(), want as u32, &mut read)
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        /// Reads one batch of available VT bytes into the parser.
        ///
        /// Only called after [`Reader::classify_input`] confirms a byte-producing
        /// record is queued, so `ReadFile` returns ≥1 byte without blocking. A
        /// full buffer leaves the remainder queued; the handle stays signalled and
        /// the next `wait` drains it.
        fn drain_input(&mut self) -> io::Result<()> {
            let mut read: u32 = 0;
            // SAFETY: valid console input handle; buffer + out-param are local.
            let ok = unsafe {
                ReadFile(
                    self.input as HANDLE,
                    self.buf.as_mut_ptr(),
                    self.buf.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if read > 0 {
                self.parser.feed_all(&self.buf[..read as usize]);
            }
            self.parser.flush_escape();
            Ok(())
        }

        /// Emits a [`ParsedEvent::Resize`] when the console dimensions change.
        fn poll_resize(&mut self) {
            if let Ok(sz) = size() {
                if self.last_size != Some(sz) {
                    self.last_size = Some(sz);
                    self.parser.push_event(ParsedEvent::Resize(sz.0, sz.1));
                }
            }
        }
    }
}
