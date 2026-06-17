//! Integration tests for focus navigation.

use tuie::prelude::*;
use tuie::emulator::Emulator;
use chord_macro::chord;

/// Root wrapper that requests tab-order focus on unhandled Tab.
struct TabHandler {
    inner: Box<dyn Widget>,
}

impl DelegateWidget for TabHandler {
    tuie::delegate_widget!(inner);

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.peek() else {
            return InputResult::Rejected;
        };
        match &event.chord {
            chord!(Tab) if queue.is_unhandled() => {
                queue.next();
                tuie::focus_next_tab_order(Sign::Positive);
                InputResult::Handled
            }
            _ => InputResult::Rejected,
        }
    }
}

#[test]
fn unhandled_tab_focuses_first_widget_when_nothing_focused() {
    let mut root = TabHandler {
        inner: Pane::new().vertical().children([Input::new(), Input::new()]),
    };
    let mut term = Emulator::new(&mut root, Vec2::new(10, 2));
    assert_eq!(tuie::get_focused_widget(), None);
    term.update(&mut root, &[chord!(Tab).into()]);
    assert!(tuie::get_focused_widget().is_some());
}
