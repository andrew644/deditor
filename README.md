# deditor

A GPU-accelerated modal text editor written in Rust (winit + wgpu + glyphon).
Vim-like modes: normal, insert, visual (character / line / block), command line,
and fuzzy panels. Optional LSP integration.

```
cargo build --release
./target/release/deditor [file]
```

A numeric prefix (`count`) works with motions and most edits, e.g. `3w`, `5x`, `2dd`, `10j`.

## Normal mode

### Motions

| Key | Action |
| --- | --- |
| `h` `j` `k` `l` | left / down / up / right |
| arrow keys | same as `h` `j` `k` `l` (also in visual mode and after an operator) |
| `w` `b` | next / previous word start (punctuation-aware) |
| `W` `B` | next / previous WORD start (whitespace-separated) |
| `e` `E` | end of word / WORD |
| `0` `^` `$` | first column / first non-blank / end of line |
| `gg` | first line (count: line N) |
| `G` | last line (count: line N) |
| `gj` `gk` | visual down / up (respects soft wrap) |
| `ge` `gE` | previous word end / WORD end |
| `%` | matching bracket |
| `PageUp` `PageDown` | full page up / down |
| `Ctrl-u` `Ctrl-d` | half page up / down |
| `Ctrl-f` | page down |
| `zz` | center cursor line in the viewport |

### Inserting text

| Key | Action |
| --- | --- |
| `i` `a` | insert before / after cursor |
| `I` `A` | insert at first non-blank / end of line |
| `o` `O` | open line below / above and insert |
| `Ctrl-a` | append at end of line |

### Editing

| Key | Action |
| --- | --- |
| `x` | delete character under cursor |
| `s` | change character under cursor (delete + insert) |
| `r{c}` | replace character under cursor with `c` |
| `dd` | delete line |
| `cc` | change line |
| `yy` | yank line |
| `d{motion}` `c{motion}` `y{motion}` | operator on a motion (`dw`, `c$`, `yG`, …) |
| `dgg` `dG` | delete to first / last line |
| `p` `P` | paste register after / before cursor |
| `u` | undo |
| `Ctrl-r` | redo |
| `J` | join lines |
| `>{motion}` `<{motion}` | indent / outdent (`>` / `<` then motion, `>>` / `<<` for a line) |

### Find and search

| Key | Action |
| --- | --- |
| `f{c}` `F{c}` | forward / backward to character |
| `t{c}` `T{c}` | forward / backward till character |
| `;` `,` | repeat last `f`/`t` / reversed |
| `/text` Enter | search forward |
| `?text` Enter | search backward |
| `n` `N` | next / previous search match |
| `*` | search word under cursor forward |
| `#` | search word under cursor backward |

### Text objects (after `d`, `c`, `y`, or in visual mode after `i`/`a`)

| Key | Action |
| --- | --- |
| `iw` `aw` | inner / around word |
| `iW` `aW` | inner / around WORD |
| `i"`, `i'`, `` i` `` and `a"`, `a'`, `` a` `` | quoted strings |
| `i(` `i)` `i[` `i]` `i{` `i}` (+ `a` forms) | bracketed blocks |

### Buffers, panels and LSP

| Key | Action |
| --- | --- |
| `Ctrl-p` | fuzzy file finder (type to filter, `~` paths work) |
| `Ctrl-b` | fuzzy buffer switcher |
| `gd` | go to definition (LSP) |
| `K` | hover documentation (LSP) |

LSP runs only for programming files: `.rs` (rust-analyzer), `.c` `.h` `.cpp`
`.hpp` (clangd), `.odin` (ols), `.java` (jdtls), `.cs` (csharp-ls), `.js` `.ts`
(typescript-language-server). Servers are started on demand; if a server binary
is missing it is silently skipped.

### Windows (splits)

The layout holds one split at a time: at most two windows, either side by side
or stacked. Splitting again reports `Only one split is allowed`, so close the
focused window before changing the axis.

Every window has its own tab strip, line-number gutter, wrap width and scroll
position, so two windows on the same buffer can sit at different lines: only
the window you type in follows the cursor, the other keeps its place until the
mouse wheel or a focus change moves it.

| Key | Action |
| --- | --- |
| `Ctrl-w w`, `Ctrl-w Ctrl-w` | focus the next window |
| `Ctrl-w c` | close the focused window (fails if its buffer is dirty) |
| `Ctrl-w s` | split into top and bottom (refused when a split is already open) |
| `Ctrl-w v` | split into left and right (refused when a split is already open) |

Clicking another window's tab strip or body focuses that window and the mouse
wheel scrolls the window under the pointer. Only the focused window draws the
caret, cursor line and fuzzy panel; unfocused tab strips are dimmed.

## Insert mode

| Key | Action |
| --- | --- |
| printable keys | insert text |
| `Enter` | newline (auto-indent continues) |
| `Backspace` `Delete` | delete backward / forward |
| arrow keys | move the caret, across line ends |
| `Tab` | insert spaces up to the next `tab_width` stop |
| `Esc` `Ctrl-c` `Ctrl-[` | return to normal mode |

LSP completion popup (when shown): `Ctrl-n` / `Ctrl-p` or `Up` / `Down` navigate,
`Ctrl-y` / `Ctrl-e` / `Enter` / `Tab` accept, `Esc` / `Ctrl-c` dismiss, `Left` /
`Right` dismiss the popup and move the caret, typing filters by prefix.

## Visual mode

Motion keys work as in normal mode and extend the selection.

| Key | Action |
| --- | --- |
| `v` | toggle char-wise visual (inside block visual: switch to char-wise) |
| `V` | toggle line-wise visual |
| `Ctrl-v` | enter / leave block visual |
| `Esc` | clear pending, leave visual |
| `d` `c` `y` `x` `p` | operators (block-aware in block visual) |
| `<` `>` | outdent / indent lines |
| `i`/`a` + object | select text object |

Block visual yanks a rectangle (short lines padded with spaces); block delete
and change apply the rectangle on every selected line; block paste inserts one
rectangle segment per line, padding short lines.

## Command line (`:`)

| Command | Action |
| --- | --- |
| `:w` | write (save) file (refused if the file changed on disk) |
| `:w!` | same, even if the file changed on disk since it was read |
| `:q` | close the focused window; quit when it is the last one (fails if a buffer is dirty) |
| `:q!` | same, discarding changes |
| `:wq` / `:x` | write and quit (fails if another buffer is dirty) |
| `:wq!` | write, discard other dirty buffers, quit |
| `:e {path}` | open file (or create a blank buffer) as a tab in the focused window |
| `:split` / `:sp` | split the focused window into left and right |
| `:vsplit` / `:vs` | split the focused window into left and right |
| `:hsplit` / `:hs` | split the focused window into top and bottom |
| `:590` | jump to line 590 |
| `:bn` / `:bnext` | next buffer |
| `:bp` / `:bprev` | previous buffer |
| `:c` | close current buffer |
| `:c 4` | close tab 4 of the focused window |
| `:c!` | close, discarding changes |
| `:pwd` | show current directory |
| `:pwd {path}` | change directory |
| `:r{pat}/{repl}` | find & replace every match of regex `pat` (`:rfoo/bar`) |
| `:settings` | open the settings panel |

The three split commands are refused, with `Only one split is allowed`, while a
split is already open.

`Esc` or `Ctrl-c` aborts; `Enter` executes. `~` is expanded in paths.

`Left` / `Right` move the caret inside the command line: typing inserts at the
caret, `Backspace` deletes before it and `Delete` at it. Search input (`/`, `?`)
edits the same way; `Up` / `Down` are taken by the completion popup and do
nothing in search.

`:e`, `:edit` and `:pwd` complete paths. `Tab` lists the entries of the typed
directory in a popup next to the command line and inserts the top match; further
`Tab` presses cycle through the list (`Shift-Tab` backwards), arrow keys step
without wrapping, and completing a directory lists its contents. Typing or
deleting re-filters the open list, `Enter` runs the command as typed, and `Esc`
dismisses the list before it aborts the command line. Hidden files are not
listed.

Closing the window with unsaved changes warns first; closing again
discards them.

Search input (`/`, `?`) works the same way: type a literal pattern, `Enter`
to jump, `Esc`/`Ctrl-c` to cancel.

## Fuzzy panel

Opened by `Ctrl-p` (files), `Ctrl-b` (buffers) or `:settings`.

| Key | Action |
| --- | --- |
| printable keys / `Space` | narrow the list (fuzzy match) |
| `Backspace` | delete query character |
| `Up` `Down` `Ctrl-n` | move selection |
| `Enter` | accept (open file/buffer, or change setting) |
| `Esc` `Ctrl-c` `Ctrl-]` | close |

The file panel searches sub-directories recursively: entries are paths relative
to the search root, shallowest first, capped at 20 000 entries, and long paths
are elided from the left so the file name stays visible. The text after the last
`/` fuzzy-matches those paths, so `clent` finds `src/lsp/client.rs`. Typing
`dir/` makes that directory the search root (accepting a `dir/` entry does the
same), `../` moves the root up, `~/` starts at the home directory, and `/path/`
makes an absolute path the root.

Files and directories that git ignores (`.gitignore`, `.git/info/exclude` and the
configured global ignore file) are left out of the list; `F5` toggles them back
in. Ignored directories are never walked. Path completion for `:e` and `:pwd` is
not filtered and lists everything.

## Git

Files in a working tree show change signs in the gutter, diffed with libgit2
against the last commit: green `+` for added lines, yellow `~` for modified
ones, and red `-` on the line below a deletion. Signs recompute when you leave
insert mode; set `git_signs: false` to drop the column entirely.

`F4` adds a `date author` column on the left showing who last changed each
line. The blame runs on the buffer's own text, so lines keep their author while
the file has unsaved edits; lines that are in no commit stay blank.

## Global

| Key | Action |
| --- | --- |
| `F1` | toggle spellcheck |
| `F2` | toggle whitespace display |
| `F3` | toggle dark / light theme |
| `F4` | show / hide git blame |
| `F5` | show / hide git-ignored files in the file panel |

Mouse: wheel scrolls, left-click switches tabs, right-click opens a context
menu of relevant keys for the word under the pointer (e.g. `gd`, `K`, `viw`).

## Configuration

Config file: `~/.config/deditor/config.yaml`
(`%LOCALAPPDATA%\deditor\config.yaml` on Windows), override location with
`DEDITOR_CONFIG=/path/to/file`. See [config.example.yaml](config.example.yaml)
for every key (font, tab width, line numbers, clipboard sync, whitespace
display, spellcheck, git signs, and a full color theme). `:settings` edits these
live and saves on change. Spellcheck (`F1`) underlines words the built-in English
dictionary does not know — no external program or dictionary package is needed.
Point `spell_dict` at a personal word list to whitelist project names and
jargon.

The default font is `MesloLGMDZ Nerd Font Mono`. `:settings` cycles
`font.family` through a short list; a family fontconfig cannot find falls back
to the system monospace face.

Text sits on a fixed cell grid and a wide character (CJK, Hangul, most emoji)
occupies two cells. Whether it actually fills both depends on the fallback font
the shaper picks for it: cosmic-text falls back to `Noto Sans CJK`, whose
advance is about 1.6 cells, so alias that family to `Noto Sans Mono CJK` in
fontconfig to get exact alignment.

## Word list licence

The dictionary is generated from the SCOWL `en_US` Hunspell dictionary
(release 2026.02.25, <https://wordlist.aspell.net/>), embedded in the binary.
It is distributed under the MIT-like licence reproduced in
[data/wordlist/LICENSE](data/wordlist/LICENSE):

> Copyright 2000-2026 by Kevin Atkinson
>
> Permission to use, copy, modify, distribute, and sell any part of SCOWLv2, or
> word lists created from it, is hereby granted without fee, provided that the
> above copyright notice appears in all copies and that both the above copyright
> notice and this notice appear in supporting documentation. Kevin Atkinson
> makes no representations about the suitability of this database for any
> purpose. It is provided "as is" without express or implied warranty.
