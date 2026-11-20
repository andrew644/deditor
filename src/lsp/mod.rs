pub mod client;

use crate::buffer::Pos;
use client::{LspClient, RpcMsg};
use lsp_types as lt;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sev {
    Error,
    Warn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    pub line: usize,
    pub severity: Sev,
    pub message: String,
}

#[derive(PartialEq)]
enum Phase {
    Idle,
    Starting,
    Ready,
}

/// Manager-side result of a poll, applied by the Editor.
pub enum Ev {
    Goto {
        path: PathBuf,
        line: usize,
        col: usize,
    },
    NoDef,
    Timeout,
    Died,
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Wait {
    None,
    Hover,
    Goto,
    Completion,
}

#[derive(Clone, Debug)]
pub struct CompItem {
    pub kind: char,
    pub text: String,
}

pub struct CompletionState {
    pub items: Vec<CompItem>,
    pub selected: usize,
    pub line: usize,
    pub col: usize,
    pub seq: u64,
    pub prefix: String,
}

pub struct LspManager {
    client: Option<LspClient>,
    server: Option<&'static str>,
    failed: Vec<&'static str>,
    phase: Phase,
    open: HashMap<PathBuf, i32>,
    pending_open: Vec<PathBuf>,
    pub waiting: Wait,
    pub waiting_id: Option<u64>,
    waiting_since: Option<std::time::Instant>,
    pub hover: Option<String>,
    pub diags: HashMap<PathBuf, Vec<Diag>>,
    pub completion: Option<CompletionState>,
    completion_seq: u64,
    last_rx: Option<std::time::Instant>,
    root: PathBuf,
}

pub fn file_url(p: &Path) -> Option<lt::Url> {
    lt::Url::from_file_path(if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(p)
    })
    .ok()
}

pub fn language_for(p: &Path) -> Option<&'static str> {
    match p.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some("rust"),
        Some("c") | Some("h") => Some("c"),
        Some("cpp") | Some("hpp") => Some("cpp"),
        Some("odin") => Some("odin"),
        Some("java") => Some("java"),
        Some("cs") => Some("csharp"),
        Some("js") => Some("javascript"),
        Some("ts") => Some("typescript"),
        _ => None,
    }
}

pub fn server_for(p: &Path) -> Option<&'static str> {
    match language_for(p)? {
        "rust" => Some("rust-analyzer"),
        "c" | "cpp" => Some("clangd"),
        "odin" => Some("ols"),
        "java" => Some("jdtls"),
        "csharp" => Some("csharp-ls"),
        "javascript" | "typescript" => Some("typescript-language-server"),
        _ => None,
    }
}

impl LspManager {
    pub fn new() -> Self {
        Self {
            client: None,
            server: None,
            failed: Vec::new(),
            open: HashMap::new(),
            phase: Phase::Idle,
            pending_open: Vec::new(),
            waiting: Wait::None,
            waiting_id: None,
            waiting_since: None,
            hover: None,
            diags: HashMap::new(),
            completion: None,
            completion_seq: 0,
            last_rx: None,
            root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    fn caps() -> serde_json::Value {
        json!({
            "textDocument": {
                "publishDiagnostics": {},
                "hover": { "contentFormat": ["markdown", "plaintext"] },
                "definition": {},
                "completion": { "completionItem": { "snippetSupport": false } }
            }
        })
    }

    /// Ensure the server for `path` runs and the doc is open.
    pub fn ensure(&mut self, path: Option<&PathBuf>) {
        let Some(path) = path else { return };
        let Some(cmd) = server_for(path) else {
            return;
        };
        if self.failed.contains(&cmd) {
            return;
        }
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if self.server != Some(cmd) || self.root != root {
            self.reset();
            self.root = root;
            match LspClient::spawn(cmd, &[], &self.root.display().to_string()) {
                Ok(mut c) => {
                    c.request(
                        "initialize",
                        json!({
                            "processId": std::process::id(),
                            "rootUri": file_url(&self.root).map(|u| u.to_string()),
                            "capabilities": Self::caps(),
                        }),
                    );
                    self.client = Some(c);
                    self.server = Some(cmd);
                    self.phase = Phase::Starting;
                }
                Err(e) => {
                    eprintln!("deditor: {e}");
                    self.failed.push(cmd);
                    return;
                }
            }
        }
        self.open_doc(path);
    }

    fn reset(&mut self) {
        self.client = None;
        self.server = None;
        self.phase = Phase::Idle;
        self.open.clear();
        self.pending_open.clear();
        self.waiting = Wait::None;
        self.waiting_id = None;
        self.hover = None;
        self.completion = None;
    }

    fn open_doc(&mut self, path: &Path) {
        if self.open.contains_key(path) {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let Some(uri) = file_url(path) else {
            return;
        };
        if self.phase != Phase::Ready {
            if !self.pending_open.iter().any(|p| p == path) {
                self.pending_open.push(path.to_path_buf());
            }
            return;
        }
        self.open.insert(path.to_path_buf(), 1);
        if let Some(c) = &self.client {
            c.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": language_for(path).unwrap_or("plaintext"),
                        "version": 1,
                        "text": text
                    }
                }),
            );
        }
    }

    /// Push full text (bumps version) if the doc is open.
    fn sync_full(&mut self, path: &Path, text: &str) {
        if !self.open.contains_key(path) || self.client.is_none() {
            return;
        }
        let version = self.open.get(path).copied().unwrap_or(1) + 1;
        self.open.insert(path.to_path_buf(), version);
        if let (Some(c), Some(uri)) = (&self.client, file_url(path)) {
            c.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": uri, "version": version },
                    "contentChanges": [{ "text": text }]
                }),
            );
        }
    }

    /// Push full text after a save.
    pub fn on_saved(&mut self, path: &Path, text: &str) {
        self.sync_full(path, text);
    }

    fn doc_json(path: &Path, p: Pos, line: &str) -> Option<serde_json::Value> {
        let uri = file_url(path)?;
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": p.line, "character": utf16_col(line, p.col) }
        }))
    }

    pub fn hover_at(&mut self, path: &Path, p: Pos, line: &str, text: &str) {
        if self.phase != Phase::Ready {
            return;
        }
        self.sync_full(path, text);
        let Some(params) = Self::doc_json(path, p, line) else {
            return;
        };
        self.hover = None;
        if let Some(c) = self.client.as_mut() {
            let id = c.request("textDocument/hover", params);
            self.waiting = Wait::Hover;
            self.waiting_id = Some(id);
            self.waiting_since = Some(std::time::Instant::now());
        }
    }

    pub fn goto_definition(&mut self, path: &Path, p: Pos, line: &str, text: &str) {
        if self.phase != Phase::Ready {
            return;
        }
        self.sync_full(path, text);
        let Some(params) = Self::doc_json(path, p, line) else {
            return;
        };
        if let Some(c) = self.client.as_mut() {
            let id = c.request("textDocument/definition", params);
            self.waiting = Wait::Goto;
            self.waiting_id = Some(id);
            self.waiting_since = Some(std::time::Instant::now());
        }
    }

    pub fn request_completion(&mut self, path: &Path, p: Pos, text: &str) {
        if self.phase != Phase::Ready {
            return;
        }
        self.sync_full(path, text);
        let line = text.lines().nth(p.line).unwrap_or("");
        let Some(params) = Self::doc_json(path, p, line) else {
            return;
        };
        self.completion_seq += 1;
        let seq = self.completion_seq;
        let prefix = ident_prefix(text, p.line, p.col);
        self.completion = Some(CompletionState {
            items: Vec::new(),
            selected: 0,
            line: p.line,
            col: p.col,
            seq,
            prefix,
        });
        if let Some(c) = self.client.as_mut() {
            let id = c.request("textDocument/completion", params);
            self.waiting = Wait::Completion;
            self.waiting_id = Some(id);
            self.waiting_since = Some(std::time::Instant::now());
        }
    }

    pub fn diags_for(&self, path: Option<&PathBuf>) -> &[Diag] {
        path.and_then(|p| self.diags.get(p))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// True while a response or diagnostics burst is likely imminent.
    pub fn busy(&self) -> bool {
        self.phase == Phase::Starting
            || self.waiting != Wait::None
            || self
                .last_rx
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3))
    }

    pub fn active(&self) -> bool {
        self.client.is_some()
    }
}

impl LspManager {
    /// Drain server traffic; returns events for the Editor to apply.
    pub fn poll(&mut self) -> Vec<Ev> {
        let mut msgs = Vec::new();
        if let Some(c) = &self.client {
            while let Ok(m) = c.rx.try_recv() {
                msgs.push(m);
            }
        }
        if !msgs.is_empty() {
            self.last_rx = Some(std::time::Instant::now());
        }
        let mut evs = Vec::new();
        for m in msgs {
            match m {
                RpcMsg::ServerRequest { id, method } => {
                    if let Some(c) = &self.client {
                        if method == "workspace/configuration" {
                            c.respond(id, serde_json::json!([null]));
                        } else {
                            c.respond_null(id);
                        }
                    }
                }
                RpcMsg::Notification { method, params } => {
                    if method == "textDocument/publishDiagnostics" {
                        let Ok(dp) = serde_json::from_value::<lt::PublishDiagnosticsParams>(params)
                        else {
                            continue;
                        };
                        let Ok(path) = dp.uri.to_file_path() else {
                            continue;
                        };
                        let items: Vec<Diag> = dp
                            .diagnostics
                            .iter()
                            .filter(|d| {
                                d.severity
                                    .map(|s| {
                                        s == lt::DiagnosticSeverity::ERROR
                                            || s == lt::DiagnosticSeverity::WARNING
                                    })
                                    .unwrap_or(true)
                            })
                            .map(|d| Diag {
                                line: d.range.start.line as usize,
                                severity: if d.severity == Some(lt::DiagnosticSeverity::WARNING) {
                                    Sev::Warn
                                } else {
                                    Sev::Error
                                },
                                message: d.message.clone(),
                            })
                            .collect();
                        self.diags.insert(path, items);
                    }
                }
                RpcMsg::Response { id, result, .. } => {
                    if self.waiting_id != Some(id) {
                        if self.waiting == Wait::None
                            && self.waiting_id.is_none()
                            && id == 1
                            && self.phase == Phase::Starting
                        {
                            if let Some(c) = &self.client {
                                c.notify("initialized", json!({}));
                            }
                            self.phase = Phase::Ready;
                            let docs = std::mem::take(&mut self.pending_open);
                            for p in docs {
                                self.open_doc(&p);
                            }
                        }
                        continue;
                    }
                    self.waiting_id = None;
                    match self.waiting {
                        Wait::Hover => {
                            self.waiting = Wait::None;
                            self.hover = Some(parse_hover(result.as_ref()).unwrap_or_default());
                        }
                        Wait::Goto => {
                            self.waiting = Wait::None;
                            evs.push(match parse_location(result.as_ref()) {
                                Some((path, line, col)) => Ev::Goto { path, line, col },
                                None => Ev::NoDef,
                            });
                        }
                        Wait::Completion => {
                            self.waiting = Wait::None;
                            let items = parse_completions(result.as_ref());
                            if let Some(cs) = self.completion.as_mut()
                                && cs.seq == self.completion_seq
                            {
                                cs.items = items
                                    .into_iter()
                                    .filter(|i| i.text.starts_with(&cs.prefix))
                                    .collect();
                                cs.selected = 0;
                            }
                        }
                        Wait::None => {}
                    }
                }
            }
        }
        let died = self.client.as_mut().is_some_and(|c| c.exited());
        if died {
            self.client = None;
            if let Some(cmd) = self.server.take() {
                self.failed.push(cmd);
            }
            self.phase = Phase::Idle;
            self.open.clear();
            self.pending_open.clear();
            self.waiting = Wait::None;
            self.waiting_id = None;
            self.waiting_since = None;
            evs.push(Ev::Died);
        }
        if self.waiting != Wait::None
            && self
                .waiting_since
                .is_some_and(|t| t.elapsed() > std::time::Duration::from_secs(15))
        {
            self.waiting = Wait::None;
            self.waiting_id = None;
            self.waiting_since = None;
            evs.push(Ev::Timeout);
        }

        evs
    }
}
fn utf16_col(line: &str, col: usize) -> usize {
    line.chars().take(col).map(|c| c.len_utf16()).sum()
}

pub fn char_col(line: &str, target: usize) -> usize {
    let mut acc = 0;
    for (c, i) in line.chars().zip(0..) {
        if target < acc + c.len_utf16() {
            return i;
        }
        acc += c.len_utf16();
    }
    line.chars().count()
}

pub fn parse_hover(result: Option<&serde_json::Value>) -> Option<String> {
    let v = result?;
    if v.is_null() {
        return None;
    }
    let hv: lt::Hover = serde_json::from_value(v.clone()).ok()?;
    let mut out = String::new();
    match hv.contents {
        lt::HoverContents::Markup(m) => out.push_str(&m.value),
        lt::HoverContents::Scalar(s) => out.push_str(&marked_to_text(&s)),
        lt::HoverContents::Array(v) => {
            for s in v {
                out.push_str(&marked_to_text(&s));
                out.push('\n');
            }
        }
    }
    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn marked_to_text(m: &lt::MarkedString) -> String {
    match m {
        lt::MarkedString::String(s) => s.clone(),
        lt::MarkedString::LanguageString(ls) => format!("{}\n{}", ls.language, ls.value),
    }
}

pub fn parse_location(result: Option<&serde_json::Value>) -> Option<(PathBuf, usize, usize)> {
    let v = result?;
    if v.is_null() {
        return None;
    }
    if let Ok(locs) = serde_json::from_value::<Vec<lt::Location>>(v.clone()) {
        let l = locs.into_iter().next()?;
        return Some((
            l.uri.to_file_path().ok()?,
            l.range.start.line as usize,
            l.range.start.character as usize,
        ));
    }
    if let Ok(l) = serde_json::from_value::<lt::Location>(v.clone()) {
        return Some((
            l.uri.to_file_path().ok()?,
            l.range.start.line as usize,
            l.range.start.character as usize,
        ));
    }
    if let Ok(links) = serde_json::from_value::<Vec<lt::LocationLink>>(v.clone()) {
        let l = links.into_iter().next()?;
        let target = l.target_range.start;
        return Some((
            l.target_uri.to_file_path().ok()?,
            target.line as usize,
            target.character as usize,
        ));
    }
    None
}

/// Identifier run immediately before (line, col) in `text`.
pub fn ident_prefix(text: &str, line: usize, col: usize) -> String {
    let l = text.split('\n').nth(line).unwrap_or("");
    let chars: Vec<char> = l.chars().take(col).collect();
    let mut i = chars.len();
    while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        i -= 1;
    }
    chars[i..].iter().collect()
}

pub fn parse_completions(result: Option<&serde_json::Value>) -> Vec<CompItem> {
    let Some(v) = result else {
        return Vec::new();
    };
    let Ok(resp) = serde_json::from_value::<lt::CompletionResponse>(v.clone()) else {
        return Vec::new();
    };
    let items = match resp {
        lt::CompletionResponse::Array(a) => a,
        lt::CompletionResponse::List(l) => l.items,
    };
    items
        .iter()
        .map(|it| CompItem {
            kind: kind_char(it.kind),
            text: insert_text_of(it),
        })
        .collect()
}

pub fn kind_char(k: Option<lt::CompletionItemKind>) -> char {
    use lt::CompletionItemKind as K;
    match k {
        Some(K::FUNCTION) => 'f',
        Some(K::METHOD) => 'm',
        Some(K::FIELD | K::VARIABLE | K::CONSTANT | K::PROPERTY | K::REFERENCE) => 'v',
        Some(K::STRUCT | K::CLASS | K::ENUM | K::INTERFACE | K::MODULE | K::TYPE_PARAMETER) => 't',
        Some(K::KEYWORD | K::SNIPPET | K::OPERATOR | K::EVENT | K::UNIT) => 'k',
        _ => '-',
    }
}

fn insert_text_of(it: &lt::CompletionItem) -> String {
    let raw = it.insert_text.clone().unwrap_or_else(|| it.label.clone());
    if it.insert_text_format == Some(lt::InsertTextFormat::SNIPPET) {
        // keep literal text, drop $0/$1/`${2:...}` placeholders
        let mut out = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '$' {
                out.push(c);
                continue;
            }
            if chars.peek().is_some_and(|c| *c == '{') {
                let mut depth = 1;
                chars.next();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    chars.next();
                }
            }
        }
        out
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_markup_and_null() {
        let v = serde_json::json!({
            "contents": { "kind": "markdown", "value": "```rust\nfn main()\n```" }
        });
        let h = parse_hover(Some(&v)).unwrap();
        assert!(h.contains("fn main()"));
        assert_eq!(parse_hover(Some(&serde_json::Value::Null)), None);
        assert_eq!(parse_hover(None), None);
    }

    #[test]
    fn poll_reaps_an_exited_server() {
        let mut m = LspManager::new();
        m.client = Some(LspClient::spawn("true", &[], ".").unwrap());
        m.server = Some("deadserver");
        m.phase = Phase::Ready;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let evs = m.poll();
            if evs.iter().any(|e| matches!(e, Ev::Died)) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "server never reaped");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(m.client.is_none());
        assert_eq!(m.server, None);
        assert_eq!(m.failed, vec!["deadserver"]);
        assert!(!m.busy());
        assert!(m.poll().is_empty());
    }

    #[test]
    fn young_wait_does_not_time_out() {
        let mut m = LspManager::new();
        m.waiting = Wait::Goto;
        m.waiting_since = Some(std::time::Instant::now());
        let evs = m.poll();
        assert!(!evs.iter().any(|e| matches!(e, Ev::Timeout)));
        assert_eq!(m.waiting, Wait::Goto);
    }

    #[test]
    fn location_array_first_wins() {
        let v = serde_json::json!([
            { "uri": "file:///a.rs", "range": {
                "start": { "line": 4, "character": 2 },
                "end": { "line": 4, "character": 6 } } },
            { "uri": "file:///b.rs", "range": {
                "start": { "line": 9, "character": 0 },
                "end": { "line": 9, "character": 1 } } }
        ]);
        let (p, line, col) = parse_location(Some(&v)).unwrap();
        assert_eq!(p, PathBuf::from("/a.rs"));
        assert_eq!((line, col), (4, 2));
        assert_eq!(parse_location(Some(&serde_json::Value::Null)), None);
    }

    #[test]
    fn location_single_and_link() {
        let v = serde_json::json!(
            { "uri": "file:///c.rs", "range": {
                "start": { "line": 1, "character": 3 },
                "end": { "line": 1, "character": 4 } } }
        );
        let (p, line, _) = parse_location(Some(&v)).unwrap();
        assert_eq!(p, PathBuf::from("/c.rs"));
        assert_eq!(line, 1);
        let v = serde_json::json!([{
            "targetUri": "file:///d.rs",
            "targetRange": {
                "start": { "line": 7, "character": 8 },
                "end": { "line": 7, "character": 9 } },
            "targetSelectionRange": {
                "start": { "line": 7, "character": 8 },
                "end": { "line": 7, "character": 9 } }
        }]);
        let (p, line, col) = parse_location(Some(&v)).unwrap();
        assert_eq!(p, PathBuf::from("/d.rs"));
        assert_eq!((line, col), (7, 8));
    }

    #[test]
    fn completions_insert_text_and_snippets() {
        let v = serde_json::json!([
            { "label": "map()", "insertText": "map" },
            { "label": "filter" }
        ]);
        let got: Vec<String> = parse_completions(Some(&v))
            .into_iter()
            .map(|i| i.text)
            .collect();
        assert_eq!(got, vec!["map", "filter"]);
        let v = serde_json::json!({ "isIncomplete": false, "items": [{ "label": "x" }] });
        let got: Vec<String> = parse_completions(Some(&v))
            .into_iter()
            .map(|i| i.text)
            .collect();
        assert_eq!(got, vec!["x"]);
        // snippet placeholders stripped
        let v = serde_json::json!([{
            "label": "ok?",
            "kind": 2,
            "insertText": "ok(${1:self: &str}$0)",
            "insertTextFormat": 2
        }]);
        let items = parse_completions(Some(&v));
        assert_eq!(items[0].text, "ok()");
        assert_eq!(items[0].kind, 'm');
        assert!(parse_completions(None).is_empty());
    }

    #[test]
    fn language_detect() {
        assert_eq!(language_for(Path::new("x.rs")), Some("rust"));
        assert_eq!(language_for(Path::new("x.c")), Some("c"));
        assert_eq!(language_for(Path::new("x.h")), Some("c"));
        assert_eq!(language_for(Path::new("x.cpp")), Some("cpp"));
        assert_eq!(language_for(Path::new("x.hpp")), Some("cpp"));
        assert_eq!(language_for(Path::new("x.odin")), Some("odin"));
        assert_eq!(language_for(Path::new("x.java")), Some("java"));
        assert_eq!(language_for(Path::new("x.cs")), Some("csharp"));
        assert_eq!(language_for(Path::new("x.js")), Some("javascript"));
        assert_eq!(language_for(Path::new("x.ts")), Some("typescript"));
        assert_eq!(language_for(Path::new("x.txt")), None);
        assert_eq!(language_for(Path::new("x.md")), None);
        assert_eq!(language_for(Path::new("noext")), None);
    }

    #[test]
    fn server_mapping() {
        assert_eq!(server_for(Path::new("x.rs")), Some("rust-analyzer"));
        assert_eq!(server_for(Path::new("x.h")), Some("clangd"));
        assert_eq!(server_for(Path::new("x.cpp")), Some("clangd"));
        assert_eq!(server_for(Path::new("x.odin")), Some("ols"));
        assert_eq!(server_for(Path::new("x.java")), Some("jdtls"));
        assert_eq!(server_for(Path::new("x.cs")), Some("csharp-ls"));
        assert_eq!(
            server_for(Path::new("x.ts")),
            Some("typescript-language-server")
        );
        assert_eq!(server_for(Path::new("x.md")), None);
        assert_eq!(server_for(Path::new("x.yaml")), None);
    }

    #[test]
    fn ensure_ignores_non_programming_files() {
        let mut m = LspManager::new();
        m.ensure(Some(&PathBuf::from("/tmp/notes.md")));
        assert!(!m.active());
        m.ensure(Some(&PathBuf::from("/tmp/notes.txt")));
        m.ensure(None);
        assert!(!m.active());
    }

    #[test]
    fn file_url_absolute_roundtrip() {
        let u = file_url(Path::new("/tmp/proj/main.rs")).unwrap();
        assert_eq!(
            u.to_file_path().unwrap(),
            PathBuf::from("/tmp/proj/main.rs")
        );
    }

    #[cfg(unix)]
    #[test]
    fn hover_shares_the_edited_text_first() {
        let mut m = LspManager::new();
        m.client = Some(LspClient::spawn("cat", &[], "/tmp").unwrap());
        m.phase = Phase::Ready;
        let path = PathBuf::from("/tmp/deditor_hover.rs");
        m.open.insert(path.clone(), 3);
        m.hover_at(&path, Pos { line: 0, col: 0 }, "old", "fn edited() {}");
        let rx = &m.client.as_ref().unwrap().rx;
        match rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() {
            RpcMsg::Notification { method, params } => {
                assert_eq!(method, "textDocument/didChange");
                assert_eq!(params["contentChanges"][0]["text"], "fn edited() {}");
                assert_eq!(params["textDocument"]["version"], 4);
            }
            _ => panic!("expected the sync notification, got other message"),
        }
        match rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() {
            RpcMsg::ServerRequest { method, .. } => assert_eq!(method, "textDocument/hover"),
            _ => panic!("expected the hover request, got other message"),
        }
    }

    #[test]
    fn a_new_working_directory_retires_the_server() {
        let mut m = LspManager::new();
        m.server = Some("rust-analyzer");
        m.phase = Phase::Ready;
        m.root = PathBuf::from("/deditor-not-the-cwd");
        m.open
            .insert(PathBuf::from("/deditor-not-the-cwd/main.rs"), 1);
        m.ensure(Some(&PathBuf::from("main.rs")));
        assert_eq!(m.root, std::env::current_dir().unwrap());
        assert!(m.open.is_empty(), "documents of the old server are retired");
    }

    #[cfg(unix)]
    #[test]
    fn framing_roundtrip_via_cat() {
        let c = LspClient::spawn("cat", &[], "/tmp").unwrap();
        c.notify("test/notify", json!({ "n": 7, "unicode": "héllo→" }));
        let msg =
            c.rx.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        match msg {
            RpcMsg::Notification { method, params } => {
                assert_eq!(method, "test/notify");
                assert_eq!(params["n"], 7);
                assert_eq!(params["unicode"], "héllo→");
            }
            _ => panic!("expected notification echo, got other message"),
        }
    }

    #[test]
    fn utf16_columns_round_trip_around_astral_chars() {
        let line = "a😀b";
        assert_eq!(utf16_col(line, 0), 0);
        assert_eq!(utf16_col(line, 1), 1);
        assert_eq!(utf16_col(line, 2), 3);
        assert_eq!(utf16_col(line, 3), 4);
        for c in 0..=3 {
            assert_eq!(char_col(line, utf16_col(line, c)), c);
        }
        assert_eq!(char_col(line, 2), 1);
        assert_eq!(char_col(line, 99), 3);
    }
}
