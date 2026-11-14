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
    if ed.panel != crate::fuzzy::Panel::None {
        panel_key(ed, ev, mods);
        return;
    }
    match ed.mode {
        Mode::Insert => insert_key(ed, ev, mods),
        Mode::Command(_) => command_key(ed, ev, mods),
        Mode::Normal | Mode::Visual => normal_key(ed, ev, mods),
    }
}

fn key_char(ev: &KeyEvent) -> Option<&'static str> {
    match &ev.logical_key {
        Key::Character(s) => s.as_str().chars().next().and_then(leak_char),
        _ => None,
    }
}

fn leak_char(c: char) -> Option<&'static str> {
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
        'u' => "u",
        '0' => "0",
        '$' => "$",
        '^' => "^",
        '%' => "%",
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

fn normal_key(ed: &mut Editor, ev: &KeyEvent, mods: ModifiersState) {
    if mods.alt_key() || mods.super_key() {
        return;
    }
    ed.lsp.hover = None;
    if ev.logical_key == Key::Named(NamedKey::Escape) {
        ed.clear_pending();
        if ed.mode == Mode::Visual {
            ed.mode = Mode::Normal;
            ed.visual_anchor = None;
        }
        return;
    }
    let k = key_char(ev);
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
    match ed.pending {
        Pending::G => {
            ed.pending = Pending::None;
            match k {
                Some("g") => ed.run_motion(Motion::GG, count),
                Some("e") => ed.run_motion(Motion::GE, count),
                Some("E") => ed.run_motion(Motion::BigGE, count),
                Some("d") => ed.goto_definition_cur(),
                _ => ed.count = 0,
            }
            return;
        }
        Pending::F { forward, to } => {
            ed.pending = Pending::None;
            if let Some(s) = plain_char(ev) {
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
            if let Some(s) = plain_char(ev) {
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
                    if let Some(mo) = motion_of(m) {
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
        visual_key(ed, ev, k, count);
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
        "i" => {
            ed.pending = Pending::TextObj { op: Operator::Change, inner: true };
        }
        "a" => {
            ed.pending = Pending::TextObj { op: Operator::Change, inner: false };
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
    ed.count = 0;
    ed.buf_mut().clamp();
}

fn visual_key(ed: &mut Editor, ev: &KeyEvent, k: &str, count: u32) {
    let _ = ev;
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
            ed.run_motion(motion_of(k).unwrap(), count);
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

fn insert_key(ed: &mut Editor, ev: &KeyEvent, mods: ModifiersState) {
    if mods.alt_key() || mods.super_key() {
        return;
    }
    let popup = ed.lsp.completion.is_some();
    if mods.control_key() {
        match key_char(ev) {
            Some("c") | Some("[") => ed.leave_insert(),
            Some("n") if popup => ed.comp_next(),
            Some("p") if popup => ed.comp_prev(),
            Some("y") | Some("e") if popup => ed.comp_accept(),
            _ => {}
        }
        return;
    }
    match &ev.logical_key {
        Key::Named(NamedKey::Escape) if popup => ed.comp_cancel(),
        Key::Named(NamedKey::Enter) if popup => ed.comp_accept(),
        Key::Named(NamedKey::Tab) if popup => ed.comp_accept(),
        Key::Named(NamedKey::Escape) => ed.leave_insert(),
        Key::Named(NamedKey::Enter) => {
            ed.comp_cancel();
            let co = ed.coalesce_open;
            ed.buf_mut().insert_text("\n", co);
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
        Key::Character(s)
            if s.chars().next().is_some_and(|c| !c.is_control()) => {
                let co = ed.coalesce_open;
                ed.buf_mut().insert_text(s, co);
                ed.maybe_completion(s);
            }
        _ => {}
    }
    ed.buf_mut().clamp();
}

fn plain_char(ev: &KeyEvent) -> Option<char> {
    match &ev.logical_key {
        Key::Character(s) => s.chars().next().filter(|c| !c.is_control()),
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

fn command_key(ed: &mut Editor, ev: &KeyEvent, mods: ModifiersState) {
    if mods.control_key() {
        match key_char(ev) {
            Some("c") | Some("[") => ed.close_cmdline(),
            _ => {}
        }
        return;
    }
    if mods.alt_key() || mods.super_key() {
        return;
    }
    match &ev.logical_key {
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
        _ => {}
    }
}

fn panel_key(ed: &mut Editor, ev: &KeyEvent, mods: ModifiersState) {
    if mods.control_key() {
        match key_char(ev) {
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
    match &ev.logical_key {
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
        _ => {}
    }
}
