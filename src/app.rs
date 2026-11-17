use crate::buffer::{Buffer, Pos, line_chars, line_text};
use crate::config::{Config, RgbTheme};
use crate::mode::{self, Mode};
use std::path::Path;

const SCROLLOFF: usize = 8;

pub struct Editor {
    pub buffers: Vec<Buffer>,
    pub active: usize,
    pub mode: Mode,
    pub status: Option<String>,
    pub status_err: bool,
    pub config: Config,
    pub theme: RgbTheme,
    pub quit: bool,
    pub pending: mode::Pending,
    pub count: u32,
    pub register: String,
    pub reg_linewise: bool,
    pub reg_block: bool,
    pub visual_anchor: Option<Pos>,
    pub visual_linewise: bool,
    pub visual_block: bool,
    pub coalesce_open: bool,
    pub last_find: Option<(mode::FindDir, char)>,
    pub view_rows: usize,
    pub view_cols: usize,
    pub prefer_x: Option<usize>,
    pub cmdline: String,
    pub cmd_kind: Option<mode::CmdKind>,
    pub search: Option<String>,
    pub search_back: bool,
    pub panel: crate::fuzzy::Panel,
    pub panel_query: String,
    pub panel_items: Vec<String>,
    pub panel_order: Vec<usize>,
    pub panel_cursor: usize,
    pub lsp: crate::lsp::LspManager,
    pub context_menu: Option<String>,
    pub spell: crate::spell::Spell,
    spell_sig: u64,
    spell_last: Option<std::time::Instant>,
}

#[derive(PartialEq, Eq, Debug)]
pub enum Cmd {
    Write,
    WriteQuit(bool),
    Quit(bool),
    Edit(String),
    BNext,
    BPrev,
    Settings,
    Goto(usize),
    Pwd(Option<String>),
    Replace(String, String),
    Close(Option<usize>, bool),
}

pub fn parse_cmd(s: &str) -> Result<Cmd, String> {
    let s = s.trim();
    let bang = s.ends_with('!');
    let core = s.trim_end_matches('!').trim();
    let (head, arg) = match core.find(char::is_whitespace) {
        Some(i) => (&core[..i], core[i..].trim()),
        None => (core, ""),
    };
    match (head, arg) {
        ("w", "") => Ok(Cmd::Write),
        ("wq" | "x", "") => Ok(Cmd::WriteQuit(bang)),
        ("q", "") => Ok(Cmd::Quit(bang)),
        ("bnext" | "bn", "") => Ok(Cmd::BNext),
        ("bprev" | "bp", "") => Ok(Cmd::BPrev),
        ("c", "") => Ok(Cmd::Close(None, bang)),
        ("c", a) => match a.parse::<usize>() {
            Ok(n) if n > 0 => Ok(Cmd::Close(Some(n), bang)),
            _ => Err(format!("E475: Invalid argument to :c: {a}")),
        },
        ("settings", "") => Ok(Cmd::Settings),
        ("pwd", "") => Ok(Cmd::Pwd(None)),
        ("pwd", a) => Ok(Cmd::Pwd(Some(a.to_string()))),
        ("e" | "edit", a) if !a.is_empty() => Ok(Cmd::Edit(a.to_string())),
        ("e" | "edit", "") => Err("E480: No file name".into()),
        (_, "") if !head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(Cmd::Goto(head.parse().unwrap_or(usize::MAX)))
        }
        ("r", a) => match a.split_once('/') {
            Some((pat, repl)) if !pat.is_empty() => {
                Ok(Cmd::Replace(pat.to_string(), repl.to_string()))
            }
            _ => Err(format!("E116: Invalid arguments to :r: {s}")),
        },
        _ => Err(format!("E492: Not an editor command: {s}")),
    }
}

impl Editor {
    pub fn new(path: Option<&Path>) -> Self {
        let (config, mut warnings) = crate::config::load();
        let (theme, twarnings) = RgbTheme::from_theme_checked(&config.theme);
        warnings.extend(twarnings);
        let mut ed = Self {
            buffers: vec![Buffer::empty()],
            active: 0,
            mode: Mode::Normal,
            status: None,
            status_err: false,
            config,
            theme,
            quit: false,
            pending: mode::Pending::None,
            count: 0,
            register: String::new(),
            reg_linewise: false,
            reg_block: false,
            visual_anchor: None,
            visual_linewise: false,
            visual_block: false,
            coalesce_open: false,
            last_find: None,
            view_rows: 24,
            view_cols: 80,
            prefer_x: None,
            cmdline: String::new(),
            cmd_kind: None,
            search_back: false,
            search: None,
            panel: crate::fuzzy::Panel::None,
            panel_query: String::new(),
            panel_items: Vec::new(),
            panel_order: Vec::new(),
            panel_cursor: 0,
            lsp: crate::lsp::LspManager::new(),
            context_menu: None,
            spell: crate::spell::Spell::new(),
            spell_sig: u64::MAX,
            spell_last: None,
        };
        if let Some(p) = path {
            match Buffer::load(p) {
                Ok(b) => ed.buffers[0] = b,
                Err(e) => {
                    let mut b = Buffer::empty();
                    b.path = Some(p.to_path_buf());
                    ed.buffers[0] = b;
                    ed.status = Some(e.to_string());
                    ed.status_err = true;
                }
            }
        }
        ed.ensure_lsp();
        if !warnings.is_empty() {
            ed.set_status(warnings.join("; "), true);
        }
        ed
    }

    pub fn buf(&self) -> &Buffer {
        &self.buffers[self.active]
    }

    pub fn buf_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.active]
    }

    pub fn tab_width(&self) -> usize {
        self.config.tab_width.max(1) as usize
    }

    pub fn expand_tabs(&self, line: &str, upto: usize) -> String {
        let mut out = String::new();
        let tw = self.tab_width();
        for ch in line.chars().take(upto) {
            if ch == '\t' {
                let pad = tw - (visual_col(&out) % tw);
                for _ in 0..pad {
                    out.push(' ');
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    pub fn visual_x(&self, line: &str, col: usize) -> usize {
        self.expand_tabs(line, col).chars().count()
    }

    pub fn set_status(&mut self, msg: String, err: bool) {
        self.status = Some(msg);
        self.status_err = err;
    }

    pub fn wrap(&self) -> crate::wrap::Wrap {
        crate::wrap::Wrap::new(self.view_cols, self.config.tab_width as usize)
    }

    pub fn scroll_to_cursor(&mut self, rows: usize) {
        if rows == 0 {
            return;
        }
        let off = SCROLLOFF.min((rows - 1) / 2);
        let wrap = self.wrap();
        let last = self.buf().rope.len_lines().saturating_sub(1);
        let rows_at =
            |line: usize| wrap.row_count(&crate::buffer::line_text(&self.buf().rope, line));
        let cursor = self.buf().cursor;
        let mut top = self.buf().view_top.min(last);
        let mut sub = self.buf().view_sub;
        loop {
            let top_rows = rows_at(top);
            if sub >= top_rows {
                sub = top_rows - 1;
            }
            let rel = wrap.screen_row(&self.buf().rope, top, sub, cursor);
            let too_high = cursor.line < top || rel.is_some_and(|r| r < off);
            let too_low = rel.is_some_and(|r| r > rows - 1 - off);
            if too_high {
                if sub > 0 {
                    sub -= 1;
                } else if top > 0 {
                    top -= 1;
                    sub = rows_at(top) - 1;
                } else {
                    break;
                }
                continue;
            }
            if too_low {
                if top == cursor.line || sub + 1 < rows_at(top) {
                    sub += 1;
                } else {
                    top += 1;
                    sub = 0;
                }
                continue;
            }
            break;
        }
        loop {
            let mut used = 0usize;
            let mut line = top;
            let mut walked = sub;
            loop {
                used += rows_at(line) - walked;
                walked = 0;
                if used >= rows || line >= last {
                    break;
                }
                line += 1;
            }
            if used >= rows || top == 0 {
                break;
            }
            if sub > 0 {
                sub -= 1;
            } else {
                top -= 1;
                sub = rows_at(top) - 1;
            }
        }
        let b = self.buf_mut();
        b.view_top = top;
        b.view_sub = sub;
    }

    pub fn center_cursor(&mut self, rows: usize) {
        let rows = rows.max(1);
        let wrap = self.wrap();
        let (top, sub) = {
            let b = self.buf();
            let cursor_row = wrap.row_of(&b.rope, b.cursor);
            let total = wrap.total_rows(&b.rope);
            let top_row = cursor_row
                .saturating_sub(rows / 2)
                .min(total.saturating_sub(rows));
            wrap.anchor(&b.rope, top_row)
        };
        let b = self.buf_mut();
        b.view_top = top;
        b.view_sub = sub;
    }

    pub fn clear_pending(&mut self) {
        self.pending = mode::Pending::None;
        self.count = 0;
    }

    pub fn leave_insert(&mut self) {
        if self.mode == Mode::Insert {
            self.coalesce_open = false;
            self.lsp.completion = None;
            self.mode = Mode::Normal;
            self.buf_mut().clamp();
        }
    }

    pub fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
        self.coalesce_open = true;
        self.clear_pending();
    }

    pub fn start_visual(&mut self, linewise: bool) {
        if self.mode == Mode::Visual {
            if self.visual_block {
                self.visual_block = false;
                self.visual_linewise = linewise;
                return;
            }
            if self.visual_linewise == linewise || !linewise {
                self.mode = Mode::Normal;
                self.visual_anchor = None;
                return;
            }
        }
        self.mode = Mode::Visual;
        self.visual_linewise = linewise;
        let c = self.buf().cursor;
        self.visual_anchor = Some(c);
        self.clear_pending();
    }

    pub fn start_visual_block(&mut self) {
        if self.mode == Mode::Visual && self.visual_block {
            self.mode = Mode::Normal;
            self.visual_anchor = None;
            self.visual_block = false;
            return;
        }
        self.mode = Mode::Visual;
        self.visual_linewise = false;
        self.visual_block = true;
        let c = self.buf().cursor;
        self.visual_anchor = Some(c);
        self.clear_pending();
    }

    pub fn block_range(&self) -> Option<(usize, usize, usize, usize)> {
        let anchor = self.visual_anchor?;
        let cur = self.buf().cursor;
        let (a, b) = if (anchor.line, anchor.col) <= (cur.line, cur.col) {
            (anchor, cur)
        } else {
            (cur, anchor)
        };
        Some((a.line, b.line, a.col, b.col))
    }

    pub fn block_apply(&mut self, op: mode::Operator) {
        use mode::Operator as Op;
        let Some((r0, r1, c0, c1)) = self.block_range() else {
            return;
        };
        let width = c1 - c0 + 1;
        let last = self.buf().rope.len_lines().saturating_sub(1);
        let r1 = r1.min(last);
        let mut yanked = Vec::new();
        for line in r0..=r1 {
            let text = crate::buffer::line_text(&self.buf().rope, line);
            let seg: String = text.chars().skip(c0).take(width).collect();
            yanked.push(format!("{:<w$}", seg, w = width));
        }
        let block = yanked.join("\n");
        self.set_register(block, false);
        self.reg_block = true;
        if op != Op::Yank {
            let mut edits: Vec<(Pos, String, String)> = Vec::new();
            for line in r0..=r1 {
                let len = line_chars(&self.buf().rope, line);
                if len <= c0 {
                    continue;
                }
                let end = (c1 + 1).min(len);
                let seg: String = crate::buffer::line_text(&self.buf().rope, line)
                    .chars()
                    .skip(c0)
                    .take(end - c0)
                    .collect();
                edits.push((Pos { line, col: c0 }, seg, String::new()));
            }
            if !edits.is_empty() {
                self.buf_mut().replace_all(edits);
            }
        }
        self.mode = Mode::Normal;
        self.visual_anchor = None;
        self.visual_block = false;
        self.buf_mut().clamp();
        if op == Op::Change {
            self.enter_insert();
        }
    }

    pub fn sel_range(&self) -> ((Pos, Pos), bool) {
        let anchor = self.visual_anchor.unwrap_or(self.buf().cursor);
        let cur = self.buf().cursor;
        let (a, b) = if (anchor.line, anchor.col) <= (cur.line, cur.col) {
            (anchor, cur)
        } else {
            (cur, anchor)
        };
        if self.visual_linewise {
            let end = if b.line + 1 < self.buf().rope.len_lines() {
                Pos {
                    line: b.line + 1,
                    col: 0,
                }
            } else {
                Pos {
                    line: b.line,
                    col: line_chars(&self.buf().rope, b.line),
                }
            };
            (
                (
                    Pos {
                        line: a.line,
                        col: 0,
                    },
                    end,
                ),
                true,
            )
        } else {
            let end = Pos {
                line: b.line,
                col: b.col + 1,
            };
            ((a, end), false)
        }
    }

    pub fn op_apply(&mut self, op: mode::Operator, (a, b): (Pos, Pos), linewise: bool) {
        use mode::Operator as Op;
        let last_line = if linewise && b.col == 0 && b.line > a.line {
            b.line - 1
        } else {
            b.line
        };
        if matches!(op, Op::Yank | Op::Delete | Op::Change) {
            let text = self
                .buf()
                .rope
                .slice(self.buf().offset_of(a)..self.buf().offset_of(b))
                .to_string();
            self.set_register(text, linewise);
        }
        match op {
            Op::Yank => {}
            Op::Delete => {
                self.buf_mut().delete_text(a, b);
                let b = self.buf_mut();
                if linewise && b.cursor.col > line_chars(&b.rope, b.cursor.line) {
                    b.cursor.col = line_chars(&b.rope, b.cursor.line);
                }
                b.clamp();
            }
            Op::Change => {
                self.buf_mut().delete_text(a, b);
                if linewise {
                    self.buf_mut().cursor = Pos {
                        line: a.line,
                        col: 0,
                    };
                }
                self.mode = Mode::Insert;
                self.coalesce_open = true;
            }
            Op::Indent => {
                self.indent_lines(a.line, last_line, true);
            }
            Op::Outdent => {
                self.indent_lines(a.line, last_line, false);
            }
        }
        if linewise {
            self.buf_mut().cursor = Pos {
                line: self.buf().cursor.line,
                col: 0,
            };
        }
        self.buf_mut().clamp();
        if self.mode == Mode::Visual {
            self.mode = Mode::Normal;
            self.visual_anchor = None;
        }
    }

    fn set_register(&mut self, text: String, linewise: bool) {
        self.register = text;
        self.reg_linewise = linewise;
        self.reg_block = false;
        if self.config.clipboard
            && let Err(e) = crate::clipboard::set(&self.register)
        {
            self.set_status(format!("Clipboard: {e}"), true);
        }
    }

    fn indent_lines(&mut self, from: usize, to: usize, indent: bool) {
        let tw = self.tab_width();
        let last = self.buf().rope.len_lines().saturating_sub(1);
        for line in from..=to.min(last) {
            if indent {
                let at = Pos { line, col: 0 };
                let off = self.buf().offset_of(at);
                let b = self.buf_mut();
                b.rope.insert(off, &" ".repeat(tw));
                b.after_edit();
            } else {
                let line_s = crate::buffer::line_text(&self.buf().rope, line);
                let n = line_s.chars().take(tw).take_while(|c| *c == ' ').count();
                if n > 0 {
                    let b = self.buf_mut();
                    let off = b.rope.line_to_char(line);
                    b.rope.remove(off..off + n);
                    b.after_edit();
                }
            }
        }
    }

    fn paste_source(
        clip: Option<String>,
        register: &str,
        register_linewise: bool,
    ) -> (String, bool) {
        match clip.filter(|s| !s.is_empty()) {
            Some(text) => {
                let linewise = text.ends_with('\n');
                (text, linewise)
            }
            None => (register.to_string(), register_linewise),
        }
    }

    pub fn paste(&mut self, after: bool) {
        let clip = if self.config.clipboard && !self.reg_block {
            crate::clipboard::get().ok()
        } else {
            None
        };
        let clip_override = clip.clone().filter(|s| !s.is_empty());
        let (reg, linewise) = Self::paste_source(clip, &self.register, self.reg_linewise);
        if reg.is_empty() {
            return;
        }
        if self.reg_block && clip_override.is_none() {
            let segs: Vec<&str> = reg.split('\n').collect();
            let c = self.buf().cursor;
            let col = c.col + if after { 1 } else { 0 };
            for (i, seg) in segs.iter().enumerate() {
                let line = c.line + i;
                while line >= self.buf().rope.len_lines() {
                    let b = self.buf_mut();
                    b.rope.insert(b.rope.len_chars(), "\n");
                    b.after_edit();
                }
                let len = line_chars(&self.buf().rope, line);
                let pad = " ".repeat(col.saturating_sub(len));
                let b = self.buf_mut();
                let off = b.rope.line_to_char(line) + len.min(col);
                b.rope.insert(off, &format!("{pad}{seg}"));
                b.after_edit();
            }
            let end = segs.last().unwrap_or(&"").chars().count();
            self.buf_mut().cursor = Pos {
                line: c.line + segs.len() - 1,
                col: col + end,
            };
            self.buf_mut().clamp();
            return;
        }
        if linewise {
            let mut text = reg.clone();
            if !text.ends_with('\n') {
                text.push('\n');
            }
            let line = self.buf().cursor.line;
            let at = if after {
                if line + 1 < self.buf().rope.len_lines() {
                    Pos {
                        line: line + 1,
                        col: 0,
                    }
                } else {
                    let len = line_chars(&self.buf().rope, line);
                    Pos { line, col: len }
                }
            } else {
                Pos { line, col: 0 }
            };
            self.buf_mut().cursor = at;
            self.buf_mut().insert_text(&text, false);
            let b = self.buf_mut();
            b.cursor = Pos {
                line: at.line,
                col: 0,
            };
        } else {
            let c = self.buf().cursor;
            let at = if after {
                if self.buf().mode_at_eol(c) {
                    Pos {
                        line: c.line,
                        col: line_chars(&self.buf().rope, c.line),
                    }
                } else {
                    Pos {
                        line: c.line,
                        col: c.col + 1,
                    }
                }
            } else {
                c
            };
            self.buf_mut().cursor = at;
            self.buf_mut().insert_text(&reg, false);
            let mut end_pos = at;
            let mut it = reg.chars().peekable();
            while let Some(ch) = it.next() {
                if it.peek().is_none() {
                    break;
                }
                if ch == '\n' {
                    end_pos.line += 1;
                    end_pos.col = 0;
                } else {
                    end_pos.col += 1;
                }
            }
            self.buf_mut().cursor = end_pos;
        }
        self.buf_mut().clamp();
        self.clear_pending();
    }

    pub fn replace_char(&mut self, ch: char) {
        let c = self.buf().cursor;
        let len = line_chars(&self.buf().rope, c.line);
        if c.col < len {
            let old = self.buf().rope.line(c.line).char(c.col).to_string();
            self.buf_mut().replace_all(vec![(c, old, ch.to_string())]);
        }
        self.clear_pending();
    }

    pub fn join_lines(&mut self, count: u32) {
        let n = count.max(1) as usize;
        for _ in 0..n {
            let line = self.buf().cursor.line;
            if line + 1 >= self.buf().rope.len_lines() {
                break;
            }
            let len = line_chars(&self.buf().rope, line);
            let next = crate::buffer::line_text(&self.buf().rope, line + 1);
            let stripped = next.trim_start();
            let glue = if len > 0 && !stripped.is_empty() {
                " "
            } else {
                ""
            };
            let b = self.buf_mut();
            let s = b.offset_of(Pos { line, col: len });
            let e = b.offset_of(Pos {
                line: line + 1,
                col: next.chars().count() - stripped.chars().count(),
            });
            b.rope.remove(s..e);
            b.rope.insert(s, glue);
            b.cursor = Pos { line, col: len };
            b.after_edit();
        }
        self.buf_mut().clamp();
        self.clear_pending();
    }

    pub fn newline_in_insert(&mut self) {
        let co = self.coalesce_open;
        let c = self.buf().cursor;
        let tw = self.tab_width();
        let before: String = crate::buffer::line_text(&self.buf().rope, c.line)
            .chars()
            .take(c.col)
            .collect();
        let ind = crate::buffer::indent_below(&before, tw);
        self.buf_mut().insert_text("\n", co);
        self.buf_mut().insert_text(&ind, co);
    }

    pub fn new_line_below(&mut self) {
        let c = self.buf().cursor;
        let len = line_chars(&self.buf().rope, c.line);
        let tw = self.tab_width();
        let ind =
            crate::buffer::indent_below(&crate::buffer::line_text(&self.buf().rope, c.line), tw);
        self.buf_mut().cursor = Pos {
            line: c.line,
            col: len,
        };
        self.buf_mut().insert_text("\n", false);
        if !ind.is_empty() {
            self.buf_mut().insert_text(&ind, false);
        }
        self.enter_insert();
    }

    pub fn new_line_above(&mut self) {
        let c = self.buf().cursor;
        let tw = self.tab_width();
        let ind =
            crate::buffer::indent_above(&crate::buffer::line_text(&self.buf().rope, c.line), tw);
        self.buf_mut().cursor = Pos {
            line: c.line,
            col: 0,
        };
        self.buf_mut().insert_text("\n", false);
        self.buf_mut().cursor = Pos {
            line: c.line,
            col: 0,
        };
        if !ind.is_empty() {
            self.buf_mut().insert_text(&ind, false);
        }
        self.enter_insert();
    }

    pub fn close_bracket_indent(&mut self, ch: char) {
        if !matches!(ch, '}' | ')' | ']') {
            return;
        }
        let c = self.buf().cursor;
        let text = crate::buffer::line_text(&self.buf().rope, c.line);
        if !text.chars().take(c.col).all(|x| x == ' ' || x == '\t') {
            return;
        }
        let tw = self.tab_width();
        let removed = self.buf_mut().dedent_line(c.line, tw);
        self.buf_mut().cursor = Pos {
            line: c.line,
            col: c.col.saturating_sub(removed),
        };
    }

    fn motion_ctx(&self, vertical: bool) -> crate::motions::Ctx {
        let wrap = self.wrap();
        let b = self.buf();
        let text = crate::buffer::line_text(&b.rope, b.cursor.line);
        let here = wrap.pos(&text, b.cursor.col).1;
        crate::motions::Ctx {
            rows: self.view_rows,
            wrap,
            prefer_x: if vertical {
                self.prefer_x.unwrap_or(here)
            } else {
                here
            },
        }
    }

    pub fn run_motion(&mut self, m: crate::motions::Motion, count: u32) {
        use crate::motions::Motion as M;
        let vertical = matches!(m, M::J | M::K);
        let ctx = self.motion_ctx(vertical);
        let to = {
            let b = self.buf();
            crate::motions::apply(&b.rope, b.cursor, m, count, &ctx)
        };
        self.prefer_x = vertical.then_some(ctx.prefer_x);
        self.buf_mut().cursor = to;
        self.buf_mut().clamp();
    }

    pub fn run_find(&mut self, dir: mode::FindDir, ch: char, count: u32) {
        use crate::motions::Motion as M;
        let m = match dir {
            mode::FindDir::F => M::Fwd(ch),
            mode::FindDir::T => M::TillFwd(ch),
            mode::FindDir::BackF => M::Back(ch),
            mode::FindDir::BackT => M::TillBack(ch),
        };
        self.run_motion(m, count);
    }

    pub fn operator_motion(&mut self, op: mode::Operator, m: crate::motions::Motion, count: u32) {
        use crate::motions::Motion as M;
        let ctx = self.motion_ctx(false);
        let (from, to) = {
            let b = self.buf();
            (
                b.cursor,
                crate::motions::apply(&b.rope, b.cursor, m, count, &ctx),
            )
        };
        self.prefer_x = None;
        let linewise = matches!(
            m,
            M::LineDown
                | M::LineUp
                | M::GG
                | M::G
                | M::PageDown
                | M::PageUp
                | M::HalfPageDown
                | M::HalfPageUp
        );
        if from == to && !linewise {
            self.clear_pending();
            return;
        }
        let inclusive = matches!(m, M::Dollar | M::E | M::BigE | M::Fwd(_));
        let (a, bnd) = if linewise {
            let (lo, hi) = (from.line.min(to.line), from.line.max(to.line));
            let bnd = if hi + 1 < self.buf().rope.len_lines() {
                Pos {
                    line: hi + 1,
                    col: 0,
                }
            } else {
                Pos {
                    line: hi,
                    col: line_chars(&self.buf().rope, hi),
                }
            };
            (Pos { line: lo, col: 0 }, bnd)
        } else if to >= from {
            let mut bnd = to;
            if inclusive {
                let len = line_chars(&self.buf().rope, bnd.line);
                bnd.col = (bnd.col + 1).min(len);
            }
            (from, bnd)
        } else {
            (to, from)
        };
        self.op_apply(op, (a, bnd), linewise);
        self.clear_pending();
    }

    pub fn operator_line(&mut self, op: mode::Operator) {
        let count = if self.count == 0 { 1 } else { self.count };
        let line = self.buf().cursor.line;
        let hi = line + count as usize - 1;
        let last = self.buf().rope.len_lines().saturating_sub(1);
        let bnd = if hi < last {
            Pos {
                line: hi + 1,
                col: 0,
            }
        } else {
            Pos {
                line: last,
                col: line_chars(&self.buf().rope, last),
            }
        };
        let a = Pos { line, col: 0 };
        if bnd == a && line > 0 {
            let prev = Pos {
                line: line - 1,
                col: line_chars(&self.buf().rope, line - 1),
            };
            self.op_apply(op, (prev, bnd), true);
        } else {
            self.op_apply(op, (a, bnd), true);
        }
        self.clear_pending();
    }

    pub fn operator_object(
        &mut self,
        op: mode::Operator,
        inner: bool,
        obj: crate::textobjects::Obj,
        count: u32,
    ) {
        for _ in 0..count.max(1) {
            let range = {
                let b = self.buf();
                crate::textobjects::object(&b.rope, b.cursor, inner, obj)
            };
            let Some((a, bnd)) = range else {
                break;
            };
            self.op_apply(op, (a, bnd), false);
        }
        self.clear_pending();
    }

    pub fn select_object(&mut self, inner: bool, obj: crate::textobjects::Obj) {
        let range = {
            let b = self.buf();
            crate::textobjects::object(&b.rope, b.cursor, inner, obj)
        };
        if let Some((a, bnd)) = range {
            let last = {
                let b = self.buf();
                let off = (b.offset_of(bnd).saturating_sub(1)).max(b.offset_of(a));
                b.pos_of(off)
            };
            self.visual_anchor = Some(a);
            self.buf_mut().cursor = last;
        }
    }

    pub fn open_cmdline(&mut self, kind: mode::CmdKind) {
        self.cmd_kind = Some(kind);
        self.mode = Mode::Command(kind);
        self.cmdline.clear();
    }

    pub fn close_cmdline(&mut self) {
        self.cmd_kind = None;
        self.cmdline.clear();
        self.mode = Mode::Normal;
        self.clear_pending();
    }

    pub fn search_step(&mut self, term: &str, back: bool) {
        if term.is_empty() {
            return;
        }
        let (found, report) = {
            let b = self.buf();
            let chars: Vec<char> = b.rope.chars().collect();
            let off = b.offset_of(b.cursor);
            let t: Vec<char> = term.chars().collect();
            let start = if back { off.saturating_sub(1) } else { off + 1 };
            let fold = crate::search::ignore_case(term);
            let found = crate::search::find_wrap(&chars, &t, start, back, fold);
            let report = found.map(|at| crate::search::match_report(&chars, &t, at, fold));
            (found, report)
        };
        match (found, report) {
            (Some(i), Some((current, total))) => {
                let p = self.buf().pos_of(i);
                self.buf_mut().cursor = p;
                self.buf_mut().clamp();
                self.set_status(format!("[{current}/{total}]"), false);
            }
            _ => {
                self.set_status(format!("Pattern not found: {term}"), true);
            }
        }
    }

    pub fn commit_search(&mut self, back: bool) {
        let term = self.cmdline.clone();
        self.close_cmdline();
        if term.is_empty() {
            return;
        }
        self.search = Some(term.clone());
        self.search_back = back;
        self.search_step(&term, back);
    }

    pub fn search_cur_word(&mut self, back: bool) {
        let term = {
            let b = self.buf();
            let text: Vec<char> = crate::buffer::line_text(&b.rope, b.cursor.line)
                .chars()
                .collect();
            let Some(&c) = text.get(b.cursor.col) else {
                return;
            };
            if crate::motions::char_is_word(c) {
                let mut a = b.cursor.col;
                let mut z = b.cursor.col;
                while a > 0 && crate::motions::char_is_word(text[a - 1]) {
                    a -= 1;
                }
                while z + 1 < text.len() && crate::motions::char_is_word(text[z + 1]) {
                    z += 1;
                }
                text[a..=z].iter().collect()
            } else if c.is_whitespace() {
                return;
            } else {
                c.to_string()
            }
        };
        self.search = Some(term.clone());
        self.search_back = back;
        self.search_step(&term, back);
    }

    pub fn repeat_search(&mut self, reverse: bool) {
        let Some(term) = self.search.clone() else {
            self.set_status("E35: No previous search".to_string(), true);
            return;
        };
        let back = self.search_back != reverse;
        self.search_step(&term, back);
    }

    pub fn execute_cmdline(&mut self) {
        let raw = self.cmdline.clone();
        match parse_cmd(&raw) {
            Err(e) => {
                self.close_cmdline();
                self.set_status(e, true);
            }
            Ok(cmd) => {
                self.cmd_kind = None;
                self.cmdline.clear();
                self.mode = Mode::Normal;
                match cmd {
                    Cmd::Write => self.write_active(),
                    Cmd::WriteQuit(_) => {
                        self.write_active();
                        if !self.status_err {
                            self.quit = true;
                        }
                    }
                    Cmd::Quit(bang) => {
                        if self.buf().dirty && !bang {
                            self.set_status(
                                "E37: No write since last change (add ! to override)".to_string(),
                                true,
                            );
                        } else {
                            self.quit = true;
                        }
                    }
                    Cmd::Edit(path) => {
                        let p = std::path::PathBuf::from(shrink_tilde(&path));
                        self.edit_path_in_new_buffer(p);
                    }
                    Cmd::BNext => {
                        if !self.buffers.is_empty() {
                            self.activate((self.active + 1) % self.buffers.len());
                        }
                        self.ensure_lsp();
                    }
                    Cmd::BPrev => {
                        let len = self.buffers.len();
                        if len > 0 {
                            self.activate((self.active + len - 1) % len);
                        }
                        self.ensure_lsp();
                    }
                    Cmd::Settings => self.open_panel(crate::fuzzy::Panel::Settings),
                    Cmd::Goto(n) => {
                        let last = self.buf().rope.len_lines().saturating_sub(1);
                        let col = self.buf().cursor.col;
                        self.buf_mut().cursor = Pos {
                            line: n.saturating_sub(1).min(last),
                            col,
                        };
                        self.buf_mut().clamp();
                    }
                    Cmd::Pwd(arg) => {
                        let target = match arg {
                            Some(p) => {
                                let dir = std::path::PathBuf::from(shrink_tilde(&p));
                                std::env::set_current_dir(dir)
                                    .and_then(|()| std::env::current_dir())
                            }
                            None => std::env::current_dir(),
                        };
                        match target {
                            Ok(d) => self.set_status(d.display().to_string(), false),
                            Err(e) => {
                                self.set_status(format!("E344: cannot change directory: {e}"), true)
                            }
                        }
                    }
                    Cmd::Replace(pat, repl) => {
                        let outcome = (|| -> Result<usize, String> {
                            let re = regex::Regex::new(&pat)
                                .map_err(|e| format!("E116: Invalid regex: {e}"))?;
                            if re.is_match("") {
                                return Err(
                                    "E909: pattern must not match the empty string".to_string()
                                );
                            }
                            let text = self.buf().rope.to_string();
                            let edits: Vec<(Pos, String, String)> = re
                                .find_iter(&text)
                                .map(|m| {
                                    let chars = self.buf().rope.byte_to_char(m.start());
                                    (
                                        self.buf().pos_of(chars),
                                        m.as_str().to_string(),
                                        repl.clone(),
                                    )
                                })
                                .collect();
                            if edits.iter().any(|(_, old, _)| old.is_empty()) {
                                return Err(
                                    "E909: pattern must not match the empty string".to_string()
                                );
                            }

                            if edits.is_empty() {
                                return Err(format!("E486: Pattern not found: {pat}"));
                            }
                            let n = edits.len();
                            self.buf_mut().replace_all(edits);
                            Ok(n)
                        })();
                        match outcome {
                            Ok(n) => self.set_status(format!("{n} substitutions"), false),
                            Err(e) => self.set_status(e, true),
                        }
                    }
                    Cmd::Close(idx, bang) => self.close_buffer(idx, bang),
                }
            }
        }
    }

    pub fn write_active(&mut self) {
        let res = self.buf().save();
        match res {
            Ok(()) => {
                let b = self.buf_mut();
                b.mark_saved();
                let name = b.display_name();
                let lines = b.rope.len_lines();
                let chars = b.rope.len_chars();
                self.set_status(format!("\"{name}\" {lines}L {chars}C written"), false);
                self.notify_saved();
            }
            Err(e) => {
                let msg = e.to_string();
                self.set_status(msg, true);
            }
        }
    }

    pub fn ensure_lsp(&mut self) {
        let p = self.buf().path.clone();
        self.lsp.ensure(p.as_ref());
    }

    pub fn pos_at_cell(&self, drow: usize, xcell: usize) -> Pos {
        let b = self.buf();
        let wrap = self.wrap();
        let abs = wrap.row_of(
            &b.rope,
            Pos {
                line: b.view_top,
                col: 0,
            },
        ) + b.view_sub
            + drow;
        let (line, sub) = wrap.anchor(&b.rope, abs);
        let col = wrap.col_at(&b.rope, line, sub, xcell);
        Pos { line, col }
    }

    pub fn context_menu_text(&self) -> String {
        let b = self.buf();
        let c = b.cursor;
        let text = line_text(&b.rope, c.line);
        let wordy = |ch: char| ch.is_alphanumeric() || ch == '_';
        let mut on_word = text.chars().nth(c.col).is_some_and(wordy);
        if !on_word && c.col > 0 && text.chars().nth(c.col).is_none() {
            on_word = text.chars().nth(c.col - 1).is_some_and(wordy);
        }
        let mut items: Vec<(&str, &str)> = Vec::new();
        if on_word {
            if self.lsp.active() {
                items.push(("gd", "go to definition"));
                items.push(("K", "hover docs"));
            }
            items.extend([
                ("*", "find word next"),
                ("#", "find word prev"),
                ("viw", "select word"),
                ("ciw", "change word"),
                ("diw", "delete word"),
            ]);
        } else {
            items.extend([
                ("i", "insert"),
                ("a", "append"),
                ("o", "new line below"),
                ("p", "paste"),
                ("u", "undo"),
            ]);
        }
        items
            .iter()
            .map(|(k, d)| format!("{k:<4} {d}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn hover_cur(&mut self) {
        if let Some(p) = self.buf().path.clone() {
            let c = self.buf().cursor;
            self.lsp.hover_at(&p, c);
        }
    }

    pub fn goto_definition_cur(&mut self) {
        if let Some(p) = self.buf().path.clone() {
            let c = self.buf().cursor;
            self.lsp.goto_definition(&p, c);
        }
    }

    /// Trigger after `.` / `::` / `->`; while open, re-request as the user
    /// keeps typing inside the member-access context.
    pub fn maybe_completion(&mut self, typed: &str) {
        let open = self.lsp.completion.is_some();
        let Some(path) = self.buf().path.clone() else {
            if open {
                self.lsp.completion = None;
            }
            return;
        };
        let line = self.buf().cursor.line;
        let col = self.buf().cursor.col;
        let line_text: String = self.buf().rope.line(line).chars().collect();
        let chars: Vec<char> = line_text.chars().collect();
        let col = col.min(chars.len());
        let before: String = chars.iter().take(col).collect();
        let mut i = col;
        while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
            i -= 1;
        }
        let ctx = i > 0 && matches!(chars[i - 1], '.' | ':' | '>');
        let fire = match typed {
            "." => before.ends_with('.'),
            ":" => before.ends_with("::"),
            "-" => before.ends_with("->"),
            _ => open && i < col && ctx,
        };
        if fire {
            let text = self.buf().rope.to_string();
            self.lsp.request_completion(&path, Pos { line, col }, &text);
        } else if open {
            self.lsp.completion = None;
        }
    }

    pub fn comp_next(&mut self) {
        if let Some(cs) = self.lsp.completion.as_mut()
            && !cs.items.is_empty()
        {
            cs.selected = (cs.selected + 1) % cs.items.len();
        }
    }

    pub fn comp_prev(&mut self) {
        if let Some(cs) = self.lsp.completion.as_mut()
            && !cs.items.is_empty()
        {
            cs.selected = (cs.selected + cs.items.len() - 1) % cs.items.len();
        }
    }

    pub fn comp_cancel(&mut self) {
        self.lsp.completion = None;
    }

    pub fn comp_accept(&mut self) {
        let Some(cs) = &self.lsp.completion else {
            return;
        };
        let Some(it) = cs.items.get(cs.selected) else {
            return;
        };
        let text = it.text.clone();
        let (line, col) = (cs.line, cs.col);
        self.lsp.completion = None;
        let co = self.coalesce_open;
        // drop the ident chars typed since the popup opened; items carry the
        // full insert text
        let c = self.buf().cursor;
        if c.line == line && c.col > col {
            self.buf_mut().delete_text(Pos { line, col }, c);
        }
        self.buf_mut().insert_text(&text, co);
    }

    pub fn activate(&mut self, i: usize) {
        if i != self.active {
            self.lsp.hover = None;
            self.lsp.completion = None;
            self.spell.clear();
            self.spell_sig = u64::MAX;
            self.spell_last = None;
        }
        self.active = i;
    }

    pub fn close_buffer(&mut self, idx: Option<usize>, bang: bool) {
        let i = match idx {
            Some(n) => n - 1,
            None => self.active,
        };
        if i >= self.buffers.len() {
            let shown = idx.unwrap_or(0);
            self.set_status(format!("E16: Invalid argument: {shown}"), true);
            return;
        }
        if self.buffers[i].dirty && !bang {
            self.set_status(
                "E37: No write since last change (add ! to override)".to_string(),
                true,
            );
            return;
        }
        let name = self.buffers[i].display_name();
        self.buffers.remove(i);
        if self.buffers.is_empty() {
            self.buffers.push(Buffer::empty());
        }
        let next = if i < self.active {
            self.active - 1
        } else {
            self.active.min(self.buffers.len() - 1)
        };
        self.activate(next);
        self.lsp.hover = None;
        self.lsp.completion = None;
        self.ensure_lsp();
        self.set_status(format!("[{name}] closed"), false);
    }

    pub fn poll_lsp(&mut self) {
        for ev in self.lsp.poll() {
            match ev {
                crate::lsp::Ev::Goto { path, line, col } => {
                    self.open_path(path.clone());
                    if let Some(i) = self
                        .buffers
                        .iter()
                        .position(|b| b.path.as_ref().is_some_and(|q| same_path(q, &path)))
                    {
                        self.activate(i);
                        self.buf_mut().cursor = Pos { line, col };
                        let rows = self.view_rows.max(1);
                        self.scroll_to_cursor(rows);
                        self.ensure_lsp();
                    } else {
                        self.set_status(format!("cannot open {}", path.display()), true);
                    }
                }
                crate::lsp::Ev::NoDef => {
                    self.set_status("No definition found".into(), true);
                }
            }
        }
    }

    pub fn poll_spell(&mut self) {
        self.spell.poll();
    }

    pub fn maybe_check_spell(&mut self, proxy: Option<winit::event_loop::EventLoopProxy<()>>) {
        if !self.config.spellcheck || self.spell.busy() {
            return;
        }
        let rev = self.buf().revision;
        if rev == self.spell_sig {
            return;
        }
        if self
            .spell_last
            .is_some_and(|t| t.elapsed().as_millis() < 500)
        {
            return;
        }
        self.spell_sig = rev;
        self.spell_last = Some(std::time::Instant::now());
        let text = self.buf().rope.to_string();
        let dict = self.config.spell_dict.clone();
        self.spell.request(text, dict, proxy);
    }

    pub fn toggle_spell(&mut self) {
        self.config.spellcheck = !self.config.spellcheck;
        if self.config.spellcheck {
            self.spell_sig = u64::MAX;
            self.spell_last = None;
        } else {
            self.spell.clear();
        }
        self.set_status(
            format!(
                "spellcheck: {}",
                if self.config.spellcheck { "on" } else { "off" }
            ),
            false,
        );
    }

    fn notify_saved(&mut self) {
        if let Some(p) = self.buf().path.clone() {
            let text = self.buf().rope.to_string();
            self.lsp.on_saved(&p, &text);
        }
    }

    pub fn open_panel(&mut self, kind: crate::fuzzy::Panel) {
        self.panel = kind;
        self.panel_query.clear();
        self.panel_cursor = 0;
        self.panel_items = match kind {
            crate::fuzzy::Panel::Files => crate::fuzzy::list_dir(&files_root("")),
            crate::fuzzy::Panel::Buffers => self
                .buffers
                .iter()
                .map(|b| format!("{}{}", b.display_name(), if b.dirty { "[+]" } else { "" }))
                .collect(),
            crate::fuzzy::Panel::Settings => crate::config::SETTINGS
                .iter()
                .map(|k| format!("{k} = {}", crate::config::setting_value(&self.config, k)))
                .collect(),
            crate::fuzzy::Panel::None => Vec::new(),
        };
        self.refresh_panel_order();
    }

    pub fn close_panel(&mut self) {
        self.panel = crate::fuzzy::Panel::None;
        self.panel_query.clear();
        self.panel_items.clear();
        self.panel_order.clear();
        self.panel_cursor = 0;
    }

    pub fn refresh_panel_order(&mut self) {
        let query = match self.panel {
            crate::fuzzy::Panel::Files => crate::fuzzy::split_path_query(&self.panel_query)
                .1
                .to_string(),
            _ => self.panel_query.clone(),
        };
        self.panel_order = crate::fuzzy::fuzzy_filter(&self.panel_items, &query);
        if self.panel_order.is_empty() {
            self.panel_cursor = 0;
        } else {
            self.panel_cursor = self.panel_cursor.min(self.panel_order.len() - 1);
        }
    }

    fn rescan_files_panel(&mut self) {
        let (dir, _) = crate::fuzzy::split_path_query(&self.panel_query);
        self.panel_items = crate::fuzzy::list_dir(&files_root(dir));
    }

    pub fn panel_type(&mut self, c: char) {
        self.panel_query.push(c);
        self.panel_cursor = 0;
        if self.panel == crate::fuzzy::Panel::Files {
            self.rescan_files_panel();
        }
        self.refresh_panel_order();
    }

    pub fn panel_backspace(&mut self) {
        self.panel_query.pop();
        self.panel_cursor = 0;
        if self.panel == crate::fuzzy::Panel::Files {
            self.rescan_files_panel();
        }
        self.refresh_panel_order();
    }

    pub fn panel_move(&mut self, delta: i32) {
        let n = self.panel_order.len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.panel_cursor as i32;
        self.panel_cursor = (((cur + delta) % n + n) % n) as usize;
    }

    pub fn panel_accept(&mut self) {
        let Some(&idx) = self.panel_order.get(self.panel_cursor) else {
            self.close_panel();
            return;
        };
        match self.panel {
            crate::fuzzy::Panel::Files => {
                let name = self.panel_items[idx].clone();
                let (dir, _) = crate::fuzzy::split_path_query(&self.panel_query);
                if name.ends_with('/') {
                    self.panel_query = format!("{dir}{name}");
                    self.panel_cursor = 0;
                    self.rescan_files_panel();
                    self.refresh_panel_order();
                } else {
                    let path = files_root(dir).join(&name);
                    self.close_panel();
                    self.open_path(path);
                }
            }
            crate::fuzzy::Panel::Buffers => {
                self.close_panel();
                if idx < self.buffers.len() {
                    self.activate(idx);
                }
            }
            crate::fuzzy::Panel::None => {}
            crate::fuzzy::Panel::Settings => {
                let key = crate::config::SETTINGS[idx].to_string();
                crate::config::setting_activate(&mut self.config, &key);
                if key == "spellcheck" {
                    self.spell.clear();
                    self.spell_sig = u64::MAX;
                    self.spell_last = None;
                }
                let (theme, warns) = RgbTheme::from_theme_checked(&self.config.theme);
                self.theme = theme;
                let val = crate::config::setting_value(&self.config, &key);
                let msg = match self.config.save_default() {
                    Ok(p) => {
                        let m = format!("{key} = {val} (saved {})", p.display());
                        if warns.is_empty() {
                            m
                        } else {
                            format!("{m}; {}", warns.join("; "))
                        }
                    }
                    Err(e) => {
                        let m = format!("{key} = {val} (not saved: {e})");
                        if warns.is_empty() {
                            m
                        } else {
                            format!("{m}; {}", warns.join("; "))
                        }
                    }
                };
                let err = msg.contains("not saved");
                self.set_status(msg, err);
                self.panel_items = crate::config::SETTINGS
                    .iter()
                    .map(|k| format!("{} = {}", k, crate::config::setting_value(&self.config, k)))
                    .collect();
                self.refresh_panel_order();
            }
        }
    }

    pub fn open_path(&mut self, p: std::path::PathBuf) {
        if let Some(i) = self
            .buffers
            .iter()
            .position(|b| b.path.as_ref().is_some_and(|q| same_path(q, &p)))
        {
            self.activate(i);
            self.ensure_lsp();
            return;
        }
        match Buffer::load(&p) {
            Ok(b) => {
                let empty_cur = {
                    let cur = self.buf();
                    !cur.dirty && cur.rope.len_chars() == 0
                };
                if empty_cur {
                    self.buffers[self.active] = b;
                    self.lsp.hover = None;
                } else {
                    self.buffers.push(b);
                    self.activate(self.buffers.len() - 1);
                }
                self.ensure_lsp();
            }
            Err(e) => {
                let msg = e.to_string();
                self.set_status(msg, true);
            }
        }
    }

    pub fn edit_path_in_new_buffer(&mut self, p: std::path::PathBuf) {
        if let Some(i) = self
            .buffers
            .iter()
            .position(|b| b.path.as_ref().is_some_and(|q| same_path(q, &p)))
        {
            let name = self.buffers[i].display_name();
            if i != self.active {
                self.activate(i);
                self.ensure_lsp();
            }
            self.set_status(format!("\"{name}\" already open"), false);
            return;
        }
        if !p.exists() {
            self.buffers.push(Buffer::new_file(&p));
            self.activate(self.buffers.len() - 1);
            self.ensure_lsp();
            self.set_status("[New File]".to_string(), false);
            return;
        }
        match Buffer::load(&p) {
            Ok(b) => {
                let n = b.rope.len_lines();
                let name = b.display_name();
                self.buffers.push(b);
                self.activate(self.buffers.len() - 1);
                self.ensure_lsp();
                self.set_status(format!("\"{name}\" {n}L loaded"), false);
            }
            Err(e) => self.set_status(e.to_string(), true),
        }
    }
}

fn shrink_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return format!("{}/{rest}", home.to_string_lossy());
    }
    p.to_string()
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

fn files_root(dir: &str) -> std::path::PathBuf {
    let expanded = shrink_tilde(dir);
    if expanded.is_empty() {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    } else {
        std::path::PathBuf::from(expanded)
    }
}

pub fn visual_col(s: &str) -> usize {
    s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motions::Motion;
    use crate::textobjects::Obj;

    fn ed_with(s: &str) -> Editor {
        let mut ed = Editor::new(None);
        ed.buf_mut().insert_text(s, false);
        ed
    }

    fn text(ed: &Editor) -> String {
        ed.buf().rope.to_string()
    }

    static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn change_inner_word_enters_insert_and_undoes() {
        let mut ed = ed_with("foo bar baz");
        ed.buf_mut().cursor = Pos { line: 0, col: 4 };
        ed.operator_object(mode::Operator::Change, true, Obj::Word, 1);
        assert_eq!(text(&ed), "foo  baz");
        assert_eq!(ed.mode, Mode::Insert);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 4 });
        ed.buf_mut().undo();
        assert_eq!(text(&ed), "foo bar baz");
    }

    #[test]
    fn delete_line_linewise_and_undo() {
        let mut ed = ed_with("one\ntwo\nthree\n");
        ed.buf_mut().cursor = Pos { line: 1, col: 0 };
        ed.count = 0;
        ed.operator_line(mode::Operator::Delete);
        assert_eq!(text(&ed), "one\nthree\n");
        ed.buf_mut().undo();
        assert_eq!(text(&ed), "one\ntwo\nthree\n");
    }

    #[test]
    fn dd_on_the_last_blank_line_removes_the_trailing_break() {
        let mut ed = ed_with("a\nb\n");
        ed.config.clipboard = false;
        ed.buf_mut().cursor = Pos { line: 2, col: 0 };
        ed.count = 0;
        ed.operator_line(mode::Operator::Delete);
        assert_eq!(text(&ed), "a\nb");
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 0 });
        assert_eq!(ed.register, "\n");
        assert!(ed.reg_linewise);
        ed.buf_mut().undo();
        assert_eq!(text(&ed), "a\nb\n");
    }

    #[test]
    fn dd_on_a_single_blank_buffer_does_not_panic() {
        let mut ed = ed_with("");
        ed.config.clipboard = false;
        ed.count = 0;
        ed.operator_line(mode::Operator::Delete);
        assert_eq!(text(&ed), "");
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 0 });
    }

    #[test]
    fn yank_line_then_linewise_paste() {
        let mut ed = ed_with("a\nb\n");
        ed.config.clipboard = false;
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.operator_line(mode::Operator::Yank);
        assert_eq!(ed.register, "a\n");
        assert!(ed.reg_linewise);
        ed.buf_mut().cursor = Pos { line: 1, col: 0 };
        ed.paste(true);
        assert_eq!(text(&ed), "a\nb\na\n");
    }

    #[test]
    fn delete_line_fills_the_register() {
        let mut ed = ed_with("one\ntwo\nthree\n");
        ed.config.clipboard = false;
        ed.buf_mut().cursor = Pos { line: 1, col: 0 };
        ed.operator_line(mode::Operator::Delete);
        assert_eq!(text(&ed), "one\nthree\n");
        assert_eq!(ed.register, "two\n");
        assert!(ed.reg_linewise);
        ed.paste(true);
        assert_eq!(text(&ed), "one\nthree\ntwo\n");
    }

    #[test]
    fn delete_char_fills_the_register() {
        let mut ed = ed_with("ab\n");
        ed.config.clipboard = false;
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.operator_motion(mode::Operator::Delete, crate::motions::Motion::L, 1);
        assert_eq!(text(&ed), "b\n");
        assert_eq!(ed.register, "a");
        assert!(!ed.reg_linewise);
    }

    #[test]
    fn change_line_fills_the_register() {
        let mut ed = ed_with("keep\ndrop\n");
        ed.config.clipboard = false;
        ed.buf_mut().cursor = Pos { line: 1, col: 2 };
        ed.operator_line(mode::Operator::Change);
        assert_eq!(ed.mode, Mode::Insert);
        assert_eq!(ed.register, "drop\n");
        assert!(ed.reg_linewise);
        assert_eq!(text(&ed), "keep\n");
    }

    #[test]
    fn paste_prefers_the_system_clipboard() {
        assert_eq!(
            Editor::paste_source(Some("from elsewhere\n".to_string()), "reg", false),
            ("from elsewhere\n".to_string(), true)
        );
        assert_eq!(
            Editor::paste_source(Some("inline".to_string()), "reg", true),
            ("inline".to_string(), false)
        );
        assert_eq!(
            Editor::paste_source(Some(String::new()), "reg", true),
            ("reg".to_string(), true)
        );
        assert_eq!(
            Editor::paste_source(None, "reg", true),
            ("reg".to_string(), true)
        );
    }

    #[test]
    fn visual_char_selection_is_inclusive() {
        let mut ed = ed_with("hello world");
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.start_visual(false);
        ed.run_motion(Motion::L, 2);
        let ((a, b), lw) = ed.sel_range();
        assert!(!lw);
        assert_eq!(a, Pos { line: 0, col: 0 });
        ed.op_apply(mode::Operator::Delete, (a, b), lw);
        assert_eq!(text(&ed), "lo world");
        assert_eq!(ed.mode, Mode::Normal);
    }

    #[test]
    fn count_multiplies_word_motion_operator() {
        let mut ed = ed_with("one two three");
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.operator_motion(mode::Operator::Delete, Motion::W, 2);
        assert_eq!(text(&ed), "three");
    }

    #[test]
    fn insert_coalesces_then_single_undo() {
        let mut ed = ed_with("");
        ed.enter_insert();
        for c in "hello".chars() {
            ed.buf_mut().insert_text(&c.to_string(), true);
        }
        assert_eq!(text(&ed), "hello");
        ed.buf_mut().undo();
        assert_eq!(text(&ed), "");
        ed.buf_mut().redo_action();
        assert_eq!(text(&ed), "hello");
    }

    #[test]
    fn linewise_selection_spans_full_lines() {
        let mut ed = ed_with("aaa\nbbb\nccc\n");
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        ed.start_visual(true);
        ed.run_motion(Motion::J, 1);
        let ((a, b), lw) = ed.sel_range();
        assert!(lw);
        ed.op_apply(mode::Operator::Delete, (a, b), lw);
        assert_eq!(text(&ed), "ccc\n");
    }

    #[test]
    fn parse_cmd_grammar() {
        assert_eq!(parse_cmd("w"), Ok(Cmd::Write));
        assert_eq!(parse_cmd("wq"), Ok(Cmd::WriteQuit(false)));
        assert_eq!(parse_cmd("q!"), Ok(Cmd::Quit(true)));
        assert_eq!(parse_cmd("x"), Ok(Cmd::WriteQuit(false)));
        assert_eq!(parse_cmd("bnext"), Ok(Cmd::BNext));
        assert_eq!(parse_cmd("bn"), Ok(Cmd::BNext));
        assert_eq!(parse_cmd("bp"), Ok(Cmd::BPrev));
        assert_eq!(
            parse_cmd("e /tmp/x.txt"),
            Ok(Cmd::Edit("/tmp/x.txt".to_string()))
        );
        assert_eq!(parse_cmd("edit"), Err("E480: No file name".to_string()));
        assert_eq!(
            parse_cmd("frobnicate"),
            Err("E492: Not an editor command: frobnicate".to_string())
        );
        assert_eq!(parse_cmd("590"), Ok(Cmd::Goto(590)));
        assert_eq!(
            parse_cmd("5x"),
            Err("E492: Not an editor command: 5x".to_string())
        );
        assert_eq!(parse_cmd("pwd"), Ok(Cmd::Pwd(None)));
        assert_eq!(
            parse_cmd("pwd /tmp"),
            Ok(Cmd::Pwd(Some("/tmp".to_string())))
        );
        assert_eq!(
            parse_cmd("r foo/bar"),
            Ok(Cmd::Replace("foo".to_string(), "bar".to_string()))
        );
        assert_eq!(
            parse_cmd("r foo"),
            Err("E116: Invalid arguments to :r: r foo".to_string())
        );
        assert_eq!(
            parse_cmd("r /x"),
            Err("E116: Invalid arguments to :r: r /x".to_string())
        );
        assert_eq!(parse_cmd("c"), Ok(Cmd::Close(None, false)));
        assert_eq!(parse_cmd("c!"), Ok(Cmd::Close(None, true)));
        assert_eq!(parse_cmd("c 4"), Ok(Cmd::Close(Some(4), false)));
        assert_eq!(
            parse_cmd("c 0"),
            Err("E475: Invalid argument to :c: 0".to_string())
        );
        assert_eq!(
            parse_cmd("c x"),
            Err("E475: Invalid argument to :c: x".to_string())
        );
    }

    #[test]
    fn close_active_tab_keeps_neighbor_and_last_close_starts_fresh() {
        let mut ed = Editor::new(None);
        ed.buffers[0] = Buffer::new_file(std::path::Path::new("/tmp/deditor_tab_a"));
        ed.buffers
            .push(Buffer::new_file(std::path::Path::new("/tmp/deditor_tab_b")));
        ed.active = 1;
        ed.cmdline = "c".to_string();
        ed.execute_cmdline();
        assert_eq!(ed.buffers.len(), 1);
        assert_eq!(ed.active, 0);
        assert_eq!(ed.buf().display_name(), "deditor_tab_a");
        ed.close_buffer(None, false);
        assert_eq!(ed.buffers.len(), 1);
        assert!(ed.buffers[0].path.is_none());
        assert_eq!(ed.buf().rope.to_string(), "");
    }

    #[test]
    fn close_dirty_tab_requires_bang() {
        let mut ed = ed_with("hello\n");
        ed.close_buffer(None, false);
        assert!(ed.status_err);
        assert_eq!(ed.buffers.len(), 1);
        assert_eq!(ed.buf().rope.to_string(), "hello\n");
        ed.close_buffer(None, true);
        assert!(!ed.status_err);
        assert_eq!(ed.buf().rope.to_string(), "");
    }

    #[test]
    fn close_tab_by_number_targets_that_tab() {
        let mut ed = Editor::new(None);
        ed.buffers[0] = Buffer::new_file(std::path::Path::new("/tmp/deditor_tab_a"));
        ed.buffers
            .push(Buffer::new_file(std::path::Path::new("/tmp/deditor_tab_b")));
        ed.buffers
            .push(Buffer::new_file(std::path::Path::new("/tmp/deditor_tab_c")));
        ed.close_buffer(Some(1), false);
        assert_eq!(ed.buffers.len(), 2);
        assert_eq!(ed.buffers[0].display_name(), "deditor_tab_b");
        assert_eq!(ed.active, 0);
        ed.close_buffer(Some(9), false);
        assert!(ed.status_err);
        assert_eq!(ed.buffers.len(), 2);
    }

    #[test]
    fn pos_at_cell_maps_screen_cells_through_wraps() {
        let mut ed = Editor::new(None);
        let body: String = (0..10).map(|i| format!("line {i}\n")).collect();
        ed.buf_mut().insert_text(&body, false);
        assert_eq!(ed.pos_at_cell(0, 0), Pos { line: 0, col: 0 });
        assert_eq!(ed.pos_at_cell(3, 4), Pos { line: 3, col: 4 });
        assert_eq!(ed.pos_at_cell(1, 100), Pos { line: 1, col: 5 });
        let mut wide = Editor::new(None);
        wide.buf_mut().insert_text(&"x".repeat(200), false);
        assert_eq!(wide.pos_at_cell(2, 5), Pos { line: 0, col: 165 });
    }

    #[test]
    fn context_menu_is_context_aware_at_the_cursor() {
        let mut ed = ed_with("hello world\n");
        ed.buf_mut().cursor = Pos { line: 0, col: 2 };
        let m = ed.context_menu_text();
        assert!(m.contains("*    find word next"));
        assert!(m.contains("viw  select word"));
        assert!(!m.contains("go to definition"));
        assert!(!m.contains("insert"));
        ed.buf_mut().cursor = Pos { line: 0, col: 11 };
        assert!(ed.context_menu_text().contains("find word next"));
        ed.buf_mut().cursor = Pos { line: 0, col: 5 };
        let m = ed.context_menu_text();
        assert!(m.contains("i    insert"));
        assert!(!m.contains("find word"));
    }

    #[test]
    fn goto_line_command_jumps_and_clamps() {
        let body: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let mut ed = ed_with(&body);
        ed.cmdline = "590".to_string();
        ed.execute_cmdline();
        assert_eq!(ed.buf().cursor.line, 589);
        assert!(!ed.status_err);
        ed.cmdline = "2000".to_string();
        ed.execute_cmdline();
        assert_eq!(ed.buf().cursor.line, ed.buf().rope.len_lines() - 1);
        ed.buf_mut().cursor = Pos { line: 0, col: 3 };
        ed.cmdline = "10".to_string();
        ed.execute_cmdline();
        assert_eq!(ed.buf().cursor, Pos { line: 9, col: 3 });
    }

    #[test]
    fn pwd_reports_and_changes_the_working_directory() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("deditor_pwd_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = std::env::current_dir().unwrap();
        let mut ed = ed_with("x\n");
        ed.cmdline = format!("pwd {}", dir.display());
        ed.execute_cmdline();
        assert!(!ed.status_err);
        assert_eq!(std::env::current_dir().unwrap(), dir);
        ed.cmdline = "pwd".to_string();
        ed.execute_cmdline();
        assert!(!ed.status_err);
        assert_eq!(ed.status.as_deref(), Some(dir.to_str().unwrap()));
        ed.cmdline = format!("pwd {}", dir.join("nope").display());
        ed.execute_cmdline();
        assert!(ed.status_err);
        assert_eq!(std::env::current_dir().unwrap(), dir);
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn files_panel_browses_typed_directories_and_fuzzy_matches_names() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("deditor_panel_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("target.txt"), "hit\n").unwrap();
        std::fs::write(dir.join("other.txt"), "x\n").unwrap();
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let mut ed = Editor::new(None);
        ed.open_panel(crate::fuzzy::Panel::Files);
        assert_eq!(
            ed.panel_items,
            vec!["other.txt".to_string(), "sub/".to_string()]
        );
        for c in "sub/".chars() {
            ed.panel_type(c);
        }
        assert_eq!(ed.panel_items, vec!["target.txt".to_string()]);
        for c in "trgt".chars() {
            ed.panel_type(c);
        }
        assert_eq!(ed.panel_order.len(), 1);
        ed.panel_accept();
        assert_eq!(ed.buf().display_name(), "target.txt");
        assert_eq!(text(&ed), "hit\n");
        assert_eq!(ed.panel, crate::fuzzy::Panel::None);
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn files_root_expands_tilde_and_defaults_to_cwd() {
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(files_root("~/"), std::path::PathBuf::from(home));
            assert_eq!(files_root("~/src/"), files_root("~/").join("src").join(""));
        }
        assert_eq!(files_root(""), std::env::current_dir().unwrap());
    }

    #[test]
    fn replace_command_rewrites_every_match() {
        let mut ed = ed_with("foo one foo two\nfoo\n");
        ed.cmdline = "r foo/baz".to_string();
        ed.execute_cmdline();
        assert_eq!(text(&ed), "baz one baz two\nbaz\n");
        assert_eq!(ed.status.as_deref(), Some("3 substitutions"));
        assert!(!ed.status_err);
        ed.buf_mut().undo();
        assert_eq!(text(&ed), "foo one foo two\nfoo\n");
        ed.cmdline = "r \\bfoo\\b/bar".to_string();
        ed.execute_cmdline();
        assert_eq!(text(&ed), "bar one bar two\nbar\n");
        ed.cmdline = "r zzz/y".to_string();
        ed.execute_cmdline();
        assert!(ed.status_err);
        assert_eq!(text(&ed), "bar one bar two\nbar\n");
        ed.cmdline = "r (bad/x".to_string();
        ed.execute_cmdline();
        assert!(ed.status_err);
        ed.cmdline = "r a*/x".to_string();
        ed.execute_cmdline();
        assert!(ed.status_err);
    }

    #[test]
    fn star_searches_the_word_under_the_cursor() {
        let mut ed = ed_with("alpha beta alpha\nbeta\n");
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.search_cur_word(false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 11 });
        assert_eq!(ed.search.as_deref(), Some("alpha"));
        assert!(!ed.status_err);
        ed.repeat_search(false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 0 });
    }

    #[test]
    fn hash_finds_word_under_cursor_backward() {
        let mut ed = ed_with("foo bar foo\n");
        ed.buf_mut().cursor = Pos { line: 0, col: 8 };
        ed.search_cur_word(true);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 0 });
        assert_eq!(ed.status.as_deref(), Some("[1/2]"));
    }

    #[test]
    fn star_on_punctuation_searches_that_char() {
        let mut ed = ed_with("foo.bar.baz");
        ed.buf_mut().cursor = Pos { line: 0, col: 3 };
        ed.search_cur_word(false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 7 });
        assert_eq!(ed.search.as_deref(), Some("."));
    }

    #[test]
    fn star_on_whitespace_is_a_no_op() {
        let mut ed = ed_with("a b");
        ed.buf_mut().cursor = Pos { line: 0, col: 1 };
        ed.search_cur_word(false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 1 });
        assert_eq!(ed.search, None);
    }

    #[test]
    fn search_wraps_and_reports_missing() {
        let mut ed = ed_with("alpha beta alpha");
        ed.buf_mut().cursor = Pos { line: 0, col: 14 };
        ed.search_step("alpha", false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 0 });
        assert!(!ed.status_err);
        ed.search_step("zzz", false);
        assert!(ed.status_err);
        assert_eq!(ed.status.as_deref(), Some("Pattern not found: zzz"));
    }

    #[test]
    fn search_backward_and_repeat_directions() {
        let mut ed = ed_with("one two one two one");
        ed.buf_mut().cursor = Pos { line: 0, col: 16 };
        ed.search = Some("two".to_string());
        ed.search_back = false;
        ed.repeat_search(false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 4 });
        ed.repeat_search(true);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 12 });
    }

    #[test]
    fn search_reports_the_match_position_and_total() {
        let mut ed = ed_with("one two one two one");
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.search_step("two", false);
        assert_eq!(ed.status.as_deref(), Some("[1/2]"));
        ed.search_step("two", false);
        assert_eq!(ed.status.as_deref(), Some("[2/2]"));
        ed.search_step("two", false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 4 });
        assert_eq!(ed.status.as_deref(), Some("[1/2]"));
        ed.search = Some("two".to_string());
        ed.repeat_search(true);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 12 });
        assert_eq!(ed.status.as_deref(), Some("[2/2]"));
        ed.search_step("one", true);
        assert_eq!(ed.status.as_deref(), Some("[2/3]"));
    }

    #[test]
    fn match_count_spans_lines() {
        let mut ed = ed_with("a\nb\na\na");
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.search_step("a", false);
        assert_eq!(ed.status.as_deref(), Some("[2/3]"));
    }

    #[test]
    fn match_count_follows_case_folding() {
        let mut ed = ed_with("Alpha ALPHA alpha");
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.search_step("alpha", false);
        assert_eq!(ed.status.as_deref(), Some("[2/3]"));
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.search_step("ALPHA", false);
        assert_eq!(ed.status.as_deref(), Some("[1/1]"));
    }

    #[test]
    fn search_case_follows_the_term_casing() {
        let mut ed = ed_with("alpha BETA gamma");
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.open_cmdline(crate::mode::CmdKind::Search);
        ed.cmdline = "beta".to_string();
        ed.commit_search(false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 6 });
        ed.repeat_search(false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 6 });
        ed.open_cmdline(crate::mode::CmdKind::Search);
        ed.cmdline = "Beta".to_string();
        ed.commit_search(false);
        assert!(ed.status_err);
        assert_eq!(ed.status.as_deref(), Some("Pattern not found: Beta"));
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 6 });
        ed.open_cmdline(crate::mode::CmdKind::Search);
        ed.cmdline = "BETA".to_string();
        ed.commit_search(false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 6 });
    }

    #[test]
    fn scrolling_keeps_eight_lines_of_context() {
        let mut ed = ed_with(&"x\n".repeat(100));
        let rows = 24;
        ed.buf_mut().view_top = 0;
        ed.buf_mut().cursor = Pos { line: 20, col: 0 };
        ed.scroll_to_cursor(rows);
        assert_eq!(ed.buf().view_top, 5);
        ed.buf_mut().cursor = Pos { line: 6, col: 0 };
        ed.scroll_to_cursor(rows);
        assert_eq!(ed.buf().view_top, 0);
        ed.buf_mut().cursor = Pos { line: 90, col: 0 };
        ed.scroll_to_cursor(rows);
        assert_eq!(ed.buf().view_top, 75);
        ed.buf_mut().cursor = Pos { line: 84, col: 0 };
        ed.scroll_to_cursor(rows);
        assert_eq!(ed.buf().view_top, 75);
    }

    #[test]
    fn scrolloff_yields_at_the_ends_of_the_buffer() {
        let mut ed = ed_with(&"x\n".repeat(20));
        ed.buf_mut().view_top = 15;
        ed.buf_mut().cursor = Pos { line: 19, col: 0 };
        ed.scroll_to_cursor(24);
        assert_eq!(ed.buf().view_top, 0);

        let mut tiny = ed_with(&"x\n".repeat(4));
        tiny.buf_mut().cursor = Pos { line: 3, col: 0 };
        tiny.scroll_to_cursor(3);
        assert_eq!(tiny.buf().view_top, 2);
    }

    #[test]
    fn write_then_quit_dirty_guards() {
        let dir = std::env::temp_dir().join("deditor_cmd_test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("w.txt");
        let mut ed = Editor::new(Some(&f));
        ed.buf_mut().insert_text("hello\n", false);
        assert!(ed.buf().dirty);
        ed.cmdline = "q".to_string();
        ed.execute_cmdline();
        assert!(!ed.quit);
        assert!(ed.status_err);
        ed.cmdline = "w".to_string();
        ed.execute_cmdline();
        assert!(!ed.buf().dirty);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "hello\n");
        ed.cmdline = "q".to_string();
        ed.execute_cmdline();
        assert!(ed.quit);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn undo_after_write_remarks_dirty_against_written_text() {
        let dir = std::env::temp_dir().join("deditor_undo_dirty_test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("u.txt");
        std::fs::write(&f, "alpha\n").unwrap();
        let mut ed = Editor::new(Some(&f));
        assert!(!ed.buf().dirty);
        ed.buf_mut().insert_text("beta\n", false);
        assert!(ed.buf().dirty);
        ed.buf_mut().undo();
        assert!(!ed.buf().dirty);
        ed.buf_mut().insert_text("gamma\n", false);
        ed.cmdline = "w".to_string();
        ed.execute_cmdline();
        assert!(!ed.buf().dirty);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "gamma\nalpha\n");
        ed.buf_mut().undo();
        assert!(ed.buf().dirty);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "gamma\nalpha\n");
        ed.cmdline = "w".to_string();
        ed.execute_cmdline();
        assert!(!ed.buf().dirty);
        ed.cmdline = "q".to_string();
        ed.execute_cmdline();
        assert!(ed.quit);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edit_command_opens_new_buffer() {
        let dir = std::env::temp_dir().join("deditor_cmd_test_e");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("e.txt");
        std::fs::write(&f, "loaded text\n").unwrap();
        let mut ed = ed_with("working\n");
        ed.cmdline = format!("e {}", f.display());
        ed.execute_cmdline();
        assert_eq!(ed.buffers.len(), 2);
        assert_eq!(ed.active, 1);
        assert_eq!(text(&ed), "loaded text\n");
        assert_eq!(ed.buf().display_name(), "e.txt");
        assert!(!ed.buf().dirty);
        assert_eq!(ed.buffers[0].rope.to_string(), "working\n");
        ed.cmdline = "e /nonexistent/nope.txt".to_string();
        ed.execute_cmdline();
        assert!(!ed.status_err);
        assert_eq!(ed.buffers.len(), 3);
        assert_eq!(ed.active, 2);
        assert_eq!(text(&ed), "");
        assert_eq!(ed.buf().display_name(), "nope.txt");
        ed.cmdline = format!("e {}", f.display());
        ed.execute_cmdline();
        assert_eq!(ed.buffers.len(), 3);
        assert_eq!(ed.active, 1);
        assert_eq!(ed.status.as_deref(), Some("\"e.txt\" already open"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edit_new_file_makes_a_blank_named_buffer_written_on_save() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("deditor_newfile_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let mut ed = ed_with("draft\n");
        ed.cmdline = "e blank.txt".to_string();
        ed.execute_cmdline();
        assert!(!ed.status_err);
        assert_eq!(ed.status.as_deref(), Some("[New File]"));
        assert_eq!(ed.buffers.len(), 2);
        assert_eq!(ed.active, 1);
        assert_eq!(ed.buffers[0].rope.to_string(), "draft\n");
        assert_eq!(text(&ed), "");
        assert_eq!(ed.buf().display_name(), "blank.txt");
        assert!(!std::fs::exists(dir.join("blank.txt")).unwrap());
        ed.buf_mut().insert_text("hello\n", false);
        ed.cmdline = "w".to_string();
        ed.execute_cmdline();
        assert!(!ed.status_err);
        assert_eq!(
            std::fs::read_to_string(dir.join("blank.txt")).unwrap(),
            "hello\n"
        );
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn vertical_motion_keeps_its_column_across_wraps() {
        let mut ed = ed_with("abcdefghij\nsecond");
        ed.view_cols = 5;
        ed.buf_mut().cursor = Pos { line: 0, col: 7 };
        ed.run_motion(Motion::J, 1);
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 2 });
        ed.run_motion(Motion::K, 1);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 7 });
        ed.run_motion(Motion::Zero, 1);
        ed.run_motion(Motion::J, 1);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 5 });
        ed.run_motion(Motion::J, 1);
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 0 });
    }

    #[test]
    fn scrolling_follows_the_cursor_through_a_wrapped_line() {
        let long = "x".repeat(40);
        let mut ed = ed_with(&format!("{long}\nend"));
        ed.view_cols = 10;
        ed.buf_mut().cursor = Pos { line: 0, col: 39 };
        ed.scroll_to_cursor(4);
        let b = ed.buf();
        assert_eq!(b.view_top, 0);
        assert_eq!(b.view_sub, 1);
        assert_eq!(
            ed.wrap()
                .screen_row(&b.rope, b.view_top, b.view_sub, b.cursor),
            Some(2)
        );
    }

    #[test]
    fn centering_puts_the_cursor_mid_screen() {
        let mut ed = ed_with(&"x\n".repeat(100));
        ed.buf_mut().cursor = Pos { line: 40, col: 0 };
        ed.center_cursor(10);
        let b = ed.buf();
        assert_eq!((b.view_top, b.view_sub), (35, 0));
        assert_eq!(
            ed.wrap()
                .screen_row(&b.rope, b.view_top, b.view_sub, b.cursor),
            Some(5)
        );
    }

    #[test]
    fn centering_stays_flush_with_the_end_of_the_buffer() {
        let mut ed = ed_with(&"x\n".repeat(10));
        ed.buf_mut().cursor = Pos { line: 9, col: 0 };
        ed.center_cursor(6);
        let b = ed.buf();
        assert_eq!((b.view_top, b.view_sub), (5, 0));
        assert_eq!(
            ed.wrap()
                .screen_row(&b.rope, b.view_top, b.view_sub, b.cursor),
            Some(4)
        );

        ed.buf_mut().cursor = Pos { line: 10, col: 0 };
        ed.center_cursor(6);
        assert_eq!((ed.buf().view_top, ed.buf().view_sub), (5, 0));

        let mut short = ed_with("a\nb\n");
        short.buf_mut().cursor = Pos { line: 2, col: 0 };
        short.center_cursor(10);
        assert_eq!((short.buf().view_top, short.buf().view_sub), (0, 0));
    }

    #[test]
    fn centering_counts_wrapped_rows() {
        let long = "y".repeat(20);
        let mut ed = ed_with(&format!("{}\n", long).repeat(8));
        ed.view_cols = 10;
        ed.buf_mut().cursor = Pos { line: 5, col: 15 };
        ed.center_cursor(6);
        let b = ed.buf();
        assert_eq!((b.view_top, b.view_sub), (4, 0));
        assert_eq!(
            ed.wrap()
                .screen_row(&b.rope, b.view_top, b.view_sub, b.cursor),
            Some(3)
        );
    }

    #[test]
    fn centering_survives_the_frame_scroll_clamp() {
        let mut ed = ed_with(&"x\n".repeat(100));
        ed.buf_mut().cursor = Pos { line: 40, col: 0 };
        ed.center_cursor(10);
        let before = (ed.buf().view_top, ed.buf().view_sub);
        ed.scroll_to_cursor(10);
        assert_eq!((ed.buf().view_top, ed.buf().view_sub), before);
    }
    #[test]
    fn open_below_on_the_last_line_moves_cursor_to_the_new_line() {
        let mut ed = ed_with("abc\n");
        ed.buf_mut().cursor = Pos { line: 1, col: 0 };
        ed.new_line_below();
        assert_eq!(ed.buf().rope.to_string(), "abc\n\n");
        assert_eq!(ed.buf().cursor, Pos { line: 2, col: 0 });
        assert_eq!(ed.mode, Mode::Insert);
    }

    #[test]
    fn open_path_reuses_the_buffer_already_open_for_that_file() {
        let dir = std::env::temp_dir().join("deditor_open_dedup");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("dup.txt");
        std::fs::write(&f, "dup text\n").unwrap();
        let mut ed = ed_with("keep\n");
        ed.open_path(f.clone());
        assert_eq!(ed.buffers.len(), 2);
        assert_eq!(ed.active, 1);
        assert_eq!(text(&ed), "dup text\n");
        ed.open_path(f.clone());
        assert_eq!(ed.buffers.len(), 2);
        assert_eq!(ed.active, 1);
        ed.open_path(dir.join(".").join("dup.txt"));
        assert_eq!(ed.buffers.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn files_panel_reopens_existing_buffer_instead_of_a_new_tab() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("deditor_panel_dup_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("p.txt"), "panel text\n").unwrap();
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let mut ed = ed_with("keep\n");
        for _ in 0..2 {
            ed.panel = crate::fuzzy::Panel::Files;
            ed.panel_query = String::new();
            ed.panel_items = vec!["p.txt".to_string()];
            ed.panel_order = vec![0];
            ed.panel_cursor = 0;
            ed.panel_accept();
        }
        assert_eq!(ed.buffers.len(), 2);
        assert_eq!(ed.active, 1);
        assert_eq!(text(&ed), "panel text\n");
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn replace_command_handles_length_change_and_unicode() {
        let mut ed = ed_with("héllo wörld foo bar foo\n");
        ed.config.clipboard = false;
        ed.cmdline = "r foo/looooong".to_string();
        ed.execute_cmdline();
        assert_eq!(text(&ed), "héllo wörld looooong bar looooong\n");
        ed.buf_mut().undo();
        assert_eq!(text(&ed), "héllo wörld foo bar foo\n");
        ed.buf_mut().redo_action();
        assert_eq!(text(&ed), "héllo wörld looooong bar looooong\n");
    }

    #[test]
    fn linewise_change_deletes_all_selected_lines() {
        let mut ed = ed_with("one\ntwo\nthree\n");
        ed.config.clipboard = false;
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.start_visual(true);
        ed.buf_mut().cursor = Pos { line: 2, col: 0 };
        let ((a, b), lw) = ed.sel_range();
        ed.op_apply(crate::mode::Operator::Change, (a, b), lw);
        assert_eq!(text(&ed), "");
        assert_eq!(ed.mode, Mode::Insert);
        ed.leave_insert();
        ed.buf_mut().undo();
        assert_eq!(text(&ed), "one\ntwo\nthree\n");
    }

    #[test]
    fn block_delete_is_undoable() {
        let mut ed = ed_with("aaaa\nbbbb\ncccc\n");
        ed.config.clipboard = false;
        ed.start_visual(true);
        ed.visual_block = true;
        ed.visual_anchor = Some(Pos { line: 0, col: 1 });
        ed.buf_mut().cursor = Pos { line: 2, col: 2 };
        ed.block_apply(crate::mode::Operator::Delete);
        assert_eq!(text(&ed), "aa\nbb\ncc\n");
        assert_eq!(ed.register, "aa\nbb\ncc");
        ed.buf_mut().undo();
        assert_eq!(text(&ed), "aaaa\nbbbb\ncccc\n");
    }

    #[test]
    fn replace_char_undo_restores_original_char() {
        let mut ed = ed_with("abc");
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.replace_char('x');
        assert_eq!(text(&ed), "xbc");
        ed.buf_mut().undo();
        assert_eq!(text(&ed), "abc");
        ed.buf_mut().redo_action();
        assert_eq!(text(&ed), "xbc");
    }

    #[test]
    fn close_buffer_shifts_active_left() {
        let mut ed = Editor::new(None);
        ed.buffers.push(Buffer::empty());
        ed.buffers.push(Buffer::empty());
        ed.activate(1);
        ed.close_buffer(Some(1), true);
        assert_eq!(ed.active, 0);
        assert_eq!(ed.buffers.len(), 2);
        ed.close_buffer(Some(2), true);
        assert_eq!(ed.active, 0);
        assert_eq!(ed.buffers.len(), 1);
    }

    #[test]
    fn replace_command_rejects_zero_width_matches() {
        let mut ed = ed_with("foo bar\n");
        ed.cmdline = "r \\b/X".to_string();
        ed.execute_cmdline();
        assert!(ed.status_err);
        assert_eq!(text(&ed), "foo bar\n");
    }

    #[test]
    fn charwise_paste_with_newline_lands_on_last_char() {
        let mut ed = ed_with("ab");
        ed.config.clipboard = false;
        ed.register = "x\ny".into();
        ed.reg_linewise = false;
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.paste(true);
        assert_eq!(text(&ed), "ax\nyb");
        assert_eq!(ed.buf().cursor, Pos { line: 1, col: 0 });
    }
}
