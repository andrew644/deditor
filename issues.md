# Issues

Items found during review that could not be fixed without a design change
or cannot be verified from this machine.

## Windows: language server survives an editor crash

On Unix the server inherits `PR_SET_PDEATHSIG` and dies with the
editor. Windows has no per-process death signal, so a killed editor
orphans `rust-analyzer` (clean exits still kill it via `Drop`). The
proper fix is a kill-on-close job object around the spawned server;
it cannot be verified from this Linux machine.

## Rust-analyzer restart storms

`ensure_lsp` respawns the configured server whenever the active
buffer's server changes and after a failed spawn is only remembered
per binary (`failed` list). A server binary that dies immediately
after a successful spawn (bad config, OOM) can be respawned on every
`activate`/`open_path`. Unverified on this machine — needs an
environment where a server crashes in its init handshake to confirm
the respawn cadence is actually user-visible before adding backoff.

## Search materializes the whole buffer on every step

`Editor::search_step` (src/app.rs) collects the entire rope into
`Vec<char>` for every `n`/`N`/`*`/`#` step, and `:r` does the same to
build its edit list. On a 10 MB file each keystroke copies ~20 MB and
scans it; the step latency grows linearly with file size. Fixing this
means scanning `Rope::chunks()` incrementally (with a char-boundary
safe cursor) instead of one flat `Vec<char>`, which touches the search,
replace and highlight paths at once.

## Clipboard calls run on the UI thread

`clipboard::set`/`get` fork helper processes (wl-copy, xclip, powershell)
and wait for them synchronously from key handling. Normal latency is a
few milliseconds, but a stalled selection owner or a huge yank still
freezes input and redraw for the duration of the subprocess. A proper fix
is a dedicated clipboard worker with a queue for stores and a request/
reply channel for loads; the internal register already covers the common
yank/paste path meanwhile.

## Hunspell input is unbounded

`Spell::request` pipes the whole buffer to hunspell on a worker thread.
A stalled or slow hunspell on a multi-megabyte buffer now no longer
deadlocks (stdin is written from a second thread), but the spellcheck
result can take arbitrarily long and stale runs are only dropped on
`F1`/clear. If this shows up in practice, kill and restart the child
when a new request arrives instead of waiting for it.
