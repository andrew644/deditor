use crate::buffer::Pos;
use ropey::{Rope, RopeSlice};

/// Cells a string occupies in the grid: one for most characters, two for the
/// wide ones used by CJK and emoji, none for a combining mark.
pub fn cells(s: &str) -> usize {
    s.chars().map(cell_of).sum()
}

pub fn cell_of(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(1)
}

/// One screen row of a wrapped buffer line: the half-open char range
/// `[start, end)` that the row displays. `start == 0` marks the row that
/// carries the line number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenRow {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

impl ScreenRow {
    pub fn first(&self) -> bool {
        self.start == 0
    }
}

/// The cell grid buffer lines are wrapped onto. Tabs advance to the next stop
/// measured from the start of the row they land on, so a continuation row never
/// overflows the grid and the first row of a line keeps absolute tab stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wrap {
    pub cols: usize,
    pub tab_width: usize,
}

/// The rows of one line, walked in order without building the whole list: a
/// viewport wants the first few rows of a long line, not all of them. `I` is
/// the line's characters, so a rope line can be measured without copying it.
pub struct Rows<'a, I> {
    wrap: &'a Wrap,
    chars: I,
    start: usize,
    ci: usize,
    x: usize,
    done: bool,
}

impl<I: Iterator<Item = char>> Iterator for Rows<'_, I> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        if self.done {
            return None;
        }
        loop {
            let Some(c) = self.chars.next() else {
                self.done = true;
                return Some((self.start, self.ci));
            };
            let w = self.wrap.width(c, self.x);
            if self.x + w > self.wrap.cols && self.x > 0 {
                let row = (self.start, self.ci);
                self.start = self.ci;
                self.x = w;
                self.ci += 1;
                return Some(row);
            }
            self.x += w;
            self.ci += 1;
        }
    }
}

impl Wrap {
    pub fn new(cols: usize, tab_width: usize) -> Self {
        Self {
            cols: cols.max(1),
            tab_width: tab_width.max(1),
        }
    }

    #[cfg(test)]
    fn body<'a>(&self, line: &'a str) -> &'a str {
        line.strip_suffix('\n').unwrap_or(line)
    }

    fn width(&self, c: char, x: usize) -> usize {
        if c == '\t' {
            self.tab_width - (x % self.tab_width)
        } else {
            cell_of(c)
        }
    }

    /// The rows the line wraps onto, in order and without a list of them.
    pub fn walk<I: Iterator<Item = char>>(&self, chars: I) -> Rows<'_, I> {
        Rows {
            wrap: self,
            chars,
            start: 0,
            ci: 0,
            x: 0,
            done: false,
        }
    }

    #[cfg(test)]
    pub fn rows(&self, line: &str) -> Vec<(usize, usize)> {
        self.walk(self.body(line).chars()).collect()
    }

    #[cfg(test)]
    pub fn row_count(&self, line: &str) -> usize {
        self.walk(self.body(line).chars()).count()
    }

    /// Rows of a rope line, counted without copying the line.
    pub fn rope_row_count(&self, rope: &Rope, line: usize) -> usize {
        self.walk(rope.line(line).chars().take_while(|c| *c != '\n'))
            .count()
    }

    /// Rows of a rope line, stopping once `limit` of them are counted: a
    /// caller that only needs to know whether a line fills `limit` rows must
    /// not measure a megabyte-long line to find out.
    pub fn rope_row_count_upto(&self, rope: &Rope, line: usize, limit: usize) -> usize {
        self.walk(rope.line(line).chars().take_while(|c| *c != '\n'))
            .take(limit)
            .count()
    }

    /// Row index of char `col` within its own line, and its x inside that row,
    /// reading the rope line directly and stopping at `col`.
    pub fn rope_pos(&self, rope: &Rope, line: usize, col: usize) -> (usize, usize) {
        let mut row = 0usize;
        let mut x = 0usize;
        for (ci, c) in rope
            .line(line)
            .chars()
            .take_while(|c| *c != '\n')
            .enumerate()
        {
            let w = self.width(c, x);
            if x + w > self.cols && x > 0 {
                row += 1;
                x = 0;
            }
            if ci == col {
                return (row, x);
            }
            x += w;
        }
        (row, x)
    }

    /// Row index of char `col` within its own line, and its x inside that row.
    #[cfg(test)]
    pub fn pos(&self, line: &str, col: usize) -> (usize, usize) {
        let mut row = 0usize;
        let mut x = 0usize;
        for (ci, c) in self.body(line).chars().enumerate() {
            let w = self.width(c, x);
            if x + w > self.cols && x > 0 {
                row += 1;
                x = 0;
            }
            if ci == col {
                return (row, x);
            }
            x += w;
        }
        (row, x)
    }

    /// The line without its newline, so a single row can be read without
    /// copying the whole line.
    fn line_body<'r>(&self, rope: &'r Rope, line: usize) -> RopeSlice<'r> {
        let ls = rope.line(line);
        if line + 1 < rope.len_lines() {
            ls.slice(0..ls.len_chars().saturating_sub(1))
        } else {
            ls
        }
    }

    /// Cell x of char `col` on the row of `line` that begins at `start`: the
    /// same x `rope_pos` reports for a `col` inside that row, reading only
    /// `[start, col)`.
    pub fn row_x(&self, rope: &Rope, line: usize, start: usize, col: usize) -> usize {
        let ls = self.line_body(rope, line);
        let mut x = 0usize;
        for c in ls
            .slice(start.min(ls.len_chars())..col.min(ls.len_chars()))
            .chars()
        {
            x += self.width(c, x);
        }
        x
    }

    /// The text of char range `[start, end)` of a rope line, with tabs expanded
    /// for a row that begins at `start`. With `vis`, blanks render as visible
    /// markers: spaces become middots and tabs a guillemet followed by
    /// middots, each one cell wide.
    pub fn row_text(
        &self,
        rope: &Rope,
        line: usize,
        start: usize,
        end: usize,
        vis: bool,
    ) -> String {
        let ls = self.line_body(rope, line);
        let mut out = String::new();
        let mut x = 0usize;
        for c in ls
            .slice(start.min(ls.len_chars())..end.min(ls.len_chars()))
            .chars()
        {
            if c == '\t' {
                let w = self.tab_width - (x % self.tab_width);
                if vis {
                    out.push('\u{bb}');
                    for _ in 1..w {
                        out.push('\u{b7}');
                    }
                } else {
                    for _ in 0..w {
                        out.push(' ');
                    }
                }
                x += w;
            } else {
                out.push(if vis && c == ' ' { '\u{b7}' } else { c });
                x += 1;
            }
        }
        out
    }

    /// The viewport anchor after scrolling `delta` screen rows, walking by
    /// rows so a partially visible line can scroll row by row.
    pub fn scroll(
        &self,
        rope: &Rope,
        view_top: usize,
        view_sub: usize,
        delta: i64,
    ) -> (usize, usize) {
        let last = rope.len_lines().saturating_sub(1);
        let mut top = view_top.min(last);
        let mut sub = view_sub;
        let mut left = delta;
        while left > 0 && top < last {
            let n = left as usize;
            let avail = self
                .rope_row_count_upto(rope, top, sub + n + 1)
                .saturating_sub(sub)
                .max(1);
            if n < avail {
                sub += n;
                left = 0;
            } else {
                left -= avail as i64;
                top += 1;
                sub = 0;
            }
        }
        while left < 0 {
            let n = -left as usize;
            if n <= sub {
                sub -= n;
                break;
            }
            left += sub as i64 + 1;
            if top == 0 {
                sub = 0;
                break;
            }
            top -= 1;
            sub = self.rope_row_count(rope, top) - 1;
        }
        (top, sub)
    }

    /// The `count` screen rows of the viewport whose first row is `view_sub`
    /// rows into line `view_top`. Rows past the last line come back with
    /// `line == rope.len_lines()` so the caller can pad them.
    pub fn screen_rows(
        &self,
        rope: &Rope,
        view_top: usize,
        view_sub: usize,
        count: usize,
    ) -> Vec<ScreenRow> {
        let mut out = Vec::with_capacity(count);
        let mut line = view_top;
        let mut skip = view_sub;
        while out.len() < count {
            if line >= rope.len_lines() {
                let blank = rope.len_lines();
                out.push(ScreenRow {
                    line: blank,
                    start: 0,
                    end: 0,
                });
                line += 1;
                continue;
            }
            for (start, end) in self.walk(rope.line(line).chars().take_while(|c| *c != '\n')) {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                if out.len() == count {
                    break;
                }
                out.push(ScreenRow { line, start, end });
            }
            line += 1;
        }
        out
    }

    /// Screen row of `pos` inside the viewport, or `None` when it is scrolled
    /// off above the top row.
    pub fn screen_row(
        &self,
        rope: &Rope,
        view_top: usize,
        view_sub: usize,
        pos: Pos,
    ) -> Option<usize> {
        let last = rope.len_lines().saturating_sub(1);
        let view_top = view_top.min(last);
        let pos = Pos {
            line: pos.line.min(last),
            col: pos.col,
        };
        if pos.line < view_top {
            return None;
        }
        let mut row = self.rope_pos(rope, pos.line, pos.col).0;
        if pos.line == view_top {
            let top_rows = self.rope_row_count_upto(rope, view_top, view_sub + 1);
            return row.checked_sub(view_sub.min(top_rows - 1));
        }
        let top_rows = self.rope_row_count(rope, view_top);
        row += top_rows - view_sub.min(top_rows - 1);
        for line in view_top + 1..pos.line {
            row += self.rope_row_count(rope, line);
        }
        Some(row)
    }

    /// Absolute screen row of `pos`, counted from the buffer's first row.
    pub fn row_of(&self, rope: &Rope, pos: Pos) -> usize {
        let line = pos.line.min(rope.len_lines().saturating_sub(1));
        let mut row = 0;
        for l in 0..line {
            row += self.rope_row_count(rope, l);
        }
        row + self.rope_pos(rope, line, pos.col).0
    }

    /// The `(line, sub-row)` holding absolute screen row `row`, clamped to the
    /// buffer's last row.
    pub fn anchor(&self, rope: &Rope, row: usize) -> (usize, usize) {
        let mut left = row;
        for line in 0..rope.len_lines() {
            let rows = self.rope_row_count(rope, line);
            if left < rows {
                return (line, left);
            }
            left -= rows;
        }
        let last = rope.len_lines().saturating_sub(1);
        (last, self.rope_row_count(rope, last).saturating_sub(1))
    }

    /// How many screen rows the whole buffer occupies.
    pub fn total_rows(&self, rope: &Rope) -> usize {
        (0..rope.len_lines())
            .map(|l| self.rope_row_count(rope, l))
            .sum()
    }

    /// The `(line, sub-row)` reached by moving `delta` screen rows from
    /// `pos`, clamped to the first and last row of the buffer.
    pub fn shift(&self, rope: &Rope, pos: Pos, delta: i64) -> (usize, usize) {
        let mut line = pos.line;
        let mut sub = self.rope_pos(rope, line, pos.col).0;
        let last = rope.len_lines().saturating_sub(1);
        let mut step = delta.abs();
        while step > 0 {
            let rows = self.rope_row_count(rope, line);
            if delta > 0 {
                if line == last && sub + 1 >= rows {
                    break;
                }
                if sub + 1 < rows {
                    sub += 1;
                } else {
                    line += 1;
                    sub = 0;
                }
            } else if sub > 0 {
                sub -= 1;
            } else if line > 0 {
                line -= 1;
                sub = self.rope_row_count(rope, line) - 1;
            } else {
                break;
            }
            step -= 1;
        }
        (line, sub)
    }

    /// Char index on row `sub` of `line` whose cell x is nearest `x`, never
    /// past the row's last cell.
    pub fn col_at(&self, rope: &Rope, line: usize, sub: usize, x: usize) -> usize {
        let mut row = (0, 0);
        for (n, r) in self
            .walk(rope.line(line).chars().take_while(|c| *c != '\n'))
            .enumerate()
        {
            row = r;
            if n >= sub {
                break;
            }
        }
        let (start, end) = row;
        let mut cx = 0usize;
        for (ci, c) in rope
            .line(line)
            .chars()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
        {
            if cx >= x {
                return ci;
            }
            cx += self.width(c, cx);
        }
        end.saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    #[test]
    fn wraps_on_cell_boundaries() {
        let w = Wrap::new(4, 4);
        assert_eq!(w.rows("abcdefgh"), vec![(0, 4), (4, 8)]);
        assert_eq!(w.rows("abc"), vec![(0, 3)]);
        assert_eq!(w.rows(""), vec![(0, 0)]);
        assert_eq!(w.rows("abcd\n"), vec![(0, 4)]);
    }

    #[test]
    fn tabs_stay_whole_and_reanchor_per_row() {
        let w = Wrap::new(4, 4);
        let txt = |s: &str, from: usize, to: usize| w.row_text(&rope(s), 0, from, to, false);
        assert_eq!(w.rows("\tab"), vec![(0, 1), (1, 3)]);
        assert_eq!(txt("\tab", 1, 3), "ab");
        assert_eq!(w.rows("\t a"), vec![(0, 1), (1, 3)]);
        assert_eq!(txt("\t a", 1, 3), " a");
    }

    #[test]
    fn vis_markers_stay_one_cell_wide() {
        let w = Wrap::new(10, 4);
        let txt = |s: &str, vis: bool| w.row_text(&rope(s), 0, 0, 3, vis);
        assert_eq!(txt("a b", true), "a\u{b7}b");
        assert_eq!(txt("\tab", true), "\u{bb}\u{b7}\u{b7}\u{b7}ab");
        assert_eq!(txt("a\tb", true), "a\u{bb}\u{b7}\u{b7}b");
        assert_eq!(
            txt("\tab", true).chars().count(),
            txt("\tab", false).chars().count()
        );
    }

    #[test]
    fn position_maps_to_row_and_x() {
        let w = Wrap::new(4, 4);
        let line = "abcdefgh";
        assert_eq!(w.pos(line, 0), (0, 0));
        assert_eq!(w.pos(line, 3), (0, 3));
        assert_eq!(w.pos(line, 4), (1, 0));
        assert_eq!(w.pos(line, 8), (1, 4));
        assert_eq!(w.pos("\tab", 1), (1, 0));
    }

    #[test]
    fn vertical_move_counts_rows_not_lines() {
        let r = rope("aaa\nbbbbbb\nccc");
        let w = Wrap::new(3, 4);
        assert_eq!(w.row_count(&r.line(1).to_string()), 2);
        assert_eq!(w.shift(&r, Pos { line: 1, col: 0 }, 1), (1, 1));
        assert_eq!(w.shift(&r, Pos { line: 1, col: 4 }, 1), (2, 0));
        assert_eq!(w.shift(&r, Pos { line: 2, col: 0 }, -1), (1, 1));
        assert_eq!(w.shift(&r, Pos { line: 1, col: 0 }, -1), (0, 0));
        assert_eq!(w.shift(&r, Pos { line: 0, col: 0 }, -2), (0, 0));
        assert_eq!(w.shift(&r, Pos { line: 2, col: 2 }, 5), (2, 0));
    }

    #[test]
    fn column_nearest_a_preferred_x() {
        let r = rope("abcdefgh");
        let w = Wrap::new(4, 4);
        assert_eq!(w.col_at(&r, 0, 1, 0), 4);
        assert_eq!(w.col_at(&r, 0, 1, 2), 6);
        assert_eq!(w.col_at(&r, 0, 1, 99), 7);
        assert_eq!(w.col_at(&r, 0, 0, 2), 2);
    }

    #[test]
    fn screen_rows_span_the_viewport() {
        let r = rope("aaa\nbbbbbb\nccc\nd");
        let w = Wrap::new(3, 4);
        let rows = w.screen_rows(&r, 0, 0, 4);
        assert_eq!(
            rows.iter()
                .map(|s| (s.line, s.start, s.end))
                .collect::<Vec<_>>(),
            vec![(0, 0, 3), (1, 0, 3), (1, 3, 6), (2, 0, 3)]
        );
        assert!(rows[1].first() && !rows[2].first());
        let past = w.screen_rows(&r, 3, 0, 3);
        assert_eq!(
            past.iter().map(|s| (s.line, s.start)).collect::<Vec<_>>(),
            vec![(3, 0), (4, 0), (4, 0)]
        );
        assert_eq!(w.screen_row(&r, 0, 0, Pos { line: 2, col: 1 }), Some(3));
        assert_eq!(w.screen_row(&r, 1, 0, Pos { line: 2, col: 1 }), Some(2));
        let scrolled = w.screen_rows(&r, 1, 1, 2);
        assert_eq!(
            scrolled
                .iter()
                .map(|s| (s.line, s.start, s.end))
                .collect::<Vec<_>>(),
            vec![(1, 3, 6), (2, 0, 3)]
        );
        assert_eq!(w.screen_row(&r, 1, 1, Pos { line: 1, col: 0 }), None);
        assert_eq!(w.screen_row(&r, 1, 1, Pos { line: 2, col: 1 }), Some(1));
    }

    #[test]
    fn absolute_rows_round_trip_to_anchors() {
        let r = rope("aaa\nbbbbbb\nccc\nd");
        let w = Wrap::new(3, 4);
        assert_eq!(w.total_rows(&r), 5);
        assert_eq!(w.row_of(&r, Pos { line: 0, col: 2 }), 0);
        assert_eq!(w.row_of(&r, Pos { line: 1, col: 4 }), 2);
        assert_eq!(w.row_of(&r, Pos { line: 3, col: 0 }), 4);
        assert_eq!(w.anchor(&r, 0), (0, 0));
        assert_eq!(w.anchor(&r, 2), (1, 1));
        assert_eq!(w.anchor(&r, 4), (3, 0));
        assert_eq!(w.anchor(&r, 99), (3, 0));
    }

    #[test]
    fn scroll_survives_a_stale_sub() {
        let r = rope("short\nnext line\n");
        let w = Wrap::new(40, 4);
        assert_eq!(w.scroll(&r, 0, 9, 1), (1, 0));
    }

    #[test]
    fn wide_glyphs_wrap_by_their_two_cells() {
        let w = Wrap::new(6, 4);
        assert_eq!(w.rows("你好世界"), vec![(0, 3), (3, 4)]);
        assert_eq!(w.row_count("日本語"), 1);
        assert_eq!(w.pos("a😀b", 2), (0, 3));
        assert_eq!(w.pos("a😀b", 3), (0, 4));
    }

    #[test]
    fn row_x_matches_pos_inside_a_row() {
        let w = Wrap::new(4, 4);
        let line = "ab\tc😀de";
        let r = rope(line);
        for row in w.rows(line) {
            for col in row.0..row.1 {
                assert_eq!(w.row_x(&r, 0, row.0, col), w.pos(line, col).1, "col {col}");
            }
        }
    }

    #[test]
    fn row_text_expands_tabs_from_the_row_start() {
        let r = rope("a\tb\nc");
        let w = Wrap::new(8, 4);
        assert_eq!(w.row_text(&r, 0, 0, 3, false), "a   b");
        assert_eq!(w.row_text(&r, 0, 2, 3, false), "b");
        assert_eq!(w.row_text(&r, 0, 0, 3, true), "a\u{bb}\u{b7}\u{b7}b");
        assert_eq!(w.row_text(&r, 1, 0, 1, false), "c");
    }

    #[test]
    fn rope_reading_matches_line_reading() {
        let r = rope("ab\tc😀de\nx\ny\tz");
        let w = Wrap::new(4, 4);
        for line in 0..r.len_lines() {
            let text = r.line(line).to_string();
            assert_eq!(w.rope_row_count(&r, line), w.row_count(&text));
            let end = text.trim_end_matches('\n').chars().count();
            for col in 0..=end {
                assert_eq!(
                    w.rope_pos(&r, line, col),
                    w.pos(&text, col),
                    "line {line} col {col}"
                );
            }
        }
    }

    #[test]
    fn viewport_rows_stop_at_the_row_budget() {
        let r = rope("aaaaa\nbbbbb");
        let w = Wrap::new(2, 4);
        let ids = |rows: &[ScreenRow]| -> Vec<(usize, usize, usize)> {
            rows.iter().map(|s| (s.line, s.start, s.end)).collect()
        };
        assert_eq!(
            ids(&w.screen_rows(&r, 0, 0, 3)),
            vec![(0, 0, 2), (0, 2, 4), (0, 4, 5)]
        );
        assert_eq!(
            ids(&w.screen_rows(&r, 0, 2, 4)),
            vec![(0, 4, 5), (1, 0, 2), (1, 2, 4), (1, 4, 5)]
        );
    }

    #[test]
    fn clicks_below_the_last_row_use_its_last_char() {
        let r = rope("abcdef");
        let w = Wrap::new(2, 4);
        assert_eq!(w.col_at(&r, 0, 0, 0), 0);
        assert_eq!(w.col_at(&r, 0, 1, 0), 2);
        assert_eq!(w.col_at(&r, 0, 9, 0), 4);
    }

    #[test]
    fn row_count_stops_at_the_limit() {
        let r = rope("aaaaaaaaa");
        let w = Wrap::new(2, 4);
        assert_eq!(w.rope_row_count(&r, 0), 5);
        assert_eq!(w.rope_row_count_upto(&r, 0, 3), 3);
        assert_eq!(w.rope_row_count_upto(&r, 0, 9), 5);
    }

    #[test]
    fn caret_row_needs_only_the_rows_above_it() {
        let r = rope(&"x".repeat(2000));
        let w = Wrap::new(10, 4);
        assert_eq!(w.screen_row(&r, 0, 0, Pos { line: 0, col: 0 }), Some(0));
        assert_eq!(
            w.screen_row(&r, 0, 7, Pos { line: 0, col: 80 }),
            Some(1),
            "row 8 of a line whose first seven rows are scrolled off"
        );
        assert_eq!(w.screen_row(&r, 0, 9, Pos { line: 0, col: 0 }), None);
    }

    #[test]
    fn scrolling_forward_leaves_a_line_once_its_rows_run_out() {
        let r = rope("aaaaa\nb");
        let w = Wrap::new(2, 4);
        assert_eq!(w.scroll(&r, 0, 0, 1), (0, 1));
        assert_eq!(w.scroll(&r, 0, 1, 1), (0, 2));
        assert_eq!(w.scroll(&r, 0, 2, 1), (1, 0));
        assert_eq!(w.scroll(&r, 0, 0, 9), (1, 0));
    }
}
