# 🔐 git-lock: Git Commit Locking System

git-lock is a lightweight commit-locking utility for Git repositories. It allows you to "lock" the current commit hash under a named label, restore it later, list and compare saved locks, and even tag them. This helps developers maintain checkpoints during complex feature development or hotfix workflows.

---

## 📦 Use Cases

- Save a known-good commit before refactoring (`git-lock lock refactor-start`)
- Lock a hotfix base before applying changes (`git-lock lock hotfix-base`)
- Tag commits after QA review using saved labels (`git-lock tag qa-passed v1.1.2`)
- Roll back instantly to a locked commit (`git-lock unlock refactor-start`)
- View or diff commit checkpoints for auditing

---

## 🛠 Commands

### `git-lock lock <label> [note] [commit]`

Locks the current commit (or a specific one) under a label.

```bash
git-lock lock dev-start "before breaking change"
git-lock lock release-candidate "for QA team" HEAD~1
```

---

### `git-lock unlock <label>`

Restores the commit saved under the given label via `git reset --hard`. Prompts before action.

```bash
git-lock unlock dev-start
```

---

### `git-lock list`

Lists all saved git-locks for the current repo, including:

- Label name
- Commit hash
- Note (if any)
- Commit subject
- Timestamp
- Marks latest label with ⭐

---

### `git-lock copy <src-label> <dst-label>`

Duplicates a saved git-lock (useful for branching or preserving milestones).

```bash
git-lock copy qa-ready staging-review
```

---

### `git-lock delete <label>`

Deletes a saved git-lock. Prompts before removal. Also cleans up latest marker if applicable.

```bash
git-lock delete dev-start
```

---

### `git-lock diff <label1> <label2>`

Compares two saved git-locks by showing commits between them using `git log`.

```bash
git-lock diff alpha beta
```

---

### `git-lock tag <label> <tag-name> [-m <msg>] [--push]`

Creates a Git tag from a saved git-lock. Optionally pushes it to origin and deletes it locally.

```bash
git-lock tag rc-1 v1.2.0 -m "Release Candidate 1" --push
```

---

## 🧱 Implementation Notes

- All lock files are stored under: `$ZSH_CACHE_DIR/git-locks`
- File format:
  - Line 1: `commit-hash # optional note`
  - Line 2: `timestamp=YYYY-MM-DD HH:MM:SS`
- `git-lock unlock` and `git-lock tag` read from these files
- `git-lock list` sorts using timestamps for recent-first ordering
- `basename` of `git rev-parse --show-toplevel` is used to isolate per-repo locking

---

## 📤 Output Preview

A sample `git-lock list` might look like:

```text
🔐 git-lock list for [my-repo]:

 - 🏷️  tag:    dev-start  ⭐ (latest)
   🧬 commit:  5a1f9e3
   📄 message: Init core structure
   📝 note:    before breaking change
   📅 time:    2025-06-06 13:45:12

 - 🏷️  tag:    release
   🧬 commit:  d0e4ca2
   📄 message: Merge pull request #12 from release
   📅 time:    2025-06-05 18:12:00
```

---

## 🧼 Cleanup Tip

To clear all git-locks in current repo:

```bash
rm $ZSH_CACHE_DIR$/git-locks/$(basename `git rev-parse --show-toplevel`)*.lock
```
