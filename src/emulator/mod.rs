//! Terminal emulator for driving widgets through the runtime.

use crate::prelude::*;

/// Serializes emulator sessions across threads.
///
/// Parts of the runtime are process-global — most importantly the message
/// `INBOX` that `tuie::send` posts to ("send from any thread" is a
/// production contract, so it cannot be thread-local). `drain_inbox` takes
/// the WHOLE inbox and silently drops messages whose target isn't in its
/// own tree. Two emulator sessions on parallel test threads therefore
/// steal (and discard) each other's messages — tests then fail on
/// "message never arrived" symptoms, but only under parallel load.
///
/// Holding this lock for the [`Emulator`]'s lifetime makes emulator
/// sessions mutually exclusive process-wide. Reentrant per thread because
/// some tests create a second emulator while a shadowed first one is still
/// in scope (its guard drops at scope end, not at the shadowing point).
struct SessionLock {
    /// Current owner thread and its reentrancy depth; `None` when free.
    state: std::sync::Mutex<Option<(std::thread::ThreadId, u32)>>,
    freed: std::sync::Condvar,
}

static EMULATOR_SESSION: SessionLock = SessionLock {
    state: std::sync::Mutex::new(None),
    freed: std::sync::Condvar::new(),
};

impl SessionLock {
    fn acquire(&self) {
        // Poison-tolerant: a panicking test (assert failures are routine)
        // must not fail every later emulator test with a PoisonError.
        let me = std::thread::current().id();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match &mut *state {
                Some((owner, depth)) if *owner == me => {
                    *depth += 1;
                    return;
                }
                None => {
                    *state = Some((me, 1));
                    return;
                }
                Some(_) => {
                    state = self
                        .freed
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            }
        }
    }

    fn release(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, depth)) = &mut *state {
            *depth -= 1;
            if *depth == 0 {
                *state = None;
                self.freed.notify_one();
            }
        }
    }
}

/// Drives a widget tree through the runtime and captures the rendered output.
///
/// Holds the process-wide emulator session lock for its lifetime (see
/// [`SessionLock`]); emulator-driven tests on parallel threads run one at
/// a time instead of corrupting each other through the runtime's global
/// message inbox.
pub struct Emulator(());

impl Emulator {
    /// Creates an [`Emulator`] with `root` rendered at `size`.
    pub fn new(root: &mut dyn Widget, size: Vec2<u16>) -> Self {
        EMULATOR_SESSION.acquire();
        crate::runtime::init_emulator(size);
        let _ = crate::runtime::update(root, &[RuntimeEvent::Resize(size)]);
        Self(())
    }

    /// Processes `events` through the runtime.
    pub fn update(&mut self, root: &mut dyn Widget, events: &[RuntimeEvent]) {
        let _ = crate::runtime::update(root, events);
    }

    /// Overrides the emulated [`RuntimeInfo`] capabilities, e.g. `cell_size` and `subcell_events`.
    pub fn update_runtime_info(&mut self, f: impl FnOnce(&mut RuntimeInfo)) {
        crate::runtime::update_runtime_info(f);
    }

    /// Returns the most recently rendered frame as a [`StyledString`].
    pub fn get_snapshot(&self) -> StyledString {
        crate::runtime::get_emulator_snapshot()
    }

    /// Returns the terminal cursor from the most recently rendered frame.
    pub fn get_cursor(&self) -> Option<(CursorShape, Vec2<i32>)> {
        crate::runtime::get_emulator_cursor()
    }

    /// Returns the most recently rendered frame as plain text.
    pub fn get_snapshot_text(&self) -> String {
        self.get_snapshot().into_string()
    }

    /// Asserts the rendered frame matches `lines` row for row.
    ///
    /// # Panics
    ///
    /// Panics on mismatch.
    #[track_caller]
    pub fn assert_lines<'a, I>(&self, lines: I)
    where
        I: IntoIterator<Item = &'a str>,
    {
        let expected: Vec<&str> = lines.into_iter().collect();
        let actual = self.get_snapshot_text();
        let actual_lines: Vec<&str> = actual.split('\n').collect();
        if expected != actual_lines {
            let mut msg = String::from("rendered output did not match expected:\n");
            let n = expected.len().max(actual_lines.len());
            for i in 0..n {
                let e = expected.get(i).copied().unwrap_or("<missing>");
                let a = actual_lines.get(i).copied().unwrap_or("<missing>");
                let marker = if e == a {
                    "  "
                } else {
                    "!="
                };
                msg.push_str(&format!("  {marker} expected {e:?}\n     actual   {a:?}\n"));
            }
            panic!("{msg}");
        }
    }
}

impl Drop for Emulator {
    fn drop(&mut self) {
        // Runs during unwinding too, so a panicking test releases the
        // session for the next one.
        EMULATOR_SESSION.release();
    }
}
