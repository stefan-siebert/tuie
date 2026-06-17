//! Input trigger variants.

use crate::prelude::*;

/// Specific action that initiated an input event.
#[derive(Clone, PartialEq, Debug)]
#[non_exhaustive]
pub enum Trigger {
    /// Key press.
    Key(Key),
    /// Scroll wheel motion in the given direction.
    MouseScroll(Direction2D),
    /// Sub-cell scroll motion in cell units.
    MouseSmoothScroll(Direction2D, f32),
    /// Mouse button press.
    MouseDown(MouseButton),
    /// Mouse drag event.
    MouseDrag(MouseButton),
    /// Mouse button release.
    MouseUp(MouseButton),
    /// Mouse motion with no buttons held.
    MouseHover,
    /// Bracketed paste event.
    Paste(String),
}

impl std::fmt::Display for Trigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Key(key) => write!(f, "KeyDown({key})"),
            Self::MouseScroll(direction) => write!(f, "MouseScroll({direction})"),
            Self::MouseSmoothScroll(direction, delta) => write!(f, "MouseSmoothScroll({direction}, {delta})"),
            Self::MouseDown(button) => write!(f, "MouseDown({button})"),
            Self::MouseDrag(button) => write!(f, "MouseDrag({button})"),
            Self::MouseUp(button) => write!(f, "MouseUp({button})"),
            Self::MouseHover => write!(f, "MouseHover"),
            Self::Paste(_) => write!(f, "Paste"),
        }
    }
}
