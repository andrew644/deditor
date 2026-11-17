use ropey::Rope;
use std::collections::VecDeque;

pub fn ignore_case(term: &str) -> bool {
    !term.chars().any(char::is_uppercase)
}

fn eq_fold(h: char, t: char, fold: bool) -> bool {
    h == t || (fold && h.to_lowercase().eq(t.to_lowercase()))
}

pub fn matches_at(hay: &[char], term: &[char], at: usize, fold: bool) -> bool {
    let Some(rest) = hay.get(at..at + term.len()) else {
        return false;
    };
    rest.iter().zip(term).all(|(h, t)| eq_fold(*h, *t, fold))
}

fn window_eq(win: &VecDeque<char>, term: &[char], fold: bool) -> bool {
    win.iter().zip(term).all(|(h, t)| eq_fold(*h, *t, fold))
}

fn scan_fwd(rope: &Rope, term: &[char], lo: usize, hi: usize, fold: bool) -> Option<usize> {
    if lo > hi {
        return None;
    }
    let tl = term.len();
    let mut win: VecDeque<char> = VecDeque::with_capacity(tl);
    for (k, c) in rope.chars_at(lo).take(hi + tl - lo).enumerate() {
        if win.len() == tl {
            win.pop_front();
        }
        win.push_back(c);
        if win.len() == tl && window_eq(&win, term, fold) {
            return Some(lo + k + 1 - tl);
        }
    }
    None
}

fn scan_bwd(rope: &Rope, term: &[char], lo: usize, hi: usize, fold: bool) -> Option<usize> {
    if lo > hi {
        return None;
    }
    let tl = term.len();
    let mut win: VecDeque<char> = VecDeque::with_capacity(tl);
    let mut found = None;
    for (k, c) in rope.chars_at(lo).take(hi + tl - lo).enumerate() {
        if win.len() == tl {
            win.pop_front();
        }
        win.push_back(c);
        if win.len() == tl && window_eq(&win, term, fold) {
            found = Some(lo + k + 1 - tl);
        }
    }
    found
}

pub fn find_wrap(
    rope: &Rope,
    term: &[char],
    start: usize,
    back: bool,
    fold: bool,
) -> Option<usize> {
    let n = rope.len_chars();
    let tl = term.len();
    if tl == 0 || n < tl {
        return None;
    }
    if back {
        let hi = start.min(n - tl);
        scan_bwd(rope, term, 0, hi, fold).or_else(|| scan_bwd(rope, term, hi + 1, n - tl, fold))
    } else {
        let lo = start.min(n - tl + 1);
        if lo <= n - tl
            && let Some(i) = scan_fwd(rope, term, lo, n - tl, fold)
        {
            return Some(i);
        }
        if lo == 0 {
            return None;
        }
        scan_fwd(rope, term, 0, (lo - 1).min(n - tl), fold)
    }
}

pub fn match_report(rope: &Rope, term: &[char], at: usize, fold: bool) -> (usize, usize) {
    let n = rope.len_chars();
    let tl = term.len();
    if term.is_empty() || n < tl {
        return (0, 0);
    }
    let mut win: VecDeque<char> = VecDeque::with_capacity(tl);
    let mut total = 0;
    let mut current = 0;
    for (k, c) in rope.chars().enumerate() {
        if win.len() == tl {
            win.pop_front();
        }
        win.push_back(c);
        if win.len() == tl && window_eq(&win, term, fold) {
            total += 1;
            if k + 1 - tl <= at {
                current += 1;
            }
        }
    }
    (current, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    fn locate(hay: &str, term: &str, start: usize, back: bool) -> Option<usize> {
        let t: Vec<char> = term.chars().collect();
        find_wrap(&rope(hay), &t, start, back, ignore_case(term))
    }

    fn report(hay: &str, term: &str, at: usize, fold: bool) -> (usize, usize) {
        let t: Vec<char> = term.chars().collect();
        match_report(&rope(hay), &t, at, fold)
    }

    #[test]
    fn terms_without_uppercase_fold_case() {
        assert!(ignore_case("alpha"));
        assert!(ignore_case("404"));
        assert!(ignore_case("ünïcode"));
        assert!(!ignore_case("Alpha"));
        assert!(!ignore_case("Ü"));
    }

    #[test]
    fn lowercase_term_matches_any_case() {
        assert_eq!(locate("Alpha ALPHA alpha", "alpha", 0, false), Some(0));
        assert_eq!(locate("Alpha ALPHA alpha", "alpha", 1, false), Some(6));
        assert_eq!(locate("éclair ÉCLAIR", "clair", 1, false), Some(1));
        assert_eq!(locate("éclair ÉCLAIR", "éclair", 6, false), Some(7));
    }

    #[test]
    fn uppercase_term_matches_only_itself() {
        assert_eq!(locate("alpha ALPHA alpha", "Alpha", 0, false), None);
        assert_eq!(locate("alpha ALPHA Alpha", "Alpha", 0, false), Some(12));
        assert_eq!(locate("one two", "TWo", 0, false), None);
        assert_eq!(locate("éclair", "Éclair", 0, false), None);
    }

    #[test]
    fn folding_is_used_on_the_wrapped_half_too() {
        assert_eq!(locate("zz ALPHA zz", "alpha", 9, false), Some(3));
        assert_eq!(locate("zz ALPHA zz", "alpha", 2, true), Some(3));
        assert_eq!(locate("zz ALPHA zz", "ALPHA", 9, false), Some(3));
    }

    #[test]
    fn backward_search_finds_the_previous_hit() {
        assert_eq!(locate("alpha x alpha", "alpha", 11, true), Some(8));
        assert_eq!(locate("xxxxx x ALPHA", "alpha", 1, true), Some(8));
        assert_eq!(locate("xxxxx x ALPHA", "ALPHA", 1, true), Some(8));
        assert_eq!(locate("xxxxx x alpha", "ALPHA", 1, true), None);
    }

    #[test]
    fn search_wraps_around_the_end() {
        assert_eq!(locate("alpha zz", "alpha", 6, false), Some(0));
        assert_eq!(locate("zz alpha", "alpha", 1, true), Some(3));
    }

    #[test]
    fn report_numbers_the_current_match_within_the_total() {
        assert_eq!(report("aaaa", "aa", 0, false), (1, 3));
        assert_eq!(report("aaaa", "aa", 2, false), (3, 3));

        assert_eq!(report("Alpha ALPHA alpha", "alpha", 6, true), (2, 3));
        assert_eq!(report("Alpha ALPHA alpha", "alpha", 12, true), (3, 3));
        assert_eq!(report("Alpha ALPHA alpha", "ALPHA", 6, false), (1, 1));
        assert_eq!(report("Alpha ALPHA alpha", "ALPHA", 6, true), (2, 3));

        assert_eq!(report("abc", "zzz", 0, true), (0, 0));
        assert_eq!(report("abc", "", 0, true), (0, 0));
        assert_eq!(report("", "aa", 0, true), (0, 0));
    }

    #[test]
    fn terms_span_line_breaks() {
        assert_eq!(locate("ab\ncd", "b\nc", 0, false), Some(1));
        assert_eq!(locate("ab\ncd", "b\nc", 2, true), Some(1));
    }

    #[test]
    fn empty_and_overlong_terms_find_nothing() {
        assert_eq!(locate("abc", "", 0, false), None);
        assert_eq!(locate("abc", "abcd", 0, false), None);
        assert_eq!(locate("", "a", 0, false), None);
    }
}
