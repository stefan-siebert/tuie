//! Integration tests for the inspector hooks (`tuie::inspect`).

use std::cell::RefCell;
use std::rc::Rc;

use chord_macro::chord;
use tuie::emulator::Emulator;
use tuie::inspect::{self, Injected};
use tuie::prelude::*;

/// What the root saw: each key with its `is_repeat`.
type Seen = Rc<RefCell<Vec<(String, bool)>>>;

/// Root that records every input event and shows a text line.
struct Recorder {
    inner: Box<Text>,
    seen: Seen,
}

impl Recorder {
    fn new(seen: Seen) -> Self {
        Self {
            inner: Text::new().content("before"),
            seen,
        }
    }
}

impl DelegateWidget for Recorder {
    tuie::delegate_widget!(inner);

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };
        self.seen
            .borrow_mut()
            .push((event.chord.to_string(), event.is_repeat));
        self.inner.set_content("after");
        InputResult::Handled
    }
}

#[test]
fn injected_key_reaches_the_widget_like_a_typed_one() {
    let seen = Seen::default();
    let mut root = Recorder::new(seen.clone());
    let mut term = Emulator::new(&mut root, Vec2::new(10, 1));

    inspect::inject([Injected::Key(chord!(a))]);
    assert!(inspect::has_pending_injection());
    assert!(!inspect::is_settled(), "a queued injection is owed work");

    term.pump_injected(&mut root);
    assert_eq!(*seen.borrow(), vec![(chord!(a).to_string(), false)]);
    assert!(inspect::is_settled());
}

#[test]
fn after_update_sees_the_frame_the_input_produced() {
    let seen = Seen::default();
    let mut root = Recorder::new(seen);
    let mut term = Emulator::new(&mut root, Vec2::new(10, 1));
    assert!(term.get_snapshot_text().starts_with("before"));

    let observed: Rc<RefCell<Option<String>>> = Rc::default();
    inspect::inject([Injected::Key(chord!(a))]);
    inspect::after_update({
        let observed = observed.clone();
        move |_root| *observed.borrow_mut() = Some(inspect::screen().into_string())
    });
    term.pump_injected(&mut root);

    let observed = observed.borrow().clone().unwrap_or_default();
    assert!(observed.starts_with("after"), "callback saw {observed:?}");
}

#[test]
fn a_callback_registered_by_a_callback_runs_one_tick_later() {
    let seen = Seen::default();
    let mut root = Recorder::new(seen);
    let mut term = Emulator::new(&mut root, Vec2::new(10, 1));

    let ticks: Rc<RefCell<Vec<&str>>> = Rc::default();
    inspect::after_update({
        let ticks = ticks.clone();
        move |_root| {
            ticks.borrow_mut().push("first");
            inspect::after_update(move |_root| ticks.borrow_mut().push("second"));
        }
    });
    term.pump_injected(&mut root);
    assert_eq!(*ticks.borrow(), vec!["first"]);
    term.pump_injected(&mut root);
    assert_eq!(*ticks.borrow(), vec!["first", "second"]);
}

#[test]
fn quickly_repeated_injection_is_inferred_as_a_repeat() {
    let seen = Seen::default();
    let mut root = Recorder::new(seen.clone());
    let mut term = Emulator::new(&mut root, Vec2::new(10, 1));
    term.set_infer_repeats(true);

    // Back to back: well inside INFERRED_REPEAT_WINDOW.
    inspect::inject([Injected::Key(chord!(Esc))]);
    term.pump_injected(&mut root);
    inspect::inject([Injected::Key(chord!(Esc))]);
    term.pump_injected(&mut root);
    // After the window: a fresh press again.
    std::thread::sleep(tuie::runtime::INFERRED_REPEAT_WINDOW * 2);
    inspect::inject([Injected::Key(chord!(Esc))]);
    term.pump_injected(&mut root);

    let repeats: Vec<bool> = seen.borrow().iter().map(|(_, repeat)| *repeat).collect();
    assert_eq!(repeats, vec![false, true, false]);
}

#[test]
fn reported_repeat_is_marked_without_inference() {
    let seen = Seen::default();
    let mut root = Recorder::new(seen.clone());
    let mut term = Emulator::new(&mut root, Vec2::new(10, 1));

    inspect::inject([Injected::Key(chord!(Esc)), Injected::KeyRepeat(chord!(Esc))]);
    term.pump_injected(&mut root);

    let repeats: Vec<bool> = seen.borrow().iter().map(|(_, repeat)| *repeat).collect();
    assert_eq!(repeats, vec![false, true]);
}

/// Popup content that can hold focus, like an Elane dialog.
struct FocusableCard {
    root: Box<Pane>,
}

impl DelegateWidget for FocusableCard {
    tuie::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        true
    }
}

#[test]
fn focus_chain_says_when_a_popup_has_the_focus() {
    let seen = Seen::default();
    let mut root = Recorder::new(seen);
    let mut term = Emulator::new(&mut root, Vec2::new(40, 10));
    assert!(!inspect::focus_chain(&root).in_popup);
    assert_eq!(inspect::open_popups(), Some(0));

    let card = Box::new(FocusableCard {
        root: Pane::new().vertical().min_width(10).min_height(3),
    });
    let card_id = card.get_id();
    tuie::open_popup(Popup::new(card as Box<dyn Widget>).placement(Placement::center()));
    term.update(&mut root, &[]);
    tuie::focus_widget(card_id);
    term.update(&mut root, &[]);

    assert_eq!(inspect::open_popups(), Some(1));
    let chain = inspect::focus_chain(&root);
    assert!(chain.in_popup, "focus is in the card: {chain:?}");
    assert_eq!(chain.names.first().copied(), Some("FocusableCard"));
}

fn find<'a>(
    elements: &'a [inspect::ElementInfo],
    test_id: &str,
) -> Option<&'a inspect::ElementInfo> {
    elements.iter().find_map(|element| {
        if element.test_id.as_deref() == Some(test_id) {
            Some(element)
        } else {
            find(&element.children, test_id)
        }
    })
}

#[test]
fn element_tree_carries_test_ids_rects_and_text() {
    let mut root = Pane::new().vertical().children([
        Text::new().content("first row").test_id("row.first") as Box<dyn Widget>,
        Text::new().content("second").test_id("row.second") as Box<dyn Widget>,
    ]);
    let _term = Emulator::new(&mut *root, Vec2::new(20, 4));

    let tree = inspect::element_tree(&*root);
    let second = find(&tree, "row.second").unwrap_or_else(|| panic!("no row.second in {tree:?}"));
    assert_eq!(second.rect.pos.y, 1);
    assert_eq!(second.popup, None);
    let text = inspect::text_in(second.rect);
    assert_eq!(text.first().map(|row| row.trim_end()), Some("second"));
}

#[test]
fn popups_follow_the_main_tree_in_the_element_tree() {
    let mut root = Pane::new()
        .vertical()
        .child(Text::new().content("background"));
    let mut term = Emulator::new(&mut *root, Vec2::new(40, 10));
    let card = Pane::new()
        .vertical()
        .min_width(10)
        .min_height(3)
        .test_id("card")
        .child(Text::new().content("CARD"));
    tuie::open_popup(Popup::new(card as Box<dyn Widget>).placement(Placement::center()));
    term.update(&mut *root, &[]);

    let tree = inspect::element_tree(&*root);
    assert_eq!(tree.len(), 2, "main tree + one popup: {tree:?}");
    let card = find(&tree, "card").unwrap_or_else(|| panic!("no card in {tree:?}"));
    assert_eq!(card.popup, Some(0));
    assert!(
        inspect::text_in(card.rect)
            .iter()
            .any(|row| row.contains("CARD"))
    );
}

#[test]
fn text_in_counts_a_wide_glyph_as_two_cells() {
    let mut root = Pane::new().vertical().child(Text::new().content("ab界cd"));
    let _term = Emulator::new(&mut *root, Vec2::new(10, 1));
    // Cells: a b 界 界 c d — the rect from cell 4 starts at "c".
    let tail = inspect::text_in(Rect::new(Vec2::new(4, 0), Vec2::new(2, 1)));
    assert_eq!(tail, vec!["cd".to_string()]);
    let whole = inspect::text_in(Rect::new(Vec2::new(0, 0), Vec2::new(6, 1)));
    assert_eq!(whole, vec!["ab界cd".to_string()]);
}
