use crate::app::Editor;
use crate::buffer::{Pos, line_chars};
use crate::mode::{FindDir, Mode, Operator, Pending};
use crate::motions::Motion;
use crate::textobjects::Obj;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, ModifiersState, NamedKey};

pub fn on_key(ed: &mut Editor, ev: &KeyEvent, mods: ModifiersState) {
    if ev.state == ElementState::Released {
        return;
    }
    key_down(ed, &ev.logical_key, mods);
}

fn key_down(ed: &mut Editor, key: &Key, mods: ModifiersState) {
    if is_modifier(key) {
        return;
    }
    if ed.panel != crate::fuzzy::Panel::None {
        panel_key(ed, key, mods);
        return;
    }
    match ed.mode {
        Mode::Insert => insert_key(ed, key, mods),
        Mode::Command(_) => command_key(ed, key, mods),
        Mode::Normal | Mode::Visual => normal_key(ed, key, mods),
    }
}

fn is_modifier(key: &Key) -> bool {
    matches!(
        key,
        Key::Dead(_)
            | Key::Named(
                NamedKey::Shift
                    | NamedKey::Control
                    | NamedKey::Alt
                    | NamedKey::AltGraph
                    | NamedKey::Super
                    | NamedKey::CapsLock
                    | NamedKey::ModeChange
            )
    )
}

fn key_char(key: &Key) -> Option<&'static str> {
    match key {
        Key::Character(s) => s.as_str().chars().next().and_then(leak_char),
        _ => None,
    }
}

const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

fn leak_char(c: char) -> Option<&'static str> {
    if let Some(d) = c.to_digit(10) {
        return Some(DIGITS[d as usize]);
    }
    Some(match c {
        'h' => "h",
        'j' => "j",
        'k' => "k",
        'l' => "l",
        'w' => "w",
        'W' => "W",
        'b' => "b",
        'B' => "B",
        'e' => "e",
        'E' => "E",
        'g' => "g",
        'G' => "G",
        'f' => "f",
        'F' => "F",
        't' => "t",
        'T' => "T",
        'd' => "d",
        'c' => "c",
        'y' => "y",
        'x' => "x",
        's' => "s",
        'p' => "p",
        'P' => "P",
        'r' => "r",
        'J' => "J",
        'o' => "o",
        'O' => "O",
        'i' => "i",
        'a' => "a",
        'I' => "I",
        'A' => "A",
        'v' => "v",
        'V' => "V",
        'z' => "z",
        'u' => "u",
        '$' => "$",
        '^' => "^",
        '%' => "%",
        '*' => "*",
        ';' => ";",
        ',' => ",",
        '>' => ">",
        '<' => "<",
        'n' => "n",
        'N' => "N",
        '(' => "(",
        ')' => ")",
        '[' => "[",
        ']' => "]",
        '{' => "{",
        '}' => "}",
        '"' => "\"",
        '\'' => "'",
        '`' => "`",
        ':' => ":",
        '/' => "/",
        '?' => "?",
        _ => return None,
    })
}

fn normal_key(ed: &mut Editor, key: &Key, mods: ModifiersState) {
    if mods.alt_key() || mods.super_key() {
        return;
    }
    ed.lsp.hover = None;
    ed.status = None;
    if *key == Key::Named(NamedKey::Escape) {
        ed.clear_pending();
        if ed.mode == Mode::Visual {
            ed.mode = Mode::Normal;
            ed.visual_anchor = None;
        }
        return;
    }
    let k = key_char(key);
    if mods.control_key() {
        match k {
            Some("u") => ed.run_motion(Motion::HalfPageUp, 1),
            Some("d") => ed.run_motion(Motion::HalfPageDown, 1),
            Some("p") => ed.open_panel(crate::fuzzy::Panel::Files),
            Some("b") => ed.open_panel(crate::fuzzy::Panel::Buffers),
            Some("f") => ed.run_motion(Motion::PageDown, 1),
            Some("r") => ed.buf_mut().redo_action(),
            Some("c") | Some("[") => ed.leave_insert(),
            Some("a") => {
                let c = ed.buf().cursor;
                let len = line_chars(&ed.buf().rope, c.line);
                ed.buf_mut().cursor = Pos { line: c.line, col: len };
                ed.enter_insert();
            }
            _ => {}
        }
        ed.buf_mut().clamp();
        return;
    }
    if let Some(c) = k
        && let Some(d) = c.chars().next().and_then(|ch| ch.to_digit(10)) {
            if ed.count == 0 && d == 0 {
                ed.run_motion(Motion::Zero, 1);
            } else {
                ed.count = ed.count * 10 + d;
            }
            return;
        }
    let count = if ed.count == 0 { 1 } else { ed.count };
    let page = match key {
        Key::Named(NamedKey::PageDown) => Some(Motion::PageDown),
        Key::Named(NamedKey::PageUp) => Some(Motion::PageUp),
        _ => None,
    };
    if let Some(m) = page {
        if let Pending::Op(op) = ed.pending {
            ed.pending = Pending::None;
            ed.operator_motion(op, m, count);
        } else {
            ed.run_motion(m, count);
        }
        ed.count = 0;
        return;
    }
    match ed.pending {
        Pending::G => {
            ed.pending = Pending::None;
            match k {
                Some("g") => ed.run_motion(Motion::GG, count),
                Some("j") => ed.run_motion(Motion::J, count),
                Some("k") => ed.run_motion(Motion::K, count),
                Some("e") => ed.run_motion(Motion::GE, count),
                Some("E") => ed.run_motion(Motion::BigGE, count),
                Some("d") => ed.goto_definition_cur(),
                _ => {}
            }
            ed.count = 0;
            return;
        }
        Pending::Z => {
            ed.pending = Pending::None;
            if matches!(k, Some("z")) {
                let rows = ed.view_rows;
                ed.center_cursor(rows);
            }
            ed.count = 0;
            return;
        }
        Pending::F { forward, to } => {
            ed.pending = Pending::None;
            if let Some(s) = key_text(key) {
                ed.count = 0;
                let dir = match (forward, to) {
                    (true, true) => FindDir::F,
                    (true, false) => FindDir::T,
                    (false, true) => FindDir::BackF,
                    (false, false) => FindDir::BackT,
                };
                ed.last_find = Some((dir, s));
                ed.run_find(dir, s, count);
            } else {
                ed.count = 0;
            }
            return;
        }
        Pending::R => {
            ed.pending = Pending::None;
            if let Some(s) = key_text(key) {
                ed.replace_char(s);
            }
            ed.count = 0;
            return;
        }
        Pending::TextObj { op, inner } => {
            ed.pending = Pending::None;
            if let Some(obj) = k.and_then(obj_of) {
                if ed.mode == Mode::Visual {
                    ed.select_object(inner, obj);
                } else {
                    ed.operator_object(op, inner, obj);
                }
            }
            ed.count = 0;
            return;
        }
        Pending::Op(op) => {
            ed.pending = Pending::None;
            match k {
                Some("i") => {
                    ed.pending = Pending::TextObj { op, inner: true };
                    return;
                }
                Some("a") => {
                    ed.pending = Pending::TextObj { op, inner: false };
                    return;
                }
                Some(s2) if s2 == op_char(op) => {
                    ed.operator_line(op);
                }
                Some(m) => {
                    let mo = match m {
                        "j" => Some(Motion::LineDown),
                        "k" => Some(Motion::LineUp),
                        _ => motion_of(m),
                    };
                    if let Some(mo) = mo {
                        ed.operator_motion(op, mo, count);
                    }
                }
                None => {}
            }
            ed.count = 0;
            return;
        }
        Pending::None => {}
    }
    let Some(k) = k else { return };
    if ed.mode == Mode::Visual {
        visual_key(ed, k, count);
        return;
    }
    match k {
        "h" | "j" | "k" | "l" => {
            let m = motion_of(k).unwrap();
            ed.run_motion(m, count);
        }
        "w" | "W" | "b" | "B" | "e" | "E" | "0" | "$" | "^" | "%" => {
            let m = motion_of(k).unwrap();
            ed.run_motion(m, count);
        }
        "G" => ed.run_motion(Motion::G, count),
        "g" => ed.pending = Pending::G,
        "z" => ed.pending = Pending::Z,
        "K" => ed.hover_cur(),
        "f" => ed.pending = Pending::F { forward: true, to: true },
        "F" => ed.pending = Pending::F { forward: false, to: true },
        "t" => ed.pending = Pending::F { forward: true, to: false },
        "T" => ed.pending = Pending::F { forward: false, to: false },
        ";" | "," => {
            if let Some((dir, ch)) = ed.last_find {
                let dir = match (k, dir) {
                    (",", FindDir::F) | (",", FindDir::T) => {
                        if matches!(dir, FindDir::F) { FindDir::T } else { FindDir::F }
                    }
                    (_, FindDir::F) => FindDir::F,
                    (_, FindDir::T) => FindDir::T,
                    (_, FindDir::BackF) => FindDir::BackF,
                    (_, FindDir::BackT) => FindDir::BackT,
                };
                let rev = k == ",";
                let d = match (dir, rev) {
                    (FindDir::F, false) | (FindDir::T, true) => FindDir::F,
                    (FindDir::T, false) | (FindDir::F, true) => FindDir::T,
                    (FindDir::BackF, false) | (FindDir::BackT, true) => FindDir::BackF,
                    _ => FindDir::BackT,
                };
                ed.run_find(d, ch, count);
            }
        }
        "n" => {
            ed.repeat_search(false);
            return;
        }
        "N" => {
            ed.repeat_search(true);
            return;
        }
        "*" => {
            ed.search_cur_word();
            return;
        }
        ":" => {
            ed.open_cmdline(crate::mode::CmdKind::Ex);
            return;
        }
        "/" => {
            ed.open_cmdline(crate::mode::CmdKind::Search);
            return;
        }
        "?" => {
            ed.open_cmdline(crate::mode::CmdKind::SearchBack);
            return;
        }
        "d" | "c" | "y" => ed.pending = Pending::Op(op_of(k)),
        ">" => ed.pending = Pending::Op(Operator::Indent),
        "<" => ed.pending = Pending::Op(Operator::Outdent),
        "x" => ed.change_char(Operator::Delete),
        "s" => ed.change_char(Operator::Change),
        "p" => ed.paste(true),
        "P" => ed.paste(false),
        "r" => ed.pending = Pending::R,
        "J" => ed.join_lines(count),
        "o" => {
            ed.new_line_below();
            ed.count = 0;
        }
        "O" => {
            ed.new_line_above();
            ed.count = 0;
        }
        "i" => ed.enter_insert(),
        "a" => {
            let c = ed.buf().cursor;
            let len = line_chars(&ed.buf().rope, c.line);
            ed.buf_mut().cursor = Pos { line: c.line, col: (c.col + 1).min(len) };
            ed.enter_insert();
        }
        "I" => {
            ed.run_motion(Motion::Caret, 1);
            ed.enter_insert();
        }
        "A" => {
            let c = ed.buf().cursor;
            let len = line_chars(&ed.buf().rope, c.line);
            ed.buf_mut().cursor = Pos { line: c.line, col: len };
            ed.enter_insert();
        }
        "v" => ed.start_visual(false),
        "V" => ed.start_visual(true),
        "u" => {
            for _ in 0..count {
                ed.buf_mut().undo();
            }
        }
        _ => {}
    }
    if matches!(ed.pending, Pending::None) {
        ed.count = 0;
    }
    ed.buf_mut().clamp();
}

fn visual_key(ed: &mut Editor, k: &str, count: u32) {
    match k {
        "v" => {
            ed.mode = Mode::Normal;
            ed.visual_anchor = None;
        }
        "V" => {
            ed.visual_linewise = !ed.visual_linewise;
        }
        "h" | "j" | "k" | "l" | "w" | "W" | "b" | "B" | "e" | "E" | "0" | "^" | "$"
        | "%" => {
            let m = match (ed.visual_linewise, motion_of(k).unwrap()) {
                (true, Motion::J) => Motion::LineDown,
                (true, Motion::K) => Motion::LineUp,
                (_, other) => other,
            };
            ed.run_motion(m, count);
        }
        "G" => ed.run_motion(Motion::G, count),
        "i" => ed.pending = Pending::TextObj { op: Operator::Change, inner: true },
        "a" => ed.pending = Pending::TextObj { op: Operator::Change, inner: false },
        "d" | "c" | "y" => {
            let ((a, b), lw) = ed.sel_range();
            ed.op_apply(op_of(k), (a, b), lw);
            ed.mode = Mode::Normal;
            ed.visual_anchor = None;
        }
        "x" => {
            let ((a, b), lw) = ed.sel_range();
            ed.op_apply(Operator::Delete, (a, b), lw);
            ed.mode = Mode::Normal;
            ed.visual_anchor = None;
        }
        ">" => {
            let ((a, b), _) = ed.sel_range();
            ed.op_apply(Operator::Indent, (a, b), true);
            ed.mode = Mode::Normal;
            ed.visual_anchor = None;
        }
        "<" => {
            let ((a, b), _) = ed.sel_range();
            ed.op_apply(Operator::Outdent, (a, b), true);
            ed.mode = Mode::Normal;
            ed.visual_anchor = None;
        }
        "p" => {
            let ((a, b), _) = ed.sel_range();
            ed.buf_mut().delete_text(a, b);
            ed.mode = Mode::Normal;
            ed.visual_anchor = None;
            ed.paste(true);
        }
        _ => {}
    }
    ed.count = 0;
}

fn insert_key(ed: &mut Editor, key: &Key, mods: ModifiersState) {
    if mods.alt_key() || mods.super_key() {
        return;
    }
    let popup = ed.lsp.completion.is_some();
    if mods.control_key() {
        match key_char(key) {
            Some("c") | Some("[") => ed.leave_insert(),
            Some("n") if popup => ed.comp_next(),
            Some("p") if popup => ed.comp_prev(),
            Some("y") | Some("e") if popup => ed.comp_accept(),
            _ => {}
        }
        return;
    }
    match key {
        Key::Named(NamedKey::Escape) if popup => ed.comp_cancel(),
        Key::Named(NamedKey::Enter) if popup => ed.comp_accept(),
        Key::Named(NamedKey::Tab) if popup => ed.comp_accept(),
        Key::Named(NamedKey::Escape) => ed.leave_insert(),
        Key::Named(NamedKey::Enter) => {
            ed.comp_cancel();
            ed.newline_in_insert();
        }
        Key::Named(NamedKey::Backspace) => {
            ed.comp_cancel();
            backspace(ed);
        }
        Key::Named(NamedKey::Delete) => {
            ed.comp_cancel();
            delete_fwd(ed);
        }
        Key::Named(NamedKey::Tab) => {
            ed.comp_cancel();
            insert_tab(ed);
        }
        Key::Named(NamedKey::Space) => {
            let co = ed.coalesce_open;
            ed.buf_mut().insert_text(" ", co);
            ed.maybe_completion(" ");
        }
        Key::Character(s)
            if s.chars().next().is_some_and(|c| !c.is_control()) => {
                if let Some(ch) = s.chars().next() {
                    ed.close_bracket_indent(ch);
                }
                let co = ed.coalesce_open;
                ed.buf_mut().insert_text(s, co);
                ed.maybe_completion(s);
            }
        _ => {}
    }
    ed.buf_mut().clamp();
}

fn key_text(key: &Key) -> Option<char> {
    match key {
        Key::Character(s) => s.chars().next().filter(|c| !c.is_control()),
        Key::Named(NamedKey::Space) => Some(' '),
        _ => None,
    }
}

fn motion_of(k: &str) -> Option<Motion> {
    Some(match k {
        "h" => Motion::H,
        "j" => Motion::J,
        "k" => Motion::K,
        "l" => Motion::L,
        "w" => Motion::W,
        "W" => Motion::BigW,
        "b" => Motion::B,
        "B" => Motion::BigB,
        "e" => Motion::E,
        "E" => Motion::BigE,
        "0" => Motion::Zero,
        "^" => Motion::Caret,
        "$" => Motion::Dollar,
        "%" => Motion::Percent,
        _ => return None,
    })
}

fn op_of(k: &str) -> Operator {
    match k {
        "d" => Operator::Delete,
        "c" => Operator::Change,
        _ => Operator::Yank,
    }
}

fn op_char(op: Operator) -> &'static str {
    match op {
        Operator::Delete => "d",
        Operator::Change => "c",
        Operator::Yank => "y",
        Operator::Indent | Operator::Outdent => "",
    }
}

fn obj_of(k: &str) -> Option<Obj> {
    let c = k.chars().next()?;
    Some(match c {
        'w' => Obj::Word,
        'W' => Obj::BigWord,
        '"' | '\'' | '`' => Obj::Quote(c),
        '(' | '[' | '{' => Obj::Bracket(c),
        _ => return None,
    })
}

fn backspace(ed: &mut Editor) {
    let c = ed.buf().cursor;
    if c.col > 0 {
        ed.buf_mut().delete_text(
            Pos {
                line: c.line,
                col: c.col - 1,
            },
            c,
        );
    } else if c.line > 0 {
        let prev_len = line_chars(&ed.buf().rope, c.line - 1);
        ed.buf_mut().delete_text(
            Pos {
                line: c.line - 1,
                col: prev_len,
            },
            c,
        );
    }
}

fn delete_fwd(ed: &mut Editor) {
    let c = ed.buf().cursor;
    let len = line_chars(&ed.buf().rope, c.line);
    if c.col < len {
        ed.buf_mut().delete_text(
            c,
            Pos {
                line: c.line,
                col: c.col + 1,
            },
        );
    } else if c.line + 1 < ed.buf().rope.len_lines() {
        ed.buf_mut().delete_text(
            c,
            Pos {
                line: c.line + 1,
                col: 0,
            },
        );
    }
}

fn insert_tab(ed: &mut Editor) {
    let line = crate::buffer::line_text(&ed.buf().rope, ed.buf().cursor.line);
    let vx = ed.visual_x(&line, ed.buf().cursor.col);
    let tw = ed.tab_width();
    let pad = tw - (vx % tw);
    let co = ed.coalesce_open;
    ed.buf_mut().insert_text(&" ".repeat(pad), co);
}

fn command_key(ed: &mut Editor, key: &Key, mods: ModifiersState) {
    if mods.control_key() {
        match key_char(key) {
            Some("c") | Some("[") => ed.close_cmdline(),
            _ => {}
        }
        return;
    }
    if mods.alt_key() || mods.super_key() {
        return;
    }
    match key {
        Key::Named(NamedKey::Escape) => ed.close_cmdline(),
        Key::Named(NamedKey::Enter) => match ed.cmd_kind {
            Some(crate::mode::CmdKind::Ex) => ed.execute_cmdline(),
            Some(crate::mode::CmdKind::Search) => ed.commit_search(false),
            Some(crate::mode::CmdKind::SearchBack) => ed.commit_search(true),
            None => ed.close_cmdline(),
        },
        Key::Named(NamedKey::Backspace) => {
            ed.cmdline.pop();
        }
        Key::Character(s) => {
            if let Some(c) = s.chars().next()
                && !c.is_control() {
                    ed.cmdline.push(c);
                }
        }
        Key::Named(NamedKey::Space) => ed.cmdline.push(' '),
        _ => {}
    }
}

fn panel_key(ed: &mut Editor, key: &Key, mods: ModifiersState) {
    if mods.control_key() {
        match key_char(key) {
            Some("c") | Some("[") | Some("]") => ed.close_panel(),
            Some("p") => ed.open_panel(crate::fuzzy::Panel::Files),
            Some("b") => ed.open_panel(crate::fuzzy::Panel::Buffers),
            Some("n") => ed.panel_move(1),
            _ => {}
        }
        return;
    }
    if mods.alt_key() || mods.super_key() {
        return;
    }
    match key {
        Key::Named(NamedKey::Escape) => ed.close_panel(),
        Key::Named(NamedKey::Enter) => ed.panel_accept(),
        Key::Named(NamedKey::Backspace) => ed.panel_backspace(),
        Key::Named(NamedKey::ArrowDown) => ed.panel_move(1),
        Key::Named(NamedKey::ArrowUp) => ed.panel_move(-1),
        Key::Character(s) => {
            if let Some(c) = s.chars().next()
                && !c.is_control() {
                    ed.panel_type(c);
                }
        }
        Key::Named(NamedKey::Space) => ed.panel_type(' '),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::CmdKind;

    fn type_keys(ed: &mut Editor, keys: &[Key]) {
        for k in keys {
            key_down(ed, k, ModifiersState::empty());
        }
    }

    #[test]
    fn space_inserts_in_insert_mode() {
        let mut ed = Editor::new(None);
        ed.buf_mut().insert_text("ab", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        ed.enter_insert();
        type_keys(
            &mut ed,
            &[
                Key::Named(NamedKey::Space),
                Key::Character("x".into()),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "a xb");
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 3 });
    }

    #[test]
    fn insert_and_append_place_the_cursor_like_vim() {
        let mut ed = Editor::new(None);
        ed.buf_mut().insert_text("ab\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        type_keys(&mut ed, &[Key::Character("i".into())]);
        assert_eq!(ed.mode, Mode::Insert);
        type_keys(&mut ed, &[Key::Character("x".into())]);
        assert_eq!(ed.buf().rope.to_string(), "axb\n");

        type_keys(&mut ed, &[Key::Named(NamedKey::Escape)]);
        type_keys(&mut ed, &[Key::Character("a".into())]);
        assert_eq!(ed.mode, Mode::Insert);
        type_keys(&mut ed, &[Key::Character("y".into())]);
        assert_eq!(ed.buf().rope.to_string(), "axby\n");

        type_keys(&mut ed, &[Key::Named(NamedKey::Escape)]);
        ed.buf_mut().cursor = Pos { line: 0, col: 4 };
        type_keys(
            &mut ed,
            &[
                Key::Character("a".into()),
                Key::Character("z".into()),
                Key::Named(NamedKey::Escape),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "axbyz\n");
        assert_eq!(ed.mode, Mode::Normal);
    }

    #[test]
    fn operators_still_take_text_objects() {
        let mut ed = Editor::new(None);
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("let s = \"hi\";\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 9 };
        type_keys(
            &mut ed,
            &[
                Key::Character("c".into()),
                Key::Character("i".into()),
                Key::Character("\"".into()),
                Key::Character("z".into()),
            ],
        );
        assert_eq!(ed.mode, Mode::Insert);
        assert_eq!(ed.buf().rope.to_string(), "let s = \"z\";\n");

        type_keys(&mut ed, &[Key::Named(NamedKey::Escape)]);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[
                Key::Character("d".into()),
                Key::Character("i".into()),
                Key::Character("w".into()),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), " s = \"z\";\n");
        assert_eq!(ed.mode, Mode::Normal);
    }

    #[test]
    fn page_keys_move_by_the_viewport() {
        let mut ed = Editor::new(None);
        ed.config.clipboard = false;
        let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
        ed.buf_mut().insert_text(&body, false);
        ed.view_rows = 10;
        ed.buf_mut().cursor = Pos { line: 0, col: 3 };
        type_keys(&mut ed, &[Key::Named(NamedKey::PageDown)]);
        assert_eq!(ed.buf().cursor, Pos { line: 10, col: 3 });
        type_keys(&mut ed, &[Key::Named(NamedKey::PageUp)]);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 3 });
        type_keys(
            &mut ed,
            &[
                Key::Character("2".into()),
                Key::Named(NamedKey::PageDown),
            ],
        );
        assert_eq!(ed.buf().cursor.line, 20);
        assert_eq!(ed.count, 0);
        type_keys(
            &mut ed,
            &[
                Key::Character("d".into()),
                Key::Named(NamedKey::PageUp),
            ],
        );
        assert_eq!(ed.buf().rope.len_lines(), 30);
        assert_eq!(ed.buf().rope.line(10).to_string(), "line 21\n");
        assert_eq!(ed.mode, Mode::Normal);
    }

    #[test]
    fn zz_centers_the_viewport_on_the_cursor() {
        let mut ed = Editor::new(None);
        let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
        ed.buf_mut().insert_text(&body, false);
        ed.view_rows = 10;
        ed.buf_mut().cursor = Pos { line: 30, col: 2 };
        type_keys(&mut ed, &[Key::Character("z".into())]);
        assert_eq!(ed.pending, Pending::Z);
        assert_eq!(ed.buf().view_top, 0);
        type_keys(&mut ed, &[Key::Character("z".into())]);
        let b = ed.buf();
        assert_eq!((b.view_top, b.view_sub), (25, 0));
        let drawn = ed
            .wrap()
            .screen_rows(&b.rope, b.view_top, b.view_sub, 10)
            .iter()
            .map(|r| r.line)
            .collect::<Vec<_>>();
        assert_eq!(drawn, (25..35).collect::<Vec<_>>());
        assert_eq!(
            ed.wrap().screen_row(&b.rope, b.view_top, b.view_sub, b.cursor),
            Some(5)
        );
        assert_eq!(ed.pending, Pending::None);
    }

    #[test]
    fn z_then_another_key_leaves_the_view_alone() {
        let mut ed = Editor::new(None);
        let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
        ed.buf_mut().insert_text(&body, false);
        ed.view_rows = 10;
        ed.buf_mut().cursor = Pos { line: 30, col: 2 };
        type_keys(
            &mut ed,
            &[Key::Character("z".into()), Key::Character("j".into())],
        );
        assert_eq!(ed.pending, Pending::None);
        assert_eq!(ed.buf().cursor, Pos { line: 30, col: 2 });
        assert_eq!(ed.buf().view_top, 0);
    }

    #[test]
    fn typed_counts_reach_motions_and_operators() {
        let mut ed = Editor::new(None);
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one two three four\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[
                Key::Character("3".into()),
                Key::Character("w".into()),
            ],
        );
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 14 });
        type_keys(
            &mut ed,
            &[
                Key::Character("0".into()),
                Key::Character("2".into()),
                Key::Character("d".into()),
                Key::Character("w".into()),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "three four\n");
    }

    #[test]
    fn open_line_indents_after_an_opening_brace() {
        let mut ed = Editor::new(None);
        ed.config.tab_width = 4;
        ed.buf_mut().insert_text("int main() {\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(&mut ed, &[Key::Character("o".into())]);
        assert_eq!(ed.mode, Mode::Insert);
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 4 });
        type_keys(
            &mut ed,
            &[
                Key::Character("f".into()),
                Key::Named(NamedKey::Escape),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "int main() {\n    f\n");
    }

    #[test]
    fn open_above_copies_the_line_below_indent() {
        let mut ed = Editor::new(None);
        ed.config.tab_width = 4;
        ed.buf_mut().insert_text("    foo();\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 2 };
        type_keys(&mut ed, &[Key::Character("O".into())]);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 4 });
        assert_eq!(ed.buf().rope.to_string(), "    \n    foo();\n");

        let mut closing = Editor::new(None);
        closing.config.tab_width = 4;
        closing.buf_mut().insert_text("    foo();\n    }\n", false);
        closing.buf_mut().cursor = Pos { line: 1, col: 0 };
        type_keys(&mut closing, &[Key::Character("O".into())]);
        assert_eq!(
            closing.buf().rope.to_string(),
            "    foo();\n\n    }\n"
        );
    }

    #[test]
    fn enter_in_insert_mode_repeats_the_indent() {
        let mut ed = Editor::new(None);
        ed.config.tab_width = 4;
        ed.buf_mut().insert_text("    foo();\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 10 };
        ed.enter_insert();
        type_keys(
            &mut ed,
            &[
                Key::Named(NamedKey::Enter),
                Key::Character("x".into()),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "    foo();\n    x\n");
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 5 });
    }

    #[test]
    fn typing_a_leading_closing_brace_dedents_the_line() {
        let mut ed = Editor::new(None);
        ed.config.tab_width = 4;
        ed.buf_mut().insert_text("int main() {\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 12 };
        type_keys(&mut ed, &[Key::Character("o".into())]);
        type_keys(
            &mut ed,
            &[
                Key::Character("x".into()),
                Key::Named(NamedKey::Enter),
                Key::Character("}".into()),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "int main() {\n    x\n}\n");
        assert_eq!(ed.buf().cursor, Pos { line: 2, col: 1 });

        let mut mid = Editor::new(None);
        mid.config.tab_width = 4;
        mid.buf_mut().insert_text("    x", false);
        mid.buf_mut().cursor = Pos { line: 0, col: 5 };
        mid.enter_insert();
        type_keys(&mut mid, &[Key::Character("}".into())]);
        assert_eq!(mid.buf().rope.to_string(), "    x}");
        assert_eq!(mid.buf().cursor, Pos { line: 0, col: 6 });
    }

    #[test]
    fn space_types_in_cmdline_and_panel_query() {
        let mut ed = Editor::new(None);
        ed.open_cmdline(CmdKind::Ex);
        type_keys(
            &mut ed,
            &[
                Key::Character("e".into()),
                Key::Named(NamedKey::Space),
                Key::Character("n".into()),
                Key::Character("o".into()),
                Key::Named(NamedKey::Space),
                Key::Character("x".into()),
            ],
        );
        assert_eq!(ed.cmdline, "e no x");

        ed.open_panel(crate::fuzzy::Panel::Buffers);
        type_keys(&mut ed, &[Key::Named(NamedKey::Space)]);
        assert_eq!(ed.panel_query, " ");
    }

    #[test]
    fn space_is_a_find_and_replace_target() {
        let mut ed = Editor::new(None);
        ed.buf_mut().insert_text("a b", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(&mut ed, &[Key::Character("f".into()), Key::Named(NamedKey::Space)]);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 1 });
        assert_eq!(ed.last_find, Some((FindDir::F, ' ')));
        type_keys(&mut ed, &[Key::Character("l".into())]);
        type_keys(&mut ed, &[Key::Character("r".into()), Key::Named(NamedKey::Space)]);
        assert_eq!(ed.buf().rope.to_string(), "a  ");
    }

    #[test]
    fn star_key_searches_forward_from_the_cursor() {
        let mut ed = Editor::new(None);
        ed.buf_mut().insert_text("x foo bar foo\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 2 };
        type_keys(&mut ed, &[Key::Character("*".into())]);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 10 });
        assert_eq!(ed.search.as_deref(), Some("foo"));
    }

    #[test]
    fn normal_key_clears_the_echo_message() {
        let mut ed = Editor::new(None);
        ed.set_status("E37: No write since last change".to_string(), true);
        type_keys(&mut ed, &[Key::Character("l".into())]);
        assert_eq!(ed.status, None);
    }

    #[test]
    fn shift_between_find_and_target_keeps_the_pending_find() {
        let mut ed = Editor::new(None);
        ed.buf_mut().insert_text("onePtwo\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[
                Key::Character("f".into()),
                Key::Named(NamedKey::Shift),
                Key::Character("P".into()),
            ],
        );
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 3 });
        assert_eq!(ed.last_find, Some((FindDir::F, 'P')));
    }

    #[test]
    fn shift_between_operator_and_motion_still_applies_the_operator() {
        let mut ed = Editor::new(None);
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one two three\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[
                Key::Character("d".into()),
                Key::Named(NamedKey::Shift),
                Key::Character("W".into()),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "two three\n");
        assert_eq!(ed.pending, Pending::None);
    }
}
