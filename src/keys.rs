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

pub(crate) fn key_down(ed: &mut Editor, key: &Key, mods: ModifiersState) {
    if is_modifier(key) {
        return;
    }
    ed.context_menu = None;
    if *key == Key::Named(NamedKey::F1) {
        ed.toggle_spell();
        return;
    }
    if *key == Key::Named(NamedKey::F5) {
        ed.toggle_ignored();
        return;
    }

    if *key == Key::Named(NamedKey::F2) {
        ed.config.show_whitespace = !ed.config.show_whitespace;
        ed.set_status(
            format!("show whitespace: {}", ed.config.show_whitespace),
            false,
        );
        return;
    }
    if *key == Key::Named(NamedKey::F4) {
        ed.toggle_blame();
        return;
    }
    if *key == Key::Named(NamedKey::F3) {
        ed.config.theme = crate::config::flip_lightness(&ed.config.theme);
        let (theme, warns) = crate::config::RgbTheme::from_theme_checked(&ed.config.theme);
        ed.theme = theme;
        if let Some(w) = warns.first() {
            ed.status = Some(w.clone());
            ed.status_err = true;
        } else {
            let bg = ed.theme.bg;
            let lum = (((bg >> 16) & 0xFF) + ((bg >> 8) & 0xFF) + (bg & 0xFF)) / 3;
            let label = if lum > 127 { "light" } else { "dark" };
            ed.set_status(format!("{label} mode"), false);
        }
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
        'K' => "K",
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
        '#' => "#",
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
            ed.visual_block = false;
        }
        return;
    }
    let k = match key {
        Key::Named(NamedKey::ArrowLeft) => Some("h"),
        Key::Named(NamedKey::ArrowRight) => Some("l"),
        Key::Named(NamedKey::ArrowUp) => Some("k"),
        Key::Named(NamedKey::ArrowDown) => Some("j"),
        _ => key_char(key),
    };
    if mods.control_key() {
        let count = if ed.count == 0 { 1 } else { ed.count };
        match k {
            Some("u") => ed.run_motion(Motion::HalfPageUp, count),
            Some("d") => ed.run_motion(Motion::HalfPageDown, count),
            Some("p") => ed.open_panel(crate::fuzzy::Panel::Files),
            Some("b") => ed.open_panel(crate::fuzzy::Panel::Buffers),
            Some("f") => ed.run_motion(Motion::PageDown, count),
            Some("r") => ed.buf_mut().redo_action(),
            Some("v") => ed.start_visual_block(),
            Some("c") | Some("[") => ed.leave_insert(),
            Some("a") => {
                let c = ed.buf().cursor;
                let len = line_chars(&ed.buf().rope, c.line);
                ed.buf_mut().cursor = Pos {
                    line: c.line,
                    col: len,
                };
                ed.enter_insert();
            }
            Some("w") => {
                if ed.pending == Pending::WinCmd {
                    ed.pending = Pending::None;
                    ed.focus_next();
                } else {
                    ed.pending = Pending::WinCmd;
                }
            }
            _ => {}
        }
        ed.count = 0;
        ed.buf_mut().clamp();
        return;
    }
    if let Some(c) = k
        && let Some(d) = c.chars().next().and_then(|ch| ch.to_digit(10))
        && !matches!(
            ed.pending,
            Pending::F { .. } | Pending::R | Pending::TextObj { .. }
        )
    {
        if ed.count == 0 && d == 0 {
            if let Pending::Op(op) = ed.pending {
                ed.operator_motion(op, Motion::Zero, 1);
            } else {
                ed.run_motion(Motion::Zero, 1);
            }
        } else {
            ed.count = ed.count.saturating_mul(10).saturating_add(d).min(9999);
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
                Some("g") => ed.run_motion(g_to_line(count, ed.count, true), 1),
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
        Pending::OpG { op } => {
            ed.pending = Pending::None;
            if matches!(k, Some("g")) {
                ed.operator_motion(op, g_to_line(count, ed.count, true), count);
            }
            ed.count = 0;
            return;
        }
        Pending::WinCmd => {
            ed.pending = Pending::None;
            ed.count = 0;
            match k {
                Some("w") | Some("W") => {
                    ed.focus_next();
                }
                Some("c") | Some("q") => ed.close_window_cmd(),
                Some("s") => ed.split(crate::window::Axis::Rows),
                Some("v") => ed.split(crate::window::Axis::Columns),
                _ => {}
            }
            return;
        }
        Pending::Z => {
            ed.pending = Pending::None;
            if matches!(k, Some("z")) {
                let rows = ed.rows();
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
                for i in 0..count {
                    ed.replace_char(s);
                    if i + 1 < count {
                        let c = ed.buf().cursor;
                        if c.col >= line_chars(&ed.buf().rope, c.line) {
                            break;
                        }
                        ed.buf_mut().cursor = Pos {
                            line: c.line,
                            col: c.col + 1,
                        };
                    }
                }
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
                    ed.operator_object(op, inner, obj, count);
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
                Some("g") => {
                    ed.pending = Pending::OpG { op };
                    return;
                }
                Some("G") => {
                    ed.operator_motion(op, g_to_line(count, ed.count, false), count);
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
        "h" | "j" | "k" | "l" | "w" | "W" | "b" | "B" | "e" | "E" | "0" | "$" | "^" | "%" => {
            if let Some(m) = motion_of(k) {
                ed.run_motion(m, count);
            }
        }
        "G" => ed.run_motion(g_to_line(count, ed.count, false), 1),
        "g" => ed.pending = Pending::G,
        "z" => ed.pending = Pending::Z,
        "K" => ed.hover_cur(),
        "f" => {
            ed.pending = Pending::F {
                forward: true,
                to: true,
            }
        }
        "F" => {
            ed.pending = Pending::F {
                forward: false,
                to: true,
            }
        }
        "t" => {
            ed.pending = Pending::F {
                forward: true,
                to: false,
            }
        }
        "T" => {
            ed.pending = Pending::F {
                forward: false,
                to: false,
            }
        }
        ";" | "," => {
            if let Some((dir, ch)) = ed.last_find {
                let d = match (k, dir) {
                    (",", FindDir::F) => FindDir::BackF,
                    (",", FindDir::BackF) => FindDir::F,
                    (",", FindDir::T) => FindDir::BackT,
                    (",", FindDir::BackT) => FindDir::T,
                    (_, d) => d,
                };
                ed.run_find(d, ch, count);
            }
        }
        "n" => {
            ed.count = 0;
            ed.repeat_search(false);
            return;
        }
        "N" => {
            ed.count = 0;
            ed.repeat_search(true);
            return;
        }
        "*" => {
            ed.count = 0;
            ed.search_cur_word(false);
            return;
        }
        "#" => {
            ed.count = 0;
            ed.search_cur_word(true);
            return;
        }
        ":" => {
            ed.count = 0;
            ed.open_cmdline(crate::mode::CmdKind::Ex);
            return;
        }
        "/" => {
            ed.count = 0;
            ed.open_cmdline(crate::mode::CmdKind::Search);
            return;
        }
        "?" => {
            ed.count = 0;
            ed.open_cmdline(crate::mode::CmdKind::SearchBack);
            return;
        }
        "d" | "c" | "y" => ed.pending = Pending::Op(op_of(k)),
        ">" => ed.pending = Pending::Op(Operator::Indent),
        "<" => ed.pending = Pending::Op(Operator::Outdent),
        "x" => ed.operator_motion(Operator::Delete, Motion::L, count),
        "s" => ed.operator_motion(Operator::Change, Motion::L, count),
        "p" => {
            for _ in 0..count {
                ed.paste(true);
            }
        }
        "P" => {
            for _ in 0..count {
                ed.paste(false);
            }
        }
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
            ed.buf_mut().cursor = Pos {
                line: c.line,
                col: (c.col + 1).min(len),
            };
            ed.enter_insert();
        }
        "I" => {
            ed.run_motion(Motion::Caret, 1);
            ed.enter_insert();
        }
        "A" => {
            let c = ed.buf().cursor;
            let len = line_chars(&ed.buf().rope, c.line);
            ed.buf_mut().cursor = Pos {
                line: c.line,
                col: len,
            };
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

fn g_to_line(count: u32, ed_count: u32, gg: bool) -> Motion {
    if ed_count > 0 {
        Motion::ToLine(count.saturating_sub(1) as usize)
    } else if gg {
        Motion::GG
    } else {
        Motion::G
    }
}

fn visual_key(ed: &mut Editor, k: &str, count: u32) {
    match k {
        "v" => {
            if ed.visual_block {
                ed.visual_block = false;
            } else {
                ed.mode = Mode::Normal;
                ed.visual_anchor = None;
            }
        }
        "V" => {
            ed.visual_block = false;
            ed.visual_linewise = !ed.visual_linewise;
        }
        "h" | "j" | "k" | "l" | "w" | "W" | "b" | "B" | "e" | "E" | "0" | "^" | "$" | "%" => {
            let Some(plain) = motion_of(k) else { return };
            let m = match (ed.visual_linewise, plain) {
                (true, Motion::J) => Motion::LineDown,
                (true, Motion::K) => Motion::LineUp,
                (_, other) => other,
            };
            ed.run_motion(m, count);
        }
        "G" => ed.run_motion(g_to_line(count, ed.count, false), 1),
        "i" => {
            ed.pending = Pending::TextObj {
                op: Operator::Change,
                inner: true,
            }
        }
        "a" => {
            ed.pending = Pending::TextObj {
                op: Operator::Change,
                inner: false,
            }
        }
        "d" | "c" | "x" | "y" => {
            if ed.visual_block {
                ed.block_apply(op_of(k));
            } else {
                let ((a, b), lw) = ed.sel_range();
                ed.op_apply(op_of(k), (a, b), lw);
                ed.visual_anchor = None;
            }
        }
        ">" | "<" => {
            let op = if k == ">" {
                Operator::Indent
            } else {
                Operator::Outdent
            };
            let ((a, b), _) = ed.sel_range();
            ed.op_apply(op, (a, b), true);
            ed.mode = Mode::Normal;
            ed.visual_anchor = None;
        }
        "p" => {
            let (reg, lw, blk) = (ed.register.clone(), ed.reg_linewise, ed.reg_block);
            if ed.visual_block {
                ed.block_apply(Operator::Delete);
            } else {
                let ((a, b), _) = ed.sel_range();
                if a < b {
                    ed.buf_mut().delete_text(a, b);
                }
                ed.mode = Mode::Normal;
                ed.visual_anchor = None;
            }
            ed.register = reg;
            ed.reg_linewise = lw;
            ed.reg_block = blk;
            // The selection is gone and the cursor sits where it started, so
            // the register goes in there rather than after it.
            ed.paste(false);
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
        Key::Named(NamedKey::ArrowUp) if popup => ed.comp_prev(),
        Key::Named(NamedKey::ArrowDown) if popup => ed.comp_next(),
        Key::Named(NamedKey::ArrowLeft) => {
            ed.comp_cancel();
            ed.move_caret(crate::motions::Caret::Left);
        }
        Key::Named(NamedKey::ArrowRight) => {
            ed.comp_cancel();
            ed.move_caret(crate::motions::Caret::Right);
        }
        Key::Named(NamedKey::ArrowUp) => ed.move_caret(crate::motions::Caret::Up),
        Key::Named(NamedKey::ArrowDown) => ed.move_caret(crate::motions::Caret::Down),
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
        Key::Character(s) if s.chars().next().is_some_and(|c| !c.is_control()) => {
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
        "d" | "x" => Operator::Delete,
        "c" => Operator::Change,
        _ => Operator::Yank,
    }
}

fn op_char(op: Operator) -> &'static str {
    match op {
        Operator::Delete => "d",
        Operator::Change => "c",
        Operator::Yank => "y",
        Operator::Indent => ">",
        Operator::Outdent => "<",
    }
}

fn obj_of(k: &str) -> Option<Obj> {
    let c = k.chars().next()?;
    Some(match c {
        'w' => Obj::Word,
        'W' => Obj::BigWord,
        '"' | '\'' | '`' => Obj::Quote(c),
        '(' | ')' => Obj::Bracket('('),
        '[' | ']' => Obj::Bracket('['),
        '{' | '}' => Obj::Bracket('{'),
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
    let ex = ed.cmd_kind == Some(crate::mode::CmdKind::Ex);
    match key {
        Key::Named(NamedKey::Escape) if ed.completions.is_some() => ed.completions = None,
        Key::Named(NamedKey::Escape) => ed.close_cmdline(),
        Key::Named(NamedKey::Enter) => match ed.cmd_kind {
            Some(crate::mode::CmdKind::Ex) => ed.execute_cmdline(),
            Some(crate::mode::CmdKind::Search) => ed.commit_search(false),
            Some(crate::mode::CmdKind::SearchBack) => ed.commit_search(true),
            None => ed.close_cmdline(),
        },
        Key::Named(NamedKey::Tab) if ex => {
            if mods.shift_key() {
                ed.completion_move(-1);
            } else {
                ed.complete_next();
            }
        }
        Key::Named(NamedKey::ArrowDown) if ex => ed.completion_move(1),
        Key::Named(NamedKey::ArrowUp) if ex => ed.completion_move(-1),
        Key::Named(NamedKey::ArrowLeft) => ed.cmdline_move(-1),
        Key::Named(NamedKey::ArrowRight) => ed.cmdline_move(1),
        Key::Named(NamedKey::Backspace) => ed.cmdline_backspace(),
        Key::Named(NamedKey::Delete) => ed.cmdline_delete(),
        Key::Character(s) => {
            if let Some(c) = s.chars().next()
                && !c.is_control()
            {
                ed.cmdline_insert(c);
            }
        }
        Key::Named(NamedKey::Space) => ed.cmdline_insert(' '),
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
                && !c.is_control()
            {
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
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("ab", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        ed.enter_insert();
        type_keys(
            &mut ed,
            &[Key::Named(NamedKey::Space), Key::Character("x".into())],
        );
        assert_eq!(ed.buf().rope.to_string(), "a xb");
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 3 });
    }

    #[test]
    fn count_stays_bounded() {
        let mut ed = Editor::test_new();
        let nines: Vec<Key> = (0..10).map(|_| Key::Character("9".into())).collect();
        type_keys(&mut ed, &nines);
        assert!(ed.count <= 9999, "count must not run away: {}", ed.count);
    }

    #[test]
    fn visual_paste_needs_a_selection() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("ab\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 2 };
        ed.register.clear();
        let undo = ed.buffers[0].undo.len();
        type_keys(
            &mut ed,
            &[Key::Character("v".into()), Key::Character("p".into())],
        );
        assert_eq!(
            ed.buffers[0].undo.len(),
            undo,
            "pasting over nothing must not be recorded as an edit"
        );
        assert_eq!(ed.buf().rope.to_string(), "ab\n");
    }

    #[test]
    fn insert_and_append_place_the_cursor_like_vim() {
        let mut ed = Editor::test_new();
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
        assert_eq!(ed.buf().rope.to_string(), "axyb\n");

        type_keys(&mut ed, &[Key::Named(NamedKey::Escape)]);
        ed.buf_mut().cursor = Pos { line: 0, col: 3 };
        type_keys(
            &mut ed,
            &[
                Key::Character("a".into()),
                Key::Character("z".into()),
                Key::Named(NamedKey::Escape),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "axybz\n");
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 4 });
        assert_eq!(ed.mode, Mode::Normal);
    }

    #[test]
    fn operators_still_take_text_objects() {
        let mut ed = Editor::test_new();
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
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
        ed.buf_mut().insert_text(&body, false);
        ed.win_mut().view_rows = 10;
        ed.buf_mut().cursor = Pos { line: 0, col: 3 };
        type_keys(&mut ed, &[Key::Named(NamedKey::PageDown)]);
        assert_eq!(ed.buf().cursor, Pos { line: 10, col: 3 });
        type_keys(&mut ed, &[Key::Named(NamedKey::PageUp)]);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 3 });
        type_keys(
            &mut ed,
            &[Key::Character("2".into()), Key::Named(NamedKey::PageDown)],
        );
        assert_eq!(ed.buf().cursor.line, 20);
        assert_eq!(ed.count, 0);
        type_keys(
            &mut ed,
            &[Key::Character("d".into()), Key::Named(NamedKey::PageUp)],
        );
        assert_eq!(ed.buf().rope.len_lines(), 30);
        assert_eq!(ed.buf().rope.line(10).to_string(), "line 21\n");
        assert_eq!(ed.mode, Mode::Normal);
    }

    #[test]
    fn zz_centers_the_viewport_on_the_cursor() {
        let mut ed = Editor::test_new();
        let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
        ed.buf_mut().insert_text(&body, false);
        ed.win_mut().view_rows = 10;
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
            ed.wrap()
                .screen_row(&b.rope, b.view_top, b.view_sub, b.cursor),
            Some(5)
        );
        assert_eq!(ed.pending, Pending::None);
    }

    #[test]
    fn z_then_another_key_leaves_the_view_alone() {
        let mut ed = Editor::test_new();
        let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
        ed.buf_mut().insert_text(&body, false);
        ed.win_mut().view_rows = 10;
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
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one two three four\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[Key::Character("3".into()), Key::Character("w".into())],
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
        let mut ed = Editor::test_new();
        ed.config.tab_width = 4;
        ed.buf_mut().insert_text("int main() {\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(&mut ed, &[Key::Character("o".into())]);
        assert_eq!(ed.mode, Mode::Insert);
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 4 });
        type_keys(
            &mut ed,
            &[Key::Character("f".into()), Key::Named(NamedKey::Escape)],
        );
        assert_eq!(ed.buf().rope.to_string(), "int main() {\n    f\n");
    }

    #[test]
    fn open_above_copies_the_line_below_indent() {
        let mut ed = Editor::test_new();
        ed.config.tab_width = 4;
        ed.buf_mut().insert_text("    foo();\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 2 };
        type_keys(&mut ed, &[Key::Character("O".into())]);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 4 });
        assert_eq!(ed.buf().rope.to_string(), "    \n    foo();\n");

        let mut closing = Editor::test_new();
        closing.config.tab_width = 4;
        closing.buf_mut().insert_text("    foo();\n    }\n", false);
        closing.buf_mut().cursor = Pos { line: 1, col: 0 };
        type_keys(&mut closing, &[Key::Character("O".into())]);
        assert_eq!(closing.buf().rope.to_string(), "    foo();\n\n    }\n");
    }

    #[test]
    fn enter_in_insert_mode_repeats_the_indent() {
        let mut ed = Editor::test_new();
        ed.config.tab_width = 4;
        ed.buf_mut().insert_text("    foo();\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 10 };
        ed.enter_insert();
        type_keys(
            &mut ed,
            &[Key::Named(NamedKey::Enter), Key::Character("x".into())],
        );
        assert_eq!(ed.buf().rope.to_string(), "    foo();\n    x\n");
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 5 });
    }

    #[test]
    fn typing_a_leading_closing_brace_dedents_the_line() {
        let mut ed = Editor::test_new();
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

        let mut mid = Editor::test_new();
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
        let mut ed = Editor::test_new();
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
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("a b", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[Key::Character("f".into()), Key::Named(NamedKey::Space)],
        );
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 1 });
        assert_eq!(ed.last_find, Some((FindDir::F, ' ')));
        type_keys(&mut ed, &[Key::Character("l".into())]);
        type_keys(
            &mut ed,
            &[Key::Character("r".into()), Key::Named(NamedKey::Space)],
        );
        assert_eq!(ed.buf().rope.to_string(), "a  ");
    }

    #[test]
    fn star_key_searches_forward_from_the_cursor() {
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("x foo bar foo\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 2 };
        type_keys(&mut ed, &[Key::Character("*".into())]);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 10 });
        assert_eq!(ed.search.as_deref(), Some("foo"));
    }

    #[test]
    fn normal_key_clears_the_echo_message() {
        let mut ed = Editor::test_new();
        ed.set_status("E37: No write since last change".to_string(), true);
        type_keys(&mut ed, &[Key::Character("l".into())]);
        assert_eq!(ed.status, None);
    }

    #[test]
    fn shift_between_find_and_target_keeps_the_pending_find() {
        let mut ed = Editor::test_new();
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
        let mut ed = Editor::test_new();
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

    #[test]
    fn any_key_press_dismisses_the_context_menu() {
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("hello\n", false);
        ed.context_menu = Some("gd   go to definition".to_string());
        type_keys(&mut ed, &[Key::Named(NamedKey::Shift)]);
        assert!(ed.context_menu.is_some());
        type_keys(&mut ed, &[Key::Character("j".into())]);
        assert!(ed.context_menu.is_none());
    }

    #[test]
    fn f2_toggles_whitespace_visibility_in_every_mode() {
        let mut ed = Editor::test_new();
        ed.config.show_whitespace = false;
        type_keys(&mut ed, &[Key::Named(NamedKey::F2)]);
        assert!(ed.config.show_whitespace);
        assert_eq!(ed.status.as_deref(), Some("show whitespace: true"));
        ed.mode = Mode::Insert;
        type_keys(&mut ed, &[Key::Named(NamedKey::F2)]);
        assert!(!ed.config.show_whitespace);
        assert!(ed.buf().rope.to_string().is_empty());
        ed.mode = Mode::Normal;
        type_keys(&mut ed, &[Key::Named(NamedKey::F2)]);
        assert!(ed.config.show_whitespace);
    }

    #[test]
    fn f3_swaps_theme_to_light_and_back() {
        let mut ed = Editor::test_new();
        ed.config.theme = crate::config::Theme::default();
        ed.theme = crate::config::RgbTheme::from_theme_checked(&ed.config.theme).0;
        let dark_cfg = ed.config.theme.clone();
        let dark_bg = ed.theme.bg;
        type_keys(&mut ed, &[Key::Named(NamedKey::F3)]);
        assert_ne!(ed.config.theme, dark_cfg);
        assert!(ed.theme.bg > dark_bg);
        assert_eq!(ed.status.as_deref(), Some("light mode"));
        type_keys(&mut ed, &[Key::Named(NamedKey::F3)]);
        assert_eq!(ed.config.theme, dark_cfg);
        assert_eq!(ed.theme.bg, dark_bg);
        assert_eq!(ed.status.as_deref(), Some("dark mode"));
    }

    #[test]
    fn ctrl_v_enters_and_leaves_block_visual() {
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("alpha\nbeta\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        key_down(
            &mut ed,
            &Key::Character("v".into()),
            ModifiersState::CONTROL,
        );
        assert_eq!(ed.mode, Mode::Visual);
        assert!(ed.visual_block);
        assert_eq!(ed.block_range(), Some((0, 0, 0, 0)));
        ed.buf_mut().cursor = Pos { line: 1, col: 2 };
        assert_eq!(ed.block_range(), Some((0, 1, 0, 2)));
        key_down(
            &mut ed,
            &Key::Character("v".into()),
            ModifiersState::CONTROL,
        );
        assert_eq!(ed.mode, Mode::Normal);
        assert!(!ed.visual_block);
        assert_eq!(ed.visual_anchor, None);
    }

    #[test]
    fn block_yank_pads_short_lines_into_the_register() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("alpha\nbeta\ngamma\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        key_down(
            &mut ed,
            &Key::Character("v".into()),
            ModifiersState::CONTROL,
        );
        ed.buf_mut().cursor = Pos { line: 2, col: 3 };
        type_keys(&mut ed, &[Key::Character("y".into())]);
        assert_eq!(ed.register, "lph\neta\namm");
        assert!(ed.reg_block);
        assert!(!ed.reg_linewise);
        assert_eq!(ed.buf().rope.to_string(), "alpha\nbeta\ngamma\n");
        assert_eq!(ed.mode, Mode::Normal);
    }

    #[test]
    fn block_delete_clamps_to_short_lines() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("alpha\nx\nb\ngamma\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        key_down(
            &mut ed,
            &Key::Character("v".into()),
            ModifiersState::CONTROL,
        );
        ed.buf_mut().cursor = Pos { line: 3, col: 3 };
        type_keys(&mut ed, &[Key::Character("d".into())]);
        assert_eq!(ed.buf().rope.to_string(), "aa\nx\nb\nga\n");
        assert_eq!(ed.register, "lph\n   \n   \namm");
        assert!(ed.reg_block);
    }

    #[test]
    fn block_paste_inserts_each_segment_on_its_line() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("aaa\nbbb\nccc\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.register = "XY\nYZ".into();
        ed.reg_block = true;
        ed.paste(true);
        assert_eq!(ed.buf().rope.to_string(), "aXYaa\nbYZbb\nccc\n");
    }

    #[test]
    fn block_paste_pads_lines_shorter_than_the_column() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("q\nx\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 2 };
        ed.register = "AB".into();
        ed.reg_block = true;
        ed.paste(true);
        assert_eq!(ed.buf().rope.to_string(), "q  AB\nx\n");
    }

    #[test]
    fn block_change_yanks_the_block_and_enters_insert() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("alpha\nbeta\n", false);
        ed.register = "keep".into();
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        key_down(
            &mut ed,
            &Key::Character("v".into()),
            ModifiersState::CONTROL,
        );
        ed.buf_mut().cursor = Pos { line: 1, col: 1 };
        type_keys(&mut ed, &[Key::Character("c".into())]);
        assert_eq!(ed.buf().rope.to_string(), "apha\nbta\n");
        assert_eq!(ed.register, "l\ne");
        assert_eq!(ed.mode, Mode::Insert);
    }

    #[test]
    fn v_converts_block_to_charwise_and_escape_exits() {
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("alpha\nbeta\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        key_down(
            &mut ed,
            &Key::Character("v".into()),
            ModifiersState::CONTROL,
        );
        ed.buf_mut().cursor = Pos { line: 1, col: 3 };
        type_keys(&mut ed, &[Key::Character("v".into())]);
        assert_eq!(ed.mode, Mode::Visual);
        assert!(!ed.visual_block);
        assert_eq!(ed.visual_anchor, Some(Pos { line: 0, col: 1 }));
        type_keys(&mut ed, &[Key::Named(NamedKey::Escape)]);
        assert_eq!(ed.mode, Mode::Normal);
        assert_eq!(ed.visual_anchor, None);
    }

    #[test]
    fn operator_capital_g_deletes_to_end_of_file() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one\ntwo\nthree\n", false);
        ed.buf_mut().cursor = Pos { line: 1, col: 0 };
        type_keys(
            &mut ed,
            &[Key::Character("d".into()), Key::Character("G".into())],
        );
        assert_eq!(ed.buf().rope.to_string(), "one\n");
        assert_eq!(ed.register, "two\nthree\n");
    }

    #[test]
    fn operator_gg_deletes_to_first_line() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one\ntwo\nthree\n", false);
        ed.buf_mut().cursor = Pos { line: 1, col: 0 };
        type_keys(
            &mut ed,
            &[
                Key::Character("y".into()),
                Key::Character("g".into()),
                Key::Character("g".into()),
            ],
        );
        assert_eq!(ed.register, "one\ntwo\n");
        assert_eq!(ed.buf().rope.to_string(), "one\ntwo\nthree\n");
        type_keys(
            &mut ed,
            &[
                Key::Character("d".into()),
                Key::Character("g".into()),
                Key::Character("g".into()),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "three\n");
    }

    #[test]
    fn operator_g_then_other_key_cancels() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one\ntwo\n", false);
        type_keys(
            &mut ed,
            &[
                Key::Character("d".into()),
                Key::Character("g".into()),
                Key::Character("x".into()),
            ],
        );
        assert_eq!(ed.pending, Pending::None);
        assert_eq!(ed.buf().rope.to_string(), "one\ntwo\n");
    }

    #[test]
    fn f1_toggles_spellcheck() {
        let mut ed = Editor::test_new();
        assert!(!ed.config.spellcheck);
        type_keys(&mut ed, &[Key::Named(NamedKey::F1)]);
        assert!(ed.config.spellcheck);
        assert_eq!(ed.status.as_deref(), Some("spellcheck: on"));
        type_keys(&mut ed, &[Key::Named(NamedKey::F1)]);
        assert!(!ed.config.spellcheck);
        assert_eq!(ed.status.as_deref(), Some("spellcheck: off"));
    }

    #[test]
    fn f5_toggles_ignored_files() {
        let mut ed = Editor::test_new();
        assert!(!ed.show_ignored);
        type_keys(&mut ed, &[Key::Named(NamedKey::F5)]);
        assert!(ed.show_ignored);
        assert_eq!(ed.status.as_deref(), Some("ignored files: shown"));
        type_keys(&mut ed, &[Key::Named(NamedKey::F5)]);
        assert!(!ed.show_ignored);
        assert_eq!(ed.status.as_deref(), Some("ignored files: hidden"));
    }

    #[test]
    fn f4_toggles_git_blame() {
        let mut ed = Editor::test_new();
        assert!(!ed.show_blame);
        type_keys(&mut ed, &[Key::Named(NamedKey::F4)]);
        assert!(ed.show_blame);
        assert_eq!(ed.status.as_deref(), Some("git blame: on"));
        type_keys(&mut ed, &[Key::Named(NamedKey::F4)]);
        assert!(!ed.show_blame);
        assert_eq!(ed.status.as_deref(), Some("git blame: off"));
    }

    #[test]
    fn arrow_keys_move_the_cursor_in_normal_mode() {
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("one\ntwo\nthree\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(&mut ed, &[Key::Named(NamedKey::ArrowDown)]);
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 0 });
        type_keys(&mut ed, &[Key::Named(NamedKey::ArrowRight)]);
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 1 });
        type_keys(
            &mut ed,
            &[Key::Character("2".into()), Key::Named(NamedKey::ArrowUp)],
        );
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 1 });
        type_keys(&mut ed, &[Key::Named(NamedKey::ArrowDown)]);
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 1 });
        type_keys(&mut ed, &[Key::Named(NamedKey::ArrowLeft)]);
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 0 });
    }

    #[test]
    fn arrow_keys_extend_the_visual_selection() {
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("one\ntwo\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[
                Key::Character("v".into()),
                Key::Named(NamedKey::ArrowDown),
                Key::Named(NamedKey::ArrowRight),
            ],
        );
        assert_eq!(ed.mode, Mode::Visual);
        assert_eq!(ed.visual_anchor, Some(Pos { line: 0, col: 0 }));
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 1 });
    }

    #[test]
    fn arrow_keys_move_the_caret_in_insert_mode() {
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("ab\ncd\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[Key::Character("i".into()), Key::Named(NamedKey::ArrowLeft)],
        );
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 0 });
        for _ in 0..3 {
            type_keys(&mut ed, &[Key::Named(NamedKey::ArrowRight)]);
        }
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 0 });
        type_keys(&mut ed, &[Key::Character("X".into())]);
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 1 });
        assert_eq!(ed.buf().rope.to_string(), "ab\nXcd\n");
        type_keys(
            &mut ed,
            &[Key::Named(NamedKey::ArrowLeft), Key::Character("Y".into())],
        );
        assert_eq!(ed.buf().rope.to_string(), "ab\nYXcd\n");
        type_keys(&mut ed, &[Key::Named(NamedKey::ArrowUp)]);
        assert_eq!(ed.buf().cursor.line, 0);
    }

    #[test]
    fn arrow_keys_move_the_command_line_caret() {
        let mut ed = Editor::test_new();
        ed.open_cmdline(crate::mode::CmdKind::Ex);
        type_keys(
            &mut ed,
            &[Key::Character("w".into()), Key::Character("q".into())],
        );
        assert_eq!(ed.cmdline, "wq");
        assert_eq!(ed.cmdline_col, 2);
        type_keys(
            &mut ed,
            &[
                Key::Named(NamedKey::ArrowLeft),
                Key::Named(NamedKey::ArrowLeft),
            ],
        );
        assert_eq!(ed.cmdline_col, 0);
        type_keys(&mut ed, &[Key::Character("x".into())]);
        assert_eq!(ed.cmdline, "xwq");
        assert_eq!(ed.cmdline_col, 1);
        type_keys(
            &mut ed,
            &[
                Key::Named(NamedKey::ArrowRight),
                Key::Named(NamedKey::ArrowRight),
                Key::Named(NamedKey::ArrowRight),
            ],
        );
        assert_eq!(ed.cmdline_col, 3);
        type_keys(&mut ed, &[Key::Named(NamedKey::ArrowRight)]);
        assert_eq!(ed.cmdline_col, 3);
    }

    #[test]
    fn command_line_inserts_and_deletes_at_the_caret() {
        let mut ed = Editor::test_new();
        ed.open_cmdline(crate::mode::CmdKind::Ex);
        for c in "abc".chars() {
            type_keys(&mut ed, &[Key::Character(c.to_string().into())]);
        }
        type_keys(
            &mut ed,
            &[
                Key::Named(NamedKey::ArrowLeft),
                Key::Named(NamedKey::ArrowLeft),
            ],
        );
        assert_eq!(ed.cmdline_col, 1);
        type_keys(&mut ed, &[Key::Named(NamedKey::Backspace)]);
        assert_eq!((ed.cmdline.as_str(), ed.cmdline_col), ("bc", 0));
        type_keys(&mut ed, &[Key::Named(NamedKey::Backspace)]);
        assert_eq!(ed.cmdline, "bc");
        type_keys(&mut ed, &[Key::Named(NamedKey::Delete)]);
        assert_eq!(ed.cmdline, "c");
        type_keys(&mut ed, &[Key::Named(NamedKey::ArrowRight)]);
        type_keys(&mut ed, &[Key::Named(NamedKey::Delete)]);
        assert_eq!(ed.cmdline, "c");
    }

    #[test]
    fn arrows_pick_completion_popup_items_in_insert_mode() {
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("wor\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 3 };
        ed.enter_insert();
        ed.lsp.completion = Some(crate::lsp::CompletionState {
            items: vec![
                crate::lsp::CompItem {
                    kind: 'f',
                    text: "word".into(),
                },
                crate::lsp::CompItem {
                    kind: 'f',
                    text: "work".into(),
                },
            ],
            selected: 0,
            line: 0,
            col: 3,
            seq: 0,
            prefix: "wor".into(),
        });
        type_keys(&mut ed, &[Key::Named(NamedKey::ArrowDown)]);
        assert_eq!(ed.lsp.completion.as_ref().unwrap().selected, 1);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 3 });
        type_keys(&mut ed, &[Key::Named(NamedKey::ArrowLeft)]);
        assert!(ed.lsp.completion.is_none());
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 2 });
    }

    #[test]
    fn comma_repeats_find_in_reverse() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("a b a b a\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[Key::Character("f".into()), Key::Character("a".into())],
        );
        assert_eq!(ed.buf().cursor.col, 4);
        type_keys(&mut ed, &[Key::Character(";".into())]);
        assert_eq!(ed.buf().cursor.col, 8);
        type_keys(&mut ed, &[Key::Character(",".into())]);
        assert_eq!(ed.buf().cursor.col, 4);
        type_keys(&mut ed, &[Key::Character(",".into())]);
        assert_eq!(ed.buf().cursor.col, 0);
    }

    #[test]
    fn double_gt_and_lt_indent_a_line() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("hello\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[Key::Character(">".into()), Key::Character(">".into())],
        );
        assert_eq!(ed.buf().rope.to_string(), "    hello\n");
        type_keys(
            &mut ed,
            &[Key::Character("<".into()), Key::Character("<".into())],
        );
        assert_eq!(ed.buf().rope.to_string(), "hello\n");
    }

    #[test]
    fn hash_key_searches_word_backward() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("bar x bar", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 6 };
        type_keys(&mut ed, &[Key::Character("#".into())]);
        assert_eq!(ed.buf().cursor.col, 0);
    }

    #[test]
    fn d_zero_deletes_to_line_start() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("hello\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 2 };
        type_keys(
            &mut ed,
            &[Key::Character("d".into()), Key::Character("0".into())],
        );
        assert_eq!(ed.buf().rope.to_string(), "llo\n");
        assert_eq!(ed.register, "he");
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 0 });
        assert_eq!(ed.pending, Pending::None);
    }

    #[test]
    fn digits_work_as_find_and_replace_targets() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("a1b2\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[Key::Character("f".into()), Key::Character("1".into())],
        );
        assert_eq!(ed.buf().cursor.col, 1);
        type_keys(
            &mut ed,
            &[Key::Character("r".into()), Key::Character("7".into())],
        );
        assert_eq!(ed.buf().rope.to_string(), "a7b2\n");
    }

    #[test]
    fn count_repeats_paste() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("abcd\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.register = "x".into();
        type_keys(
            &mut ed,
            &[Key::Character("3".into()), Key::Character("p".into())],
        );
        assert_eq!(ed.buf().rope.to_string(), "axxxbcd\n");
    }

    fn press(ed: &mut Editor, s: &str) {
        for c in s.chars() {
            let mut buf = [0u8; 4];
            type_keys(ed, &[Key::Character(c.encode_utf8(&mut buf).into())]);
        }
    }

    fn esc(ed: &mut Editor) {
        type_keys(ed, &[Key::Named(NamedKey::Escape)]);
    }

    #[test]
    fn change_line_in_the_middle_keeps_the_line() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("keep\ndrop\nmore\n", false);
        ed.buf_mut().cursor = Pos { line: 1, col: 2 };
        press(&mut ed, "cc");
        press(&mut ed, "X");
        esc(&mut ed);
        assert_eq!(ed.buf().rope.to_string(), "keep\nX\nmore\n");
    }

    #[test]
    fn linewise_paste_repeats_whole_lines() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("x\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.register = "L\n".into();
        ed.reg_linewise = true;
        press(&mut ed, "2p");
        assert_eq!(ed.buf().rope.to_string(), "x\nL\nL\n");
    }

    #[test]
    fn charwise_paste_before_repeats_in_order() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("abcd\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.register = "XY".into();
        press(&mut ed, "2P");
        assert_eq!(ed.buf().rope.to_string(), "XYXYabcd\n");
    }

    #[test]
    fn visual_paste_replaces_the_selection() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("hello world", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.register = "XY".into();
        press(&mut ed, "vll");
        press(&mut ed, "p");
        assert_eq!(ed.buf().rope.to_string(), "XYlo world");
    }

    #[test]
    fn visual_line_paste_replaces_the_line() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("a\nb\nc\n", false);
        ed.buf_mut().cursor = Pos { line: 1, col: 0 };
        ed.register = "X\n".into();
        ed.reg_linewise = true;
        press(&mut ed, "Vp");
        assert_eq!(ed.buf().rope.to_string(), "a\nX\nc\n");
    }

    #[test]
    fn joining_lines_can_be_undone() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("aaa\nbbb\nccc\nddd\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        press(&mut ed, "3J");
        assert_eq!(ed.buf().rope.to_string(), "aaa bbb ccc\nddd\n");
        press(&mut ed, "u");
        assert_eq!(ed.buf().rope.to_string(), "aaa\nbbb\nccc\nddd\n");
    }

    #[test]
    fn a_join_count_counts_the_lines_that_take_part() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("aaa\nbbb\nccc\nddd\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        press(&mut ed, "2J");
        assert_eq!(ed.buf().rope.to_string(), "aaa bbb\nccc\nddd\n");
        press(&mut ed, "2J");
        assert_eq!(ed.buf().rope.to_string(), "aaa bbb ccc\nddd\n");
    }

    #[test]
    fn indenting_a_line_can_be_undone() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("hello\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        press(&mut ed, ">>");
        press(&mut ed, "u");
        assert_eq!(ed.buf().rope.to_string(), "hello\n");
    }

    #[test]
    fn two_insert_sessions_undo_separately() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("ab\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        press(&mut ed, "i");
        press(&mut ed, "xx");
        esc(&mut ed);
        press(&mut ed, "Ay");
        esc(&mut ed);
        assert_eq!(ed.buf().rope.to_string(), "xxaby\n");
        press(&mut ed, "u");
        assert_eq!(ed.buf().rope.to_string(), "xxab\n");
    }

    #[test]
    fn a_count_before_n_does_not_leak_into_the_next_command() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one two three one", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.search = Some("one".into());
        press(&mut ed, "3n");
        assert_eq!(ed.count, 0);
        press(&mut ed, "x");
        assert_eq!(ed.buf().rope.to_string(), "one two three ne");
    }

    #[test]
    fn typing_after_a_line_change_undoes_with_the_change() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one\ntwo\nthree\n", false);
        ed.buf_mut().cursor = Pos { line: 1, col: 1 };
        press(&mut ed, "cc");
        press(&mut ed, "new");
        esc(&mut ed);
        assert_eq!(ed.buf().rope.to_string(), "one\nnew\nthree\n");
        press(&mut ed, "u");
        assert_eq!(ed.buf().rope.to_string(), "one\ntwo\nthree\n");
    }

    #[test]
    fn typing_after_a_word_change_undoes_with_the_change() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one two\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        press(&mut ed, "ciw");
        press(&mut ed, "X");
        esc(&mut ed);
        assert_eq!(ed.buf().rope.to_string(), "X two\n");
        press(&mut ed, "u");
        assert_eq!(ed.buf().rope.to_string(), "one two\n");
    }

    #[test]
    fn a_later_insert_undoes_apart_from_an_earlier_change() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one\ntwo\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        press(&mut ed, "cc");
        press(&mut ed, "new");
        esc(&mut ed);
        press(&mut ed, "A");
        press(&mut ed, "!");
        esc(&mut ed);
        assert_eq!(ed.buf().rope.to_string(), "new!\ntwo\n");
        press(&mut ed, "u");
        assert_eq!(ed.buf().rope.to_string(), "new\ntwo\n");
    }

    #[test]
    fn count_replaces_several_chars() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("abcd\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[
                Key::Character("3".into()),
                Key::Character("r".into()),
                Key::Character("!".into()),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "!!!d\n");
        assert_eq!(ed.buf().cursor.col, 2);
    }

    #[test]
    fn count_deletes_several_chars() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("abcd\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[Key::Character("2".into()), Key::Character("x".into())],
        );
        assert_eq!(ed.buf().rope.to_string(), "cd\n");
    }

    #[test]
    fn closing_bracket_text_objects_work() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("(abc)\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 2 };
        type_keys(
            &mut ed,
            &[
                Key::Character("d".into()),
                Key::Character("i".into()),
                Key::Character(")".into()),
            ],
        );
        assert_eq!(ed.buf().rope.to_string(), "()\n");
    }

    #[test]
    fn visual_change_enters_insert_mode() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("hello\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(
            &mut ed,
            &[
                Key::Character("v".into()),
                Key::Character("l".into()),
                Key::Character("c".into()),
            ],
        );
        assert_eq!(ed.mode, Mode::Insert);
        assert_eq!(ed.visual_anchor, None);
    }

    #[test]
    fn esc_from_cmdline_clears_count() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one two three\n", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        type_keys(&mut ed, &[Key::Character("5".into())]);
        type_keys(&mut ed, &[Key::Character(":".into())]);
        type_keys(&mut ed, &[Key::Named(NamedKey::Escape)]);
        assert_eq!(ed.count, 0);
        type_keys(&mut ed, &[Key::Character("w".into())]);
        assert_eq!(ed.buf().cursor.col, 4);
    }

    #[test]
    fn count_g_jumps_to_line() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("l1\nl2\nl3\nl4\nl5\nl6", false);
        type_keys(&mut ed, &["5", "G"].map(|s| Key::Character(s.into())));
        assert_eq!(ed.buf().cursor.line, 4);
        type_keys(&mut ed, &["g", "g"].map(|s| Key::Character(s.into())));
        assert_eq!(ed.buf().cursor.line, 0);
        type_keys(&mut ed, &["3", "g", "g"].map(|s| Key::Character(s.into())));
        assert_eq!(ed.buf().cursor.line, 2);
        type_keys(&mut ed, &["G"].map(|s| Key::Character(s.into())));
        assert_eq!(ed.buf().cursor.line, 5);
    }

    #[test]
    fn di_quote_with_cursor_on_closing_quote() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("hi \"bob\" ok", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 7 };
        type_keys(&mut ed, &["d", "i", "\""].map(|s| Key::Character(s.into())));
        assert_eq!(ed.buf().rope.to_string(), "hi \"\" ok");
        assert_eq!(ed.mode, Mode::Normal);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 4 });
    }

    #[test]
    fn d_count_g_deletes_whole_lines() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut()
            .insert_text("one\ntwo\nthree\nfour\nfive\n", false);
        ed.buf_mut().cursor = Pos { line: 2, col: 1 };
        type_keys(&mut ed, &["d", "5", "G"].map(|s| Key::Character(s.into())));
        assert_eq!(ed.buf().rope.to_string(), "one\ntwo\n");
        assert!(ed.reg_linewise);
        assert_eq!(ed.register, "three\nfour\nfive\n");
    }

    #[test]
    fn x_deletes_last_char_of_line() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("abcd", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 3 };
        type_keys(&mut ed, &[Key::Character("x".into())]);
        assert_eq!(ed.buf().rope.to_string(), "abc");
        assert_eq!(ed.register, "d");
    }

    #[test]
    fn x_count_clamps_at_line_end() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("abcd", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        type_keys(&mut ed, &["3", "x"].map(|s| Key::Character(s.into())));
        assert_eq!(ed.buf().rope.to_string(), "a");
    }

    #[test]
    fn d_dollar_on_last_char_deletes_it() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("ab", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        type_keys(&mut ed, &["d", "$"].map(|s| Key::Character(s.into())));
        assert_eq!(ed.buf().rope.to_string(), "a");
    }

    #[test]
    fn ci_quote_targets_second_string_opener() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("foo(\"a\", \"b\")", false);
        ed.buf_mut().cursor = Pos { line: 0, col: 9 };
        type_keys(&mut ed, &["c", "i", "\""].map(|s| Key::Character(s.into())));
        assert_eq!(ed.buf().rope.to_string(), "foo(\"a\", \"\")");
        assert_eq!(ed.mode, Mode::Insert);
    }

    #[test]
    fn empty_text_object_keeps_register() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("x = \"\"", false);
        ed.register = "keep".into();
        ed.buf_mut().cursor = Pos { line: 0, col: 4 };
        type_keys(&mut ed, &["d", "i", "\""].map(|s| Key::Character(s.into())));
        assert_eq!(ed.buf().rope.to_string(), "x = \"\"");
        assert_eq!(ed.register, "keep");
        assert_eq!(ed.mode, Mode::Normal);
    }
}
