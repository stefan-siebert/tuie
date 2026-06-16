//! Popup management and placement.

use crate::prelude::*;
use std::cell::RefCell;

struct PopupQueues {
    open: Vec<Popup>,
    close: Vec<WidgetId>,
    dismiss: Vec<WidgetId>,
}

thread_local! {
    static POPUP_QUEUES: RefCell<PopupQueues> = const {
        RefCell::new(PopupQueues {
            open: Vec::new(),
            close: Vec::new(),
            dismiss: Vec::new(),
        })
    };
}

/// Anchor and popup alignment points plus a cell offset that resolve a popup's screen position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// Alignment point on the anchor rect that the popup point lines up with.
    pub anchor_point: Vec2<Align>,
    /// Alignment point on the popup rect that is positioned at the anchor point.
    pub popup_point: Vec2<Align>,
    /// Additional cell offset applied after alignment resolution.
    pub offset: Vec2<i16>,
}

impl Placement {
    /// Centers the popup on the anchor.
    pub fn center() -> Self {
        Self {
            anchor_point: Vec2::of(Align::Middle),
            popup_point: Vec2::of(Align::Middle),
            offset: Vec2::of(0),
        }
    }

    /// Places the popup adjacent to the anchor along `dir`.
    pub fn side(dir: Direction2D, sign: Sign, align: Align) -> Self {
        let axis = dir.axis();
        let cross = axis.flip();

        let anchor_edge = match dir.screen_sign() {
            Sign::Positive => Align::End,
            Sign::Negative => Align::Start,
        };

        let popup_edge = match sign {
            Sign::Positive => anchor_edge.flip(),
            Sign::Negative => anchor_edge,
        };

        let mut anchor_point = Vec2::of(Align::Start);
        let mut popup_point = Vec2::of(Align::Start);

        anchor_point[axis] = anchor_edge;
        popup_point[axis] = popup_edge;
        anchor_point[cross] = align;
        popup_point[cross] = align;

        Self {
            anchor_point,
            popup_point,
            offset: Vec2::of(0),
        }
    }

    /// Sets the cell offset applied after alignment resolution.
    pub fn offset(mut self, offset: Vec2<i16>) -> Self {
        self.offset = offset;
        self
    }
}

/// Floating widget overlay with a [`Placement`] and dismissal policy.
pub struct Popup {
    pub(crate) content: Box<dyn Widget>,
    placement: Placement,
    dismissible: bool,
}

impl Popup {
    /// Creates a popup wrapping `content`.
    pub fn new(content: Box<dyn Widget>) -> Self {
        Self {
            content,
            placement: Placement::center(),
            dismissible: false,
        }
    }

    /// Overrides the [`Placement`].
    pub fn placement(mut self, placement: Placement) -> Self {
        self.placement = placement;
        self
    }

    /// Sets whether outside interaction closes the popup automatically.
    pub fn dismissible(mut self, dismissible: bool) -> Self {
        self.dismissible = dismissible;
        self
    }
}

pub(crate) struct ActivePopup {
    pub content: Box<dyn Widget>,
    pub placement: Placement,
    pub dismissible: bool,
    pub focus_chain: Vec<WidgetId>,
}

impl ActivePopup {
    pub(crate) fn from_popup(popup: Popup, focus_chain: Vec<WidgetId>) -> Self {
        Self {
            content: popup.content,
            placement: popup.placement,
            dismissible: popup.dismissible,
            focus_chain,
        }
    }
}

/// Event signalling the user attempted to dismiss a non-dismissible [`Popup`].
pub struct PopupDismissRequested;

/// Event signalling a [`Popup`] was closed.
pub struct PopupClosed;

/// Opens `popup`.
pub fn open_popup(popup: Popup) {
    POPUP_QUEUES.with_borrow_mut(|q| q.open.push(popup));
}

/// Closes the popup containing `id`.
pub fn close_popup(id: WidgetId<impl ?Sized>) {
    POPUP_QUEUES.with_borrow_mut(|q| q.close.push(id.untyped()));
}

/// Queues a [`PopupDismissRequested`] event for the popup containing `id`.
pub fn dismiss_popup(id: WidgetId<impl ?Sized>) {
    POPUP_QUEUES.with_borrow_mut(|q| q.dismiss.push(id.untyped()));
}

pub(crate) fn drain_open_requests() -> Vec<Popup> {
    POPUP_QUEUES.with_borrow_mut(|q| std::mem::take(&mut q.open))
}

pub(crate) fn drain_close_requests() -> Vec<WidgetId> {
    POPUP_QUEUES.with_borrow_mut(|q| std::mem::take(&mut q.close))
}

pub(crate) fn drain_dismiss_requests() -> Vec<WidgetId> {
    POPUP_QUEUES.with_borrow_mut(|q| std::mem::take(&mut q.dismiss))
}

pub(crate) fn resolve_placement(
    placement: &Placement,
    anchor_rect: Rect<i32, u16>,
    popup_size: Vec2<u16>,
) -> Vec2<i32> {
    Axis2D::map(|a| {
        let anchor_pos = anchor_rect.pos[a];
        let anchor_size = anchor_rect.size[a] as i32;
        let popup_size = popup_size[a] as i32;

        let anchor_coord = match placement.anchor_point[a] {
            Align::Start => anchor_pos,
            Align::Middle => anchor_pos + anchor_size / 2,
            Align::End => anchor_pos + anchor_size,
        };

        let popup_coord = match placement.popup_point[a] {
            Align::Start => 0,
            Align::Middle => popup_size / 2,
            Align::End => popup_size,
        };

        anchor_coord - popup_coord + placement.offset[a] as i32
    })
}

/// Shift a resolved popup position so the popup stays within the window when it
/// can: if it would overflow the far (right/bottom) edge, pull it back so its far
/// edge meets the window edge; never push its near edge past the origin. A popup
/// larger than the window is pinned to the top-left so its top-left stays visible
/// (the overflow is clipped at the far edge rather than hidden off the near one).
///
/// Without this, an anchored popup placed near the right/bottom edge — e.g. a
/// right-click context menu opened low in a panel — renders partly off-screen.
fn clamp_to_window(pos: Vec2<i32>, popup_size: Vec2<u16>, window_size: Vec2<u16>) -> Vec2<i32> {
    Axis2D::map(|a| {
        let size = popup_size[a] as i32;
        let window = window_size[a] as i32;
        pos[a].min(window - size).max(0)
    })
}

pub(crate) fn position_popup(popup: &mut ActivePopup, window_size: Vec2<u16>) {
    let window_rect = Rect::new(Vec2::of(0i32), window_size);
    let popup_size = popup.content.get_outer_size();
    let pos = resolve_placement(&popup.placement, window_rect, popup_size);
    let pos = clamp_to_window(pos, popup_size, window_size);
    let margin_before = popup.content.get_layout().get_margin_before().map(|v| v as i32);
    popup.content.set_pos(pos + margin_before);
    popup.content.layout_position();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fully_visible_popup_is_unchanged() {
        let pos = clamp_to_window(Vec2::new(5, 3), Vec2::new(20, 10), Vec2::new(80, 24));
        assert_eq!((pos.x, pos.y), (5, 3));
    }

    #[test]
    fn popup_overflowing_bottom_is_pulled_up() {
        // A 20x10 popup at y=20 in a 24-row window would reach row 30 → pulled up
        // so its bottom meets the window edge (24 - 10 = 14).
        let pos = clamp_to_window(Vec2::new(5, 20), Vec2::new(20, 10), Vec2::new(80, 24));
        assert_eq!((pos.x, pos.y), (5, 14));
    }

    #[test]
    fn popup_overflowing_right_is_pulled_left() {
        // A 30-wide popup at x=70 in an 80-col window would reach col 100 → pulled
        // left so its right edge meets the window edge (80 - 30 = 50).
        let pos = clamp_to_window(Vec2::new(70, 2), Vec2::new(30, 5), Vec2::new(80, 24));
        assert_eq!((pos.x, pos.y), (50, 2));
    }

    #[test]
    fn popup_larger_than_window_pins_to_origin() {
        // A 30-row popup in a 24-row window can't fit; pin to the top so its
        // top-left stays visible rather than scrolling off the near edge.
        let pos = clamp_to_window(Vec2::new(2, 5), Vec2::new(20, 30), Vec2::new(80, 24));
        assert_eq!((pos.x, pos.y), (2, 0));
    }
}
