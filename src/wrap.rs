use crate::buffer::{Pos, line_text};
use ropey::Rope;

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

impl Wrap {
    pub fn new(cols: usize, tab_width: usize) -> Self {
        Self {
            cols: cols.max(1),
            tab_width: tab_width.max(1),
        }
    }

    fn body<'a>(&self, line: &'a str) -> &'a str {
        line.strip_suffix('\n').unwrap_or(line)
    }

    fn width(&self, c: char, x: usize) -> usize {
        if c == '\t' {
            self.tab_width - (x % self.tab_width)
        } else {
            1
        }
    }

    pub fn rows(&self, line: &str) -> Vec<(usize, usize)> {
        let body = self.body(line);
        let mut out = Vec::new();
        let mut start = 0usize;
        let mut ci = 0usize;
        let mut x = 0usize;
        for c in body.chars() {
            let w = self.width(c, x);
            if x + w > self.cols && x > 0 {
                out.push((start, ci));
                start = ci;
                x = 0;
            }
            x += w;
            ci += 1;
        }
        out.push((start, ci));
        out
    }

    pub fn row_count(&self, line: &str) -> usize {
        self.rows(line).len()
    }

    /// Row index of char `col` within its own line, and its x inside that row.
    pub fn pos(&self, line: &str, col: usize) -> (usize, usize) {
        let body = self.body(line);
        let mut row = 0usize;
        let mut x = 0usize;
        for (ci, c) in body.chars().enumerate() {
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

    /// The text of char range `[start, end)` with tabs expanded for its row.
    /// With `vis`, blanks render as visible markers: spaces become middots
    /// and tabs an guillemet followed by middots, each one cell wide.
    pub fn text(&self, line: &str, start: usize, end: usize, vis: bool) -> String {
        let body = self.body(line);
        let mut out = String::new();
        let mut x = 0usize;
        for (ci, c) in body.chars().enumerate() {
            if ci >= end {
                break;
            }
            if ci < start {
                continue;
            }
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
            let avail = self.row_count(&line_text(rope, top)) - sub;
            let n = left as usize;
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
            sub = self.row_count(&line_text(rope, top)) - 1;
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
            for (start, end) in self.rows(&line_text(rope, line)) {
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
        if pos.line < view_top {
            return None;
        }
        let top_rows = self.row_count(&line_text(rope, view_top));
        let mut row = self.pos(&line_text(rope, pos.line), pos.col).0;
        if pos.line == view_top {
            return row.checked_sub(view_sub.min(top_rows - 1));
        }
        row += top_rows - view_sub.min(top_rows - 1);
        for line in view_top + 1..pos.line {
            row += self.row_count(&line_text(rope, line));
        }
        Some(row)
    }

    /// Absolute screen row of `pos`, counted from the buffer's first row.
    pub fn row_of(&self, rope: &Rope, pos: Pos) -> usize {
        let line = pos.line.min(rope.len_lines().saturating_sub(1));
        let mut row = 0;
        for l in 0..line {
            row += self.row_count(&line_text(rope, l));
        }
        row + self.pos(&line_text(rope, line), pos.col).0
    }

    /// The `(line, sub-row)` holding absolute screen row `row`, clamped to the
    /// buffer's last row.
    pub fn anchor(&self, rope: &Rope, row: usize) -> (usize, usize) {
        let mut left = row;
        for line in 0..rope.len_lines() {
            let rows = self.row_count(&line_text(rope, line));
            if left < rows {
                return (line, left);
            }
            left -= rows;
        }
        let last = rope.len_lines().saturating_sub(1);
        (last, self.row_count(&line_text(rope, last)).saturating_sub(1))
    }

    /// How many screen rows the whole buffer occupies.
    pub fn total_rows(&self, rope: &Rope) -> usize {
        (0..rope.len_lines())
            .map(|l| self.row_count(&line_text(rope, l)))
            .sum()
    }

    /// The `(line, sub-row)` reached by moving `delta` screen rows from
    /// `pos`, clamped to the first and last row of the buffer.
    pub fn shift(&self, rope: &Rope, pos: Pos, delta: i64) -> (usize, usize) {
        let mut line = pos.line;
        let mut sub = self.pos(&line_text(rope, line), pos.col).0;
        let last = rope.len_lines().saturating_sub(1);
        let mut step = delta.abs();
        while step > 0 {
            let rows = self.row_count(&line_text(rope, line));
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
                sub = self.row_count(&line_text(rope, line)) - 1;
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
        let text = line_text(rope, line);
        let body = self.body(&text);
        let rows = self.rows(body);
        let (start, end) = rows[sub.min(rows.len() - 1)];
        let mut cx = 0usize;
        for (ci, c) in body.chars().enumerate().skip(start) {
            if ci >= end {
                break;
            }
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
        assert_eq!(w.rows("\tab"), vec![(0, 1), (1, 3)]);
        assert_eq!(w.text("\tab", 1, 3, false), "ab");
        assert_eq!(w.rows("\t a"), vec![(0, 1), (1, 3)]);
        assert_eq!(w.text("\t a", 1, 3, false), " a");
    }

    #[test]
    fn vis_markers_stay_one_cell_wide() {
        let w = Wrap::new(10, 4);
        assert_eq!(w.text("a b", 0, 3, true), "a\u{b7}b");
        assert_eq!(w.text("\tab", 0, 3, true), "\u{bb}\u{b7}\u{b7}\u{b7}ab");
        assert_eq!(w.text("a\tb", 0, 3, true), "a\u{bb}\u{b7}\u{b7}b");
        assert_eq!(
            w.text("\tab", 0, 3, true).chars().count(),
            w.text("\tab", 0, 3, false).chars().count()
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
        assert_eq!(w.row_count(&line_text(&r, 1)), 2);
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
            rows.iter().map(|s| (s.line, s.start, s.end)).collect::<Vec<_>>(),
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
            scrolled.iter().map(|s| (s.line, s.start, s.end)).collect::<Vec<_>>(),
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
}
