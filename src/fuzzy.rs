use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    None,
    Files,
    Buffers,
    Settings,
}

pub fn fuzzy_filter(items: &[String], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..items.len()).collect();
    }
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::new(query, CaseMatching::Ignore, Normalization::Smart, nucleo_matcher::pattern::AtomKind::Fuzzy);
    let scored = pattern.match_list(items.iter(), &mut matcher);
    scored.into_iter().map(|(item, _)| {
        items
            .iter()
            .position(|i| i == item)
            .expect("matched item came from items")
    }).collect()
}

pub fn split_path_query(q: &str) -> (&str, &str) {
    match q.rfind('/') {
        Some(i) => (&q[..=i], &q[i + 1..]),
        None => ("", q),
    }
}

pub fn list_dir(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = entry.path().is_dir();
        out.push(if is_dir { format!("{name}/") } else { name });
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_query_keeps_order() {
        let it = items(&["b.rs", "a.rs"]);
        assert_eq!(fuzzy_filter(&it, ""), vec![0, 1]);
        assert_eq!(fuzzy_filter(&it, "   "), vec![0, 1]);
    }

    #[test]
    fn ranks_path_separators() {
        let it = items(&["docs/srcnote.txt", "src/main.rs", "README.md"]);
        let ord = fuzzy_filter(&it, "srcmain");
        assert_eq!(ord[0], 1);
        assert!(!ord.contains(&2));
    }

    #[test]
    fn no_match_filters_all() {
        let it = items(&["a.rs", "b.rs"]);
        assert!(fuzzy_filter(&it, "zzzz").is_empty());
    }

    #[test]
    fn path_queries_split_at_the_last_slash() {
        assert_eq!(split_path_query("main"), ("", "main"));
        assert_eq!(split_path_query("~/src/ma"), ("~/src/", "ma"));
        assert_eq!(split_path_query("~/"), ("~/", ""));
    }

    #[test]
    fn list_dir_marks_directories_and_skips_hidden() {
        let dir = std::env::temp_dir().join(format!("deditor_ls_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("b.txt"), "x").unwrap();
        std::fs::write(dir.join(".hidden"), "x").unwrap();
        let got = list_dir(&dir);
        assert_eq!(got, vec!["b.txt".to_string(), "sub/".to_string()]);
        assert!(list_dir(&dir.join("missing")).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
