//! Terminal cursor shape enum.

/// Terminal cursor shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    /// Solid block over the cell.
    Block,
    /// Vertical bar at the left edge of the cell.
    Beam,
    /// Underline along the bottom of the cell.
    Underline,
}

impl std::fmt::Display for CursorShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Block => write!(f, "Block"),
            Self::Beam => write!(f, "Beam"),
            Self::Underline => write!(f, "Underline"),
        }
    }
}

/// Mouse pointer (hardware cursor) shape, emitted to the terminal via `OSC 22`.
///
/// Terminals that understand `OSC 22 ; <css-name>` (kitty, ghostty, foot,
/// WezTerm, contour, …) switch the mouse pointer; terminals that don't simply
/// ignore the sequence. Use it as a hover affordance — e.g. a resize pointer
/// over a draggable [`Split`](crate::widget::widgets::split::Split) divider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MousePointerShape {
    /// The terminal's default arrow pointer.
    #[default]
    Default,
    /// Horizontal resize (↔) — for a vertical divider.
    ColResize,
    /// Vertical resize (↕) — for a horizontal divider.
    RowResize,
}

impl MousePointerShape {
    /// The CSS cursor name used in the `OSC 22` payload. Uses the plain
    /// `ew-resize`/`ns-resize` arrows (recognized by more terminals than the
    /// `col-resize`/`row-resize` aliases).
    pub fn css_name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::ColResize => "ew-resize",
            Self::RowResize => "ns-resize",
        }
    }
}
