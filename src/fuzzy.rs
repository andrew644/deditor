use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

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

pub fn spawn_file_walk(dir: PathBuf) -> Receiver<Vec<String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut walk = ignore::WalkBuilder::new(&dir);
        walk.hidden(true);
        for entry in walk.build() {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&dir) else {
                continue;
            };
            out.push(rel.to_string_lossy().replace('\\', "/"));
            if out.len() >= 50_000 {
                break;
            }
        }
        let _ = tx.send(out);
    });
    rx
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
}
