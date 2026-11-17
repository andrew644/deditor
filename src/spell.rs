use std::io::Read;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread;
use winit::event_loop::EventLoopProxy;

#[derive(Default)]
pub struct Spell {
    pub words: Vec<String>,
    pub spans: std::collections::HashMap<usize, (u64, Vec<(usize, usize)>)>,
    inflight: Option<Receiver<Vec<String>>>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Spell {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn busy(&self) -> bool {
        self.inflight.is_some()
    }

    pub fn clear(&mut self) {
        self.abort();
        self.inflight = None;
        self.words.clear();
        self.spans.clear();
    }

    fn abort(&mut self) {
        if let Some(mut ch) = self.child.lock().unwrap().take() {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }

    pub fn request(&mut self, text: String, dict: String, proxy: Option<EventLoopProxy<()>>) {
        self.abort();
        let (tx, rx) = channel();
        self.inflight = Some(rx);
        let slot = self.child.clone();
        thread::spawn(move || {
            let words = run_hunspell_slot(&text, &dict, &slot);
            let _ = tx.send(words);
            if let Some(p) = proxy {
                let _ = p.send_event(());
            }
        });
    }

    pub fn poll(&mut self) {
        let update = self.inflight.as_ref().map(|rx| rx.try_recv());
        match update {
            Some(Ok(words)) => {
                self.words = words;
                self.spans.clear();
                self.inflight = None;
            }
            Some(Err(TryRecvError::Disconnected)) => self.inflight = None,
            _ => {}
        }
    }
}

#[cfg(test)]
pub fn run_hunspell(text: &str, dict: &str) -> Vec<String> {
    run_hunspell_slot(text, dict, &Arc::new(Mutex::new(None)))
}

fn run_hunspell_slot(text: &str, dict: &str, slot: &Arc<Mutex<Option<Child>>>) -> Vec<String> {
    let mut cmd = Command::new("hunspell");
    cmd.arg("-l");
    if !dict.is_empty() {
        cmd.arg("-d").arg(dict);
    }
    let mut child = match cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    if let Some(mut si) = child.stdin.take() {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = si.write_all(text.as_bytes());
        });
    }
    let so = child.stdout.take();
    *slot.lock().unwrap() = Some(child);
    let mut out = String::new();
    if let Some(mut so) = so {
        let _ = so.read_to_string(&mut out);
    }
    if let Some(mut ch) = slot.lock().unwrap().take() {
        let _ = ch.wait();
    }
    let mut words: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in out.lines() {
        let w = line.split('/').next().unwrap_or("").trim();
        if !w.is_empty() && seen.insert(w.to_string()) {
            words.push(w.to_string());
        }
    }
    words
}

pub fn misspelled_spans(line: &str, words: &[String]) -> Vec<(usize, usize)> {
    use crate::motions::char_is_word;
    let chars: Vec<char> = line.chars().collect();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for w in words {
        let t: Vec<char> = w.chars().collect();
        if t.is_empty() {
            continue;
        }
        let mut i = 0;
        while i + t.len() <= chars.len() {
            if chars[i..i + t.len()] == t[..] {
                let before_ok = i == 0 || !char_is_word(chars[i - 1]) || !char_is_word(t[0]);
                let after_ok = i + t.len() >= chars.len()
                    || !char_is_word(chars[i + t.len()])
                    || !char_is_word(t[t.len() - 1]);
                if before_ok && after_ok {
                    spans.push((i, t.len()));
                }
                i += t.len();
            } else {
                i += 1;
            }
        }
    }
    spans.sort();
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    static SPELL_PROC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn spans_find_whole_word_occurrences() {
        let words = vec!["mistaked".to_string()];
        let spans = misspelled_spans("a mistaked mistakedx (mistaked).", &words);
        assert_eq!(spans, vec![(2, 8), (22, 8)]);
    }

    #[test]
    fn spans_cover_multiple_words() {
        let words = vec!["one".to_string(), "two".to_string()];
        let spans = misspelled_spans("one two one", &words);
        assert_eq!(spans, vec![(0, 3), (4, 3), (8, 3)]);
    }

    #[test]
    fn run_hunspell_flags_unknown_words() {
        let _g = SPELL_PROC_LOCK.lock().unwrap();
        if Command::new("hunspell").arg("-D").output().is_err() {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("deditor-spell-test-dict-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("testdict.aff"), "SET UTF-8\n").unwrap();
        std::fs::write(dir.join("testdict.dic"), "3\nhello\nworld\nmistaked\n").unwrap();
        let dict = dir.join("testdict").display().to_string();
        let words = run_hunspell("hello mistaked blorp world zorp\n", &dict);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(words, vec!["blorp".to_string(), "zorp".to_string()]);
        let missing = run_hunspell("anything\n", "/nonexistent/dict");
        assert!(missing.is_empty());
    }

    #[test]
    fn request_poll_delivers_words() {
        let _g = SPELL_PROC_LOCK.lock().unwrap();
        let mut sp = Spell::new();
        assert!(!sp.busy());
        sp.request("hello blorp\n".to_string(), String::new(), None);
        assert!(sp.busy());
        for _ in 0..500 {
            sp.poll();
            if !sp.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!sp.busy());
        sp.clear();
        assert!(sp.words.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_new_request_replaces_the_running_check() {
        let _g = SPELL_PROC_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("deditor-spell-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("hunspell"),
            "#!/bin/sh\ncase \" $* \" in *\" slow \"*) sleep 5 ;; esac\ncat\n",
        )
        .unwrap();
        std::fs::set_permissions(
            dir.join("hunspell"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let old_path = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", format!("{}:{old_path}", dir.display())) };
        let mut sp = Spell::new();
        sp.request("alpha\n".to_string(), "slow".to_string(), None);
        std::thread::sleep(std::time::Duration::from_millis(100));
        sp.request("beta\n".to_string(), String::new(), None);
        for _ in 0..1000 {
            sp.poll();
            if !sp.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        unsafe { std::env::set_var("PATH", old_path) };
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(sp.words, vec!["beta".to_string()]);
    }
}
