use crate::buffer::{Pos, line_chars, line_text};
use ropey::Rope;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    H,
    J,
    K,
    L,
    Zero,
    Caret,
    Dollar,
    W,
    BigW,
    B,
    BigB,
    E,
    BigE,
    GE,
    BigGE,
    GG,
    G,
    ToLine(usize),
    Fwd(char),
    Back(char),
    TillFwd(char),
    TillBack(char),
    Percent,
    HalfPageDown,
    HalfPageUp,
    PageDown,
    PageUp,
    /// Linewise counterpart of `J`, used by operators and linewise visual:
    /// `J`/`K` move by screen rows once lines wrap.
    LineDown,
    LineUp,
}

/// Caret movement for insert mode, where the caret may sit after the last
/// character of a line and crosses line boundaries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Caret {
    Left,
    Right,
    Up,
    Down,
}

pub fn char_is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub struct Ctx {
    pub rows: usize,
    pub wrap: crate::wrap::Wrap,
    /// Cell column vertical motions try to keep, Vim's preferred column.
    pub prefer_x: usize,
}

impl Ctx {
    #[cfg(test)]
    pub fn plain(rows: usize) -> Self {
        Self {
            rows,
            wrap: crate::wrap::Wrap::new(usize::MAX / 4, 4),
            prefer_x: 0,
        }
    }
}

fn clamp(rope: &Rope, line: usize, col: usize) -> Pos {
    let line = line.min(rope.len_lines().saturating_sub(1));
    let col = col.min(line_chars(rope, line));
    Pos { line, col }
}

pub fn apply(rope: &Rope, pos: Pos, m: Motion, count: u32, ctx: &Ctx) -> Pos {
    if rope.len_lines() == 0 {
        return pos;
    }
    let n = count.max(1) as usize;
    let last = rope.len_lines() - 1;
    match m {
        Motion::H => clamp(rope, pos.line, pos.col.saturating_sub(n)),
        Motion::J | Motion::K => {
            let delta = if m == Motion::J {
                n as i64
            } else {
                -(n as i64)
            };
            let (line, sub) = ctx.wrap.shift(rope, pos, delta);
            clamp(rope, line, ctx.wrap.col_at(rope, line, sub, ctx.prefer_x))
        }
        Motion::LineDown => clamp(rope, pos.line + n, pos.col),
        Motion::LineUp => clamp(rope, pos.line.saturating_sub(n), pos.col),
        Motion::L => {
            let len = line_chars(rope, pos.line);
            let max = len.saturating_sub(1);
            clamp(rope, pos.line, (pos.col + n).min(max))
        }
        Motion::Zero => Pos {
            line: pos.line,
            col: 0,
        },
        Motion::Caret => Pos {
            line: pos.line,
            col: first_non_blank(rope, pos.line),
        },
        Motion::Dollar => {
            let c = line_chars(rope, pos.line);
            Pos {
                line: pos.line,
                col: if c == 0 { 0 } else { c - 1 },
            }
        }
        Motion::W => (0..n).fold(pos, |p, _| w_one(rope, p, false)),
        Motion::BigW => (0..n).fold(pos, |p, _| w_one(rope, p, true)),
        Motion::B => (0..n).fold(pos, |p, _| b_one(rope, p, false)),
        Motion::BigB => (0..n).fold(pos, |p, _| b_one(rope, p, true)),
        Motion::E => (0..n).fold(pos, |p, _| e_one(rope, p, false)),
        Motion::BigE => (0..n).fold(pos, |p, _| e_one(rope, p, true)),
        Motion::GE => (0..n).fold(pos, |p, _| ge_one(rope, p, false)),
        Motion::BigGE => (0..n).fold(pos, |p, _| ge_one(rope, p, true)),
        Motion::GG => clamp(rope, 0, pos.col),
        Motion::G => clamp(rope, last, pos.col),
        Motion::ToLine(l) => clamp(rope, l, pos.col),
        Motion::Fwd(c) => find_fwd(rope, pos, c, n, false).unwrap_or(pos),
        Motion::TillFwd(c) => find_fwd(rope, pos, c, n, true).unwrap_or(pos),
        Motion::Back(c) => find_back(rope, pos, c, n).unwrap_or(pos),
        Motion::TillBack(c) => find_back(rope, pos, c, n)
            .map(|p| clamp(rope, p.line, p.col + 1))
            .unwrap_or(pos),
        Motion::Percent => percent(rope, pos),
        Motion::HalfPageDown => clamp(rope, pos.line + n * (ctx.rows / 2).max(1), pos.col),
        Motion::HalfPageUp => clamp(
            rope,
            pos.line.saturating_sub(n * (ctx.rows / 2).max(1)),
            pos.col,
        ),
        Motion::PageDown => clamp(rope, pos.line + n * ctx.rows.max(1), pos.col),
        Motion::PageUp => clamp(rope, pos.line.saturating_sub(n * ctx.rows.max(1)), pos.col),
    }
}

fn first_non_blank(rope: &Rope, line: usize) -> usize {
    line_text(rope, line)
        .chars()
        .enumerate()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn next_word_line(rope: &Rope, line: usize, last: usize) -> Pos {
    for l in line + 1..=last {
        let chars: Vec<char> = line_text(rope, l).chars().collect();
        if let Some(col) = chars.iter().position(|c| !c.is_whitespace()) {
            return Pos { line: l, col };
        }
    }
    Pos { line: last, col: 0 }
}

fn w_one(rope: &Rope, p: Pos, big: bool) -> Pos {
    let last = rope.len_lines().saturating_sub(1);
    let line = p.line;
    let mut col = p.col;
    let len = line_chars(rope, line);
    if col >= len {
        if line >= last {
            return Pos { line, col };
        }
        return next_word_line(rope, line, last);
    }
    let text = line_text(rope, line);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if chars[col].is_whitespace() {
        while col < len && chars[col].is_whitespace() {
            col += 1;
        }
        if col >= len {
            if line >= last {
                return Pos { line, col };
            }
            return next_word_line(rope, line, last);
        }
        return Pos { line, col };
    }
    let word_start = char_is_word(chars[col]);
    while col < len && continues(chars[col], word_start, big) {
        col += 1;
    }
    if col >= len {
        if line >= last {
            return Pos { line, col };
        }
        return next_word_line(rope, line, last);
    }
    while col < len && chars[col].is_whitespace() {
        col += 1;
    }
    if col < len {
        return Pos { line, col };
    }
    next_word_line(rope, line, last)
}

fn b_one(rope: &Rope, p: Pos, big: bool) -> Pos {
    let mut line = p.line;
    let mut col = p.col;
    if col == 0 {
        if line == 0 {
            return Pos { line: 0, col: 0 };
        }
        line -= 1;
        col = line_chars(rope, line);
    }
    loop {
        let chars: Vec<char> = line_text(rope, line).chars().collect();
        while col > 0 && chars[col - 1].is_whitespace() {
            col -= 1;
        }
        if col > 0 {
            let word_start = char_is_word(chars[col - 1]);
            while col > 0 && continues(chars[col - 1], word_start, big) {
                col -= 1;
            }
            return Pos { line, col };
        }
        if line == 0 {
            return Pos { line: 0, col: 0 };
        }
        line -= 1;
        col = line_chars(rope, line);
        while col == 0 && line > 0 {
            line -= 1;
            col = line_chars(rope, line);
        }
    }
}

fn e_one(rope: &Rope, p: Pos, big: bool) -> Pos {
    let last = rope.len_lines().saturating_sub(1);
    let mut line = p.line;
    let mut col = p.col;
    let len = line_chars(rope, line);
    if col >= len.saturating_sub(1) && len > 0 {
        if line >= last {
            return Pos {
                line,
                col: len.saturating_sub(1),
            };
        }
        line += 1;
        col = 0;
    } else if len == 0 {
        if line >= last {
            return Pos { line, col: 0 };
        }
        line += 1;
        col = 0;
    } else {
        col += 1;
    }
    loop {
        let chars: Vec<char> = line_text(rope, line).chars().collect();
        if chars.is_empty() {
            if line >= last {
                return Pos { line, col: 0 };
            }
            line += 1;
            col = 0;
            continue;
        }
        if col >= chars.len() {
            if line >= last {
                return Pos {
                    line,
                    col: chars.len() - 1,
                };
            }
            line += 1;
            col = 0;
            continue;
        }
        while col < chars.len() && chars[col].is_whitespace() {
            col += 1;
        }
        if col >= chars.len() {
            if line >= last {
                return Pos {
                    line,
                    col: chars.len().saturating_sub(1),
                };
            }
            line += 1;
            col = 0;
            continue;
        }
        let word_start = char_is_word(chars[col]);
        let mut end = col;
        while end < chars.len() && continues(chars[end], word_start, big) {
            end += 1;
        }
        return Pos { line, col: end - 1 };
    }
}

fn ge_one(rope: &Rope, p: Pos, big: bool) -> Pos {
    let mut p2 = p;
    loop {
        let start = b_one(rope, p2, big);
        if start == p2 && p2.line == 0 && p2.col == 0 {
            return p;
        }
        let chars: Vec<char> = line_text(rope, start.line).chars().collect();
        let mut col = start.col;
        while col + 1 < chars.len() && same_class(chars[col], chars[col + 1], big) {
            col += 1;
        }
        let end = Pos {
            line: start.line,
            col,
        };
        if end != p {
            return end;
        }
        if start == p2 {
            return p;
        }
        p2 = start;
        if p2.line == 0 && p2.col == 0 {
            return p;
        }
    }
}

/// Does `c` continue a run that began on a word character (`word_start`)? A
/// WORD (`big`) run only ends at whitespace.
fn continues(c: char, word_start: bool, big: bool) -> bool {
    if big {
        !c.is_whitespace()
    } else if word_start {
        char_is_word(c)
    } else {
        !c.is_whitespace() && !char_is_word(c)
    }
}

fn same_class(a: char, b: char, big: bool) -> bool {
    if big {
        !a.is_whitespace() && !b.is_whitespace()
    } else {
        continues(b, char_is_word(a), false)
    }
}

fn find_fwd(rope: &Rope, p: Pos, c: char, n: usize, till: bool) -> Option<Pos> {
    let text: Vec<char> = line_text(rope, p.line).chars().collect();
    let mut seen = 0;
    for (i, &ch) in text.iter().enumerate().skip(p.col + 1) {
        if ch == c {
            seen += 1;
            if seen == n {
                let col = if till { i - 1 } else { i };
                return Some(Pos { line: p.line, col });
            }
        }
    }
    None
}

fn find_back(rope: &Rope, p: Pos, c: char, n: usize) -> Option<Pos> {
    let text: Vec<char> = line_text(rope, p.line).chars().collect();
    let mut seen = 0;
    let limit = p.col.saturating_sub(1);
    for i in (0..=limit).rev() {
        if i == p.col {
            continue;
        }
        if text[i] == c {
            seen += 1;
            if seen == n {
                return Some(Pos {
                    line: p.line,
                    col: i,
                });
            }
        }
    }
    None
}

fn is_open(c: char) -> bool {
    matches!(c, '(' | '[' | '{')
}

fn is_close(c: char) -> bool {
    matches!(c, ')' | ']' | '}')
}

fn pairs(c: char) -> Option<(char, char)> {
    match c {
        '(' | ')' => Some(('(', ')')),
        '[' | ']' => Some(('[', ']')),
        '{' | '}' => Some(('{', '}')),
        _ => None,
    }
}

fn percent(rope: &Rope, p: Pos) -> Pos {
    let text: Vec<char> = line_text(rope, p.line).chars().collect();
    let i = if p.col < text.len() && (is_open(text[p.col]) || is_close(text[p.col])) {
        p.col
    } else {
        return p;
    };
    let (open, close) = match pairs(text[i]) {
        Some(pr) => pr,
        None => return p,
    };
    if is_open(text[i]) {
        let mut depth = 0isize;
        for (j, &ch) in text.iter().enumerate().skip(i) {
            if ch == open {
                depth += 1;
            } else if ch == close {
                depth -= 1;
                if depth == 0 {
                    return Pos {
                        line: p.line,
                        col: j,
                    };
                }
            }
        }
    } else {
        let mut depth = 0isize;
        for j in (0..=i).rev() {
            if text[j] == close {
                depth += 1;
            } else if text[j] == open {
                depth -= 1;
                if depth == 0 {
                    return Pos {
                        line: p.line,
                        col: j,
                    };
                }
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    fn at(line: usize, col: usize) -> Pos {
        Pos { line, col }
    }

    fn m(rope: &Rope, p: Pos, motion: Motion) -> Pos {
        let mut ctx = Ctx::plain(20);
        ctx.prefer_x = p.col;
        apply(rope, p, motion, 1, &ctx)
    }

    #[test]
    fn w_moves_to_next_word() {
        let r = rope("hello world");
        assert_eq!(m(&r, at(0, 0), Motion::W), at(0, 6));
    }

    #[test]
    fn w_three_on_words() {
        let r = rope("a b c d");
        assert_eq!(apply(&r, at(0, 0), Motion::W, 3, &Ctx::plain(20)), at(0, 6));
    }

    #[test]
    fn w_punct_then_space() {
        let r = rope("foo.bar baz");
        assert_eq!(m(&r, at(0, 0), Motion::W), at(0, 3));
        assert_eq!(m(&r, at(0, 3), Motion::W), at(0, 4));
        assert_eq!(m(&r, at(0, 7), Motion::BigW), at(0, 8));
    }

    #[test]
    fn w_at_line_end_goes_to_next_line() {
        let r = rope("hello\nworld");
        assert_eq!(m(&r, at(0, 5), Motion::W), at(1, 0));
    }

    #[test]
    fn b_back_to_word_start() {
        let r = rope("hello world");
        assert_eq!(m(&r, at(0, 6), Motion::B), at(0, 0));
        let r = rope("hello world\nagain");
        assert_eq!(m(&r, at(1, 0), Motion::B), at(0, 6));
    }

    #[test]
    fn b_reverse_big_word() {
        let r = rope("foo.bar baz");
        assert_eq!(m(&r, at(0, 10), Motion::BigB), at(0, 8));
    }

    #[test]
    fn e_lands_on_word_end() {
        let r = rope("hello world");
        assert_eq!(m(&r, at(0, 0), Motion::E), at(0, 4));
        assert_eq!(m(&r, at(0, 4), Motion::E), at(0, 10));
    }

    #[test]
    fn ge_prev_word_end() {
        let r = rope("hello world");
        assert_eq!(m(&r, at(0, 10), Motion::GE), at(0, 4));
    }

    #[test]
    fn gg_and_g_clamp() {
        let r = rope("one\ntwo\nthree");
        assert_eq!(m(&r, at(2, 3), Motion::GG), at(0, 3));
        assert_eq!(m(&r, at(0, 1), Motion::G), at(2, 1));
        assert_eq!(
            apply(&r, at(0, 0), Motion::G, 99, &Ctx::plain(20)),
            at(2, 0)
        );
    }

    #[test]
    fn basic_moves_clamp() {
        let r = rope("ab\ncdef");
        assert_eq!(m(&r, at(0, 1), Motion::L), at(0, 1));
        assert_eq!(m(&r, at(0, 0), Motion::H), at(0, 0));
        assert_eq!(m(&r, at(0, 1), Motion::J), at(1, 1));
        assert_eq!(m(&r, at(0, 0), Motion::K), at(0, 0));
        assert_eq!(
            apply(&r, at(0, 0), Motion::J, 50, &Ctx::plain(20)),
            at(1, 0)
        );
        assert_eq!(m(&r, at(1, 0), Motion::Zero), at(1, 0));
        assert_eq!(m(&r, at(0, 1), Motion::Dollar), at(0, 1));
    }

    #[test]
    fn caret_first_non_blank() {
        let r = rope("    indented");
        assert_eq!(m(&r, at(0, 9), Motion::Caret), at(0, 4));
    }

    #[test]
    fn find_forward_and_till() {
        let r = rope("xaxbx");
        assert_eq!(m(&r, at(0, 0), Motion::Fwd('x')), at(0, 2));
        assert_eq!(
            apply(&r, at(0, 0), Motion::Fwd('x'), 2, &Ctx::plain(20)),
            at(0, 4)
        );
        assert_eq!(m(&r, at(0, 0), Motion::TillFwd('x')), at(0, 1));
        assert_eq!(m(&r, at(0, 0), Motion::Fwd('z')), at(0, 0));
        assert_eq!(m(&r, at(0, 0), Motion::TillFwd('z')), at(0, 0));
    }

    #[test]
    fn find_backward_and_till_back() {
        let r = rope("xaxbx");
        assert_eq!(m(&r, at(0, 4), Motion::Back('x')), at(0, 2));
        assert_eq!(m(&r, at(0, 4), Motion::TillBack('x')), at(0, 3));
        assert_eq!(m(&r, at(0, 0), Motion::Back('x')), at(0, 0));
    }

    #[test]
    fn percent_matches_paren() {
        let r = rope("(a(b)c)");
        assert_eq!(m(&r, at(0, 0), Motion::Percent), at(0, 6));
        assert_eq!(m(&r, at(0, 6), Motion::Percent), at(0, 0));
        assert_eq!(m(&r, at(0, 2), Motion::Percent), at(0, 4));
        assert_eq!(m(&r, at(0, 1), Motion::Percent), at(0, 1));
    }

    #[test]
    fn page_and_half_page_clamp() {
        let text = "l\n".repeat(30);
        let text = text.trim_end();
        let r = rope(text);
        assert_eq!(
            apply(&r, at(0, 0), Motion::PageDown, 1, &Ctx::plain(10)),
            at(10, 0)
        );
        assert_eq!(
            apply(&r, at(0, 0), Motion::HalfPageDown, 1, &Ctx::plain(10)),
            at(5, 0)
        );
        assert_eq!(
            apply(&r, at(0, 0), Motion::PageUp, 1, &Ctx::plain(10)),
            at(0, 0)
        );
        assert_eq!(
            apply(&r, at(29, 0), Motion::PageDown, 1, &Ctx::plain(10)),
            at(29, 0)
        );
    }

    #[test]
    fn empty_rope_no_panic() {
        let r = rope("");
        assert_eq!(m(&r, at(0, 0), Motion::Dollar), at(0, 0));
        assert_eq!(m(&r, at(0, 0), Motion::W), at(0, 0));
        assert_eq!(m(&r, at(0, 0), Motion::B), at(0, 0));
        assert_eq!(m(&r, at(0, 0), Motion::E), at(0, 0));
        assert_eq!(m(&r, at(0, 0), Motion::G), at(0, 0));
        assert_eq!(m(&r, at(0, 0), Motion::GE), at(0, 0));
    }

    #[test]
    fn j_and_k_step_screen_rows_and_keep_a_preferred_column() {
        let r = rope("aaaaaaaaa\nb");
        let mut ctx = Ctx::plain(20);
        ctx.wrap = crate::wrap::Wrap::new(3, 4);
        ctx.prefer_x = 2;
        assert_eq!(apply(&r, at(0, 0), Motion::J, 1, &ctx), at(0, 5));
        assert_eq!(apply(&r, at(0, 5), Motion::K, 1, &ctx), at(0, 2));
        ctx.prefer_x = 0;
        assert_eq!(apply(&r, at(0, 5), Motion::K, 1, &ctx), at(0, 0));
        assert_eq!(apply(&r, at(0, 0), Motion::J, 2, &ctx), at(0, 6));
        assert_eq!(apply(&r, at(0, 8), Motion::J, 1, &ctx), at(1, 0));
        assert_eq!(apply(&r, at(0, 8), Motion::LineDown, 1, &ctx), at(1, 1));
        assert_eq!(apply(&r, at(1, 0), Motion::LineUp, 1, &ctx), at(0, 0));
    }

    #[test]
    fn w_skips_indent_after_newline() {
        let r = rope("ab\n  cd");
        assert_eq!(m(&r, at(0, 0), Motion::W), at(1, 2));
    }

    #[test]
    fn w_skips_blank_lines() {
        let r = rope("ab\n\n  cd");
        assert_eq!(m(&r, at(0, 0), Motion::W), at(2, 2));
    }

    #[test]
    fn b_skips_blank_lines() {
        let r = rope("hello\n\n\nworld");
        assert_eq!(m(&r, at(3, 0), Motion::B), at(0, 0));
    }
}
