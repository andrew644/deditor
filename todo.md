# TODO
- Indenting should work like autoindent is on and smartindent is on.
- When a line is long it goes off the screen. Instead, it should wrap around. Navigation up and down should act as though the wrapped line is a normal line. This setting in neovim:
keymap("n", "j", "gj", opts)
keymap("n", "k", "gk", opts)
- Yanked lines should be copied to the clipboard. Pasted lines with p should paste from the clipboard. (when starting the app and hitting p, the clipboard should be pasted.)

