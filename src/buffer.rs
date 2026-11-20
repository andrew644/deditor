use ropey::Rope;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

pub enum Change {
    Insert { at: Pos, text: String },
    Delete { at: Pos, text: String },
    Replace { edits: Vec<(usize, String, String)> },
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
    pub signs: Vec<crate::git::Sign>,
    pub sign_rev: u64,
    pub blame: Vec<crate::git::BlameLine>,
    pub blame_rev: u64,
    pub disk_time: Option<SystemTime>,
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

/// The absolute location a buffer reads and writes. Resolving it when the
/// buffer is created keeps a later `:pwd` from pointing a save at a different
/// file, and gives the LSP client one key to address the document by.
fn own_path(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    match (parent.canonicalize(), path.file_name()) {
        (Ok(dir), Some(name)) => dir.join(name),
        _ => path.to_path_buf(),
    }
}

/// Name of the scratch file a save is staged through before it replaces the
/// real one, so an interrupted write never leaves the file truncated.
fn staged_name(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "buffer".into());
    path.with_file_name(format!(".{name}.{}.deditor-tmp", std::process::id()))
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
            signs: Vec::new(),
            sign_rev: u64::MAX,
            blame: Vec::new(),
            blame_rev: u64::MAX,
            disk_time: None,
        }
    }

    pub fn load(path: &Path) -> crate::Result<Self> {
        let path = own_path(path);
        let bytes = fs::read(&path).map_err(|e| format!("open {}: {e}", path.display()))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("open {}: not valid UTF-8", path.display()))?;
        let crlf = text.contains("\r\n");
        let text = text.replace("\r\n", "\n");
        let rope = Rope::from_str(&text);
        Ok(Self {
            disk_time: Self::stamp(Some(&path)),
            path: Some(path),
            saved: rope.clone(),
            rope,
            crlf,
            ..Self::empty()
        })
    }

    pub fn new_file(path: &Path) -> Self {
        let path = own_path(path);
        Self {
            disk_time: Self::stamp(Some(&path)),
            path: Some(path),
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
        let staged = staged_name(path);
        let stage_and_move = || -> crate::Result<()> {
            fs::write(&staged, &text).map_err(|e| format!("write {}: {e}", path.display()))?;
            if let Ok(meta) = fs::metadata(path) {
                let _ = fs::set_permissions(&staged, meta.permissions());
            }
            fs::rename(&staged, path).map_err(|e| format!("write {}: {e}", path.display()))?;
            Ok(())
        };
        let written = stage_and_move();
        if written.is_err() {
            let _ = fs::remove_file(&staged);
        }
        written
    }

    pub fn display_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "[No Name]".into())
    }

    pub fn offset_of(&self, p: Pos) -> usize {
        self.rope
            .try_line_to_char(p.line)
            .unwrap_or(self.rope.len_chars())
            .saturating_add(p.col)
            .min(self.rope.len_chars())
    }
    pub fn mode_at_eol(&self, c: Pos) -> bool {
        c.col >= line_chars(&self.rope, c.line).saturating_sub(1)
    }

    pub fn pos_of(&self, offset: usize) -> Pos {
        let offset = offset.min(self.rope.len_chars());
        let line = self.rope.char_to_line(offset);
        Pos {
            line,
            col: (offset - self.rope.line_to_char(line)).min(line_chars(&self.rope, line)),
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
        self.disk_time = Self::stamp(self.path.as_deref());
    }

    /// Modification time of the file on disk, if it is there.
    fn stamp(path: Option<&Path>) -> Option<SystemTime> {
        path.and_then(|p| fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
    }

    /// Whether something else has rewritten the file since this buffer read or
    /// wrote it, in which case saving would drop those changes.
    pub fn changed_on_disk(&self) -> bool {
        let Some(written) = self.disk_time else {
            return false;
        };
        Self::stamp(self.path.as_deref()).is_some_and(|m| m != written)
    }

    pub fn after_edit(&mut self) {
        self.revision += 1;
        self.redo.clear();
        self.dirty = true;
    }

    /// Compares the text with the version on disk. Only undo and redo need it:
    /// they can bring the document back to exactly what was saved.
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
        let off = self.offset_of(at);
        self.rope.insert(off, text);
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
        let s = self.offset_of(start).min(self.rope.len_chars());
        let e = self.offset_of(end).min(self.rope.len_chars()).max(s);
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

    fn apply_change(&mut self, change: &Change, reverse: bool) {
        match change {
            Change::Insert { at, text } | Change::Delete { at, text } => {
                let off = self.offset_of(*at);
                let add = matches!(change, Change::Insert { .. }) != reverse;
                if add {
                    self.rope.insert(off, text);
                } else {
                    cut(&mut self.rope, off, text.chars().count());
                }
            }
            Change::Replace { edits } => {
                let prefixes = reverse.then(|| replace_prefix_shifts(edits));
                for (n, (off, old, new)) in edits.iter().enumerate().rev() {
                    let (removed, restored) = if reverse { (new, old) } else { (old, new) };
                    let pre = prefixes.as_ref().map_or(0, |p| p[n]);
                    let o = (*off as i64 + pre).clamp(0, self.rope.len_chars() as i64) as usize;
                    cut(&mut self.rope, o, removed.chars().count());
                    self.rope.insert(o, restored);
                }
            }
        }
    }

    pub fn undo(&mut self) {
        let Some(e) = self.undo.pop() else {
            return;
        };
        self.apply_change(&e.change, true);
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
        self.apply_change(&e.change, false);
        self.cursor = e.cursor_after;
        self.clamp();
        self.revision += 1;
        self.refresh_dirty();
        self.undo.push(e);
    }

    pub fn replace_all(&mut self, edits: Vec<(Pos, String, String)>) {
        let cursor_before = self.cursor;
        let edits: Vec<(usize, String, String)> = edits
            .into_iter()
            .map(|(at, old, new)| (self.offset_of(at).min(self.rope.len_chars()), old, new))
            .collect();
        for (off, old, new) in edits.iter().rev() {
            cut(&mut self.rope, *off, old.chars().count());
            self.rope.insert(*off, new);
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

fn cut(rope: &mut Rope, off: usize, count: usize) {
    let off = off.min(rope.len_chars());
    rope.remove(off..(off + count).min(rope.len_chars()));
}

fn replace_prefix_shifts(edits: &[(usize, String, String)]) -> Vec<i64> {
    let mut prefixes = Vec::with_capacity(edits.len());
    let mut acc: i64 = 0;
    for (_, old, new) in edits {
        prefixes.push(acc);
        acc += new.chars().count() as i64 - old.chars().count() as i64;
    }
    prefixes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_outside_the_text_map_to_its_ends() {
        let mut b = Buffer::empty();
        b.insert_text("one\ntwo", false);
        assert_eq!(b.offset_of(Pos { line: 9, col: 0 }), 7);
        assert_eq!(b.offset_of(Pos { line: 1, col: 40 }), 7);
        assert_eq!(b.pos_of(99), Pos { line: 1, col: 3 });
        assert_eq!(
            b.pos_of(b.offset_of(Pos { line: 1, col: 1 })),
            Pos { line: 1, col: 1 }
        );
    }

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
    fn saving_leaves_only_the_file_behind() {
        let dir = std::env::temp_dir().join(format!("deditor_atomic_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.txt");
        std::fs::write(&f, "old\n").unwrap();
        let mut b = Buffer::load(&f).unwrap();
        b.insert_text("new ", false);
        b.save().unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "new old\n");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_buffer_owns_the_real_file_behind_a_link() {
        let dir = std::env::temp_dir().join(format!("deditor_link_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("real.txt");
        std::fs::write(&real, "x\n").unwrap();
        let link = dir.join("link.txt");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let b = Buffer::load(&link).unwrap();
        assert_eq!(b.path.as_deref(), Some(real.as_path()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_new_file_is_addressed_absolutely() {
        let b = Buffer::new_file(Path::new("Cargo.toml"));
        assert!(b.path.as_ref().is_some_and(|p| p.is_absolute()));
    }

    #[test]
    fn an_edit_of_the_same_length_marks_the_buffer_dirty() {
        let mut b = Buffer::empty();
        b.insert_text("ab\n", false);
        b.mark_saved();
        b.cursor = Pos { line: 0, col: 0 };
        b.delete_text(Pos { line: 0, col: 0 }, Pos { line: 0, col: 1 });
        b.insert_text("c", false);
        assert!(b.dirty, "an edit that keeps the length is still an edit");
        b.undo();
        b.undo();
        assert!(!b.dirty, "undoing back to the saved text clears the flag");
    }

    #[test]
    fn changed_on_disk_notices_an_external_write() {
        let dir = std::env::temp_dir().join("deditor_mtime_test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("m.txt");
        std::fs::write(&f, "alpha\n").unwrap();
        let mut b = Buffer::load(&f).unwrap();
        assert!(!b.changed_on_disk());
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        assert!(
            b.changed_on_disk(),
            "a rewrite by anything else must not be silently overwritten"
        );
        b.mark_saved();
        assert!(!b.changed_on_disk());
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

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    #[test]
    fn fuzz_buffer_ops_keep_cursor_in_bounds() {
        for seed in 1..=30u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut b = Buffer::empty();
            for step in 0..400 {
                let l = rng.below(b.rope.len_lines());
                let col = rng.below(b.rope.line(l).len_chars() + 1);
                b.cursor = Pos { line: l, col };
                match rng.below(6) {
                    0 => b.insert_text("ab\u{e9}cd\n", false),
                    1 => b.insert_text("x", true),
                    2 => {
                        let l2 = rng.below(b.rope.len_lines());
                        let c2 = rng.below(b.rope.line(l2).len_chars() + 1);
                        b.delete_text(Pos { line: l, col }, Pos { line: l2, col: c2 });
                    }
                    3 => b.undo(),
                    4 => b.redo_action(),
                    _ => b.replace_all(vec![(
                        Pos { line: l, col },
                        "ab".to_string(),
                        "q\u{fc}z".to_string(),
                    )]),
                }
                b.clamp();
                assert!(
                    b.cursor.line < b.rope.len_lines(),
                    "seed {seed} step {step} line out of range"
                );
                assert!(
                    b.cursor.col <= b.rope.line(b.cursor.line).len_chars(),
                    "seed {seed} step {step} col out of range"
                );
            }
        }
    }

    #[test]
    fn fuzz_undo_restores_every_previous_snapshot() {
        for seed in 1..=30u64 {
            let mut rng = Rng(seed.wrapping_mul(0xDEAD_BEEF) | 1);
            let mut b = Buffer::empty();
            b.insert_text("hello world\nsecond\n", false);
            let mut snapshots = vec![b.rope.to_string()];
            for step in 0..150 {
                let l = rng.below(b.rope.len_lines());
                let col = rng.below(b.rope.line(l).len_chars() + 1);
                b.cursor = Pos { line: l, col };
                match rng.below(3) {
                    0 => b.insert_text("z\n", false),
                    1 => b.insert_text("q", false),
                    _ => {
                        let l2 = rng.below(b.rope.len_lines());
                        let c2 = rng.below(b.rope.line(l2).len_chars() + 1);
                        b.delete_text(Pos { line: l, col }, Pos { line: l2, col: c2 });
                    }
                }
                snapshots.push(b.rope.to_string());
                b.undo();
                snapshots.pop();
                assert_eq!(
                    b.rope.to_string(),
                    *snapshots.last().unwrap(),
                    "undo mismatch seed {seed} step {step}"
                );
            }
        }
    }

    #[test]
    fn undo_replace_across_lines_round_trips() {
        let mut b = Buffer::empty();
        b.insert_text("aaa\nbbb\n", false);
        b.redo.clear();
        b.replace_all(vec![
            (Pos { line: 0, col: 2 }, "a".to_string(), "XY".to_string()),
            (Pos { line: 1, col: 0 }, "b".to_string(), "Z".to_string()),
        ]);
        assert_eq!(b.rope.to_string(), "aaXY\nZbb\n");
        b.undo();
        assert_eq!(b.rope.to_string(), "aaa\nbbb\n");
        b.redo_action();
        assert_eq!(b.rope.to_string(), "aaXY\nZbb\n");
    }
}
