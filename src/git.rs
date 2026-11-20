use git2::{DiffOptions, Repository};
use std::cell::RefCell;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sign {
    None,
    Added,
    Modified,
    Deleted,
}

/// One run of changed lines: `added` lines starting at `line` in the current
/// buffer, plus `removed` lines deleted just before that position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Group {
    pub line: usize,
    pub added: usize,
    pub removed: usize,
}

pub fn signs(groups: &[Group], lines: usize) -> Vec<Sign> {
    let mut out = vec![Sign::None; lines];
    for g in groups {
        let mark = match (g.added > 0, g.removed > 0) {
            (true, true) => Sign::Modified,
            (true, false) => Sign::Added,
            (false, true) => Sign::Deleted,
            (false, false) => continue,
        };
        let count = g.added.max(1);
        let start = g.line.min(lines.saturating_sub(1));
        for row in out.iter_mut().skip(start).take(count) {
            *row = mark;
        }
    }
    out
}

/// Collects diff line events into groups. Deleted lines have no position in the
/// new text, so they wait in `pending` until the line that follows them shows
/// up; a run of deletions directly followed by insertions is one modification.
#[derive(Default)]
pub struct Grouper {
    groups: Vec<Group>,
    open: Option<Group>,
    pending: usize,
    anchor: usize,
}

impl Grouper {
    pub fn hunk(&mut self, first: usize) {
        self.flush();
        self.anchor = first;
        if self.pending > 0 {
            self.groups.push(Group {
                line: first,
                added: 0,
                removed: self.pending,
            });
            self.pending = 0;
        }
    }

    pub fn add(&mut self, index: usize) {
        match self.open {
            Some(g) if g.removed == 0 && g.line + g.added == index && self.pending == 0 => {
                if let Some(g) = self.open.as_mut() {
                    g.added += 1;
                }
            }
            _ => {
                self.flush();
                let removed = self.pending;
                self.pending = 0;
                self.open = Some(Group {
                    line: index,
                    added: 1,
                    removed,
                });
            }
        }
    }

    pub fn del(&mut self) {
        self.flush();
        self.pending += 1;
    }

    pub fn ctx(&mut self, index: usize) {
        self.hunk(index);
    }

    pub fn finish(mut self, lines: usize) -> Vec<Group> {
        self.flush();
        if self.pending > 0 {
            self.groups.push(Group {
                line: self.anchor.min(lines),
                added: 0,
                removed: self.pending,
            });
        }
        self.groups
    }

    fn flush(&mut self) {
        if let Some(g) = self.open.take() {
            self.groups.push(g);
        }
    }
}

pub fn changes(path: &Path, text: &str) -> Vec<Sign> {
    let lines = text.lines().count().max(1);
    match groups(path, text) {
        Some(groups) => signs(&groups, lines),
        None => vec![Sign::None; lines],
    }
}

/// How `text` differs from the file's committed blob, or `None` when there is
/// nothing to compare against: no repository, or untracked in one that has
/// commits. A repository without any commit yet reports every line as new.
fn groups(path: &Path, text: &str) -> Option<Vec<Group>> {
    let file = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let repo = Repository::discover(file.parent()?).ok()?;
    let root = std::fs::canonicalize(repo.workdir()?).ok()?;
    let rel = file.strip_prefix(&root).ok()?;
    let old = match repo.head().and_then(|h| h.peel_to_tree()) {
        Ok(tree) => Some(repo.find_blob(tree.get_path(rel).ok()?.id()).ok()?),
        Err(_) => None,
    };
    let new = repo.find_blob(repo.blob(text.as_bytes()).ok()?).ok()?;
    let mut opts = DiffOptions::new();
    opts.context_lines(0).indent_heuristic(true);
    let g = RefCell::new(Grouper::default());
    let mut hunk = |_delta: git2::DiffDelta<'_>, h: git2::DiffHunk<'_>| {
        let first = if h.new_lines() > 0 {
            h.new_start().saturating_sub(1)
        } else {
            h.new_start()
        };
        g.borrow_mut().hunk(first as usize);
        true
    };
    let mut line = |_delta: git2::DiffDelta<'_>,
                    _hunk: Option<git2::DiffHunk<'_>>,
                    diff_line: git2::DiffLine<'_>| {
        let mut g = g.borrow_mut();
        match diff_line.origin() {
            '+' => g.add(row(diff_line.new_lineno())),
            '-' => g.del(),
            _ => g.ctx(row(diff_line.new_lineno())),
        }
        true
    };
    repo.diff_blobs(
        old.as_ref(),
        None,
        Some(&new),
        None,
        Some(&mut opts),
        None,
        None,
        Some(&mut hunk),
        Some(&mut line),
    )
    .ok()?;
    Some(g.into_inner().finish(text.lines().count().max(1)))
}

fn row(line: Option<u32>) -> usize {
    line.unwrap_or(1).saturating_sub(1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(line: usize, added: usize, removed: usize) -> Group {
        Group {
            line,
            added,
            removed,
        }
    }

    #[test]
    fn groups_map_to_the_three_signs() {
        let groups = [g(1, 2, 1), g(4, 1, 0), g(6, 0, 3)];
        assert_eq!(
            signs(&groups, 8),
            vec![
                Sign::None,
                Sign::Modified,
                Sign::Modified,
                Sign::None,
                Sign::Added,
                Sign::None,
                Sign::Deleted,
                Sign::None,
            ]
        );
    }

    #[test]
    fn deletion_at_end_of_file_marks_the_last_line() {
        assert_eq!(
            signs(&[g(9, 0, 2)], 3),
            vec![Sign::None, Sign::None, Sign::Deleted]
        );
    }

    #[test]
    fn deletions_before_insertions_read_as_one_modification() {
        let mut gr = Grouper::default();
        gr.hunk(1);
        gr.del();
        gr.add(1);
        assert_eq!(gr.finish(4), vec![g(1, 1, 1)]);
    }

    #[test]
    fn bare_deletions_anchor_on_the_following_line() {
        let mut gr = Grouper::default();
        gr.hunk(2);
        gr.del();
        gr.del();
        gr.ctx(2);
        gr.add(3);
        assert_eq!(gr.finish(6), vec![g(2, 0, 2), g(3, 1, 0)]);
    }

    #[test]
    fn separate_hunks_do_not_merge_runs() {
        let mut gr = Grouper::default();
        gr.add(0);
        gr.hunk(1);
        gr.add(1);
        assert_eq!(gr.finish(4), vec![g(0, 1, 0), g(1, 1, 0)]);
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("deditor_git_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn repo_committed(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let repo = Repository::init(dir).unwrap();
        let file = dir.join(name);
        std::fs::write(&file, body).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new(name)).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("t", "t@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        file
    }

    const BASE: &str = "one\ntwo\nthree\nfour\nfive\n";

    #[test]
    fn unchanged_file_has_no_signs() {
        let d = dir("clean");
        let f = repo_committed(&d, "f.txt", BASE);
        assert_eq!(
            changes(&f, BASE),
            vec![Sign::None; 5],
            "committed text must not show signs"
        );
    }

    #[test]
    fn added_modified_and_removed_lines_are_located() {
        let d = dir("mixed");
        let f = repo_committed(&d, "f.txt", BASE);
        let added = "one\ntwo\nnew\nthree\nfour\nfive\n";
        assert_eq!(
            changes(&f, added),
            vec![
                Sign::None,
                Sign::None,
                Sign::Added,
                Sign::None,
                Sign::None,
                Sign::None,
            ]
        );
        let modified = "one\ntwo\nTHREE\nfour\nfive\n";
        assert_eq!(
            changes(&f, modified),
            vec![
                Sign::None,
                Sign::None,
                Sign::Modified,
                Sign::None,
                Sign::None,
            ]
        );
        let removed = "one\ntwo\nfour\nfive\n";
        assert_eq!(
            changes(&f, removed),
            vec![Sign::None, Sign::None, Sign::Deleted, Sign::None,]
        );
        let tail = "one\ntwo\nthree\n";
        assert_eq!(
            changes(&f, tail),
            vec![Sign::None, Sign::None, Sign::Deleted,]
        );
    }

    #[test]
    fn untracked_files_are_left_alone() {
        let d = dir("untracked");
        let _ = repo_committed(&d, "tracked.txt", BASE);
        let f = d.join("fresh.txt");
        std::fs::write(&f, BASE).unwrap();
        assert_eq!(changes(&f, BASE), vec![Sign::None; 5]);
    }

    #[test]
    fn every_line_is_new_before_the_first_commit() {
        let d = dir("unborn");
        Repository::init(&d).unwrap();
        let f = d.join("f.txt");
        std::fs::write(&f, BASE).unwrap();
        assert_eq!(changes(&f, BASE), vec![Sign::Added; 5]);
    }

    #[test]
    fn files_outside_a_repository_have_no_signs() {
        let d = dir("plain");
        let f = d.join("f.txt");
        std::fs::write(&f, BASE).unwrap();
        let outside = changes(&f, BASE);
        assert!(
            outside.iter().all(|s| *s != Sign::Added),
            "a bare directory must not be treated as a repository: {outside:?}"
        );
    }
}
