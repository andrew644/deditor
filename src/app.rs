use crate::buffer::{Buffer, Pos, line_chars, line_text};
use crate::config::{Config, RgbTheme};
use crate::mode::{self, Mode};
use crate::window::{Axis, Node, Window};
use std::path::Path;

const SCROLLOFF: usize = 8;
const MIN_SPLIT_COLS: usize = 20;
const MIN_SPLIT_ROWS: usize = 5;

static CLIPBOARD: std::sync::Mutex<Option<arboard::Clipboard>> = std::sync::Mutex::new(None);

fn with_clipboard<T>(
    op: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> crate::Result<T> {
    let mut slot = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(arboard::Clipboard::new()?);
    }
    op(slot.as_mut().unwrap()).map_err(Into::into)
}

fn clipboard_set(text: &str) -> crate::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let text = text.to_string();
    with_clipboard(move |c| c.set_text(text))
}

fn clipboard_get() -> crate::Result<String> {
    let text = with_clipboard(|c| c.get_text())?;
    #[cfg(windows)]
    let text = text.replace("\r\n", "\n");
    Ok(text)
}

pub fn clipboard_shutdown() {
    *CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub struct Editor {
    pub buffers: Vec<Buffer>,
    pub windows: Vec<Window>,
    pub tree: Node,
    pub active_win: usize,
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
    pub prefer_x: Option<usize>,
    pub cmdline: String,
    pub cmdline_col: usize,
    pub cmd_kind: Option<mode::CmdKind>,
    pub search: Option<String>,
    pub search_back: bool,
    pub panel: crate::fuzzy::Panel,
    pub panel_query: String,
    pub panel_items: Vec<String>,
    pub panel_order: Vec<usize>,
    pub panel_cursor: usize,
    pub panel_root: String,
    pub show_ignored: bool,
    pub show_blame: bool,
    pub completions: Option<Completions>,
    pub lsp: crate::lsp::LspManager,
    pub context_menu: Option<String>,
    pub spell: crate::spell::Spell,
}

pub struct Completions {
    pub items: Vec<String>,
    pub order: Vec<usize>,
    pub cursor: usize,
    pub prefix: String,
}

#[derive(PartialEq, Eq, Debug)]
pub enum Cmd {
    Write(bool),
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
    Split(crate::window::Axis),
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
        ("w", "") => Ok(Cmd::Write(bang)),
        ("wq" | "x", "") => Ok(Cmd::WriteQuit(bang)),
        ("q", "") => Ok(Cmd::Quit(bang)),
        ("bnext" | "bn", "") => Ok(Cmd::BNext),
        ("bprev" | "bp", "") => Ok(Cmd::BPrev),
        ("split" | "sp" | "vsplit" | "vs", "") => Ok(Cmd::Split(crate::window::Axis::Columns)),
        ("hsplit" | "hs", "") => Ok(Cmd::Split(crate::window::Axis::Rows)),
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
            windows: vec![Window::new(vec![0])],
            tree: Node::Leaf(0),
            active_win: 0,
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
            prefer_x: None,
            cmdline: String::new(),
            cmdline_col: 0,
            cmd_kind: None,
            search_back: false,
            search: None,
            panel: crate::fuzzy::Panel::None,
            panel_query: String::new(),
            panel_items: Vec::new(),
            panel_order: Vec::new(),
            panel_cursor: 0,
            panel_root: String::new(),
            show_ignored: false,
            show_blame: false,
            completions: None,
            lsp: crate::lsp::LspManager::new(),
            context_menu: None,
            spell: crate::spell::Spell::new(),
        };
        if let Some(p) = path {
            match Buffer::load(p) {
                Ok(b) => ed.buffers[0] = b,
                Err(e) => {
                    ed.buffers[0] = Buffer::new_file(p);
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

    #[cfg(test)]
    pub fn test_new() -> Self {
        let mut ed = Self::new(None);
        ed.config = crate::config::Config::default();
        let (theme, _) = RgbTheme::from_theme_checked(&ed.config.theme);
        ed.theme = theme;
        ed.status = None;
        ed
    }

    pub fn buf(&self) -> &Buffer {
        &self.buffers[self.active()]
    }

    pub fn buf_mut(&mut self) -> &mut Buffer {
        let i = self.windows[self.active_win].cur_buf();
        &mut self.buffers[i]
    }

    pub fn active(&self) -> usize {
        self.windows[self.active_win].cur_buf()
    }

    pub fn win(&self) -> &Window {
        &self.windows[self.active_win]
    }

    pub fn win_mut(&mut self) -> &mut Window {
        &mut self.windows[self.active_win]
    }

    pub fn rows(&self) -> usize {
        self.windows[self.active_win].view_rows
    }

    pub fn cols(&self) -> usize {
        self.windows[self.active_win].view_cols
    }

    fn win_ids(&self) -> Vec<usize> {
        let mut out = Vec::new();
        self.tree.leaves(&mut out);
        out
    }

    pub fn split(&mut self, axis: Axis) {
        let (room, min) = match axis {
            Axis::Columns => (self.win().view_cols, MIN_SPLIT_COLS),
            Axis::Rows => (self.win().view_rows, MIN_SPLIT_ROWS),
        };
        if room < min {
            self.set_status("E36: Not enough room to split".to_string(), true);
            return;
        }
        let id = self.windows.len();
        let mut w = Window::new(self.win().tabs.clone());
        w.cur = self.win().cur;
        if !self.tree.split_leaf(self.active_win, axis, id) {
            return;
        }
        self.windows.push(w);
        self.active_win = id;
        self.set_status(
            format!("[Window {} of {}]", id + 1, self.windows.len()),
            false,
        );
    }

    pub fn focus_next(&mut self) -> bool {
        let ids = self.win_ids();
        if ids.len() < 2 {
            return false;
        }
        let pos = ids.iter().position(|i| *i == self.active_win).unwrap_or(0);
        let next = ids[(pos + 1) % ids.len()];
        let prev = self.active();
        self.active_win = next;
        if self.active() != prev {
            self.reset_buffer_state();
            self.ensure_lsp();
        }
        true
    }

    pub fn close_window(&mut self) -> bool {
        if self.windows.len() < 2 {
            return false;
        }
        let dead = self.active_win;
        let ids = self.win_ids();
        let next = ids
            .iter()
            .copied()
            .find(|i| *i > dead)
            .or_else(|| ids.iter().copied().find(|i| *i < dead))
            .unwrap();
        self.tree.remove_leaf(dead);
        self.tree.drop_id(dead);
        self.windows.remove(dead);
        self.active_win = if next > dead { next - 1 } else { next };
        self.reset_buffer_state();
        self.ensure_lsp();
        true
    }

    pub fn close_window_cmd(&mut self) {
        if self.windows.len() < 2 {
            self.set_status("E444: Cannot close last window".to_string(), true);
            return;
        }
        if self.buf().dirty {
            self.set_status(
                "E37: No write since last change (add ! to override)".to_string(),
                true,
            );
            return;
        }
        self.close_window();
    }

    pub fn buffers_removed(&mut self, removed: usize) {
        for w in 0..self.windows.len() {
            let win = &mut self.windows[w];
            // `cur` is a slot in the tab list, so dropping a tab ahead of the
            // selected one takes a slot with it; dropping one behind does not.
            let before = win.tabs[..win.cur.min(win.tabs.len())].contains(&removed);
            win.tabs = win
                .tabs
                .iter()
                .filter_map(|b| {
                    if *b == removed {
                        None
                    } else {
                        Some(if *b > removed { *b - 1 } else { *b })
                    }
                })
                .collect();
            win.cur = if before {
                win.cur.saturating_sub(1)
            } else {
                win.cur
            }
            .min(win.tabs.len().saturating_sub(1));
            if win.tabs.is_empty() {
                // The buffer this window showed is gone: give it a scratch
                // buffer rather than stealing a file another window is on.
                self.buffers.push(Buffer::empty());
                win.tabs.push(self.buffers.len() - 1);
                win.cur = 0;
            }
        }
    }

    pub fn focus_win(&mut self, win: usize) {
        if win >= self.windows.len() || win == self.active_win {
            return;
        }
        let prev = self.active();
        self.active_win = win;
        if self.active() != prev {
            self.reset_buffer_state();
            self.ensure_lsp();
        }
    }

    pub fn scroll_win_to_cursor(&mut self, win: usize, rows: usize) {
        if win >= self.windows.len() {
            return;
        }
        let saved = self.active_win;
        self.active_win = win;
        self.scroll_to_cursor(rows);
        self.active_win = saved;
    }

    pub fn tab_width(&self) -> usize {
        self.config.tab_width.max(1) as usize
    }

    pub fn expand_tabs(&self, line: &str, upto: usize) -> String {
        let mut out = String::new();
        let tw = self.tab_width();
        for ch in line.chars().take(upto) {
            if ch == '\t' {
                let pad = tw - (crate::wrap::cells(&out) % tw);
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
        self.wrap_with(self.cols())
    }

    pub fn wrap_with(&self, cols: usize) -> crate::wrap::Wrap {
        crate::wrap::Wrap::new(cols, self.config.tab_width as usize)
    }

    /// Paint reads every window's cursor position directly, so each buffer is
    /// pulled back inside its text before a frame is drawn.
    pub fn clamp_cursors(&mut self) {
        for b in &mut self.buffers {
            b.clamp();
        }
    }

    pub fn scroll_to_cursor(&mut self, rows: usize) {
        self.buf_mut().clamp();
        let last = self.buf().rope.len_lines().saturating_sub(1);
        if self.buf().view_top > last {
            let b = self.buf_mut();
            b.view_top = last;
            b.view_sub = 0;
        }
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
        let mut sub = self.buf().view_sub.min(rows_at(top) - 1);
        let wc = wrap
            .pos(
                &crate::buffer::line_text(&self.buf().rope, cursor.line),
                cursor.col,
            )
            .0;
        let want_low = rows - 1 - off;
        let place = |target: usize| -> (usize, usize) {
            if target <= wc {
                return (cursor.line, wc - target);
            }
            let mut need = target - wc;
            let mut line = cursor.line;
            while need > 0 && line > 0 {
                line -= 1;
                let rc = rows_at(line);
                if need > rc {
                    need -= rc;
                } else {
                    return (line, rc - need);
                }
            }
            (0, 0)
        };
        let target = if cursor.line < top {
            Some(off)
        } else if cursor.line == top {
            let rel = wc.saturating_sub(sub);
            if rel < off {
                Some(off)
            } else if rel > want_low {
                Some(want_low)
            } else {
                None
            }
        } else {
            let mut rel = wc + (rows_at(top) - sub);
            let mut line = top + 1;
            while line < cursor.line && rel <= want_low {
                rel += rows_at(line);
                line += 1;
            }
            if rel < off {
                Some(off)
            } else if rel > want_low {
                Some(want_low)
            } else {
                None
            }
        };
        if let Some(target) = target {
            (top, sub) = place(target);
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

    pub fn refresh_spell_spans(&mut self) {
        use std::hash::{Hash, Hasher};
        if !self.config.spellcheck {
            return;
        }
        self.spell.sync_personal(&self.config.spell_dict);
        let last = self.buf().rope.len_lines().saturating_sub(1);
        let start = self.buf().view_top.min(last);
        let end = (self.buf().view_top + self.rows()).min(last);
        let rope = &self.buf().rope;
        let stale: Vec<(usize, u64, String)> = (start..=end)
            .filter_map(|line| {
                let raw = crate::buffer::line_text(rope, line);
                let mut h = std::collections::hash_map::DefaultHasher::new();
                raw.hash(&mut h);
                let hv = h.finish();
                if self.spell.spans.get(&line).is_some_and(|(k, _)| *k == hv) {
                    None
                } else {
                    Some((line, hv, raw.to_string()))
                }
            })
            .collect();
        let spell = &mut self.spell;
        for (line, hv, raw) in stale {
            let spans = spell.spans_for(&raw);
            spell.spans.insert(line, (hv, spans));
        }
    }

    /// Recomputes the gutter marks of the visible buffers. Insert mode is
    /// skipped so that typing never pays for a diff or a blame.
    pub fn refresh_marks(&mut self) {
        if self.mode == Mode::Insert {
            return;
        }
        let want_blame = self.show_blame;
        let mut seen: Vec<usize> = Vec::new();
        for w in &self.windows {
            let b = w.cur_buf();
            if seen.contains(&b) {
                continue;
            }
            seen.push(b);
            let buf = &mut self.buffers[b];
            if buf.sign_rev != buf.revision {
                let path = buf.path.clone();
                let enabled = self.config.git_signs;
                buf.sign_rev = buf.revision;
                buf.signs = match (&path, enabled) {
                    (Some(p), true) => crate::git::changes(p, &buf.rope.to_string()),
                    _ => Vec::new(),
                };
            }
            if !want_blame || buf.blame_rev == buf.revision {
                continue;
            }
            let path = buf.path.clone();
            buf.blame_rev = buf.revision;
            buf.blame = match &path {
                Some(p) => crate::git::blame(p, &buf.rope.to_string()),
                None => Vec::new(),
            };
        }
    }

    pub fn has_signs(&self, b: usize) -> bool {
        self.config.git_signs
            && self.buffers[b]
                .signs
                .iter()
                .any(|s| *s != crate::git::Sign::None)
    }

    /// Shows or hides the author column; the marks are dropped so the next
    /// draw re-blames whatever is visible.
    pub fn toggle_blame(&mut self) {
        self.show_blame = !self.show_blame;
        if self.show_blame {
            for b in &mut self.buffers {
                b.blame_rev = u64::MAX;
            }
        }
        let state = if self.show_blame { "on" } else { "off" };
        self.set_status(format!("git blame: {state}"), false);
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
            let mark = self.buf_mut().change_mark.take();
            if let Some(mark) = mark {
                self.buf_mut().fold_change(mark);
            }
            self.lsp.completion = None;
            self.mode = Mode::Normal;
            let b = self.buf_mut();
            if b.cursor.col > 0 {
                b.cursor.col -= 1;
            }
            b.clamp();
        }
    }

    pub fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
        self.coalesce_open = true;
        self.buf_mut().change_mark = None;
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
        Some((
            anchor.line.min(cur.line),
            anchor.line.max(cur.line),
            anchor.col.min(cur.col),
            anchor.col.max(cur.col),
        ))
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
            let len = line_chars(&self.buf().rope, b.line);
            let end = Pos {
                line: b.line,
                col: (b.col + 1).min(len),
            };
            ((a, end), false)
        }
    }

    pub fn op_apply(&mut self, op: mode::Operator, (a, b): (Pos, Pos), linewise: bool) {
        use mode::Operator as Op;
        if !linewise && a >= b {
            return;
        }
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
                // A linewise range runs on to the next line's first column, so
                // it takes the break that holds the line open: put one back so
                // there is a line to type into, as one undo step. A range that
                // ends at the end of the text, or that took the break above the
                // line instead, already leaves an empty line where the text was.
                let ends_at_eof = self.buf().offset_of(b) >= self.buf().rope.len_chars();
                let restore =
                    linewise && a.col == 0 && b.col == 0 && b.line > a.line && !ends_at_eof;
                if restore {
                    let text = self
                        .buf()
                        .rope
                        .slice(self.buf().offset_of(a)..self.buf().offset_of(b))
                        .to_string();
                    self.buf_mut()
                        .replace_all(vec![(a, text, "\n".to_string())]);
                } else {
                    self.buf_mut().delete_text(a, b);
                }
                if linewise {
                    self.buf_mut().cursor = Pos {
                        line: a.line,
                        col: 0,
                    };
                }
                let mark = self.buf().undo.len();
                self.buf_mut().change_mark = Some(mark);
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
            && let Err(e) = clipboard_set(&self.register)
        {
            eprintln!("deditor: clipboard: {e}");
        }
    }

    fn indent_lines(&mut self, from: usize, to: usize, indent: bool) {
        let tw = self.tab_width();
        let last = self.buf().rope.len_lines().saturating_sub(1);
        let mut edits = Vec::new();
        for line in from..=to.min(last) {
            let at = Pos { line, col: 0 };
            if indent {
                edits.push((at, String::new(), " ".repeat(tw)));
            } else {
                let line_s = crate::buffer::line_text(&self.buf().rope, line);
                let n = line_s.chars().take(tw).take_while(|c| *c == ' ').count();
                if n > 0 {
                    edits.push((at, line_s.chars().take(n).collect(), String::new()));
                }
            }
        }
        if !edits.is_empty() {
            self.buf_mut().replace_all(edits);
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
            clipboard_get().ok()
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
            let lines = self.buf().rope.len_lines();
            let end_line = Pos {
                line: lines - 1,
                col: line_chars(&self.buf().rope, lines - 1),
            };
            let mut edits = Vec::new();
            for (i, seg) in segs.iter().enumerate() {
                let line = c.line + i;
                let len = if line < lines {
                    line_chars(&self.buf().rope, line)
                } else {
                    0
                };
                let pad = " ".repeat(col.saturating_sub(len));
                if line < lines {
                    edits.push((
                        Pos {
                            line,
                            col: len.min(col),
                        },
                        String::new(),
                        format!("{pad}{seg}"),
                    ));
                } else {
                    edits.push((end_line, String::new(), format!("\n{pad}{seg}")));
                }
            }
            if !edits.is_empty() {
                self.buf_mut().replace_all(edits);
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
            // `p` leaves the cursor on the last pasted character, `P` on the
            // first: repeating `P` then stacks copies instead of nesting them.
            if after {
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
        // The count is how many lines take part, so one fewer break goes away;
        // `2J` is the same as `J`.
        let n = (count as usize).saturating_sub(1).max(1);
        let line = self.buf().cursor.line;
        let last = self.buf().rope.len_lines().saturating_sub(1);
        let mut edits = Vec::new();
        let mut acc = line_chars(&self.buf().rope, line);
        let mut col = acc;
        for k in 1..=n {
            if line + k > last {
                break;
            }
            let prev = line + k - 1;
            let prev_len = line_chars(&self.buf().rope, prev);
            let next = crate::buffer::line_text(&self.buf().rope, line + k);
            let stripped = next.trim_start();
            let skip = next.chars().count() - stripped.chars().count();
            let glue = if prev_len > 0 && !stripped.is_empty() {
                " "
            } else {
                ""
            };
            let old = {
                let b = self.buf();
                let s = b.offset_of(Pos {
                    line: prev,
                    col: prev_len,
                });
                let e = b.offset_of(Pos {
                    line: line + k,
                    col: skip,
                });
                b.rope.slice(s..e).to_string()
            };
            edits.push((
                Pos {
                    line: prev,
                    col: prev_len,
                },
                old,
                glue.to_string(),
            ));
            col = acc;
            acc += glue.chars().count() + stripped.chars().count();
        }
        if !edits.is_empty() {
            self.buf_mut().replace_all(edits);
            self.buf_mut().cursor = Pos { line, col };
        }
        self.buf_mut().clamp();
        self.clear_pending();
    }

    fn open_line(&mut self, at: Pos, after: Option<Pos>, ind: String, coalesce: bool) {
        self.buf_mut().cursor = at;
        self.buf_mut().insert_text("\n", coalesce);
        if let Some(p) = after {
            self.buf_mut().cursor = p;
        }
        if !ind.is_empty() {
            self.buf_mut().insert_text(&ind, coalesce);
        }
    }

    pub fn newline_in_insert(&mut self) {
        let c = self.buf().cursor;
        let before: String = crate::buffer::line_text(&self.buf().rope, c.line)
            .chars()
            .take(c.col)
            .collect();
        let ind = crate::buffer::indent_below(&before, self.tab_width());
        self.open_line(c, None, ind, self.coalesce_open);
    }

    pub fn new_line_below(&mut self) {
        let c = self.buf().cursor;
        let line = crate::buffer::line_text(&self.buf().rope, c.line);
        let ind = crate::buffer::indent_below(&line, self.tab_width());
        let len = line_chars(&self.buf().rope, c.line);
        self.open_line(
            Pos {
                line: c.line,
                col: len,
            },
            None,
            ind,
            false,
        );
        self.enter_insert();
    }

    pub fn new_line_above(&mut self) {
        let c = self.buf().cursor;
        let line = crate::buffer::line_text(&self.buf().rope, c.line);
        let ind = crate::buffer::indent_above(&line, self.tab_width());
        let top = Pos {
            line: c.line,
            col: 0,
        };
        self.open_line(top, Some(top), ind, false);
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
            rows: self.rows(),
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

    pub fn move_caret(&mut self, dir: crate::motions::Caret) {
        use crate::motions::{Caret as C, Motion as M};
        if matches!(dir, C::Up | C::Down) {
            self.run_motion(if dir == C::Up { M::K } else { M::J }, 1);
            return;
        }
        let to = {
            let b = self.buf();
            let c = b.cursor;
            let len = line_chars(&b.rope, c.line);
            match dir {
                C::Left if c.col > 0 => Pos {
                    line: c.line,
                    col: c.col - 1,
                },
                C::Left if c.line > 0 => Pos {
                    line: c.line - 1,
                    col: line_chars(&b.rope, c.line - 1),
                },
                C::Right if c.col < len => Pos {
                    line: c.line,
                    col: c.col + 1,
                },
                C::Right if c.line + 1 < b.rope.len_lines() => Pos {
                    line: c.line + 1,
                    col: 0,
                },
                _ => c,
            }
        };
        self.prefer_x = None;
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
                | M::ToLine(_)
                | M::PageDown
                | M::PageUp
                | M::HalfPageDown
                | M::HalfPageUp
        );
        if m == M::L {
            let len = line_chars(&self.buf().rope, from.line);
            let bnd = Pos {
                line: from.line,
                col: (from.col + count.max(1) as usize).min(len),
            };
            if bnd == from {
                self.clear_pending();
                return;
            }
            self.op_apply(op, (from, bnd), false);
            self.clear_pending();
            return;
        }
        if from == to && !linewise && !matches!(m, M::Dollar | M::E | M::BigE) {
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
        self.cmdline_col = 0;
    }

    pub fn close_cmdline(&mut self) {
        self.cmd_kind = None;
        self.cmdline.clear();
        self.cmdline_col = 0;
        self.completions = None;
        self.mode = Mode::Normal;
        self.clear_pending();
    }

    pub fn cmdline_move(&mut self, delta: i32) {
        let len = self.cmdline.chars().count();
        let col = self.cmdline_col.min(len);
        self.cmdline_col = if delta < 0 {
            col.saturating_sub((-delta) as usize)
        } else {
            (col + delta as usize).min(len)
        };
    }

    pub fn cmdline_insert(&mut self, c: char) {
        let col = self.cmdline_col.min(self.cmdline.chars().count());
        let at = self.cmdline_byte(col);
        self.cmdline.insert(at, c);
        self.cmdline_col = col + 1;
        self.sync_completions();
    }

    pub fn cmdline_backspace(&mut self) {
        let col = self.cmdline_col.min(self.cmdline.chars().count());
        if col == 0 {
            return;
        }
        let (a, b) = (self.cmdline_byte(col - 1), self.cmdline_byte(col));
        self.cmdline.replace_range(a..b, "");
        self.cmdline_col = col - 1;
        self.sync_completions();
    }

    pub fn cmdline_delete(&mut self) {
        let count = self.cmdline.chars().count();
        let col = self.cmdline_col.min(count);
        if col == count {
            return;
        }
        let (a, b) = (self.cmdline_byte(col), self.cmdline_byte(col + 1));
        self.cmdline.replace_range(a..b, "");
        self.sync_completions();
    }

    fn cmdline_byte(&self, col: usize) -> usize {
        self.cmdline
            .char_indices()
            .nth(col)
            .map_or(self.cmdline.len(), |(i, _)| i)
    }

    pub fn search_step(&mut self, term: &str, back: bool) {
        if term.is_empty() {
            return;
        }
        let (found, report) = {
            let b = self.buf();
            let off = b.offset_of(b.cursor);
            let t: Vec<char> = term.chars().collect();
            let start = if back { off.saturating_sub(1) } else { off + 1 };
            let fold = crate::search::ignore_case(term);
            let found = crate::search::find_wrap(&b.rope, &t, start, back, fold);
            let report = found.map(|at| crate::search::match_report(&b.rope, &t, at, fold));
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
                self.completions = None;
                self.cmdline.clear();
                self.cmdline_col = 0;
                self.mode = Mode::Normal;
                match cmd {
                    Cmd::Write(force) => self.write_active(force),
                    Cmd::WriteQuit(bang) => {
                        self.write_active(bang);
                        if self.status_err {
                            return;
                        }
                        self.quit_or_close_window(bang);
                    }
                    Cmd::Quit(bang) => self.quit_or_close_window(bang),
                    Cmd::Split(axis) => self.split(axis),
                    Cmd::Edit(path) => {
                        let p = std::path::PathBuf::from(shrink_tilde(&path));
                        self.edit_path_in_new_buffer(p);
                    }
                    Cmd::BNext => self.cycle_tab(true),
                    Cmd::BPrev => self.cycle_tab(false),
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

    pub fn write_active(&mut self, force: bool) {
        if !force && self.buf().changed_on_disk() {
            self.set_status("E13: file changed on disk, use :w!".into(), true);
            return;
        }
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

    pub fn pos_at_window(&self, win: usize, drow: usize, xcell: usize) -> Pos {
        let Some(w) = self.windows.get(win) else {
            return Pos { line: 0, col: 0 };
        };
        let b = &self.buffers[w.cur_buf()];
        let wrap = self.wrap_with(w.view_cols);
        let rows = wrap.screen_rows(&b.rope, b.view_top, b.view_sub, drow + 1);
        let Some(row) = rows.last() else {
            return Pos {
                line: b.view_top,
                col: 0,
            };
        };
        let last = b.rope.len_lines().saturating_sub(1);
        let line = row.line.min(last);
        let sub = wrap
            .rows(&crate::buffer::line_text(&b.rope, line))
            .iter()
            .position(|&(s, _)| s == row.start)
            .unwrap_or(0);
        Pos {
            line,
            col: wrap.col_at(&b.rope, line, sub, xcell),
        }
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
            let line = crate::buffer::line_text(&self.buf().rope, c.line);
            let text = self.buf().rope.to_string();
            self.lsp.hover_at(&p, c, &line, &text);
        }
    }

    pub fn goto_definition_cur(&mut self) {
        if let Some(p) = self.buf().path.clone() {
            let c = self.buf().cursor;
            let line = crate::buffer::line_text(&self.buf().rope, c.line);
            let text = self.buf().rope.to_string();
            self.lsp.goto_definition(&p, c, &line, &text);
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

    fn reset_buffer_state(&mut self) {
        self.lsp.hover = None;
        self.lsp.completion = None;
        self.spell.clear();
        self.mode = Mode::Normal;
        self.pending = mode::Pending::None;
        self.visual_anchor = None;
        self.visual_linewise = false;
        self.visual_block = false;
        self.prefer_x = None;
    }

    pub fn activate(&mut self, i: usize) {
        if i >= self.buffers.len() {
            return;
        }
        let changed = self.active() != i;
        self.win_mut().show(i);
        if changed {
            self.reset_buffer_state();
        }
    }

    fn cycle_tab(&mut self, forward: bool) {
        let before = self.active();
        if forward {
            self.win_mut().next_tab();
        } else {
            self.win_mut().prev_tab();
        }
        if self.active() != before {
            self.reset_buffer_state();
            self.ensure_lsp();
        }
    }

    fn quit_or_close_window(&mut self, bang: bool) {
        if self.windows.len() > 1 {
            if bang {
                self.close_window();
            } else {
                self.close_window_cmd();
            }
            return;
        }
        if self.buffers.iter().any(|b| b.dirty) && !bang {
            self.set_status(
                "E37: No write since last change (add ! to override)".to_string(),
                true,
            );
        } else {
            self.quit = true;
        }
    }

    pub fn close_buffer(&mut self, idx: Option<usize>, bang: bool) {
        let i = match idx {
            Some(n) => match self.win().tabs.get(n - 1) {
                Some(b) => *b,
                None => {
                    self.set_status(format!("E16: Invalid argument: {n}"), true);
                    return;
                }
            },
            None => self.active(),
        };
        if i >= self.buffers.len() {
            self.set_status(format!("E16: Invalid argument: {}", idx.unwrap_or(0)), true);
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
        let before = self.active();
        let was_active = i == before;
        self.buffers.remove(i);
        self.buffers_removed(i);
        if self.buffers.is_empty() {
            self.buffers.push(Buffer::empty());
        }
        if self.win().tabs.is_empty() && self.windows.len() > 1 {
            self.close_window();
            self.set_status(format!("[{name}] closed"), false);
            return;
        }
        let reset = was_active || self.active() != before;
        if reset {
            self.reset_buffer_state();
        }
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
                        self.buf_mut().cursor = goto_pos(&self.buffers[i].rope, line, col);
                        let rows = self.rows().max(1);
                        self.scroll_to_cursor(rows);
                        self.ensure_lsp();
                    } else {
                        self.set_status(format!("cannot open {}", path.display()), true);
                    }
                }
                crate::lsp::Ev::NoDef => {
                    self.set_status("No definition found".into(), true);
                }
                crate::lsp::Ev::Timeout => {
                    self.comp_cancel();
                    self.set_status("LSP request timed out".into(), true);
                }
                crate::lsp::Ev::Died => {
                    self.comp_cancel();
                    self.lsp.hover = None;
                    self.set_status("language server exited".into(), true);
                }
            }
        }
    }

    pub fn toggle_ignored(&mut self) {
        self.show_ignored = !self.show_ignored;
        self.set_status(
            format!(
                "ignored files: {}",
                if self.show_ignored { "shown" } else { "hidden" }
            ),
            false,
        );
        if self.panel == crate::fuzzy::Panel::Files {
            let dir = crate::fuzzy::split_path_query(&self.panel_query)
                .0
                .to_string();
            self.load_files_entries(&dir);
            self.refresh_panel_order();
        }
    }

    pub fn toggle_spell(&mut self) {
        self.config.spellcheck = !self.config.spellcheck;
        if !self.config.spellcheck {
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
        self.panel_root = String::new();
        let files = files_entries("", !self.show_ignored);
        self.panel_items = match kind {
            crate::fuzzy::Panel::Files => files,
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
        self.panel_root.clear();
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
        let dir = crate::fuzzy::split_path_query(&self.panel_query)
            .0
            .to_string();
        if dir == self.panel_root {
            return;
        }
        self.panel_root = dir.clone();
        self.load_files_entries(&dir);
    }

    fn load_files_entries(&mut self, dir: &str) {
        self.panel_items = files_entries(dir, !self.show_ignored);
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
                    self.ensure_lsp();
                }
            }
            crate::fuzzy::Panel::None => {}
            crate::fuzzy::Panel::Settings => {
                let key = crate::config::SETTINGS[idx].to_string();
                crate::config::setting_activate(&mut self.config, &key);
                if key == "spellcheck" {
                    self.spell.clear();
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

    pub fn refresh_completions(&mut self) {
        let Some((prefix, dir, word)) = completion_target(&self.cmdline) else {
            self.completions = None;
            return;
        };
        let items = crate::fuzzy::list_dir(&files_root(&dir));
        let order = crate::fuzzy::fuzzy_filter(&items, &word);
        self.completions = if order.is_empty() {
            None
        } else {
            Some(Completions {
                items,
                order,
                cursor: 0,
                prefix,
            })
        };
    }

    pub fn sync_completions(&mut self) {
        if self.completions.is_some() {
            self.refresh_completions();
        }
    }

    pub fn complete_next(&mut self) {
        if self.completions.is_none() {
            self.refresh_completions();
            self.completion_select(0, true);
        } else {
            self.completion_select(1, true);
        }
    }

    pub fn completion_move(&mut self, delta: i32) {
        if self.completions.is_some() {
            self.completion_select(delta, false);
        }
    }

    fn completion_select(&mut self, delta: i32, wrap: bool) {
        let Some((prefix, name)) = self.completion_step(delta, wrap) else {
            return;
        };
        self.cmdline = format!("{prefix}{name}");
        self.cmdline_col = self.cmdline.chars().count();
        if name.ends_with('/') {
            self.refresh_completions();
        }
    }

    fn completion_step(&mut self, delta: i32, wrap: bool) -> Option<(String, String)> {
        let c = self.completions.as_mut()?;
        let n = c.order.len() as i32;
        if n == 0 {
            return None;
        }
        let cur = c.cursor as i32;
        c.cursor = if wrap {
            ((cur + delta) % n + n) % n
        } else {
            (cur + delta).clamp(0, n - 1)
        } as usize;
        let name = c.items[c.order[c.cursor]].clone();
        Some((c.prefix.clone(), name))
    }

    fn buffer_with_path(&self, p: &std::path::Path) -> Option<usize> {
        self.buffers
            .iter()
            .position(|b| b.path.as_ref().is_some_and(|q| same_path(q, p)))
    }

    pub fn open_path(&mut self, p: std::path::PathBuf) {
        if let Some(i) = self.buffer_with_path(&p) {
            self.activate(i);
            self.ensure_lsp();
            return;
        }
        match Buffer::load(&p) {
            Ok(b) => {
                let reuse = {
                    let cur = self.active();
                    let scratch = {
                        let c = self.buf();
                        !c.dirty && c.rope.len_chars() == 0
                    };
                    scratch && self.windows.iter().filter(|w| w.cur_buf() == cur).count() < 2
                };
                if reuse {
                    let cur = self.active();
                    self.buffers[cur] = b;
                    self.reset_buffer_state();
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
        if let Some(i) = self.buffer_with_path(&p) {
            let name = self.buffers[i].display_name();
            if i != self.active() {
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

/// Where an LSP jump lands. A stale or malformed reply can name a line past
/// the end of the file the definition was found in, so the position is clamped
/// into the buffer that is about to be shown.
fn goto_pos(rope: &ropey::Rope, line: usize, col: usize) -> Pos {
    let line = line.min(rope.len_lines().saturating_sub(1));
    let text = crate::buffer::line_text(rope, line);
    Pos {
        line,
        col: crate::lsp::char_col(&text, col),
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

fn completion_target(cmdline: &str) -> Option<(String, String, String)> {
    let sep = cmdline.find(char::is_whitespace)?;
    if !matches!(&cmdline[..sep], "e" | "edit" | "pwd") {
        return None;
    }
    let arg = cmdline[sep..].trim_start_matches(char::is_whitespace);
    let cut = arg.rfind('/').map(|i| i + 1).unwrap_or(0);
    let dir = &arg[..cut];
    Some((
        format!("{} {dir}", &cmdline[..sep]),
        dir.to_string(),
        arg[cut..].to_string(),
    ))
}

fn files_entries(dir: &str, hide_ignored: bool) -> Vec<String> {
    let root = files_root(dir);
    let mut matchers = ignore::WalkBuilder::new(&root).build_matchers();
    let matcher = matchers
        .first_mut()
        .expect("the walk has one configured root");
    crate::fuzzy::walk_tree(&root, crate::fuzzy::MAX_TREE_ENTRIES, &mut |dir, prefix| {
        crate::fuzzy::visible_entries(dir)
            .into_iter()
            .filter(|(name, is_dir)| {
                !hide_ignored
                    || !matcher
                        .matched(format!("{prefix}{name}"), *is_dir)
                        .is_ignore()
            })
            .collect()
    })
}

fn files_root(dir: &str) -> std::path::PathBuf {
    let expanded = shrink_tilde(dir);
    if expanded.is_empty() {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    } else {
        std::path::PathBuf::from(expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::key_down;
    use crate::mode::Pending;
    use crate::motions::Motion;
    use crate::textobjects::Obj;
    use winit::keyboard::{Key, ModifiersState};

    #[test]
    fn a_goto_past_the_end_of_the_file_lands_inside_it() {
        let rope = ropey::Rope::from_str("one\ntwo");
        assert_eq!(goto_pos(&rope, 9, 0), Pos { line: 1, col: 0 });
        assert_eq!(goto_pos(&rope, 0, 7), Pos { line: 0, col: 3 });
        assert_eq!(goto_pos(&rope, 1, 2), Pos { line: 1, col: 2 });
    }

    #[test]
    fn every_buffer_is_clamped_before_a_frame_is_painted() {
        let mut ed = ed_with("one\ntwo");
        ed.buffers.push(Buffer::empty());
        ed.buffers[0].cursor = Pos { line: 40, col: 9 };
        ed.buffers[1].cursor = Pos { line: 3, col: 2 };
        ed.clamp_cursors();
        assert_eq!(ed.buffers[0].cursor, Pos { line: 1, col: 3 });
        assert_eq!(ed.buffers[1].cursor, Pos { line: 0, col: 0 });
    }

    #[test]
    fn a_block_selection_spans_the_columns_of_both_ends() {
        let mut ed = ed_with("alpha\nbeta\ngamma\n");
        ed.buf_mut().cursor = Pos { line: 0, col: 3 };
        ed.start_visual_block();
        ed.run_motion(Motion::LineDown, 1);
        ed.run_motion(Motion::Zero, 1);
        assert_eq!(ed.block_range(), Some((0, 1, 0, 3)));
    }

    #[test]
    fn closing_a_buffer_keeps_other_windows_on_their_file() {
        let mut ed = ed_with("a");
        ed.buffers.push(Buffer::empty());
        ed.buffers.push(Buffer::empty());
        ed.buffers[1].insert_text("second", false);
        ed.split(crate::window::Axis::Columns);
        ed.windows[0].tabs = vec![0, 1, 2];
        ed.windows[1].tabs = vec![0, 1, 2];
        ed.windows[1].cur = 1;
        let shown = ed.buffers[ed.windows[1].cur_buf()].rope.to_string();
        ed.buffers.remove(0);
        ed.buffers_removed(0);
        assert_eq!(ed.windows[1].tabs, [0, 1]);
        assert_eq!(ed.buffers[ed.windows[1].cur_buf()].rope.to_string(), shown);
    }

    #[test]
    fn closing_the_only_tab_leaves_a_scratch_buffer() {
        let mut ed = ed_with("a");
        ed.buffers.push(Buffer::empty());
        ed.split(crate::window::Axis::Columns);
        ed.windows[1].tabs = vec![0];
        ed.windows[1].cur = 0;
        ed.buffers.remove(0);
        ed.buffers_removed(0);
        let scratch = ed.windows[1].cur_buf();
        assert!(ed.buffers[scratch].path.is_none());
        assert_ne!(scratch, ed.windows[0].cur_buf());
    }

    fn ed_with(s: &str) -> Editor {
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text(s, false);
        ed
    }

    fn git_repo(tag: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("deditor_signs_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, body).unwrap();
        let repo = git2::Repository::init(&dir).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("f.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("t", "t@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        file
    }

    fn editor_on(tag: &str) -> (Editor, std::path::PathBuf) {
        let f = git_repo(tag, "one\ntwo\nthree\n");
        let mut ed = Editor::test_new();
        ed.buffers[0] = crate::buffer::Buffer::load(&f).unwrap();
        ed.buf_mut().cursor = Pos { line: 1, col: 0 };
        ed.buf_mut().insert_text("new\n", false);
        (ed, f)
    }

    #[test]
    fn signs_wait_until_insert_ends() {
        let (mut ed, _) = editor_on("typing");
        ed.mode = Mode::Insert;
        ed.refresh_marks();
        assert!(
            ed.buffers[0].signs.is_empty(),
            "the diff must not run on every keystroke"
        );
        ed.mode = Mode::Normal;
        ed.refresh_marks();
        assert_eq!(
            ed.buffers[0].signs,
            vec![
                crate::git::Sign::None,
                crate::git::Sign::Added,
                crate::git::Sign::None,
                crate::git::Sign::None,
            ]
        );
        assert!(ed.has_signs(0));
    }

    #[test]
    fn blame_is_only_computed_while_it_is_shown() {
        let (mut ed, _) = editor_on("blamehidden");
        ed.refresh_marks();
        assert!(
            ed.buffers[0].blame.is_empty(),
            "a hidden author column must not cost a blame"
        );
        ed.show_blame = true;
        ed.refresh_marks();
        assert_eq!(ed.buffers[0].blame.len(), 4);
        assert_eq!(ed.buffers[0].blame[0].who, "t");
        assert_eq!(ed.buffers[0].blame[1], crate::git::BlameLine::default());
        assert!(
            ed.buffers[0].blame[2..].iter().all(|b| b.who == "t"),
            "hunks a buffer blame carries over from the file must keep their author"
        );
    }

    #[test]
    fn toggling_blame_reblames_the_buffer() {
        let (mut ed, _) = editor_on("blametoggle");
        ed.toggle_blame();
        ed.refresh_marks();
        assert_eq!(ed.status.as_deref(), Some("git blame: on"));
        assert_eq!(ed.buffers[0].blame[0].who, "t");
        ed.buf_mut().insert_text("more\n", false);
        ed.refresh_marks();
        assert_eq!(ed.buffers[0].blame.len(), 5);
        ed.toggle_blame();
        assert_eq!(ed.status.as_deref(), Some("git blame: off"));
        assert!(!ed.show_blame);
    }

    #[test]
    fn disabling_signs_clears_the_gutter() {
        let (mut ed, _) = editor_on("off");
        ed.refresh_marks();
        assert!(ed.has_signs(0));
        ed.config.git_signs = false;
        ed.buffers[0].sign_rev = u64::MAX;
        ed.refresh_marks();
        assert!(ed.buffers[0].signs.is_empty());
        assert!(!ed.has_signs(0));
    }

    #[test]
    fn buffer_without_a_file_has_no_signs() {
        let mut ed = Editor::test_new();
        ed.buf_mut().insert_text("hello\n", false);
        ed.refresh_marks();
        assert!(ed.buffers[0].signs.is_empty());
        assert!(!ed.has_signs(0));
    }

    #[test]
    fn visual_operator_at_virtual_eol_is_inert() {
        let mut ed = ed_with("ab\ncd");
        ed.buf_mut().cursor = Pos { line: 0, col: 2 };
        ed.register = "keep".into();
        ed.start_visual(false);
        let ((a, b), lw) = ed.sel_range();
        ed.op_apply(crate::mode::Operator::Delete, (a, b), lw);
        assert_eq!(ed.buf().rope.to_string(), "ab\ncd");
        assert_eq!(ed.register, "keep");
    }

    #[test]
    fn leaving_insert_steps_back_onto_the_last_char() {
        let mut ed = ed_with("ab");
        ed.enter_insert();
        ed.buf_mut().insert_text("x", false);
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 3 });
        ed.leave_insert();
        assert_eq!(ed.buf().cursor, Pos { line: 0, col: 2 });
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
        assert_eq!(parse_cmd("w"), Ok(Cmd::Write(false)));
        assert_eq!(parse_cmd("w!"), Ok(Cmd::Write(true)));
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
        let mut ed = Editor::test_new();
        ed.buffers[0] = Buffer::new_file(std::path::Path::new("/tmp/deditor_tab_a"));
        ed.buffers
            .push(Buffer::new_file(std::path::Path::new("/tmp/deditor_tab_b")));
        ed.activate(1);
        ed.cmdline = "c".to_string();
        ed.execute_cmdline();
        assert_eq!(ed.buffers.len(), 1);
        assert_eq!(ed.active(), 0);
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
        let mut ed = Editor::test_new();
        ed.buffers[0] = Buffer::new_file(std::path::Path::new("/tmp/deditor_tab_a"));
        ed.buffers
            .push(Buffer::new_file(std::path::Path::new("/tmp/deditor_tab_b")));
        ed.buffers
            .push(Buffer::new_file(std::path::Path::new("/tmp/deditor_tab_c")));
        ed.win_mut().show(1);
        ed.win_mut().show(2);
        ed.close_buffer(Some(1), false);
        assert_eq!(ed.buffers.len(), 2);
        assert_eq!(ed.buffers[0].display_name(), "deditor_tab_b");
        assert_eq!(ed.win().tabs, [0, 1]);
        assert_eq!(ed.buf().display_name(), "deditor_tab_c");
        ed.close_buffer(Some(9), false);
        assert!(ed.status_err);
        assert_eq!(ed.buffers.len(), 2);
    }

    #[test]
    fn pos_at_window_maps_screen_cells_through_wraps() {
        let mut ed = Editor::test_new();
        let body: String = (0..10).map(|i| format!("line {i}\n")).collect();
        ed.buf_mut().insert_text(&body, false);
        assert_eq!(ed.pos_at_window(0, 0, 0), Pos { line: 0, col: 0 });
        assert_eq!(ed.pos_at_window(0, 3, 4), Pos { line: 3, col: 4 });
        assert_eq!(ed.pos_at_window(0, 1, 100), Pos { line: 1, col: 5 });
        ed.buf_mut().view_top = 6;
        assert_eq!(ed.pos_at_window(0, 1, 2), Pos { line: 7, col: 2 });
        assert_eq!(
            ed.pos_at_window(0, 60, 0),
            Pos { line: 10, col: 0 },
            "a click below the text clamps to the last line"
        );
        let mut wide = Editor::test_new();
        wide.buf_mut().insert_text(&"x".repeat(200), false);
        assert_eq!(wide.pos_at_window(0, 2, 5), Pos { line: 0, col: 165 });
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
        let mut ed = Editor::test_new();
        ed.open_panel(crate::fuzzy::Panel::Files);
        assert_eq!(
            ed.panel_items,
            vec![
                "other.txt".to_string(),
                "sub/".to_string(),
                "sub/target.txt".to_string()
            ]
        );
        for c in "sub/".chars() {
            ed.panel_type(c);
        }
        assert_eq!(ed.panel_root, "sub/");
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
    fn files_panel_finds_nested_files_without_browsing() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("deditor_panel_rec_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("src/lsp")).unwrap();
        std::fs::write(dir.join("README.md"), "x\n").unwrap();
        std::fs::write(dir.join("src/lsp/client.rs"), "nested\n").unwrap();
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let mut ed = Editor::test_new();
        ed.open_panel(crate::fuzzy::Panel::Files);
        assert!(ed.panel_items.contains(&"src/lsp/client.rs".to_string()));
        for c in "clent".chars() {
            ed.panel_type(c);
        }
        assert_eq!(ed.panel_items.len(), 4);
        assert_eq!(ed.panel_order.len(), 1);
        assert_eq!(ed.panel_items[ed.panel_order[0]], "src/lsp/client.rs");
        ed.panel_accept();
        assert_eq!(ed.buf().display_name(), "client.rs");
        assert_eq!(text(&ed), "nested\n");
        assert_eq!(ed.panel, crate::fuzzy::Panel::None);
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn files_panel_hides_ignored_files_until_toggled() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("deditor_panel_ignore_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("target/deep")).unwrap();
        std::fs::create_dir_all(dir.join(".git/info")).unwrap();
        std::fs::write(dir.join(".git/info/exclude"), "excluded.txt\n").unwrap();
        std::fs::write(dir.join(".gitignore"), "ignored.txt\ntarget/\n").unwrap();
        std::fs::write(dir.join("excluded.txt"), "w\n").unwrap();
        std::fs::write(dir.join("ignored.txt"), "x\n").unwrap();
        std::fs::write(dir.join("main.rs"), "y\n").unwrap();
        std::fs::write(dir.join("target/deep/out.o"), "z\n").unwrap();
        std::fs::write(dir.join(".dotfile"), "q\n").unwrap();
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let mut ed = Editor::test_new();
        ed.open_panel(crate::fuzzy::Panel::Files);
        assert_eq!(ed.panel_items, vec!["main.rs".to_string()]);
        ed.toggle_ignored();
        assert_eq!(
            ed.panel_items,
            vec![
                "excluded.txt".to_string(),
                "ignored.txt".to_string(),
                "main.rs".to_string(),
                "target/".to_string(),
                "target/deep/".to_string(),
                "target/deep/out.o".to_string()
            ]
        );
        assert_eq!(ed.status.as_deref(), Some("ignored files: shown"));
        ed.toggle_ignored();
        assert_eq!(ed.panel_items, vec!["main.rs".to_string()]);
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn path_completions_include_ignored_files() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = completion_tree("ignored");
        std::fs::write(dir.join(".gitignore"), "*.txt\n").unwrap();
        let saved = std::env::current_dir().unwrap();
        let mut ed = ed_in(&dir);
        ed.cmdline = "e b".to_string();
        ed.complete_next();
        assert_eq!(completion_names(&ed), vec!["beta.rs", "beta.txt"]);
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn selecting_a_directory_narrows_the_panel_to_that_subtree() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("deditor_panel_sub_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("src/lsp")).unwrap();
        std::fs::write(dir.join("README.md"), "x\n").unwrap();
        std::fs::write(dir.join("src/main.rs"), "y\n").unwrap();
        std::fs::write(dir.join("src/lsp/client.rs"), "z\n").unwrap();
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let mut ed = Editor::test_new();
        ed.open_panel(crate::fuzzy::Panel::Files);
        for c in "src/".chars() {
            ed.panel_type(c);
        }
        assert_eq!(ed.panel_root, "src/");
        assert_eq!(
            ed.panel_items,
            vec![
                "lsp/".to_string(),
                "lsp/client.rs".to_string(),
                "main.rs".to_string()
            ]
        );
        ed.panel_accept();
        assert_eq!(ed.panel_query, "src/lsp/");
        assert_eq!(ed.panel_items, vec!["client.rs".to_string()]);
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn typing_a_parent_path_moves_the_panel_search_root() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("deditor_panel_up_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("here")).unwrap();
        std::fs::create_dir_all(dir.join("there")).unwrap();
        std::fs::write(dir.join("here/a.txt"), "a\n").unwrap();
        std::fs::write(dir.join("there/b.txt"), "b\n").unwrap();
        let saved = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.join("here")).unwrap();
        let mut ed = Editor::test_new();
        ed.open_panel(crate::fuzzy::Panel::Files);
        assert_eq!(ed.panel_items, vec!["a.txt".to_string()]);
        for c in "../there/".chars() {
            ed.panel_type(c);
        }
        assert_eq!(ed.panel_root, "../there/");
        assert_eq!(ed.panel_items, vec!["b.txt".to_string()]);
        ed.panel_accept();
        assert_eq!(ed.buf().display_name(), "b.txt");
        assert!(!ed.status_err);
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn files_root_expands_tilde_and_defaults_to_cwd() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    fn write_refuses_a_file_changed_on_disk() {
        let dir = std::env::temp_dir().join(format!("deditor_stale_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.txt");
        std::fs::write(&f, "on disk\n").unwrap();
        let mut ed = Editor::new(Some(&f));
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("in editor\n", false);
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        ed.cmdline = "w".to_string();
        ed.execute_cmdline();
        assert!(ed.status_err, "a stale write must report an error");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "on disk\n");
        ed.cmdline = "w!".to_string();
        ed.execute_cmdline();
        assert!(!ed.status_err);
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "in editor\non disk\n",
            ":w! must write anyway"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn quit_checks_every_buffer() {
        let dir = std::env::temp_dir().join(format!("deditor_quit_all_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("q.txt");
        std::fs::write(&f, "base\n").unwrap();
        let mut ed = Editor::new(Some(&f));
        ed.buffers.push(Buffer::empty());
        ed.activate(1);
        ed.buf_mut().insert_text("scratch\n", false);
        ed.activate(0);
        assert!(!ed.buf().dirty);
        ed.cmdline = "q".to_string();
        ed.execute_cmdline();
        assert!(!ed.quit);
        assert!(ed.status_err);
        ed.cmdline = "q!".to_string();
        ed.execute_cmdline();
        assert!(ed.quit);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_quit_blocks_on_other_dirty_buffers() {
        let dir = std::env::temp_dir().join(format!("deditor_wq_all_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("w.txt");
        std::fs::write(&f, "base\n").unwrap();
        let mut ed = Editor::new(Some(&f));
        ed.buffers.push(Buffer::empty());
        ed.activate(1);
        ed.buf_mut().insert_text("scratch\n", false);
        ed.activate(0);
        ed.cmdline = "wq".to_string();
        ed.execute_cmdline();
        assert!(!ed.buf().dirty);
        assert!(!ed.quit);
        assert!(ed.status_err);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn force_write_quit_discards_other_dirty_buffers() {
        let dir = std::env::temp_dir().join(format!("deditor_wq_force_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("w.txt");
        std::fs::write(&f, "base\n").unwrap();
        let mut ed = Editor::new(Some(&f));
        ed.buffers.push(Buffer::empty());
        ed.activate(1);
        ed.buf_mut().insert_text("scratch\n", false);
        ed.activate(0);
        ed.cmdline = "wq!".to_string();
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
        assert_eq!(ed.active(), 1);
        assert_eq!(text(&ed), "loaded text\n");
        assert_eq!(ed.buf().display_name(), "e.txt");
        assert!(!ed.buf().dirty);
        assert_eq!(ed.buffers[0].rope.to_string(), "working\n");
        ed.cmdline = "e /nonexistent/nope.txt".to_string();
        ed.execute_cmdline();
        assert!(!ed.status_err);
        assert_eq!(ed.buffers.len(), 3);
        assert_eq!(ed.active(), 2);
        assert_eq!(text(&ed), "");
        assert_eq!(ed.buf().display_name(), "nope.txt");
        ed.cmdline = format!("e {}", f.display());
        ed.execute_cmdline();
        assert_eq!(ed.buffers.len(), 3);
        assert_eq!(ed.active(), 1);
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
        assert_eq!(ed.active(), 1);
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
        ed.win_mut().view_cols = 5;
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
        ed.win_mut().view_cols = 10;
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
        ed.win_mut().view_cols = 10;
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
        assert_eq!(ed.active(), 1);
        assert_eq!(text(&ed), "dup text\n");
        ed.open_path(f.clone());
        assert_eq!(ed.buffers.len(), 2);
        assert_eq!(ed.active(), 1);
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
        assert_eq!(ed.active(), 1);
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
    fn linewise_change_at_the_end_of_the_text_empties_the_buffer() {
        let mut ed = ed_with("one\ntwo\nthree\n");
        ed.config.clipboard = false;
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.start_visual(true);
        ed.buf_mut().cursor = Pos { line: 2, col: 0 };
        let ((a, b), lw) = ed.sel_range();
        ed.op_apply(crate::mode::Operator::Change, (a, b), lw);
        // Deleting through the end of the text leaves nothing behind; there is
        // no break to put back, so no line is restored.
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
    fn a_block_paste_past_the_end_adds_the_missing_lines() {
        let mut ed = ed_with("aa\nbb\n");
        ed.config.clipboard = false;
        ed.register = "1\n2\n3".into();
        ed.reg_block = true;
        ed.buf_mut().cursor = Pos { line: 1, col: 1 };
        ed.paste(false);
        assert_eq!(text(&ed), "aa\nb1b\n 2\n 3");
        ed.buf_mut().undo();
        assert_eq!(text(&ed), "aa\nbb\n");
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
        let mut ed = Editor::test_new();
        ed.buffers.push(Buffer::empty());
        ed.buffers.push(Buffer::empty());
        ed.activate(1);
        ed.win_mut().show(2);
        assert_eq!(ed.win().tabs, [0, 1, 2]);
        ed.close_buffer(Some(2), true);
        assert_eq!(ed.buffers.len(), 2);
        assert_eq!(ed.active(), 1);
        ed.close_buffer(Some(2), true);
        assert_eq!(ed.active(), 0);
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

    #[test]
    fn spell_span_cache_recomputes_edited_lines() {
        let mut ed = ed_with("teh one\ntwo teh\n");
        ed.config.spellcheck = true;
        ed.refresh_spell_spans();
        assert_eq!(
            ed.spell.spans.get(&0).map(|(_, s)| s.clone()),
            Some(vec![(0, 3)])
        );
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.buf_mut()
            .delete_text(Pos { line: 0, col: 0 }, Pos { line: 0, col: 3 });
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        ed.buf_mut().insert_text("the", false);
        ed.refresh_spell_spans();
        assert_eq!(ed.spell.spans.get(&0).map(|(_, s)| s.clone()), Some(vec![]));
        assert_eq!(ed.spell.spans.get(&1).map(|(_, s)| s.len()), Some(1));
    }

    #[test]
    fn closing_active_buffer_clears_per_buffer_state() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("aaa", false);
        ed.spell.spans.insert(0, (1, vec![(0, 3)]));
        ed.lsp.hover = Some("h".into());
        ed.lsp.completion = Some(crate::lsp::CompletionState {
            items: Vec::new(),
            selected: 0,
            line: 0,
            col: 0,
            seq: 0,
            prefix: String::new(),
        });
        ed.buffers.push(Buffer::empty());
        ed.start_visual(false);
        ed.close_buffer(None, true);
        // The closed buffer was the window's only tab, so a scratch buffer
        // takes its place: the closed one is gone, the window still shows
        // something unnamed.
        assert_eq!(ed.buffers.len(), 2);
        assert!(ed.buf().path.is_none());
        assert!(ed.spell.spans.is_empty());
        assert!(ed.lsp.hover.is_none());
        assert!(ed.lsp.completion.is_none());
        assert_eq!(ed.mode, Mode::Normal);
        assert!(ed.visual_anchor.is_none());
    }

    #[test]
    fn visual_selection_does_not_follow_a_buffer_switch() {
        let mut ed = Editor::test_new();
        ed.config.clipboard = false;
        ed.buf_mut().insert_text("one\ntwo\nthree\n", false);
        ed.start_visual(true);
        ed.buf_mut().cursor = Pos { line: 2, col: 0 };
        ed.buffers.push(Buffer::empty());
        ed.activate(1);
        ed.buf_mut().insert_text("alpha\nbeta\n", false);
        let before = ed.buffers[1].rope.to_string();
        key_down(
            &mut ed,
            &Key::Character("d".into()),
            ModifiersState::empty(),
        );
        assert_eq!(
            ed.buffers[1].rope.to_string(),
            before,
            "a selection made in another buffer must not delete here"
        );
    }

    #[test]
    fn open_path_replacing_scratch_buffer_clears_state() {
        let dir = std::env::temp_dir().join(format!("deditor_open_reset_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.txt");
        std::fs::write(&f, "hello\n").unwrap();
        let mut ed = Editor::test_new();
        ed.spell.spans.insert(0, (1, vec![(0, 3)]));
        ed.lsp.hover = Some("h".into());
        ed.open_path(f.clone());
        assert!(ed.buf().path.as_ref().is_some_and(|p| p == &f));
        assert!(ed.spell.spans.is_empty());
        assert!(ed.lsp.hover.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[ignore]
    fn system_clipboard_round_trip() {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
            eprintln!("skipping: no display");
            return;
        }
        let text = "deditor smoke é中\nsecond line";
        clipboard_set(text).unwrap();
        assert_eq!(clipboard_get().unwrap(), text);
        if std::process::Command::new("wl-copy")
            .arg("-n")
            .arg("external text")
            .status()
            .is_ok()
        {
            assert_eq!(clipboard_get().unwrap(), "external text");
        }
    }

    #[test]
    fn jumping_to_document_ends_avoids_scanning_the_buffer() {
        let mut ed = Editor::test_new();
        let body: String = (0..120_000).map(|i| format!("line {i}\n")).collect();
        ed.buf_mut().rope = ropey::Rope::from_str(&body);
        ed.buf_mut().after_edit();
        let last = ed.buf().rope.len_lines() - 1;
        ed.buf_mut().cursor = Pos { line: last, col: 0 };
        let t = std::time::Instant::now();
        ed.scroll_to_cursor(50);
        assert_eq!(ed.buf().view_top, last - 49);
        assert_eq!(ed.buf().view_sub, 0);
        assert!(
            t.elapsed().as_secs() < 2,
            "scroll_to_cursor took {:?}",
            t.elapsed()
        );
        ed.buf_mut().cursor = Pos { line: 0, col: 0 };
        let t = std::time::Instant::now();
        ed.scroll_to_cursor(50);
        assert_eq!(ed.buf().view_top, 0);
        assert!(t.elapsed().as_millis() < 500);
    }

    fn completion_tree(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("deditor_complete_{}_{tag}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("beta.rs"), "a").unwrap();
        std::fs::write(dir.join("beta.txt"), "b").unwrap();
        std::fs::write(dir.join("gamma.rs"), "c").unwrap();
        std::fs::write(dir.join("src").join("main.rs"), "d").unwrap();
        dir
    }

    fn completion_names(ed: &Editor) -> Vec<String> {
        let Some(c) = &ed.completions else {
            return Vec::new();
        };
        c.order.iter().map(|&i| c.items[i].clone()).collect()
    }

    fn press(ed: &mut Editor, keys: &[winit::keyboard::Key]) {
        for k in keys {
            crate::keys::key_down(ed, k, winit::keyboard::ModifiersState::empty());
        }
    }

    fn ch(c: char) -> winit::keyboard::Key {
        winit::keyboard::Key::Character(c.to_string().into())
    }

    fn ed_in(dir: &std::path::Path) -> Editor {
        std::env::set_current_dir(dir).unwrap();
        let mut ed = Editor::test_new();
        ed.open_cmdline(crate::mode::CmdKind::Ex);
        ed.cmdline.clear();
        ed
    }

    #[test]
    fn tab_completion_inserts_a_match_and_keeps_the_list_open() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = completion_tree("insert");
        let saved = std::env::current_dir().unwrap();
        let mut ed = ed_in(&dir);
        ed.cmdline = "e beta".to_string();
        ed.complete_next();
        assert!(ed.cmdline == "e beta.rs" || ed.cmdline == "e beta.txt");
        assert_eq!(completion_names(&ed), vec!["beta.rs", "beta.txt"]);
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn arrows_move_the_path_completion_list() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = completion_tree("arrows");
        let saved = std::env::current_dir().unwrap();
        let mut ed = ed_in(&dir);
        ed.cmdline = "e beta".to_string();
        ed.complete_next();
        let first = ed.cmdline.clone();
        press(
            &mut ed,
            &[winit::keyboard::Key::Named(
                winit::keyboard::NamedKey::ArrowDown,
            )],
        );
        assert!(ed.cmdline.starts_with("e beta."));
        assert_ne!(ed.cmdline, first);
        assert_eq!(ed.cmdline_col, ed.cmdline.chars().count());
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tab_cycles_through_matches_and_up_clamps() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = completion_tree("cycle");
        let saved = std::env::current_dir().unwrap();
        let mut ed = ed_in(&dir);
        ed.cmdline = "e beta".to_string();
        ed.complete_next();
        let first = ed.cmdline.clone();
        ed.completion_move(-1);
        assert_eq!(ed.cmdline, first);
        ed.complete_next();
        let second = ed.cmdline.clone();
        assert_ne!(first, second);
        assert!(second == "e beta.rs" || second == "e beta.txt");
        ed.complete_next();
        assert_eq!(ed.cmdline, first);
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tab_completion_descends_into_directories() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = completion_tree("descend");
        let saved = std::env::current_dir().unwrap();
        let mut ed = ed_in(&dir);
        ed.cmdline = "e s".to_string();
        ed.complete_next();
        assert_eq!(ed.cmdline, "e src/");
        assert_eq!(completion_names(&ed), vec!["main.rs"]);
        ed.complete_next();
        assert_eq!(ed.cmdline, "e src/main.rs");
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn deleting_the_inserted_text_relists_the_directory() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = completion_tree("relist");
        let saved = std::env::current_dir().unwrap();
        let mut ed = ed_in(&dir);
        press(
            &mut ed,
            &[ch('e'), winit::keyboard::NamedKey::Space.into(), ch('b')],
        );
        assert!(ed.completions.is_none());
        press(&mut ed, &[winit::keyboard::NamedKey::Tab.into()]);
        assert_eq!(completion_names(&ed).len(), 2);
        let dels = ed.cmdline.chars().count() - 2;
        for _ in 0..dels {
            press(&mut ed, &[winit::keyboard::NamedKey::Backspace.into()]);
        }
        assert_eq!(ed.cmdline, "e ");
        assert_eq!(completion_names(&ed).len(), 4);
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn escape_hides_the_completion_list_before_the_command_line() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = completion_tree("escape");
        let saved = std::env::current_dir().unwrap();
        let mut ed = ed_in(&dir);
        ed.cmdline = "e beta".to_string();
        ed.complete_next();
        let typed = ed.cmdline.clone();
        press(&mut ed, &[winit::keyboard::NamedKey::Escape.into()]);
        assert!(ed.completions.is_none());
        assert_eq!(ed.cmdline, typed);
        assert_eq!(
            ed.mode,
            crate::mode::Mode::Command(crate::mode::CmdKind::Ex)
        );
        press(&mut ed, &[winit::keyboard::NamedKey::Escape.into()]);
        assert_eq!(ed.mode, crate::mode::Mode::Normal);
        assert!(ed.cmdline.is_empty());
        std::env::set_current_dir(&saved).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_edit_and_pwd_get_path_completions() {
        assert_eq!(completion_target("w alpha"), None);
        assert_eq!(completion_target("e"), None);
        assert_eq!(completion_target("beta.rs"), None);
        assert_eq!(
            completion_target("pwd sr"),
            Some(("pwd ".into(), String::new(), "sr".into()))
        );
        assert_eq!(
            completion_target("edit src/ma"),
            Some(("edit src/".into(), "src/".into(), "ma".into()))
        );
    }

    #[test]
    fn split_commands_stack_windows_on_the_same_tabs() {
        let mut ed = ed_with("one\ntwo\n");
        ed.cmdline = "split".to_string();
        ed.execute_cmdline();
        assert_eq!(ed.windows.len(), 2);
        assert_eq!(ed.active_win, 1);
        assert_eq!(text(&ed), "one\ntwo\n");
        ed.cmdline = "vsplit".to_string();
        ed.execute_cmdline();
        ed.cmdline = "hsplit".to_string();
        ed.execute_cmdline();
        assert_eq!(ed.windows.len(), 4);
        assert_eq!(ed.win_ids(), vec![0, 1, 2, 3]);
        assert!(ed.windows.iter().all(|w| w.tabs == vec![0]));
    }

    #[test]
    fn split_refuses_when_the_window_is_too_narrow() {
        let mut ed = ed_with("one\n");
        ed.win_mut().view_cols = MIN_SPLIT_COLS - 1;
        ed.cmdline = "vsplit".to_string();
        ed.execute_cmdline();
        assert_eq!(ed.windows.len(), 1);
        assert!(ed.status_err);
        assert_eq!(ed.status.as_deref(), Some("E36: Not enough room to split"));
    }

    #[test]
    fn ctrl_w_prefix_cycles_and_closes_windows() {
        let mut ed = ed_with("a\n");
        ed.split(Axis::Columns);
        assert_eq!(ed.active_win, 1);
        let b = ed.buf_mut();
        b.saved = b.rope.clone();
        b.dirty = false;
        key_down(
            &mut ed,
            &Key::Character("w".into()),
            ModifiersState::CONTROL,
        );
        assert_eq!(ed.pending, Pending::WinCmd);
        key_down(
            &mut ed,
            &Key::Character("w".into()),
            ModifiersState::CONTROL,
        );
        assert_eq!(ed.pending, Pending::None);
        assert_eq!(ed.active_win, 0);
        key_down(
            &mut ed,
            &Key::Character("w".into()),
            ModifiersState::CONTROL,
        );
        key_down(
            &mut ed,
            &Key::Character("w".into()),
            ModifiersState::empty(),
        );
        assert_eq!(ed.active_win, 1);
        key_down(
            &mut ed,
            &Key::Character("w".into()),
            ModifiersState::CONTROL,
        );
        key_down(
            &mut ed,
            &Key::Character("c".into()),
            ModifiersState::empty(),
        );
        assert_eq!(ed.windows.len(), 1);
        assert_eq!(ed.active_win, 0);
    }

    #[test]
    fn quit_closes_the_focused_window_and_needs_a_bang_when_dirty() {
        let mut ed = ed_with("a\n");
        ed.split(Axis::Columns);
        ed.buf_mut().insert_text("x", false);
        ed.cmdline = "q".to_string();
        ed.execute_cmdline();
        assert!(ed.status_err);
        assert_eq!(ed.windows.len(), 2);
        ed.cmdline = "q!".to_string();
        ed.execute_cmdline();
        assert_eq!(ed.windows.len(), 1);
        assert_eq!(ed.active_win, 0);
    }

    #[test]
    fn windows_show_independent_buffers() {
        let dir = std::env::temp_dir().join(format!("deditor_split_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        let mut ed = ed_with("a\n");
        ed.split(Axis::Columns);
        ed.open_path(dir.join("b.txt"));
        assert_eq!(ed.win().tabs.len(), 2);
        assert_eq!(text(&ed), "b\n");
        ed.focus_next();
        assert_eq!(ed.active_win, 0);
        assert_eq!(ed.win().tabs.len(), 1);
        assert_eq!(text(&ed), "a\n");
        std::fs::remove_dir_all(&dir).ok();
    }
}
