# TODO
- Fuzzy search with ctrl-p should include sub directories recursively. Currently, the user needs to select the directory for it's items to be searchable. The user should still be able to select a sub-directory, and that will still filter the results. They should also still be able to change the pwd source directory by typing with ctrl-p.
- git ignored files won't show in ctrl-p unless a toggle f5 is pressed. This doesn't apply to :e or :pwd files or paths. Don't call git commands for git features. If needed use something like libgit2 or its rust binding.
- Arrow keys should work in all modes for cursor navigation.
- Add :split :vsplit :hsplit to split the view. :split and :vsplit will virtically split the viewer into left and right. :hsplit horizontal split. Swap windows with ctrl-w w like neovim. Show separate tab views in each split.
- Use tokyo night theme as default. You can reference ~/dotfiles/kitty/current-theme.conf for the color values.
- Show git changes on the side. green +, red -, yellow ~ for add remove modify. Don't call git commands for git features. If needed use something like libgit2 or its rust binding.
- Toggle git blame with f4. Show date/user on the left side. Don't call git commands for git features. If needed use something like libgit2 or its rust binding.
- If you complete all other items, do code review on the codebase and fix tech debt. Don't delete this item.
