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

    pub fn insert_text(&mut self, text: &str, coalesce: bool) {
        let at = self.cursor;
        let before = self.rope.len_chars();
        let off = self.offset_of(at);
        self.rope.insert(off, text);
        let _ = before;
        self.cursor = self.pos_of(off + text.chars().count());
        self.dirty = true;
        self.redo.clear();
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
        self.dirty = true;
        self.redo.clear();
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
        self.dirty = true;
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
        self.dirty = true;
        self.undo.push(e);
    }
}
