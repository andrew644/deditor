pub fn ignore_case(term: &str) -> bool {
    !term.chars().any(char::is_uppercase)
}

pub fn matches_at(hay: &[char], term: &[char], at: usize, fold: bool) -> bool {
    let Some(rest) = hay.get(at..at + term.len()) else {
        return false;
    };
    if !fold {
        return rest == term;
    }
    rest.iter()
        .zip(term)
        .all(|(h, t)| h.to_lowercase().eq(t.to_lowercase()))
}

pub fn find_wrap(
    chars: &[char],
    term: &[char],
    start: usize,
    back: bool,
    fold: bool,
) -> Option<usize> {
    let n = chars.len();
    let tl = term.len();
    if tl == 0 || n < tl {
        return None;
    }
    let hit = |i: usize| matches_at(chars, term, i, fold);
    if back {
        let hi = start.min(n - tl);
        (0..=hi)
            .rev()
            .find(|&i| hit(i))
            .or_else(|| ((hi + 1)..=(n - tl)).rev().find(|&i| hit(i)))
    } else {
        let lo = start.min(n - tl + 1);
        if lo <= n - tl
            && let Some(i) = (lo..=(n - tl)).find(|&i| hit(i))
        {
            return Some(i);
        }
        (0..lo.min(n - tl + 1)).find(|&i| hit(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn locate(hay: &str, term: &str, start: usize, back: bool) -> Option<usize> {
        let (h, t) = (chars(hay), chars(term));
        find_wrap(&h, &t, start, back, ignore_case(term))
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
    fn empty_and_overlong_terms_find_nothing() {
        assert_eq!(locate("abc", "", 0, false), None);
        assert_eq!(locate("abc", "abcd", 0, false), None);
        assert_eq!(locate("", "a", 0, false), None);
    }
}
