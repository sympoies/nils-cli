# Completions & wrappers strategy

## Decision
Keep zsh completion and wrapper assets in this repo under `completions/zsh/` and `wrappers/`.

## Rationale
- Keeps shell UX assets versioned alongside the Rust CLIs they accompany.
- Makes local setup reproducible without hopping between repos.
- Enables future automation to generate and update completions in one place.

## Location
- `completions/zsh/`: zsh completion files (generated or curated)
  - `completions/zsh/_git-scope`: completion for `git-scope` (and alias `gs`)
  - `completions/zsh/_git-summary`: completion for `git-summary`
- `wrappers/`: wrapper scripts for invoking CLI binaries or enforcing env setup

## Integration Steps
1. Add `wrappers/` to your PATH (or symlink wrapper scripts into a bin directory).
2. Add `completions/zsh/` to your `fpath`, then run `compinit` in your shell init.
3. Regenerate completions when CLIs change, and commit updates alongside code.
