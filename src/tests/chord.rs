//! Integration tests for chord parsing and the `chord!` proc macro.

use chord_macro::chord;

use tuie::prelude::*;

fn key(k: Key) -> Chord {
    Chord::new(Trigger::Key(k), Modifiers::new())
}

fn key_mods(k: Key, m: Modifiers) -> Chord {
    Chord::new(Trigger::Key(k), m)
}

#[test]
fn parser_single_literal_char() {
    let chords = Chord::parse_seq("a").unwrap();
    assert_eq!(chords, vec![key(Key::Char('a'))]);
}

#[test]
fn parser_uppercase_literal_is_char() {
    let chords = Chord::parse_seq("A").unwrap();
    assert_eq!(chords, vec![key(Key::Char('A'))]);
}

#[test]
fn parser_digits_and_punctuation() {
    let chords = Chord::parse_seq("0?").unwrap();
    assert_eq!(chords, vec![key(Key::Char('0')), key(Key::Char('?'))]);
}

#[test]
fn parser_ctrl_a() {
    let chords = Chord::parse_seq("<C-a>").unwrap();
    assert_eq!(chords, vec![key_mods(Key::Char('a'), Modifiers::new().with(Modifier::Ctrl))]);
}

#[test]
fn parser_modifier_letters_case_insensitive() {
    assert_eq!(Chord::parse_seq("<c-x>").unwrap(), Chord::parse_seq("<C-x>").unwrap());
    assert_eq!(Chord::parse_seq("<a-x>").unwrap(), Chord::parse_seq("<A-x>").unwrap());
    assert_eq!(Chord::parse_seq("<s-Up>").unwrap(), Chord::parse_seq("<S-Up>").unwrap());
    assert_eq!(Chord::parse_seq("<m-x>").unwrap(), Chord::parse_seq("<M-x>").unwrap());
    assert_eq!(Chord::parse_seq("<d-x>").unwrap(), Chord::parse_seq("<D-x>").unwrap());

    let c = &Chord::parse_seq("<C-x>").unwrap()[0];
    assert!(c.modifiers.has(Modifier::Ctrl));
    let a = &Chord::parse_seq("<A-x>").unwrap()[0];
    assert!(a.modifiers.has(Modifier::Alt));
    let s = &Chord::parse_seq("<S-Up>").unwrap()[0];
    assert!(s.modifiers.has(Modifier::Shift));
    let m = &Chord::parse_seq("<M-x>").unwrap()[0];
    assert!(m.modifiers.has(Modifier::Meta));
    let d = &Chord::parse_seq("<D-x>").unwrap()[0];
    assert!(d.modifiers.has(Modifier::Super));
}

#[test]
fn parser_shift_up() {
    let chords = Chord::parse_seq("<S-Up>").unwrap();
    assert_eq!(
        chords,
        vec![key_mods(Key::Arrow(Direction2D::Up), Modifiers::new().with(Modifier::Shift))],
    );
}

#[test]
fn parser_combined_modifiers() {
    let cs_a = &Chord::parse_seq("<C-S-a>").unwrap()[0];
    assert!(cs_a.modifiers.has(Modifier::Ctrl));
    assert!(cs_a.modifiers.has(Modifier::Shift));
    assert_eq!(cs_a.trigger, Trigger::Key(Key::Char('a')));

    let ca_del = &Chord::parse_seq("<C-A-Del>").unwrap()[0];
    assert!(ca_del.modifiers.has(Modifier::Ctrl));
    assert!(ca_del.modifiers.has(Modifier::Alt));
    assert_eq!(ca_del.trigger, Trigger::Key(Key::Delete));
}

#[test]
fn parser_named_keys() {
    assert_eq!(Chord::parse_seq("<Enter>").unwrap(), vec![key(Key::Enter)]);
    assert_eq!(Chord::parse_seq("<Esc>").unwrap(), vec![key(Key::Esc)]);
    assert_eq!(Chord::parse_seq("<Tab>").unwrap(), vec![key(Key::Tab)]);
    assert_eq!(Chord::parse_seq("<Space>").unwrap(), vec![key(Key::Char(' '))]);
    assert_eq!(Chord::parse_seq("<Backspace>").unwrap(), vec![key(Key::Backspace)]);
    assert_eq!(Chord::parse_seq("<Delete>").unwrap(), vec![key(Key::Delete)]);
    assert_eq!(Chord::parse_seq("<Insert>").unwrap(), vec![key(Key::Insert)]);
    assert_eq!(Chord::parse_seq("<Home>").unwrap(), vec![key(Key::Home)]);
    assert_eq!(Chord::parse_seq("<End>").unwrap(), vec![key(Key::End)]);
    assert_eq!(Chord::parse_seq("<PageUp>").unwrap(), vec![key(Key::PageUp)]);
    assert_eq!(Chord::parse_seq("<PageDown>").unwrap(), vec![key(Key::PageDown)]);
}

#[test]
fn parser_arrow_keys() {
    assert_eq!(Chord::parse_seq("<Up>").unwrap(), vec![key(Key::Arrow(Direction2D::Up))]);
    assert_eq!(Chord::parse_seq("<Down>").unwrap(), vec![key(Key::Arrow(Direction2D::Down))]);
    assert_eq!(Chord::parse_seq("<Left>").unwrap(), vec![key(Key::Arrow(Direction2D::Left))]);
    assert_eq!(Chord::parse_seq("<Right>").unwrap(), vec![key(Key::Arrow(Direction2D::Right))]);
}

#[test]
fn parser_named_keys_case_insensitive() {
    assert_eq!(Chord::parse_seq("<enter>").unwrap(), Chord::parse_seq("<Enter>").unwrap());
    assert_eq!(Chord::parse_seq("<ESC>").unwrap(), Chord::parse_seq("<Esc>").unwrap());
    assert_eq!(Chord::parse_seq("<pageup>").unwrap(), Chord::parse_seq("<PageUp>").unwrap());
}

#[test]
fn parser_aliased_named_keys() {
    assert_eq!(Chord::parse_seq("<CR>").unwrap(), vec![key(Key::Enter)]);
    assert_eq!(Chord::parse_seq("<BS>").unwrap(), vec![key(Key::Backspace)]);
    assert_eq!(Chord::parse_seq("<Del>").unwrap(), vec![key(Key::Delete)]);
    assert_eq!(Chord::parse_seq("<lt>").unwrap(), vec![key(Key::Char('<'))]);
    assert_eq!(Chord::parse_seq("<gt>").unwrap(), vec![key(Key::Char('>'))]);
}

#[test]
fn parser_function_keys_f1_through_f12() {
    for n in 1..=12u8 {
        let chords = Chord::parse_seq(&format!("<F{}>", n)).unwrap();
        assert_eq!(chords, vec![key(Key::F(n))], "F{} should parse", n);
    }
}

#[test]
fn parser_function_keys_cap_at_f12() {
    assert!(Chord::parse_seq("<F13>").is_err());
}

#[test]
fn parser_sequence_two_chords() {
    let chords = Chord::parse_seq("<C-x><C-c>").unwrap();
    assert_eq!(chords.len(), 2);
    assert_eq!(chords[0], key_mods(Key::Char('x'), Modifiers::new().with(Modifier::Ctrl)));
    assert_eq!(chords[1], key_mods(Key::Char('c'), Modifiers::new().with(Modifier::Ctrl)));
}

#[test]
fn parser_sequence_mixed_literal_and_spec() {
    let chords = Chord::parse_seq("a<Enter>b").unwrap();
    assert_eq!(
        chords,
        vec![key(Key::Char('a')), key(Key::Enter), key(Key::Char('b'))],
    );
}

#[test]
fn parser_unclosed_bracket_is_error() {
    assert!(Chord::parse_seq("<unclosed").is_err());
}

#[test]
fn parser_unknown_spec_is_error() {
    assert!(Chord::parse_seq("<bogus>").is_err());
}

#[test]
fn parser_empty_spec_is_error() {
    assert!(Chord::parse_seq("<>").is_err());
}

#[test]
fn parser_escaped_bracket_is_literal() {
    let chords = Chord::parse_seq("\\<C-a>").unwrap();
    assert_eq!(
        chords,
        vec![
            key(Key::Char('<')),
            key(Key::Char('C')),
            key(Key::Char('-')),
            key(Key::Char('a')),
            key(Key::Char('>')),
        ],
    );
}

#[test]
fn macro_matches_parser_for_ctrl_a() {
    let from_macro = chord!(Ctrl + a);
    let from_parser = Chord::parse_seq("<C-a>").unwrap().pop().unwrap();
    assert_eq!(from_macro, from_parser);
}

#[test]
fn macro_matches_parser_for_named_keys() {
    assert_eq!(chord!(Enter), Chord::parse_seq("<Enter>").unwrap().pop().unwrap());
    assert_eq!(chord!(Esc), Chord::parse_seq("<Esc>").unwrap().pop().unwrap());
    assert_eq!(chord!(Tab), Chord::parse_seq("<Tab>").unwrap().pop().unwrap());
    assert_eq!(chord!(Space), Chord::parse_seq("<Space>").unwrap().pop().unwrap());
    assert_eq!(chord!(Up), Chord::parse_seq("<Up>").unwrap().pop().unwrap());
    assert_eq!(chord!(PageDown), Chord::parse_seq("<PageDown>").unwrap().pop().unwrap());
}

#[test]
fn macro_function_keys() {
    assert_eq!(chord!(F1), Chord::parse_seq("<F1>").unwrap().pop().unwrap());
    assert_eq!(chord!(F12), Chord::parse_seq("<F12>").unwrap().pop().unwrap());
    assert_eq!(chord!(F1).trigger, Trigger::Key(Key::F(1)));
    assert_eq!(chord!(F12).trigger, Trigger::Key(Key::F(12)));
}

#[test]
fn macro_combined_modifiers() {
    let m = chord!(Ctrl + Alt + Delete);
    let p = Chord::parse_seq("<C-A-Del>").unwrap().pop().unwrap();
    assert_eq!(m, p);
}

#[test]
fn macro_mouse_chords() {
    assert_eq!(
        chord!(LeftClick),
        Chord::new(Trigger::MouseDown(MouseButton::Left), Modifiers::new()),
    );
    assert_eq!(
        chord!(RightClick),
        Chord::new(Trigger::MouseDown(MouseButton::Right), Modifiers::new()),
    );
    assert_eq!(
        chord!(LeftDrag),
        Chord::new(Trigger::MouseDrag(MouseButton::Left), Modifiers::new()),
    );
    assert_eq!(
        chord!(LeftRelease),
        Chord::new(Trigger::MouseUp(MouseButton::Left), Modifiers::new()),
    );
}

#[test]
fn parser_does_not_recognise_mouse() {
    assert!(Chord::parse_seq("<LeftClick>").is_err());
}

#[test]
fn divergence_shift_with_char_parser_allows_macro_forbids() {
    let p = Chord::parse_seq("<S-a>").unwrap().pop().unwrap();
    assert!(p.modifiers.has(Modifier::Shift));
    assert_eq!(p.trigger, Trigger::Key(Key::Char('a')));

    let m = chord!(A);
    assert_eq!(m.trigger, Trigger::Key(Key::Char('A')));
    assert!(!m.modifiers.has(Modifier::Shift));

    assert_ne!(p, m);
}

#[test]
fn divergence_macro_has_no_meta_or_hyper() {
    let p = Chord::parse_seq("<M-a>").unwrap().pop().unwrap();
    assert!(p.modifiers.has(Modifier::Meta));
}
