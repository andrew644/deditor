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

## Insert mode

| Key | Action |
| --- | --- |
| printable keys | insert text |
| `Enter` | newline (auto-indent continues) |
| `Backspace` `Delete` | delete backward / forward |
| `Tab` | insert spaces up to the next `tab_width` stop |
| `Esc` `Ctrl-c` `Ctrl-[` | return to normal mode |

LSP completion popup (when shown): `Ctrl-n` / `Ctrl-p` navigate,
`Ctrl-y` / `Ctrl-e` / `Enter` / `Tab` accept, `Esc` / `Ctrl-c` dismiss,
typing filters by prefix.

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
| `:w` | write (save) file |
| `:q` | quit (fails if buffer is dirty) |
| `:q!` | quit discarding changes |
| `:wq` / `:x` | write and quit |
| `:wq!` | write and quit |
| `:e {path}` | open file (or create a blank buffer) in a new tab |
| `:590` | jump to line 590 |
| `:bn` / `:bnext` | next buffer |
| `:bp` / `:bprev` | previous buffer |
| `:c` | close current buffer |
| `:c 4` | close buffer 4 |
| `:c!` | close, discarding changes |
| `:pwd` | show current directory |
| `:pwd {path}` | change directory |
| `:r{pat}/{repl}` | find & replace every match of regex `pat` (`:rfoo/bar`) |
| `:settings` | open the settings panel |

`Esc` or `Ctrl-c` aborts; `Enter` executes. `~` is expanded in paths.

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

In the file panel, typing `dir/` browses into that directory; `~/` starts at
the home directory.

## Global

| Key | Action |
| --- | --- |
| `F1` | toggle spellcheck |
| `F2` | toggle whitespace display |
| `F3` | toggle dark / light theme |

Mouse: wheel scrolls, left-click switches tabs, right-click opens a context
menu of relevant keys for the word under the pointer (e.g. `gd`, `K`, `viw`).

## Configuration

Config file: `~/.config/deditor/config.yaml`
(`%LOCALAPPDATA%\deditor\config.yaml` on Windows), override location with
`DEDITOR_CONFIG=/path/to/file`. See [config.example.yaml](config.example.yaml)
for every key (font, tab width, line numbers, clipboard sync, whitespace
display, spellcheck, and a full color theme). `:settings` edits these live and
saves on change. Spellcheck (`F1`) underlines words `hunspell` does not know;
it needs the `hunspell` binary and a dictionary (e.g. `hunspell_en_us`), with
an optional `spell_dict` path. Without them it stays silent.
