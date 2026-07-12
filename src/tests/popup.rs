//! Popup dismissal routing: a click outside a non-dismissible popup must
//! deliver `PopupDismissRequested` to the popup *content* (whose focus chain it
//! is rooted in), not the app root — otherwise the event is silently dropped
//! and the popup can never be closed by mouse. Regression test for that bug.

use crate::prelude::*;
use crate::emulator::Emulator;
use std::cell::RefCell;
use std::rc::Rc;

/// Popup content that closes itself when asked to dismiss — the same contract
/// Elane's dialogs follow (handle `PopupDismissRequested` → close).
struct DismissProbe {
    root: Box<Pane>,
    dismissed: Rc<RefCell<bool>>,
}

impl DelegateWidget for DismissProbe {
    crate::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        true
    }

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        if event.take::<PopupDismissRequested>().is_some() {
            *self.dismissed.borrow_mut() = true;
            crate::close_popup(self.get_id());
        }
    }
}

fn mouse_down(pos: Vec2<i32>) -> RuntimeEvent {
    // The runtime treats an injected event's `pos` as the window position and
    // translates it to leaf-local during dispatch, so window coords go here.
    RuntimeEvent::Input(InputEvent {
        chord: Chord::new(Trigger::MouseDown(MouseButton::Left), Modifiers::new()),
        pos: pos.map(|v| v as f32),
        count: 1,
    })
}

#[test]
fn outside_click_dismisses_nondismissible_popup() {
    let mut root = Pane::new().vertical().flex(1).child(Text::new().content("background"));
    let mut term = Emulator::new(&mut *root, Vec2::new(80, 24));

    let dismissed = Rc::new(RefCell::new(false));
    let content = Box::new(DismissProbe {
        root: Pane::new()
            .vertical()
            .min_width(20)
            .min_height(5)
            .child(Text::new().content("POPUP")),
        dismissed: dismissed.clone(),
    });
    crate::open_popup(
        Popup::new(content as Box<dyn Widget>)
            .placement(Placement::center())
            .dismissible_if(false),
    );

    // One cycle drains the open queue and lays out / renders the centered card.
    term.update(&mut *root, &[]);
    assert!(
        term.get_snapshot_text().contains("POPUP"),
        "popup should be open before the click:\n{}",
        term.get_snapshot_text()
    );

    // Click the top-left corner, well outside the centered card.
    term.update(&mut *root, &[mouse_down(Vec2::new(0, 0))]);

    assert!(
        *dismissed.borrow(),
        "an outside click must deliver PopupDismissRequested to the popup content"
    );
    assert!(
        !term.get_snapshot_text().contains("POPUP"),
        "popup should be closed after the outside click:\n{}",
        term.get_snapshot_text()
    );
}

// ── schedule() targets inside popups ─────────────────────────────────────────

/// Popup content that arms a zero-delay timer on itself. Regression probe for
/// timers whose target widget lives in a popup, not the main tree.
struct TimerProbe {
    root: Box<Pane>,
    fired: Rc<RefCell<bool>>,
    armed: bool,
}

impl DelegateWidget for TimerProbe {
    crate::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        true
    }

    fn after_before_layout(&mut self) {
        if !self.armed {
            self.armed = true;
            crate::schedule(
                self.get_id(),
                std::time::Duration::from_millis(0),
                |probe: &mut TimerProbe| {
                    *probe.fired.borrow_mut() = true;
                },
            );
        }
    }
}

#[test]
fn scheduled_timer_fires_on_popup_hosted_widget() {
    // Regression: drain_task_queue resolved timer targets against the main
    // tree only, so a schedule() on a popup-hosted widget (e.g. a dialog
    // debouncing its search input) never fired. It must mirror drain_inbox's
    // root-then-popups resolution.
    let mut root = Pane::new().vertical().flex(1).child(Text::new().content("background"));
    let mut term = Emulator::new(&mut *root, Vec2::new(80, 24));

    let fired = Rc::new(RefCell::new(false));
    let content = Box::new(TimerProbe {
        root: Pane::new()
            .vertical()
            .min_width(20)
            .min_height(5)
            .child(Text::new().content("POPUP")),
        fired: fired.clone(),
        armed: false,
    });
    crate::open_popup(
        Popup::new(content as Box<dyn Widget>)
            .placement(Placement::center())
            .dismissible_if(false),
    );

    // First cycle opens the popup and arms the timer in before_layout; the
    // second drains the (already due) timer.
    term.update(&mut *root, &[]);
    term.update(&mut *root, &[]);

    assert!(
        *fired.borrow(),
        "a schedule() whose target lives in a popup must fire on that widget"
    );
}
