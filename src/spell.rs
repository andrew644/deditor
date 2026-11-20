use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::SystemTime;

const WORDS: &str = include_str!("../data/wordlist/en_US.txt");

static BASE_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| WORDS.lines().collect());

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '\u{2019}'
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || is_apostrophe(c)
}

fn normalize(word: &str) -> String {
    word.replace('\u{2019}', "'").to_lowercase()
}

#[derive(Default)]
pub struct Spell {
    pub spans: HashMap<usize, (u64, Vec<(usize, usize)>)>,
    personal: HashSet<String>,
    personal_path: String,
    personal_stamp: Option<SystemTime>,
}

impl Spell {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.spans.clear();
    }

    pub fn sync_personal(&mut self, path: &str) {
        let stamp = if path.is_empty() {
            None
        } else {
            std::fs::metadata(path).and_then(|m| m.modified()).ok()
        };
        if path == self.personal_path && stamp == self.personal_stamp {
            return;
        }
        self.personal_path = path.to_string();
        self.personal_stamp = stamp;
        self.personal = match stamp {
            Some(_) => std::fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(normalize)
                .collect(),
            None => HashSet::new(),
        };
        self.spans.clear();
    }

    fn known(&self, word: &str) -> bool {
        self.personal.contains(word) || BASE_WORDS.contains(word)
    }

    pub fn spans_for(&self, line: &str) -> Vec<(usize, usize)> {
        let chars: Vec<char> = line.chars().collect();
        let mut spans = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if !is_token_char(chars[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && is_token_char(chars[i]) {
                i += 1;
            }
            let mut s = start;
            let mut e = i;
            while s < e && !chars[s].is_alphanumeric() {
                s += 1;
            }
            while e > s && !chars[e - 1].is_alphanumeric() {
                e -= 1;
            }
            if e - s < 2 {
                continue;
            }
            let word = normalize(&chars[s..e].iter().collect::<String>());
            if !word.chars().any(|c| c.is_alphabetic()) || word.chars().any(|c| c.is_numeric()) {
                continue;
            }
            if !self.known(&word) {
                spans.push((s, e - s));
            }
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("deditor-spell-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn flags_unknown_words_with_char_offsets() {
        let sp = Spell::new();
        assert_eq!(sp.spans_for("the quik brown fox"), vec![(4, 4)]);
        assert_eq!(
            sp.spans_for("one mistaked, two teh."),
            vec![(4, 8), (18, 3)]
        );
        assert!(sp.spans_for("all fine here").is_empty());
    }

    #[test]
    fn offsets_count_characters_not_bytes() {
        let sp = Spell::new();
        assert_eq!(sp.spans_for("\u{2013} quik"), vec![(2, 4)]);
        assert_eq!(sp.spans_for("naïve quik"), vec![(0, 5), (6, 4)]);
    }

    #[test]
    fn accepts_inflected_possessive_and_contracted_forms() {
        let sp = Spell::new();
        for line in [
            "running walked happier quickly",
            "the dog's bone and the dogs' bones",
            "don't isn't can't",
            "London london MacDonald",
        ] {
            assert!(sp.spans_for(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn ignores_short_words_numbers_and_identifiers() {
        let sp = Spell::new();
        for line in [
            "x = y + 42",
            "utf8 sha256 x86 mp3",
            "foo_bar _private name_of",
            "a I 42",
            "... --- ???",
        ] {
            assert!(sp.spans_for(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn identifier_segments_are_checked_one_by_one() {
        let sp = Spell::new();
        assert_eq!(sp.spans_for("name_mistaked"), vec![(5, 8)]);
        assert_eq!(sp.spans_for("quik_2_x"), vec![(0, 4)]);
    }

    #[test]
    fn personal_words_are_accepted_and_reloaded() {
        let path = temp("personal.txt", "# comment\n\nMistaked\nquik\n");
        let mut sp = Spell::new();
        assert_eq!(sp.spans_for("mistaked quik").len(), 2);
        sp.sync_personal(path.to_str().unwrap());
        assert!(sp.spans_for("mistaked quik").is_empty());
        std::fs::write(&path, "blorp\n").unwrap();
        touch(&path);
        sp.sync_personal(path.to_str().unwrap());
        assert_eq!(sp.spans_for("blorp mistaked"), vec![(6, 8)]);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn clearing_personal_path_drops_its_words() {
        let path = temp("personal-drop.txt", "blorp\n");
        let mut sp = Spell::new();
        sp.sync_personal(path.to_str().unwrap());
        assert!(sp.spans_for("blorp").is_empty());
        sp.sync_personal("");
        assert_eq!(sp.spans_for("blorp"), vec![(0, 5)]);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn clear_drops_cached_spans() {
        let mut sp = Spell::new();
        sp.spans.insert(3, (7, vec![(1, 2)]));
        sp.clear();
        assert!(sp.spans.is_empty());
    }

    fn touch(path: &std::path::Path) {
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(later)
            .unwrap();
    }
}
