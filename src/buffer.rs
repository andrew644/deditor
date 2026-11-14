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
    pub dirty: bool,
    pub undo: Vec<UndoEntry>,
    pub redo: Vec<UndoEntry>,
    pub crlf: bool,
}

pub fn line_chars(rope: &Rope, line: usize) -> usize {
    rope.line(line)
        .chars()
        .take_while(|c| *c != '\n')
        .count()
}

pub fn line_text(rope: &Rope, line: usize) -> String {
    rope.line(line).chars().take_while(|c| *c != '\n').collect()
}

impl Buffer {
    pub fn empty() -> Self {
        Self {
            path: None,
            rope: Rope::from_str(""),
            saved: Rope::from_str(""),
            cursor: Pos { line: 0, col: 0 },
            view_top: 0,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            crlf: false,
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
        let line = self.rope.char_to_line(offset.min(self.rope.len_chars().saturating_sub(1)));
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
        self.redo.clear();
        self.refresh_dirty();
    }

    pub fn refresh_dirty(&mut self) {
        self.dirty = self.rope != self.saved;
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
            let start = self.rope.line_to_char(last_at.line) + last_at.col;
            let cont = start + last_text.chars().count() == off;
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
        }
        self.cursor = e.cursor_before;
        self.clamp();
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
        }
        self.cursor = e.cursor_after;
        self.clamp();
        self.refresh_dirty();
        self.undo.push(e);
    }
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
}
