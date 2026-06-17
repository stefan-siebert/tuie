//! Keyboard modifier flags.

/// Single keyboard modifier flag.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Modifier {
    /// Shift modifier.
    Shift = 1 << 0,
    /// Ctrl modifier.
    Ctrl = 1 << 1,
    /// Alt or Option modifier.
    Alt = 1 << 2,
    /// Super or Command modifier.
    Super = 1 << 3,
    /// Meta modifier.
    Meta = 1 << 4,
    /// Hyper modifier.
    Hyper = 1 << 5,
}

impl std::fmt::Display for Modifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ctrl => write!(f, "Ctrl"),
            Self::Alt => write!(f, "Alt"),
            Self::Shift => write!(f, "Shift"),
            Self::Super => write!(f, "Super"),
            Self::Meta => write!(f, "Meta"),
            Self::Hyper => write!(f, "Hyper"),
        }
    }
}

/// Bitset of [`Modifier`] flags held simultaneously.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Modifiers {
    /// Packed bits, one per [`Modifier`] variant. Public only for `chord!` pattern expansion.
    #[doc(hidden)]
    pub modifiers: u8,
}

impl std::fmt::Display for Modifiers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let display_order = [
            Modifier::Super,
            Modifier::Meta,
            Modifier::Hyper,
            Modifier::Ctrl,
            Modifier::Alt,
            Modifier::Shift,
        ];
        let mut written = false;
        for modifier in display_order {
            if !self.has(modifier) {
                continue;
            }
            if written {
                write!(f, " + ")?;
            }
            written = true;
            write!(f, "{modifier}")?;
        }
        Ok(())
    }
}

impl Default for Modifiers {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifiers {
    /// Creates an empty modifier set.
    pub const fn new() -> Self {
        Self { modifiers: 0 }
    }

    /// Returns whether `modifier` is set.
    pub const fn has(&self, modifier: Modifier) -> bool {
        self.modifiers & (modifier as u8) != 0
    }

    /// Sets or clears `modifier`.
    pub const fn set(&mut self, modifier: Modifier, value: bool) {
        if value {
            self.modifiers |= modifier as u8;
        } else {
            self.modifiers &= !(modifier as u8);
        }
    }

    /// Returns a copy with `modifier` enabled.
    pub const fn with(mut self, modifier: Modifier) -> Self {
        self.set(modifier, true);
        self
    }

    /// Returns a copy with `modifier` set to `value`.
    pub const fn with_if(mut self, modifier: Modifier, value: bool) -> Self {
        self.set(modifier, value);
        self
    }

    /// Returns whether no modifier flags are set.
    pub const fn is_empty(&self) -> bool {
        self.modifiers == 0
    }
}
