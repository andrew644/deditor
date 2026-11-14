# TODO
- find with / should show total number of matches and which one we are currently on e.g. 4/25 this is the 4th match out of 25 matches.
- find should act like ignorecase and smartcase are enabled in neovim.
- Indenting should work like autoindent is on and smartindent is on.
- Act like scrolloff = 8 in neovim.
- Show a cursorline (highlight the current line slightly)
- When a line is long it goes off the screen. Instead, it should wrap around. Navigation up and down should act as though the wrapped line is a normal line. This setting in neovim:
keymap("n", "j", "gj", opts)
keymap("n", "k", "gk", opts)
- Yanked lines should be copied to the clipboard. Pasted lines with p should paste from the clipboard. (when starting the app and hitting p, the clipboard should be pasted.)

