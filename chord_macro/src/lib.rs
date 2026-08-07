//! Macro that expands human readable syntax into `tuie::prelude::Chord` values, usable as both constructors and match patterns.

extern crate proc_macro;
use quote::quote;
use syn::{self, parse_macro_input, spanned::Spanned};

macro_rules! fail {
    ($where:expr, $($args:expr),*) => {
        Err(syn::Error::new(syn::spanned::Spanned::span($where), format!($($args),*)).to_compile_error())
    };
}

enum Key {
    Ident(&'static str),
    Arrow(&'static str),
    F(u8),
    Char(char),
    CharIdent(char),
}

enum Trigger {
    Key(Key),
    MouseScroll(&'static str),
    Mouse(&'static str, &'static str),
    MouseHover,
}

fn parse_trigger_ident(from: &dyn Spanned, ident: &str) -> Result<Trigger, proc_macro2::TokenStream> {
    Ok(match ident {
        "Backspace" => Trigger::Key(Key::Ident("Backspace")),
        "Enter" => Trigger::Key(Key::Ident("Enter")),
        "Home" => Trigger::Key(Key::Ident("Home")),
        "End" => Trigger::Key(Key::Ident("End")),
        "PageUp" => Trigger::Key(Key::Ident("PageUp")),
        "PageDown" => Trigger::Key(Key::Ident("PageDown")),
        "Tab" => Trigger::Key(Key::Ident("Tab")),
        "Delete" => Trigger::Key(Key::Ident("Delete")),
        "Insert" => Trigger::Key(Key::Ident("Insert")),
        "Esc" => Trigger::Key(Key::Ident("Esc")),

        "Up" => Trigger::Key(Key::Arrow("Up")),
        "Down" => Trigger::Key(Key::Arrow("Down")),
        "Left" => Trigger::Key(Key::Arrow("Left")),
        "Right" => Trigger::Key(Key::Arrow("Right")),

        "F1" => Trigger::Key(Key::F(1)),
        "F2" => Trigger::Key(Key::F(2)),
        "F3" => Trigger::Key(Key::F(3)),
        "F4" => Trigger::Key(Key::F(4)),
        "F5" => Trigger::Key(Key::F(5)),
        "F6" => Trigger::Key(Key::F(6)),
        "F7" => Trigger::Key(Key::F(7)),
        "F8" => Trigger::Key(Key::F(8)),
        "F9" => Trigger::Key(Key::F(9)),
        "F10" => Trigger::Key(Key::F(10)),
        "F11" => Trigger::Key(Key::F(11)),
        "F12" => Trigger::Key(Key::F(12)),

        s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic() => {
            Trigger::Key(Key::CharIdent(s.as_bytes()[0] as char))
        }

        "Space" => Trigger::Key(Key::Char(' ')),

        "LeftClick" | "LeftMouseDown" => Trigger::Mouse("MouseDown", "Left"),
        "MiddleClick" | "MiddleMouseDown" => Trigger::Mouse("MouseDown", "Middle"),
        "RightClick" | "RightMouseDown" => Trigger::Mouse("MouseDown", "Right"),

        "LeftDrag" | "LeftMouseDrag" => Trigger::Mouse("MouseDrag", "Left"),
        "MiddleDrag" | "MiddleMouseDrag" => Trigger::Mouse("MouseDrag", "Middle"),
        "RightDrag" | "RightMouseDrag" => Trigger::Mouse("MouseDrag", "Right"),

        "LeftRelease" | "LeftMouseUp" => Trigger::Mouse("MouseUp", "Left"),
        "MiddleRelease" | "MiddleMouseUp" => Trigger::Mouse("MouseUp", "Middle"),
        "RightRelease" | "RightMouseUp" => Trigger::Mouse("MouseUp", "Right"),

        "ScrollUp" => Trigger::MouseScroll("Up"),
        "ScrollDown" => Trigger::MouseScroll("Down"),
        "ScrollLeft" => Trigger::MouseScroll("Left"),
        "ScrollRight" => Trigger::MouseScroll("Right"),

        "Hover" => Trigger::MouseHover,

        _ => {
            return fail!(from, "'{}' is not a valid trigger", ident);
        }
    })
}

fn parse_mod_ident(ident: &str) -> Option<u8> {
    match ident {
        "Shift" => Some(1),
        "Ctrl" => Some(2),
        "Alt" => Some(4),
        "Super" => Some(8),
        _ => None,
    }
}

fn parse_trigger(expr: syn::Expr) -> Result<Vec<(proc_macro2::TokenStream, bool)>, proc_macro2::TokenStream> {
    match expr {
        syn::Expr::Path(expr_path) => {
            if let Some(ident) = expr_path.path.get_ident() {
                Ok(vec!(match parse_trigger_ident(&ident, &ident.to_string())? {
                    Trigger::Key(Key::F(n)) => {
                        (quote!(
                            tuie::prelude::Trigger::Key(
                                tuie::prelude::Key::F(#n)
                            )
                        ), false)
                    }
                    Trigger::Key(Key::Char(c)) => {
                        (quote!(
                            tuie::prelude::Trigger::Key(
                                tuie::prelude::Key::Char(#c)
                            )
                        ), c.is_alphabetic())
                    }
                    Trigger::Key(Key::CharIdent(c)) => {
                        (quote!(
                            tuie::prelude::Trigger::Key(
                                tuie::prelude::Key::Char(#c)
                            )
                        ), true)
                    }
                    Trigger::Key(Key::Ident(name)) => {
                        let ident = syn::Ident::new(name, ident.span());
                        (quote!(
                            tuie::prelude::Trigger::Key(
                                tuie::prelude::Key::#ident
                            )
                        ), false)
                    }
                    Trigger::Key(Key::Arrow(direction)) => {
                        let ident = syn::Ident::new(direction, ident.span());
                        (quote!(
                            tuie::prelude::Trigger::Key(
                                tuie::prelude::Key::Arrow(tuie::prelude::Direction2D::#ident)
                            )
                        ), false)
                    }
                    Trigger::Mouse(event_name, button_name) => {
                        let event_ident = syn::Ident::new(event_name, ident.span());
                        let button_ident = syn::Ident::new(button_name, ident.span());
                        (quote!(
                            tuie::prelude::Trigger::#event_ident(
                                tuie::prelude::MouseButton::#button_ident
                            )
                        ), false)
                    }
                    Trigger::MouseScroll(direction) => {
                        let direction_ident = syn::Ident::new(direction, ident.span());
                        (quote!(
                            tuie::prelude::Trigger::MouseScroll(
                                tuie::prelude::Direction2D::#direction_ident
                            )
                        ), false)
                    }
                    Trigger::MouseHover => {
                        (quote!(
                            tuie::prelude::Trigger::MouseHover
                        ), false)
                    }
                }))
            }
            else {
                fail!(&expr_path, "Expected an identifier")
            }
        }
        syn::Expr::Lit(expr_lit) => {
            Ok(vec!(match expr_lit.lit {
                syn::Lit::Int(lit_int) => {
                    match lit_int.base10_parse::<u8>() {
                        Ok(n) => {
                            let s = n.to_string();
                            if s.len() != 1 {
                                return fail!(&lit_int.span(), "'{}' is not a valid key", s);
                            }
                            let ch = s.chars().next().unwrap();
                            (quote!(
                                tuie::prelude::Trigger::Key(
                                    tuie::prelude::Key::Char(#ch)
                                )
                            ), false)
                        }
                        Err(e) => {
                            return fail!(&lit_int, "{}", e);
                        }
                    }
                }
                syn::Lit::Char(lit_char) => {
                    let is_letter = lit_char.value().is_alphabetic();
                    (quote!(
                        tuie::prelude::Trigger::Key(
                            tuie::prelude::Key::Char(#lit_char)
                        )
                    ), is_letter)
                }
                _ => {
                    return fail!(&expr_lit, "Expected an integer or char literal");
                }
            }))
        }
        syn::Expr::Call(expr_call) => {
            if let syn::Expr::Path(expr_path) = &*expr_call.func {
                if let Some(ident) = expr_path.path.get_ident() {
                    match ident.to_string().as_str() {
                        "Char" => {
                            Ok(vec!((quote! {
                                tuie::prelude::Trigger::Key(
                                    tuie::prelude::Key::#expr_call
                                )
                            }, true)))
                        }
                        "F" | "Arrow" => {
                            Ok(vec!((quote! {
                                tuie::prelude::Trigger::Key(
                                    tuie::prelude::Key::#expr_call
                                )
                            }, false)))
                        }
                        "MouseScroll" | "MouseSmoothScroll" | "MouseDown" | "MouseDrag" | "MouseUp" | "Key" => {
                            Ok(vec!((quote! {
                                tuie::prelude::Trigger::#expr_call
                            }, false)))
                        }
                        _ => {
                            Ok(vec!((quote!(#expr_call), false)))
                        }
                    }
                }
                else {
                    Ok(vec!((quote!(#expr_call), false)))
                }
            }
            else {
                Ok(vec!((quote!(#expr_call), false)))
            }
        }
        syn::Expr::Paren(expr_paren) => {
            parse_trigger(*expr_paren.expr)
        }
        syn::Expr::Binary(expr_binary) => {
            if let syn::BinOp::BitOr(_) = expr_binary.op {
                let mut triggers = Vec::new();
                triggers.extend(parse_trigger(*expr_binary.left)?);
                triggers.extend(parse_trigger(*expr_binary.right)?);
                Ok(triggers)
            }
            else {
                fail!(&expr_binary.op, "Unexpected binary operator")
            }
        }
        _ => fail!(&expr, "Expected an identifier, literal or function call"),
    }
}

fn parse_modifiers(expr: &syn::Expr) -> Result<Vec<u8>, proc_macro2::TokenStream> {
    match expr {
        syn::Expr::Paren(expr_paren) => {
            parse_modifiers(&expr_paren.expr)
        }
        syn::Expr::Binary(expr_binary) => {
            if let syn::BinOp::Add(_) = expr_binary.op {
                let left = parse_modifiers(&expr_binary.left)?;
                let right = parse_modifiers(&expr_binary.right)?;
                let mut combined = Vec::new();
                for l in &left {
                    for r in &right {
                        combined.push(l | r);
                    }
                }
                Ok(combined)
            }
            else if let syn::BinOp::BitOr(_) = expr_binary.op {
                let left = parse_modifiers(&expr_binary.left)?;
                let right = parse_modifiers(&expr_binary.right)?;
                let mut alternatives = Vec::new();
                for l in &left {
                    alternatives.push(*l);
                }
                for r in &right {
                    alternatives.push(*r);
                }
                Ok(alternatives)
            }
            else {
                fail!(&expr_binary.op, "Unexpected operator")
            }
        }
        syn::Expr::Path(expr_path) => {
            if let Some(ident) = expr_path.path.get_ident() {
                let modifier = ident.to_string();
                if let Some(value) = parse_mod_ident(&modifier) {
                    Ok(vec!(value))
                }
                else {
                    fail!(expr_path, "Expected Alt, Ctrl, Shift, or Super")
                }
            }
            else {
                fail!(expr_path, "Expected Alt, Ctrl, Shift, or Super")
            }
        }
        syn::Expr::Try(expr_try) => {
            let mut mods = parse_modifiers(&expr_try.expr)?;
            mods.push(0);
            Ok(mods)
        }
        _ => fail!(expr, "Expected Alt, Ctrl, Shift, or Super"),
    }
}

fn build_mods_node(mods: &[u8], span: proc_macro2::Span) -> syn::Expr {
    let mut iter = mods.iter();
    let first = iter.next().copied().unwrap_or(0);
    let mut node = syn::Expr::Lit(syn::ExprLit {
        attrs: Vec::new(),
        lit: syn::Lit::Int(syn::LitInt::new(&format!("{:#06b}", first), span)),
    });
    for &next in iter {
        let next_node = syn::Expr::Lit(syn::ExprLit {
            attrs: Vec::new(),
            lit: syn::Lit::Int(syn::LitInt::new(&format!("{:#06b}", next), span)),
        });
        node = syn::Expr::Binary(syn::ExprBinary {
            attrs: Vec::new(),
            left: Box::new(node),
            op: syn::BinOp::BitOr(Default::default()),
            right: Box::new(next_node),
        })
    }
    node
}

fn build_chord(mods_node: &syn::Expr, triggers: &[proc_macro2::TokenStream]) -> proc_macro2::TokenStream {
    let mut iter = triggers.iter();
    let first = iter.next().unwrap();
    let mut triggers_node = quote!(#first);
    for next in iter {
        triggers_node = quote!(#triggers_node | #next)
    }
    quote!{ tuie::prelude::Chord { modifiers: tuie::prelude::Modifiers { modifiers: #mods_node }, trigger: #triggers_node } }
}

fn find_shift_span(expr: &syn::Expr) -> Option<proc_macro2::Span> {
    match expr {
        syn::Expr::Path(expr_path) => {
            if let Some(ident) = expr_path.path.get_ident() {
                if ident == "Shift" {
                    return Some(ident.span());
                }
            }
            None
        }
        syn::Expr::Paren(expr_paren) => find_shift_span(&expr_paren.expr),
        syn::Expr::Binary(expr_binary) => {
            find_shift_span(&expr_binary.left).or_else(|| find_shift_span(&expr_binary.right))
        }
        syn::Expr::Try(expr_try) => find_shift_span(&expr_try.expr),
        _ => None,
    }
}

fn parse_chord(expr: syn::Expr) -> Result<proc_macro2::TokenStream, proc_macro2::TokenStream> {
    let root_span = expr.span();

    let mut mods: Vec<u8> = Vec::new();
    let mut mods_expr: Option<syn::Expr> = None;

    let trigger_expr = match expr {
        syn::Expr::Binary(expr_binary) => {
            if let syn::BinOp::Add(_) = expr_binary.op {
                mods = parse_modifiers(&expr_binary.left)?;
                mods_expr = Some(*expr_binary.left);
                *expr_binary.right
            }
            else {
                syn::Expr::Binary(expr_binary)
            }
        }
        other => other,
    };

    let has_shift = mods.iter().any(|m| m & 1 != 0);
    let triggers = parse_trigger(trigger_expr)?;

    // Letter triggers express Shift through their case, so combining them
    // with an explicit Shift is a contradiction. Non-letter chars (digits,
    // symbols, Space) DO carry a Shift bit — their shifted symbol is
    // keyboard-layout-dependent, so `Shift + '1'` is the portable spelling.
    if has_shift && triggers.iter().any(|(_, is_letter)| *is_letter) {
        let span = mods_expr.as_ref().and_then(find_shift_span).unwrap_or(root_span);
        return fail!(&span, "Shift cannot be used with letter triggers, use uppercase idents like `V` or Char('V') directly");
    }

    let mods_node = build_mods_node(&mods, root_span);
    let trigger_tokens: Vec<_> = triggers.into_iter().map(|(tokens, _)| tokens).collect();
    Ok(build_chord(&mods_node, &trigger_tokens))
}

fn is_chord_expr(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Paren(expr_paren) => {
            is_chord_expr(&expr_paren.expr)
        }
        syn::Expr::Binary(expr_binary) => {
            if let syn::BinOp::BitOr(_) = expr_binary.op {
                is_chord_expr(&expr_binary.left) || is_chord_expr(&expr_binary.right)
            }
            else if let syn::BinOp::Add(_) = expr_binary.op {
                true
            }
            else {
                false
            }
        }
        _ => false,
    }
}

fn parse_chords(expr: syn::Expr) -> Result<Vec<proc_macro2::TokenStream>, proc_macro2::TokenStream> {
    match expr {
        syn::Expr::Paren(expr_paren) => {
            parse_chords(*expr_paren.expr)
        }
        syn::Expr::Binary(expr_binary) => {
            if let syn::BinOp::BitOr(_) = expr_binary.op {
                if is_chord_expr(&expr_binary.left) || is_chord_expr(&expr_binary.right) {
                    let mut chords = Vec::new();
                    chords.extend(parse_chords(*expr_binary.left)?);
                    chords.extend(parse_chords(*expr_binary.right)?);
                    return Ok(chords);
                }
            }
            Ok(vec!(parse_chord(syn::Expr::Binary(expr_binary))?))
        }
        _ => {
            Ok(vec!(parse_chord(expr)?))
        }
    }
}

/// Expands a chord literal into a `tuie::prelude::Chord` match pattern.
#[proc_macro]
pub fn chord(attr: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let root = parse_macro_input!(attr as syn::Expr);
    match parse_chords(root) {
        Ok(mut chords) => {
            chords.reverse();
            let mut node = chords.pop().unwrap();
            while let Some(next) = chords.pop() {
                node = quote!(#node | #next);
            }
            node.into()
        }
        Err(stream) => stream.into(),
    }
}
