use crate::buffer::Pos;
use crate::motions::{class_of, in_run};
use ropey::Rope;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Obj {
    Word,
    BigWord,
    Quote(char),
    Bracket(char),
}

fn abs_of(rope: &Rope, p: Pos) -> usize {
    let line = p.line.min(rope.len_lines().saturating_sub(1));
    let col = p.col.min(rope.line(line).chars().count().saturating_sub(1));
    rope.line_to_char(line) + col
}

fn pos_of(rope: &Rope, off: usize) -> Pos {
    let line = rope.char_to_line(off.min(rope.len_chars()));
    Pos {
        line,
        col: off - rope.line_to_char(line),
    }
}

fn char_at(rope: &Rope, off: usize) -> Option<char> {
    if off < rope.len_chars() {
        Some(rope.char(off))
    } else {
        None
    }
}

fn word_run(rope: &Rope, cursor: usize) -> Option<(usize, usize)> {
    let run = class_of(char_at(rope, cursor)?);
    let mut start = cursor;
    while start > 0 {
        if !in_run(char_at(rope, start - 1)?, run, false) {
            break;
        }
        start -= 1;
    }
    let mut end = cursor + 1;
    while let Some(c2) = char_at(rope, end) {
        if !in_run(c2, run, false) {
            break;
        }
        end += 1;
    }
    Some((start, end))
}

fn big_run(rope: &Rope, cursor: usize) -> Option<(usize, usize)> {
    let c = char_at(rope, cursor)?;
    if c.is_whitespace() {
        return word_run(rope, cursor);
    }
    let mut start = cursor;
    while start > 0 {
        let p = char_at(rope, start - 1)?;
        if p == '\n' || p.is_whitespace() {
            break;
        }
        start -= 1;
    }
    let mut end = cursor + 1;
    while let Some(c2) = char_at(rope, end) {
        if c2 == '\n' || c2.is_whitespace() {
            break;
        }
        end += 1;
    }
    Some((start, end))
}

fn extend_around(rope: &Rope, start: usize, end: usize) -> (usize, usize) {
    let mut ws_end = end;
    while let Some(c) = char_at(rope, ws_end) {
        if !c.is_whitespace() {
            break;
        }
        ws_end += 1;
    }
    if ws_end < rope.len_chars() {
        return (start, ws_end);
    }
    let mut lead = start;
    while lead > 0 {
        let Some(p) = char_at(rope, lead - 1) else {
            break;
        };
        if !p.is_whitespace() {
            break;
        }
        lead -= 1;
    }
    if lead < start {
        (lead, end)
    } else {
        (start, ws_end)
    }
}

fn quote_obj(rope: &Rope, line: usize, col: usize, q: char, inner: bool) -> Option<(usize, usize)> {
    let text: Vec<char> = rope.line(line).chars().take_while(|c| *c != '\n').collect();
    if text.is_empty() {
        return None;
    }
    let base = rope.line_to_char(line);
    if col < text.len() && text[col] == q {
        let before = (0..col).filter(|&i| text[i] == q).count();
        if before % 2 == 1 {
            let b = (0..col).rev().find(|&i| text[i] == q)?;
            return Some(if inner {
                (base + b + 1, base + col)
            } else {
                (base + b, base + col + 1)
            });
        }
        let a = (col + 1..text.len()).find(|&j| text[j] == q)?;
        return Some(if inner {
            (base + col + 1, base + a)
        } else {
            (base + col, base + a + 1)
        });
    }
    let b = (0..col).rev().find(|&i| text[i] == q)?;
    let a = (col + 1..text.len()).find(|&j| text[j] == q)?;
    if inner {
        Some((base + b + 1, base + a))
    } else {
        Some((base + b, base + a + 1))
    }
}

fn match_open(text: &[char], i: usize) -> Option<usize> {
    let open = text[i];
    let close = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        _ => return None,
    };
    let mut depth = 0isize;
    for (j, &ch) in text.iter().enumerate().skip(i) {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(j);
            }
        }
    }
    None
}

fn match_close(text: &[char], i: usize) -> Option<usize> {
    let close = text[i];
    let open = match close {
        ')' => '(',
        ']' => '[',
        '}' => '{',
        _ => return None,
    };
    let mut depth = 0isize;
    for j in (0..=i).rev() {
        if text[j] == close {
            depth += 1;
        } else if text[j] == open {
            depth -= 1;
            if depth == 0 {
                return Some(j);
            }
        }
    }
    None
}

fn bracket_for(
    rope: &Rope,
    line: usize,
    col: usize,
    want: char,
    inner: bool,
) -> Option<(usize, usize)> {
    let text: Vec<char> = rope.line(line).chars().take_while(|c| *c != '\n').collect();
    if text.is_empty() {
        return None;
    }
    let (o, c) = match want {
        '(' => ('(', ')'),
        '[' => ('[', ']'),
        '{' => ('{', '}'),
        _ => return None,
    };
    let mut found: Option<(usize, usize)> = None;
    if col < text.len() && text[col] == o {
        found = match_open(&text, col).map(|j| (col, j));
    }
    if found.is_none() && col < text.len() && text[col] == c {
        found = match_close(&text, col).map(|j| (j, col));
    }
    if found.is_none() {
        let limit = col.min(text.len().saturating_sub(1));
        for i in (0..=limit).rev() {
            if text[i] == o
                && let Some(j) = match_open(&text, i)
                && j >= col
            {
                found = Some((i, j));
                break;
            }
        }
    }
    if found.is_none() {
        for i in col..text.len() {
            if text[i] == o {
                found = match_open(&text, i).map(|j| (i, j));
                if found.is_some() {
                    break;
                }
            }
        }
    }
    if found.is_none() {
        let limit = col.min(text.len().saturating_sub(1));
        for i in (0..=limit).rev() {
            if text[i] == c {
                found = match_close(&text, i).map(|j| (j, i));
                if found.is_some() {
                    break;
                }
            }
        }
    }
    let (oi, ci) = found?;
    let base = rope.line_to_char(line);
    if inner {
        Some((base + oi + 1, base + ci))
    } else {
        Some((base + oi, base + ci + 1))
    }
}

pub fn object(rope: &Rope, pos: Pos, inner: bool, obj: Obj) -> Option<(Pos, Pos)> {
    let line = pos.line.min(rope.len_lines().saturating_sub(1));
    match obj {
        Obj::Quote(q) => {
            let (s, e) = quote_obj(
                rope,
                line,
                pos.col.min(crate::buffer::line_chars(rope, line)),
                q,
                inner,
            )?;
            Some((pos_of(rope, s), pos_of(rope, e)))
        }
        Obj::Bracket(b) => {
            let (s, e) = bracket_for(
                rope,
                line,
                pos.col.min(crate::buffer::line_chars(rope, line)),
                b,
                inner,
            )?;
            Some((pos_of(rope, s), pos_of(rope, e)))
        }
        Obj::Word | Obj::BigWord => {
            let cursor = abs_of(rope, pos);
            if cursor >= rope.len_chars() {
                return None;
            }
            let (mut start, mut end) = match obj {
                Obj::BigWord => big_run(rope, cursor)?,
                _ => word_run(rope, cursor)?,
            };
            if !inner {
                if rope.char(start).is_whitespace() {
                    let c = char_at(rope, end);
                    if c.is_some_and(|c| c.is_whitespace()) {
                        let (s, e) = extend_around(rope, start, end);
                        start = s;
                        end = e;
                    }
                } else {
                    let (s, e) = extend_around(rope, start, end);
                    start = s;
                    end = e;
                }
            }
            start = start.min(rope.len_chars());
            end = end.max(start).min(rope.len_chars());
            Some((pos_of(rope, start), pos_of(rope, end)))
        }
    }
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

    #[test]
    fn inner_word() {
        let r = rope("foo bar baz");
        assert_eq!(
            object(&r, at(0, 4), true, Obj::Word),
            Some((at(0, 4), at(0, 7)))
        );
    }

    #[test]
    fn around_word_consumes_trailing_space() {
        let r = rope("foo bar baz");
        assert_eq!(
            object(&r, at(0, 4), false, Obj::Word),
            Some((at(0, 4), at(0, 8)))
        );
    }

    #[test]
    fn word_on_whitespace_falls_back_to_space_run() {
        let r = rope("foo bar");
        assert_eq!(
            object(&r, at(0, 3), true, Obj::Word),
            Some((at(0, 3), at(0, 4)))
        );
    }

    #[test]
    fn big_word_spans_punctuation() {
        let r = rope("foo.bar baz");
        assert_eq!(
            object(&r, at(0, 4), true, Obj::BigWord),
            Some((at(0, 0), at(0, 7)))
        );
        assert_eq!(
            object(&r, at(0, 4), false, Obj::BigWord),
            Some((at(0, 0), at(0, 8)))
        );
    }

    #[test]
    fn around_word_at_end_takes_leading_space() {
        let r = rope("foo bar ");
        assert_eq!(
            object(&r, at(0, 4), false, Obj::Word),
            Some((at(0, 3), at(0, 7)))
        );
    }

    #[test]
    fn inner_quote_selects_content() {
        let r = rope("\"hi\"");
        assert_eq!(
            object(&r, at(0, 1), true, Obj::Quote('"')),
            Some((at(0, 1), at(0, 3)))
        );
        assert_eq!(
            object(&r, at(0, 1), false, Obj::Quote('"')),
            Some((at(0, 0), at(0, 4)))
        );
    }

    #[test]
    fn quote_missing_returns_none() {
        let r = rope("no quotes here");
        assert_eq!(object(&r, at(0, 2), true, Obj::Quote('"')), None);
    }

    #[test]
    fn single_backtick_quote() {
        let r = rope("'hi' and 'yo'");
        assert_eq!(
            object(&r, at(0, 11), true, Obj::Quote('\'')),
            Some((at(0, 10), at(0, 12)))
        );
    }

    #[test]
    fn inner_paren_inside_call() {
        let r = rope("foo(bar)baz");
        assert_eq!(
            object(&r, at(0, 4), true, Obj::Bracket('(')),
            Some((at(0, 4), at(0, 7)))
        );
        assert_eq!(
            object(&r, at(0, 4), false, Obj::Bracket('(')),
            Some((at(0, 3), at(0, 8)))
        );
    }

    #[test]
    fn inner_paren_on_opener_and_nesting() {
        let r = rope("(a(b)c)");
        assert_eq!(
            object(&r, at(0, 0), true, Obj::Bracket('(')),
            Some((at(0, 1), at(0, 6)))
        );
        assert_eq!(
            object(&r, at(0, 2), true, Obj::Bracket('(')),
            Some((at(0, 3), at(0, 4)))
        );
        assert_eq!(
            object(&r, at(0, 6), true, Obj::Bracket('(')),
            Some((at(0, 1), at(0, 6)))
        );
    }

    #[test]
    fn brace_and_bracket_objects() {
        let r = rope("let x = {a: 1};");
        assert_eq!(
            object(&r, at(0, 10), true, Obj::Bracket('{')),
            Some((at(0, 9), at(0, 13)))
        );
        let r2 = rope("arr[7] = 1");
        assert_eq!(
            object(&r2, at(0, 4), false, Obj::Bracket('[')),
            Some((at(0, 3), at(0, 6)))
        );
    }

    #[test]
    fn bracket_missing_returns_none() {
        let r = rope("plain line");
        assert_eq!(object(&r, at(0, 2), true, Obj::Bracket('(')), None);
    }

    #[test]
    fn multi_line_word_object_spans_newline() {
        let r = rope("one\ntwo\n");
        assert_eq!(
            object(&r, at(0, 2), false, Obj::Word),
            Some((at(0, 0), at(1, 0)))
        );
    }

    #[test]
    fn empty_rope_no_panic() {
        let r = rope("");
        assert_eq!(object(&r, at(0, 0), true, Obj::Word), None);
        assert_eq!(object(&r, at(0, 0), true, Obj::Quote('"')), None);
        assert_eq!(object(&r, at(0, 0), true, Obj::Bracket('(')), None);
    }
}
