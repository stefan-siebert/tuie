//! Mouse button enum.

/// Mouse button identifier.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum MouseButton {
    /// Left mouse button.
    Left,
    /// Middle mouse button.
    Middle,
    /// Right mouse button.
    Right,
}

impl std::fmt::Display for MouseButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Left => write!(f, "Left"),
            Self::Middle => write!(f, "Middle"),
            Self::Right => write!(f, "Right"),
        }
    }
}
