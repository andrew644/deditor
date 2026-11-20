use nucleo_matcher::{
    Config, Matcher,
    pattern::{CaseMatching, Normalization, Pattern},
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
    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        nucleo_matcher::pattern::AtomKind::Fuzzy,
    );
    let scored = pattern.match_list(items.iter(), &mut matcher);
    let mut used = vec![false; items.len()];
    let mut out = Vec::with_capacity(scored.len());
    for (item, _) in scored {
        let idx = items
            .iter()
            .enumerate()
            .find(|(i, x)| *x == item && !used[*i])
            .map(|(i, _)| i)
            .expect("matched item came from items");
        used[idx] = true;
        out.push(idx);
    }
    out
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

pub const MAX_TREE_ENTRIES: usize = 20_000;

pub type EntriesReader<'a> = dyn FnMut(&Path, &str) -> Vec<(String, bool)> + 'a;

pub fn walk_tree(dir: &Path, max: usize, read: &mut EntriesReader<'_>) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([String::new()]);
    while let Some(prefix) = queue.pop_front() {
        for (name, is_dir) in read(&dir.join(&prefix), &prefix) {
            let rel = format!("{prefix}{name}");
            if is_dir {
                out.push(format!("{rel}/"));
                queue.push_back(format!("{rel}/"));
            } else {
                out.push(rel);
            }
            if out.len() >= max {
                out.sort();
                return out;
            }
        }
    }
    out.sort();
    out
}

pub fn visible_entries(dir: &Path) -> Vec<(String, bool)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, bool)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (!name.starts_with('.')).then(|| (name, e.file_type().is_ok_and(|t| t.is_dir())))
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(dir: &Path, max: usize) -> Vec<String> {
        walk_tree(dir, max, &mut |d, _| visible_entries(d))
    }

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

    #[test]
    fn duplicate_items_map_to_distinct_indices() {
        let items = vec!["a/x.txt".to_string(), "b/x.txt".to_string()];
        let mut got = fuzzy_filter(&items, "x.txt");
        got.sort();
        assert_eq!(got, vec![0, 1]);
    }

    #[test]
    fn walk_tree_lists_subdirectories_shallow_first() {
        let dir = std::env::temp_dir().join(format!("deditor_tree_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("sub/deep")).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        std::fs::write(dir.join("sub/b.txt"), "x").unwrap();
        std::fs::write(dir.join("sub/deep/c.txt"), "x").unwrap();
        std::fs::write(dir.join(".hidden"), "x").unwrap();
        assert_eq!(
            tree(&dir, MAX_TREE_ENTRIES),
            items(&["a.txt", "sub/", "sub/b.txt", "sub/deep/", "sub/deep/c.txt"])
        );
        assert_eq!(tree(&dir, 2), items(&["a.txt", "sub/"]));
        assert!(tree(&dir.join("missing"), MAX_TREE_ENTRIES).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn walk_tree_skips_branches_the_reader_drops() {
        let dir = std::env::temp_dir().join(format!("deditor_walk_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("sub/deep")).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        std::fs::write(dir.join("sub/b.txt"), "x").unwrap();
        let dropped = walk_tree(&dir, MAX_TREE_ENTRIES, &mut |d, _| {
            visible_entries(d)
                .into_iter()
                .filter(|(name, _)| name != "sub")
                .collect()
        });
        assert_eq!(dropped, items(&["a.txt"]));
        std::fs::remove_dir_all(&dir).ok();
    }
}
