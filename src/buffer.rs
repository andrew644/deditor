use ropey::Rope;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

pub enum Change {
    Insert { at: Pos, text: String },
    Delete { at: Pos, text: String },
    Replace { edits: Vec<(Pos, String, String)> },
}

pub struct UndoEntry {
    pub change: Change,
    pub cursor_before: Pos,
    pub cursor_after: Pos,
}

pub struct Buffer {
    pub path: Option<PathBuf>,
    pub rope: Rope,
    pub saved: Rope,
    pub cursor: Pos,
    pub view_top: usize,
    pub view_sub: usize,
    pub dirty: bool,
    pub undo: Vec<UndoEntry>,
    pub redo: Vec<UndoEntry>,
    pub crlf: bool,
    pub revision: u64,
}

pub fn line_chars(rope: &Rope, line: usize) -> usize {
    rope.line(line).chars().take_while(|c| *c != '\n').count()
}

pub fn line_text(rope: &Rope, line: usize) -> String {
    rope.line(line).chars().take_while(|c| *c != '\n').collect()
}

fn leading_ws(text: &str) -> String {
    text.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

pub fn indent_below(prev: &str, tab_width: usize) -> String {
    let lead = leading_ws(prev);
    let code = code_part(prev.trim_start());
    if code.starts_with('#') {
        return lead;
    }
    match code.chars().last() {
        Some('{') | Some(':') => lead + &" ".repeat(tab_width),
        _ => lead,
    }
}

pub fn indent_above(line: &str, tab_width: usize) -> String {
    let lead = leading_ws(line);
    let code = code_part(line.trim_start());
    if code.starts_with(['}', ')', ']']) {
        return trim_one(&lead, tab_width);
    }
    lead
}

fn trim_one(lead: &str, tab_width: usize) -> String {
    if let Some(rest) = lead.strip_prefix('\t') {
        return rest.to_string();
    }
    lead.chars()
        .skip(tab_width.min(lead.chars().count()))
        .collect()
}

fn code_part(text: &str) -> &str {
    let mut in_str = None;
    let mut prev = '\0';
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match in_str {
            Some(q) => {
                if c == q && prev != '\\' {
                    in_str = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    in_str = Some(c);
                } else if c == '/' && matches!(chars.peek(), Some((_, '/')) | Some((_, '*'))) {
                    return text[..i].trim_end();
                }
            }
        }
        prev = c;
    }
    text.trim_end()
}

impl Buffer {
    pub fn empty() -> Self {
        Self {
            path: None,
            rope: Rope::from_str(""),
            saved: Rope::from_str(""),
            cursor: Pos { line: 0, col: 0 },
            view_top: 0,
            view_sub: 0,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            crlf: false,
            revision: 0,
        }
    }

    pub fn load(path: &Path) -> crate::Result<Self> {
        let bytes = fs::read(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("open {}: not valid UTF-8", path.display()))?;
        let crlf = text.contains("\r\n");
        let text = text.replace("\r\n", "\n");
        let rope = Rope::from_str(&text);
        Ok(Self {
            path: Some(path.to_path_buf()),
            saved: rope.clone(),
            rope,
            crlf,
            ..Self::empty()
        })
    }

    pub fn new_file(path: &Path) -> Self {
        Self {
            path: Some(path.to_path_buf()),
            ..Self::empty()
        }
    }

    pub fn save(&self) -> crate::Result<()> {
        let path = self.path.as_ref().ok_or("no file name")?;
        self.save_as(path)
    }

    pub fn save_as(&self, path: &Path) -> crate::Result<()> {
        let mut text = self.rope.to_string();
        text = text.replace("\r\n", "\n");
        if self.crlf {
            text = text.replace('\n', "\r\n");
        }
        fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(())
    }

    pub fn display_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "[No Name]".into())
    }

    pub fn offset_of(&self, p: Pos) -> usize {
        self.rope.line_to_char(p.line) + p.col
    }
    pub fn mode_at_eol(&self, c: Pos) -> bool {
        c.col >= line_chars(&self.rope, c.line).saturating_sub(1)
    }

    pub fn pos_of(&self, offset: usize) -> Pos {
        let line = self.rope.char_to_line(offset.min(self.rope.len_chars()));
        Pos {
            line,
            col: offset - self.rope.line_to_char(line),
        }
    }

    pub fn clamp(&mut self) {
        let lines = self.rope.len_lines().saturating_sub(1);
        if self.cursor.line > lines {
            self.cursor.line = lines;
        }
        let len = line_chars(&self.rope, self.cursor.line);
        if self.cursor.col > len {
            self.cursor.col = len;
        }
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.rope.clone();
        self.dirty = false;
    }

    pub fn after_edit(&mut self) {
        self.revision += 1;
        self.redo.clear();
        self.refresh_dirty();
    }

    pub fn refresh_dirty(&mut self) {
        self.dirty = self.rope != self.saved;
    }

    pub fn dedent_line(&mut self, line: usize, tab_width: usize) -> usize {
        let lead = leading_ws(&line_text(&self.rope, line));
        if lead.is_empty() {
            return 0;
        }
        let chars = lead.chars().count();
        let remove = if lead.starts_with('\t') {
            1
        } else {
            tab_width.min(chars)
        };
        self.delete_text(Pos { line, col: 0 }, Pos { line, col: remove });
        remove
    }

    pub fn insert_text(&mut self, text: &str, coalesce: bool) {
        let at = self.cursor;
        let before = self.rope.len_chars();
        let off = self.offset_of(at);
        self.rope.insert(off, text);
        let _ = before;
        self.cursor = self.pos_of(off + text.chars().count());
        self.after_edit();
        if coalesce
            && let Some(last) = self.undo.last_mut()
            && let Change::Insert {
                at: last_at,
                text: last_text,
            } = &mut last.change
        {
            let start = if last_at.line < self.rope.len_lines() {
                self.rope.line_to_char(last_at.line) + last_at.col
            } else {
                usize::MAX
            };
            let cont = start.checked_add(last_text.chars().count()) == Some(off);
            if cont {
                last_text.push_str(text);
                last.cursor_after = self.cursor;
                return;
            }
        }
        self.undo.push(UndoEntry {
            change: Change::Insert {
                at,
                text: text.to_string(),
            },
            cursor_before: at,
            cursor_after: self.cursor,
        });
    }
    pub fn delete_text(&mut self, from: Pos, to: Pos) -> String {
        let (start, end) = if (from.line, from.col) <= (to.line, to.col) {
            (from, to)
        } else {
            (to, from)
        };
        let cursor_before = self.cursor;
        let s = self.offset_of(start);
        let e = self.offset_of(end);
        let text = self.rope.slice(s..e).to_string();
        self.rope.remove(s..e);
        self.cursor = start;
        self.after_edit();
        self.undo.push(UndoEntry {
            change: Change::Delete {
                at: start,
                text: text.clone(),
            },
            cursor_before,
            cursor_after: start,
        });
        text
    }

    pub fn undo(&mut self) {
        let Some(e) = self.undo.pop() else {
            return;
        };
        match &e.change {
            Change::Insert { at, text } => {
                let off = self.offset_of(*at);
                self.rope.remove(off..off + text.chars().count());
            }
            Change::Delete { at, text } => {
                let off = self.offset_of(*at);
                self.rope.insert(off, text);
            }
            Change::Replace { edits } => {
                let prefixes = replace_prefix_shifts(edits);
                for ((at, old, new), pre) in edits.iter().zip(&prefixes).rev() {
                    let off = shifted_offset(self, *at, *pre);
                    self.rope.remove(off..off + new.chars().count());
                    self.rope.insert(off, old);
                }
            }
        }
        self.cursor = e.cursor_before;
        self.clamp();
        self.revision += 1;
        self.refresh_dirty();
        self.redo.push(e);
    }

    pub fn redo_action(&mut self) {
        let Some(e) = self.redo.pop() else {
            return;
        };
        match &e.change {
            Change::Insert { at, text } => {
                let off = self.offset_of(*at);
                self.rope.insert(off, text);
            }
            Change::Delete { at, text } => {
                let off = self.offset_of(*at);
                self.rope.remove(off..off + text.chars().count());
            }
            Change::Replace { edits } => {
                for (at, old, new) in edits.iter().rev() {
                    let off = self.offset_of(*at);
                    self.rope.remove(off..off + old.chars().count());
                    self.rope.insert(off, new);
                }
            }
        }
        self.cursor = e.cursor_after;
        self.clamp();
        self.revision += 1;
        self.refresh_dirty();
        self.undo.push(e);
    }

    pub fn replace_all(&mut self, edits: Vec<(Pos, String, String)>) {
        let cursor_before = self.cursor;
        for (at, old, new) in edits.iter().rev() {
            let off = self.offset_of(*at);
            self.rope.remove(off..off + old.chars().count());
            self.rope.insert(off, new);
        }
        self.clamp();
        let cursor_after = self.cursor;
        self.after_edit();
        self.undo.push(UndoEntry {
            change: Change::Replace { edits },
            cursor_before,
            cursor_after,
        });
    }
}

fn replace_prefix_shifts(edits: &[(Pos, String, String)]) -> Vec<i64> {
    let mut prefixes = Vec::with_capacity(edits.len());
    let mut acc: i64 = 0;
    let mut prev_line = None;
    for (at, old, new) in edits {
        if prev_line != Some(at.line) {
            acc = 0;
            prev_line = Some(at.line);
        }
        prefixes.push(acc);
        acc += new.chars().count() as i64 - old.chars().count() as i64;
    }
    prefixes
}

fn shifted_offset(b: &Buffer, at: Pos, pre: i64) -> usize {
    (b.offset_of(at) as i64 + pre).clamp(0, b.rope.len_chars() as i64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_back_to_saved_content_clears_dirty() {
        let mut b = Buffer::empty();
        b.insert_text("one\ntwo\n", false);
        assert!(b.dirty);
        b.mark_saved();
        assert!(!b.dirty);
        b.cursor = Pos { line: 1, col: 0 };
        b.delete_text(Pos { line: 1, col: 0 }, Pos { line: 1, col: 3 });
        assert!(b.dirty);
        b.undo();
        assert_eq!(b.rope.to_string(), "one\ntwo\n");
        assert!(!b.dirty);
        b.redo_action();
        assert!(b.dirty);
    }

    #[test]
    fn unnamed_buffer_is_clean_when_content_returns_to_empty() {
        let mut b = Buffer::empty();
        b.insert_text("abc", true);
        assert!(b.dirty);
        b.undo();
        assert!(!b.dirty);
    }

    #[test]
    fn loaded_buffer_tracks_the_file_on_disk() {
        let dir = std::env::temp_dir().join("deditor_saved_test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.txt");
        std::fs::write(&f, "alpha\nbeta\n").unwrap();
        let mut b = Buffer::load(&f).unwrap();
        assert!(!b.dirty);
        let at = Pos { line: 1, col: 0 };
        b.delete_text(at, Pos { line: 1, col: 4 });
        assert!(b.dirty);
        b.undo();
        assert!(!b.dirty);
        b.insert_text("gamma\n", false);
        assert!(b.dirty);
        b.save().unwrap();
        b.mark_saved();
        assert!(!b.dirty);
        b.undo();
        assert!(b.dirty);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn new_lines_inherit_the_indent_level() {
        assert_eq!(indent_below("    foo();", 4), "    ");
        assert_eq!(indent_below("", 4), "");
        assert_eq!(indent_below("\tcall();", 4), "\t");
        assert_eq!(indent_below("    if (x) {", 4), "        ");
        assert_eq!(indent_below("\tswitch (v) {", 4), "\t    ");
        assert_eq!(indent_below("case 1:", 4), "    ");
    }

    #[test]
    fn indent_level_ignores_comments_and_strings() {
        assert_eq!(indent_below("    // note {", 4), "    ");
        assert_eq!(indent_below("    return 1; // }", 4), "    ");
        assert_eq!(indent_below("    s = \"{\"", 4), "    ");
        assert_eq!(indent_below("#define X {", 4), "");
        assert_eq!(indent_below("    #pragma once", 4), "    ");
    }

    #[test]
    fn closing_lines_dedent_one_level() {
        assert_eq!(indent_above("    }", 4), "");
        assert_eq!(indent_above("        }", 4), "    ");
        assert_eq!(indent_above("\t}", 4), "");
        assert_eq!(indent_above("}", 4), "");
        assert_eq!(indent_above("    foo();", 4), "    ");
    }

    #[test]
    fn dedent_line_removes_one_indent_unit() {
        let mut b = Buffer::empty();
        b.insert_text("    x\n", false);
        assert_eq!(b.dedent_line(0, 4), 4);
        assert_eq!(b.rope.to_string(), "x\n");
        assert_eq!(b.dedent_line(0, 4), 0);

        let mut short = Buffer::empty();
        short.insert_text("  y\n", false);
        assert_eq!(short.dedent_line(0, 4), 2);
        assert_eq!(short.rope.to_string(), "y\n");

        let mut tabs = Buffer::empty();
        tabs.insert_text("\t\tx\n", false);
        assert_eq!(tabs.dedent_line(0, 4), 1);
        assert_eq!(tabs.rope.to_string(), "\tx\n");
    }

    #[test]
    fn replace_all_undoes_and_redoes_as_one_step() {
        let mut b = Buffer::empty();
        b.insert_text("foo bar foo\n", false);
        b.undo.clear();
        b.mark_saved();
        b.replace_all(vec![
            (
                Pos { line: 0, col: 0 },
                "foo".to_string(),
                "baz".to_string(),
            ),
            (
                Pos { line: 0, col: 8 },
                "foo".to_string(),
                "baz".to_string(),
            ),
        ]);
        assert_eq!(b.rope.to_string(), "baz bar baz\n");
        assert_eq!(b.undo.len(), 1);
        assert!(b.dirty);
        b.undo();
        assert_eq!(b.rope.to_string(), "foo bar foo\n");
        assert!(!b.dirty);
        b.redo_action();
        assert_eq!(b.rope.to_string(), "baz bar baz\n");
        assert!(b.dirty);
    }

    #[test]
    fn pos_of_resolves_the_offset_just_past_a_line_break() {
        let mut b = Buffer::empty();
        b.insert_text("abc\n", false);
        assert_eq!(b.pos_of(4), Pos { line: 1, col: 0 });
        b.insert_text("\n", false);
        assert_eq!(b.pos_of(5), Pos { line: 2, col: 0 });
        let mut plain = Buffer::empty();
        plain.insert_text("abc", false);
        assert_eq!(plain.pos_of(3), Pos { line: 0, col: 3 });
    }

    #[test]
    fn replace_all_with_length_change_round_trips() {
        let mut b = Buffer::empty();
        b.insert_text("foo bar foo bar\n", false);
        b.replace_all(vec![
            (
                Pos { line: 0, col: 0 },
                "foo".to_string(),
                "quux".to_string(),
            ),
            (
                Pos { line: 0, col: 8 },
                "foo".to_string(),
                "bazzle".to_string(),
            ),
        ]);
        assert_eq!(b.rope.to_string(), "quux bar bazzle bar\n");
        b.undo();
        assert_eq!(b.rope.to_string(), "foo bar foo bar\n");
        b.redo_action();
        assert_eq!(b.rope.to_string(), "quux bar bazzle bar\n");
    }

    #[test]
    fn insert_coalesce_survives_shrunk_buffer() {
        let mut b = Buffer::empty();
        b.insert_text("l0\nl1\n", false);
        b.cursor = Pos { line: 1, col: 0 };
        b.insert_text("q", false);
        b.rope.remove(0..b.rope.len_chars());
        b.cursor = Pos { line: 0, col: 0 };
        b.insert_text("z", true);
        assert_eq!(b.rope.to_string(), "z");
    }
}
