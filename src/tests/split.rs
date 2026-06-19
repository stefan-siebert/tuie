use chord_macro::chord;
use tuie::prelude::*;
use tuie::emulator::Emulator;

fn pane_with_text(s: &str) -> Box<Pane> {
    Pane::new().children([Text::new().content(s.to_string())])
}

#[test]
fn renders_two_pane_horizontal_split() {
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(pane_with_text("L")),
            SplitPaneChild::from(pane_with_text("R")),
        ]),
    );
    let term = Emulator::new(&mut *split, Vec2::new(10, 3));
    let snap = term.get_snapshot_text();
    let rows: Vec<&str> = snap.split('\n').collect();
    assert_eq!(rows.len(), 3);
    for row in &rows {
        assert_eq!(row.chars().count(), 10);
    }
    for row in &rows {
        assert!(row.contains('│'), "vertical divider on every row: {:?}", row);
    }
    assert!(snap.contains('L'), "left pane content: {:?}", snap);
    assert!(snap.contains('R'), "right pane content: {:?}", snap);
}

#[test]
fn renders_two_pane_vertical_split() {
    let mut split = Split::new(
        SplitPane::new().children([
            SplitPaneChild::from(pane_with_text("top")),
            SplitPaneChild::from(pane_with_text("bot")),
        ]),
    );
    let term = Emulator::new(&mut *split, Vec2::new(10, 8));
    let snap = term.get_snapshot_text();
    let rows: Vec<&str> = snap.split('\n').collect();
    assert_eq!(rows.len(), 8);
    assert!(snap.contains("top"), "top pane content: {:?}", snap);
    assert!(snap.contains("bot"), "bottom pane content: {:?}", snap);
    let any_horizontal_divider = rows.iter().any(|r| r.chars().filter(|&c| c == '─').count() >= 4);
    assert!(any_horizontal_divider, "expected at least one horizontal divider row, got {:?}", rows);
}

#[test]
fn renders_three_pane_horizontal_split() {
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(pane_with_text("A")),
            SplitPaneChild::from(pane_with_text("B")),
            SplitPaneChild::from(pane_with_text("C")),
        ]),
    );
    let term = Emulator::new(&mut *split, Vec2::new(15, 3));
    let snap = term.get_snapshot_text();
    assert!(snap.contains('A'));
    assert!(snap.contains('B'));
    assert!(snap.contains('C'));
    for row in snap.split('\n') {
        let dividers = row.chars().filter(|&c| c == '│').count();
        assert_eq!(dividers, 2, "two vertical dividers between three panes: {:?}", row);
    }
}

#[test]
fn flex_ratio_distributes_widths() {
    let left = Pane::new().flex(1).children([Text::new().content("L")]);
    let right = Pane::new().flex(3).children([Text::new().content("R")]);
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(left),
            SplitPaneChild::from(right),
        ]),
    );
    let term = Emulator::new(&mut *split, Vec2::new(20, 3));
    let top = term.get_snapshot_text().split('\n').next().unwrap().to_string();
    let pivot_col = top.chars().position(|c| c == '│').expect("vertical divider on top row");
    let left_cells = pivot_col;
    let right_cells = top.chars().count() - pivot_col - 1;
    assert!(
        right_cells > left_cells * 2,
        "right (flex=3) should be much wider than left (flex=1), got L={} R={} top={:?}",
        left_cells,
        right_cells,
        top
    );
}

#[test]
fn nested_splits_render() {
    let inner = SplitPane::new().children([
        SplitPaneChild::from(pane_with_text("TR")),
        SplitPaneChild::from(pane_with_text("BR")),
    ]);
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(pane_with_text("L")),
            SplitPaneChild::from(inner),
        ]),
    );
    let term = Emulator::new(&mut *split, Vec2::new(14, 7));
    let snap = term.get_snapshot_text();
    assert!(snap.contains('L'));
    assert!(snap.contains("TR"));
    assert!(snap.contains("BR"));
    let any_t_left = snap.chars().any(|c| c == '├');
    assert!(any_t_left, "expected ├ junction where inner vertical divider meets outer vertical divider: {:?}", snap);
}

#[test]
fn resize_reflows_split() {
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(pane_with_text("L")),
            SplitPaneChild::from(pane_with_text("R")),
        ]),
    );
    let mut term = Emulator::new(&mut *split, Vec2::new(10, 3));
    let small_top = term.get_snapshot_text().split('\n').next().unwrap().to_string();
    assert_eq!(small_top.chars().count(), 10);

    term.update(&mut *split, &[RuntimeEvent::Resize(Vec2::new(30, 5))]);
    let big_snap = term.get_snapshot_text();
    let big_rows: Vec<&str> = big_snap.split('\n').collect();
    assert_eq!(big_rows.len(), 5);
    for row in &big_rows {
        assert_eq!(row.chars().count(), 30);
    }
    assert!(big_snap.contains('L'));
    assert!(big_snap.contains('R'));
}

#[test]
fn mouse_drag_moves_divider() {
    let left = Pane::new().flex(1).children([Text::new().content("L")]);
    let right = Pane::new().flex(1).children([Text::new().content("R")]);
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(left),
            SplitPaneChild::from(right),
        ]),
    );
    let mut term = Emulator::new(&mut *split, Vec2::new(20, 5));
    let before_top = term.get_snapshot_text().split('\n').next().unwrap().to_string();
    let before_pivot = before_top.chars().position(|c| c == '│').expect("vertical divider on top row") as i32;

    term.update(&mut *split, &[
        RuntimeEvent::input_at(chord!(LeftClick), Vec2::new(before_pivot, 2)),
        RuntimeEvent::input_at(chord!(LeftDrag), Vec2::new(before_pivot + 4, 2)),
        RuntimeEvent::input_at(chord!(LeftRelease), Vec2::new(before_pivot + 4, 2)),
    ]);

    let after_top = term.get_snapshot_text().split('\n').next().unwrap().to_string();
    let after_pivot = after_top
        .chars()
        .enumerate()
        .filter_map(|(i, c)| (c == '│').then_some(i))
        .last()
        .expect("vertical divider still on top row") as i32;
    assert!(
        after_pivot > before_pivot,
        "divider should have moved right after drag, before={} after={} top={:?}",
        before_pivot,
        after_pivot,
        after_top
    );
}

#[test]
fn outer_border_wraps_split() {
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(pane_with_text("L")).borderless(),
            SplitPaneChild::from(pane_with_text("R")).borderless(),
        ]),
    )
    .bordered()
    .border(Border::SINGLE);
    let term = Emulator::new(&mut *split, Vec2::new(10, 3));
    term.assert_lines([
        "┌────────┐",
        "│LR      │",
        "└────────┘",
    ]);
}

#[test]
fn minimum_width_constraint_respected() {
    let left = Pane::new().min_width(8).children([Text::new().content("L")]);
    let right = Pane::new().min_width(8).children([Text::new().content("R")]);
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(left).borderless(),
            SplitPaneChild::from(right).borderless(),
        ]),
    );
    let constraints = split.measure_constraints();
    assert!(
        constraints.min_size.x >= 16,
        "min width should sum the two min_width=8 panes, got {}",
        constraints.min_size.x,
    );
}

#[test]
fn remove_collapses_pane() {
    let left = pane_with_text("L");
    let mid = pane_with_text("M");
    let right = pane_with_text("R");
    let left_id = left.get_id();
    let mid_id = mid.get_id();
    let right_id = right.get_id();
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(left),
            SplitPaneChild::from(mid),
            SplitPaneChild::from(right),
        ]),
    );
    let term = Emulator::new(&mut *split, Vec2::new(15, 3));
    let before = term.get_snapshot_text();
    assert!(before.contains('M'));
    assert!(split.contains(mid_id));

    let removed = split.remove(mid_id);
    assert!(removed.is_some());
    assert!(!split.contains(mid_id));
    assert!(split.contains(left_id));
    assert!(split.contains(right_id));

    let term = Emulator::new(&mut *split, Vec2::new(15, 3));
    let after = term.get_snapshot_text();
    assert!(after.contains('L'));
    assert!(after.contains('R'));
    assert!(!after.contains('M'), "removed pane should not render: {:?}", after);
    let top = after.split('\n').next().unwrap();
    let dividers = top.chars().filter(|&c| c == '│').count();
    assert_eq!(dividers, 1, "exactly one vertical divider left after removing middle: {:?}", top);
}

#[test]
fn split_root_adds_pane_at_runtime() {
    let first = pane_with_text("1");
    let mut split = Split::new(
        SplitPane::new().horizontal().children([SplitPaneChild::from(first)]),
    );
    let mut term = Emulator::new(&mut *split, Vec2::new(20, 3));
    assert!(term.get_snapshot_text().contains('1'));

    split.split_root(
        SplitPaneChild::from(pane_with_text("2")),
        Axis2D::X,
        Sign::Positive,
    );
    split.redistribute();
    term.update(&mut *split, &[RuntimeEvent::Resize(Vec2::new(20, 3))]);
    let after = term.get_snapshot_text();
    assert!(after.contains('1'), "old pane content after split_root: {:?}", after);
    assert!(after.contains('2'), "new pane content after split_root: {:?}", after);
    let top = after.split('\n').next().unwrap();
    assert!(top.contains('│'), "vertical divider appears after adding second pane: {:?}", top);
}

/// Two equal-flex panes with the resize tooltip + snap behaviour enabled.
fn feedback_split() -> Box<Split> {
    let left = Pane::new().flex(1).children([Text::new().content("L")]);
    let right = Pane::new().flex(1).children([Text::new().content("R")]);
    Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(left),
            SplitPaneChild::from(right),
        ]),
    )
    .resize_feedback(ResizeFeedback {
        normal_style: Style::new(),
        snap_style: Style::new(),
        hover_style: Style::new().fg(Color::CYAN).bold(),
        snap_threshold_percent: 3.0,
    })
}

/// Column of the (single) vertical divider on the top row.
fn top_pivot(term: &Emulator) -> i32 {
    let top = term.get_snapshot_text().split('\n').next().unwrap().to_string();
    top.chars()
        .enumerate()
        .filter_map(|(i, c)| (c == '│').then_some(i))
        .last()
        .expect("vertical divider on top row") as i32
}

#[test]
fn resize_tooltip_shows_and_clears() {
    let mut split = feedback_split();
    let mut term = Emulator::new(&mut *split, Vec2::new(100, 5));
    let pivot = top_pivot(&term);
    assert!(
        !term.get_snapshot_text().contains('%'),
        "no tooltip before a drag starts"
    );

    // Mid-drag: the live percentage tooltip is painted.
    term.update(&mut *split, &[
        RuntimeEvent::input_at(chord!(LeftClick), Vec2::new(pivot, 2)),
        RuntimeEvent::input_at(chord!(LeftDrag), Vec2::new(pivot + 10, 2)),
    ]);
    assert!(
        term.get_snapshot_text().contains('%'),
        "tooltip shows the split percentage while dragging: {:?}",
        term.get_snapshot_text()
    );

    // After release the tooltip disappears.
    term.update(&mut *split, &[
        RuntimeEvent::input_at(chord!(LeftRelease), Vec2::new(pivot + 10, 2)),
    ]);
    assert!(
        !term.get_snapshot_text().contains('%'),
        "tooltip clears on release: {:?}",
        term.get_snapshot_text()
    );
}

#[test]
fn release_snaps_divider_to_center_when_near() {
    let mut split = feedback_split();
    let mut term = Emulator::new(&mut *split, Vec2::new(200, 5));
    let before = top_pivot(&term);

    // Nudge the divider two cells (well inside the 3% snap radius of ~3 cells).
    term.update(&mut *split, &[
        RuntimeEvent::input_at(chord!(LeftClick), Vec2::new(before, 2)),
        RuntimeEvent::input_at(chord!(LeftDrag), Vec2::new(before + 2, 2)),
        RuntimeEvent::input_at(chord!(LeftRelease), Vec2::new(before + 2, 2)),
    ]);

    let after = top_pivot(&term);
    assert!(
        (after - before).abs() <= 1,
        "small nudge should snap back to centre, before={before} after={after}"
    );
}

#[test]
fn release_keeps_divider_when_far_from_center() {
    let mut split = feedback_split();
    let mut term = Emulator::new(&mut *split, Vec2::new(200, 5));
    let before = top_pivot(&term);

    // Drag well outside the snap radius — the divider stays where it lands.
    term.update(&mut *split, &[
        RuntimeEvent::input_at(chord!(LeftClick), Vec2::new(before, 2)),
        RuntimeEvent::input_at(chord!(LeftDrag), Vec2::new(before + 12, 2)),
        RuntimeEvent::input_at(chord!(LeftRelease), Vec2::new(before + 12, 2)),
    ]);

    let after = top_pivot(&term);
    assert!(
        after - before >= 6,
        "far drag should not snap back, before={before} after={after}"
    );
}

#[test]
fn double_click_resets_divider_to_center() {
    let mut split = feedback_split();
    let mut term = Emulator::new(&mut *split, Vec2::new(200, 5));
    let before = top_pivot(&term);

    // Drag far off-centre and release (no snap).
    term.update(&mut *split, &[
        RuntimeEvent::input_at(chord!(LeftClick), Vec2::new(before, 2)),
        RuntimeEvent::input_at(chord!(LeftDrag), Vec2::new(before + 18, 2)),
        RuntimeEvent::input_at(chord!(LeftRelease), Vec2::new(before + 18, 2)),
    ]);
    let moved = top_pivot(&term);
    assert!(moved - before >= 6, "drag moved the divider, before={before} moved={moved}");

    // Double-click on the divider snaps it back to its flex default.
    term.update(&mut *split, &[
        RuntimeEvent::input_at_count(chord!(LeftClick), Vec2::new(moved, 2), 2),
    ]);
    let after = top_pivot(&term);
    assert!(
        after < moved && (after - before).abs() <= 1,
        "double-click resets to centre, before={before} moved={moved} after={after}"
    );
}

#[test]
fn no_feedback_means_no_tooltip() {
    // Without resize_feedback the divider still drags, but no tooltip is drawn.
    let left = Pane::new().flex(1).children([Text::new().content("L")]);
    let right = Pane::new().flex(1).children([Text::new().content("R")]);
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(left),
            SplitPaneChild::from(right),
        ]),
    );
    let mut term = Emulator::new(&mut *split, Vec2::new(100, 5));
    let pivot = top_pivot(&term);
    term.update(&mut *split, &[
        RuntimeEvent::input_at(chord!(LeftClick), Vec2::new(pivot, 2)),
        RuntimeEvent::input_at(chord!(LeftDrag), Vec2::new(pivot + 10, 2)),
    ]);
    assert!(
        !term.get_snapshot_text().contains('%'),
        "no tooltip when resize_feedback is disabled: {:?}",
        term.get_snapshot_text()
    );
}

#[test]
fn divider_hover_requests_resize_pointer() {
    // The mouse-pointer hint must work even without resize_feedback: dividers
    // are draggable by default, so hovering one should advertise a resize cursor.
    let left = Pane::new().flex(1).children([Text::new().content("L")]);
    let right = Pane::new().flex(1).children([Text::new().content("R")]);
    let mut split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(left),
            SplitPaneChild::from(right),
        ]),
    );
    let term = Emulator::new(&mut *split, Vec2::new(40, 5));
    let pivot = top_pivot(&term);
    assert_eq!(
        split.get_mouse_pointer_shape(Vec2::new(pivot, 2)),
        Some(MousePointerShape::ColResize),
        "vertical divider → horizontal resize pointer"
    );
    assert_eq!(
        split.get_mouse_pointer_shape(Vec2::new(pivot + 5, 2)),
        None,
        "over a pane → no special pointer"
    );
}

#[test]
fn horizontal_divider_requests_row_resize_pointer() {
    let mut split = Split::new(
        SplitPane::new().children([
            SplitPaneChild::from(pane_with_text("top")),
            SplitPaneChild::from(pane_with_text("bot")),
        ]),
    );
    let term = Emulator::new(&mut *split, Vec2::new(12, 9));
    let snap = term.get_snapshot_text();
    let divider_row = snap
        .split('\n')
        .position(|r| r.chars().filter(|&c| c == '─').count() >= 4)
        .expect("horizontal divider row") as i32;
    assert_eq!(
        split.get_mouse_pointer_shape(Vec2::new(2, divider_row)),
        Some(MousePointerShape::RowResize),
        "horizontal divider → vertical resize pointer"
    );
}

#[test]
fn hovering_divider_highlights_it() {
    // Mirror the real consumer tree: the split is a *child* of a root pane (so
    // hit-testing puts the split in the hover path), bordered, with two bordered
    // panes whose merged border is the draggable divider.
    let left = Pane::new().flex(1).children([Text::new().content("L")]);
    let right = Pane::new().flex(1).children([Text::new().content("R")]);
    let split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(left).border(Border::SINGLE),
            SplitPaneChild::from(right).border(Border::SINGLE),
        ]),
    )
    .bordered()
    .flex(1)
    .resize_feedback(ResizeFeedback {
        normal_style: Style::new(),
        snap_style: Style::new(),
        hover_style: Style::new().fg(Color::CYAN).bold(),
        snap_threshold_percent: 3.0,
    });
    let mut root = Pane::new().vertical().flex(1).children([split]);
    let mut term = Emulator::new(&mut *root, Vec2::new(40, 7));

    let highlighted = |term: &Emulator| {
        term.get_snapshot()
            .iter_chunks(..)
            .any(|(t, s)| t.contains('│') && s.get_fg() == Some(Color::CYAN))
    };
    assert!(!highlighted(&term), "no highlight before hovering");

    // Sweep the interior columns; hovering the divider tints it.
    let mut found = false;
    for x in 2..38 {
        term.update(&mut *root, &[RuntimeEvent::input_at(chord!(Hover), Vec2::new(x, 3))]);
        if highlighted(&term) {
            found = true;
            break;
        }
    }
    assert!(found, "hovering the divider should highlight it:\n{}", term.get_snapshot_text());
}

#[test]
fn hovering_divider_emits_osc22_resize_cursor() {
    use std::sync::{Arc, Mutex};
    struct Cap(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Cap {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let left = Pane::new().flex(1).children([Text::new().content("L")]);
    let right = Pane::new().flex(1).children([Text::new().content("R")]);
    let split = Split::new(
        SplitPane::new().horizontal().children([
            SplitPaneChild::from(left).border(Border::SINGLE),
            SplitPaneChild::from(right).border(Border::SINGLE),
        ]),
    )
    .bordered()
    .flex(1)
    .resize_feedback(ResizeFeedback {
        normal_style: Style::new(),
        snap_style: Style::new(),
        hover_style: Style::new().fg(Color::CYAN).bold(),
        snap_threshold_percent: 3.0,
    });
    let mut root = Pane::new().vertical().flex(1).children([split]);
    let mut term = Emulator::new(&mut *root, Vec2::new(40, 7));

    let cap = Arc::new(Mutex::new(Vec::new()));
    tuie::set_output(Cap(cap.clone()));

    let mut emitted = false;
    for x in 18..23 {
        term.update(&mut *root, &[RuntimeEvent::input_at(chord!(Hover), Vec2::new(x, 3))]);
        // OSC 22 with the resize shape: `ESC ] 22 ; ew-resize ST`.
        if String::from_utf8_lossy(&cap.lock().unwrap()).contains("\x1b]22;ew-resize") {
            emitted = true;
            break;
        }
    }
    assert!(
        emitted,
        "expected OSC 22 ew-resize on divider hover; captured: {:?}",
        String::from_utf8_lossy(&cap.lock().unwrap())
    );
}

#[test]
fn dragging_keeps_resize_pointer_off_divider() {
    let mut split = feedback_split();
    let mut term = Emulator::new(&mut *split, Vec2::new(40, 5));
    let pivot = top_pivot(&term);
    term.update(&mut *split, &[
        RuntimeEvent::input_at(chord!(LeftClick), Vec2::new(pivot, 2)),
        RuntimeEvent::input_at(chord!(LeftDrag), Vec2::new(pivot + 3, 2)),
    ]);
    // Mid-drag the resize pointer sticks even where the pointer is nowhere near
    // the divider line.
    assert_eq!(
        split.get_mouse_pointer_shape(Vec2::new(0, 0)),
        Some(MousePointerShape::ColResize),
        "resize pointer persists during an active drag"
    );
}
