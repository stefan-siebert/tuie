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
        is_repeat: false,
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

// ── send() into a popup opened in the same tick ──────────────────────────────

struct Fill(&'static str);

/// Popup content that records the [`Fill`] messages it receives.
struct InboxProbe {
    root: Box<Pane>,
    received: Rc<RefCell<Vec<&'static str>>>,
}

impl DelegateWidget for InboxProbe {
    crate::delegate_widget!(root);

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        if let Some(Fill(text)) = event.take::<Fill>() {
            self.received.borrow_mut().push(text);
        }
    }
}

#[test]
fn a_message_sent_to_a_popup_opened_in_the_same_tick_arrives() {
    // Regression: open_popup only queues the popup, and the queue is drained
    // after the inbox. A host that opens a popup and forwards a message into
    // it in one tick — Elane's TUI opening a plugin dialog and passing on the
    // plugin's first update — had the message dropped: drain_inbox found
    // the target neither in the tree nor among the mounted popups.
    let mut root = Pane::new().vertical().flex(1).child(Text::new().content("background"));
    let mut term = Emulator::new(&mut *root, Vec2::new(80, 24));

    let received = Rc::new(RefCell::new(Vec::new()));
    let mut probe_id = WidgetId::EMPTY;
    let content = Box::new(InboxProbe {
        root: Pane::new().vertical().min_width(20).min_height(5),
        received: received.clone(),
    })
    .id(&mut probe_id);
    crate::open_popup(Popup::new(content as Box<dyn Widget>).dismissible_if(false));
    crate::send(probe_id, Fill("first"));

    term.update(&mut *root, &[]);
    crate::send(probe_id, Fill("second"));
    term.update(&mut *root, &[]);

    assert_eq!(*received.borrow(), ["first", "second"]);
}

// ── initial focus of a popup ─────────────────────────────────────────────────

/// A focusable leaf that records the keys it is given.
struct KeyProbe {
    root: Box<Pane>,
    name: &'static str,
    keys: Rc<RefCell<Vec<&'static str>>>,
}

impl DelegateWidget for KeyProbe {
    crate::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        true
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        if queue.next().is_some() {
            self.keys.borrow_mut().push(self.name);
            return InputResult::Handled;
        }
        InputResult::Rejected
    }
}

/// Popup content with two focusable probes; it asks for the second.
struct InitialFocusProbe {
    root: Box<Pane>,
    initial: WidgetId,
}

impl DelegateWidget for InitialFocusProbe {
    crate::delegate_widget!(root);

    fn override_get_initial_focus(&self) -> Option<WidgetId> {
        Some(self.initial)
    }
}

fn key(c: char) -> RuntimeEvent {
    RuntimeEvent::Input(InputEvent {
        chord: Chord::new(Trigger::Key(Key::Char(c)), Modifiers::new()),
        pos: Vec2::of(0.0),
        count: 1,
        is_repeat: false,
    })
}

#[test]
fn a_popup_mounts_with_its_initial_focus_before_its_first_layout() {
    // Regression: a dialog that focused its default button from
    // `after_before_layout` got it only at the layout after the first one;
    // a key in between went to the first focusable. Elane's TUI conflict
    // dialog: an Enter that came early toggled "apply to all" instead of
    // pressing Overwrite, and the dialog stayed open.
    let mut root = Pane::new().vertical().flex(1).child(Text::new().content("background"));
    let mut term = Emulator::new(&mut *root, Vec2::new(80, 24));

    let keys = Rc::new(RefCell::new(Vec::new()));
    let mut first_id = WidgetId::EMPTY;
    let mut second_id = WidgetId::EMPTY;
    let probe = |name| KeyProbe {
        root: Pane::new().min_width(4).min_height(1),
        name,
        keys: keys.clone(),
    };
    let content = Box::new(InitialFocusProbe {
        root: Pane::new()
            .vertical()
            .min_width(20)
            .min_height(5)
            .child(Box::new(probe("first")).id(&mut first_id))
            .child(Box::new(probe("second")).id(&mut second_id)),
        // Fields are evaluated in order: `root` above has set the id.
        initial: second_id.untyped(),
    });
    crate::open_popup(Popup::new(content as Box<dyn Widget>).dismissible_if(false));

    // Mounted at the end of the first update; the key comes at the start
    // of the second, before that update lays anything out.
    term.update(&mut *root, &[]);
    assert_eq!(crate::get_focused_widget(), Some(second_id.untyped()));
    term.update(&mut *root, &[key('x')]);

    assert_eq!(*keys.borrow(), ["second"]);
}

#[test]
fn a_popup_without_an_initial_focus_focuses_its_first_focusable() {
    let mut root = Pane::new().vertical().flex(1).child(Text::new().content("background"));
    let mut term = Emulator::new(&mut *root, Vec2::new(80, 24));

    let keys = Rc::new(RefCell::new(Vec::new()));
    let content = Pane::new()
        .vertical()
        .min_width(20)
        .min_height(5)
        .child(Box::new(KeyProbe {
            root: Pane::new().min_width(4).min_height(1),
            name: "first",
            keys: keys.clone(),
        }));
    crate::open_popup(Popup::new(content as Box<dyn Widget>).dismissible_if(false));

    term.update(&mut *root, &[]);
    term.update(&mut *root, &[key('x')]);

    assert_eq!(*keys.borrow(), ["first"]);
}
