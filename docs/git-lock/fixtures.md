# git-lock fixtures

## Lock default label + HEAD
- Setup:
  - Init temp repo with at least one commit.
  - Set `ZSH_CACHE_DIR` to a temp dir.
- Command:
  - `git-lock lock`
- Expected markers:
  - `🔐 [<repo>:default] Locked:`
  - `at YYYY-MM-DD HH:MM:SS`
  - Lock file created at `$ZSH_CACHE_DIR/git-locks/<repo>-default.lock`.

## Lock with explicit label + note + commit
- Setup:
  - Two commits in repo; capture `HEAD~1` hash.
- Command:
  - `git-lock lock wip "before refactor" HEAD~1`
- Expected markers:
  - Output includes `# before refactor`.
  - Lock file first line includes `<hash> # before refactor`.

## Unlock uses latest marker
- Setup:
  - Create two locks; ensure `<repo>-latest` points to the second label.
- Command:
  - `git-lock unlock` then respond `n` to prompt.
- Expected markers:
  - `🔐 Found [<repo>:<latest>] → <hash>`
  - `⚠️  Hard reset to [<label>]? [y/N]`
  - `🚫 Aborted`.

## Unlock missing latest
- Setup:
  - Ensure no `<repo>-latest` file.
- Command:
  - `git-lock unlock`
- Expected markers:
  - `❌ No recent git-lock found for <repo>`

## List ordering + latest marker
- Setup:
  - Create two locks with distinct timestamps (or adjust file timestamps).
- Command:
  - `git-lock list`
- Expected markers:
  - Newest lock appears first.
  - Latest label shows `⭐ (latest)`.

## Copy label overwrite prompt
- Setup:
  - Existing labels `a` and `b`.
- Command:
  - `git-lock copy a b` then respond `n`.
- Expected markers:
  - `⚠️  Target git-lock [<repo>:b] already exists. Overwrite? [y/N]`
  - `🚫 Aborted`.

## Delete latest cleanup
- Setup:
  - Latest marker points to label `wip`.
- Command:
  - `git-lock delete wip` then respond `y`.
- Expected markers:
  - `🗑️  Deleted git-lock [<repo>:wip]`
  - `🧼 Removed latest marker`.

## Diff no-color
- Setup:
  - Two labels pointing at different commits.
- Command:
  - `git-lock diff a b --no-color`
- Expected markers:
  - `🧮 Comparing commits: [<repo>:a] → [b]`
  - `git log` output without ANSI color sequences.

## Tag overwrite prompt
- Setup:
  - Existing tag `v1.0.0`.
- Command:
  - `git-lock tag <label> v1.0.0` then respond `n`.
- Expected markers:
  - `⚠️  Git tag [v1.0.0] already exists.`
  - `❓ Overwrite it? [y/N] `
  - `🚫 Aborted`.

## Not a git repo
- Setup:
  - Run in a temp directory without `.git`.
- Command:
  - `git-lock list`
- Expected markers:
  - `❗ Not a Git repository. Run this command inside a Git project.`
