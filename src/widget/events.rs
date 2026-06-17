//! Standard widget event payloads dispatched along a [`WidgetPath`](super::WidgetPath).

/// Scroll offset change notification.
pub struct ScrollEvent;

/// Click notification.
pub struct ClickEvent;

/// Value change notification carrying the new value.
pub struct ChangeEvent<T>(pub T);
