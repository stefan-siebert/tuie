//! Logical keyboard key type.

use crate::prelude::*;

/// Logical keyboard key without modifiers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Key {
    /// Backspace.
    Backspace,
    /// Return or Enter.
    Enter,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Tab or BackTab.
    Tab,
    /// Delete.
    Delete,
    /// Insert.
    Insert,
    /// Escape.
    Esc,
    /// Arrow key in the given direction.
    Arrow(Direction2D),
    /// Function key `F1` through `F12`.
    F(u8),
    /// Printable character key.
    Char(char),
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backspace => write!(f, "Backspace"),
            Self::Enter => write!(f, "Enter"),
            Self::Home => write!(f, "Home"),
            Self::End => write!(f, "End"),
            Self::PageUp => write!(f, "PageUp"),
            Self::PageDown => write!(f, "PageDown"),
            Self::Tab => write!(f, "Tab"),
            Self::Delete => write!(f, "Delete"),
            Self::Insert => write!(f, "Insert"),
            Self::Esc => write!(f, "Esc"),
            Self::Arrow(direction) => write!(f, "Arrow{}", direction),
            Self::F(n) => write!(f, "F{n}"),
            Self::Char(c) => write!(f, "'{c}'"),
        }
    }
}

