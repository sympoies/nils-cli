# ────────────────────────────────────────────────────────
# Git lock / unlock helpers (manual commit fallback, repo-safe)
# ────────────────────────────────────────────────────────


# ────────────────────────────────────────────────────────
# Aliases and Unalias
# ────────────────────────────────────────────────────────
if command -v safe_unalias >/dev/null; then
  safe_unalias _git_lock
fi

typeset -gr GIT_LOCK_TIMESTAMP_PATTERN='^timestamp='
typeset -gr GIT_LOCK_ABORTED_MSG='🚫 Aborted'

# _git_lock_confirm <prompt> [printf_args...]
# Prompt for y/N confirmation (returns 0 only on "y"/"Y").
# Usage: _git_lock_confirm <prompt> [printf_args...]
_git_lock_confirm() {
  emulate -L zsh

  typeset prompt="${1-}"
  [[ -n "$prompt" ]] || return 1
  shift || true

  printf "$prompt" "$@"

  typeset confirm=''
  IFS= read -r confirm
  if [[ "$confirm" != [yY] ]]; then
    printf "%s\n" "$GIT_LOCK_ABORTED_MSG"
    return 1
  fi

  return 0
}

# _git_lock_resolve_label [label]
# Resolve a git-lock label (explicit label or per-repo "latest").
# Usage: _git_lock_resolve_label [label]
# Output:
# - Prints the resolved label to stdout.
_git_lock_resolve_label() {
  typeset input_label="$1"
  typeset repo_id='' lock_dir='' latest_file=''

  repo_id=$(basename "$(git rev-parse --show-toplevel 2>/dev/null)")
  lock_dir="$ZSH_CACHE_DIR/git-locks"
  [[ -d "$lock_dir" ]] || mkdir -p "$lock_dir"
  latest_file="$lock_dir/${repo_id}-latest"

  if [[ -n "$input_label" ]]; then
    printf "%s\n" "$input_label"
  elif [[ -f "$latest_file" ]]; then
    cat "$latest_file"
  else
    return 1
  fi
}


# _git_lock [label] [note] [commit]
# Save a git-lock label pointing at a commit (writes a per-repo lock file).
# Usage: _git_lock [label] [note] [commit]
# Notes:
# - Defaults: label=default, commit=HEAD.
# - Stores lock files under `$ZSH_CACHE_DIR/git-locks/<repo>-<label>.lock`.
_git_lock() {
  typeset label='' note='' commit='' repo_id='' lock_dir='' lock_file='' latest_file='' timestamp='' hash=''

  typeset label_arg="${1-}"
  typeset note_arg="${2-}"
  typeset commit_arg="${3-}"

  label="${label_arg:-default}"
  note="$note_arg"
  commit="${commit_arg:-HEAD}"

  repo_id=$(basename "$(git rev-parse --show-toplevel 2>/dev/null)")
  lock_dir="$ZSH_CACHE_DIR/git-locks"
  lock_file="$lock_dir/${repo_id}-${label}.lock"
  latest_file="$lock_dir/${repo_id}-latest"
  timestamp=$(date "+%Y-%m-%d %H:%M:%S")

  hash=$(git rev-parse "$commit" 2>/dev/null) || {
    printf "❌ Invalid commit: %s\n" "$commit"
    return 1
  }

  [[ -d "$lock_dir" ]] || mkdir -p "$lock_dir"

  {
    printf "%s # %s\n" "$hash" "$note"
    printf "timestamp=%s\n" "$timestamp"
  } > "$lock_file"

  printf "%s\n" "$label" > "$latest_file"

  printf "🔐 [%s:%s] Locked: %s" "$repo_id" "$label" "$hash"
  [[ -n "$note" ]] && printf "  # %s" "$note"
  printf "\n"
  printf "    at %s\n" "$timestamp"
}

# _git_lock_unlock [label]
# Hard reset the current repo to the commit recorded by a git-lock label (DANGEROUS).
# Usage: _git_lock_unlock [label]
# Notes:
# - When label is omitted, it uses the per-repo "latest" label if present.
# Safety:
# - Runs `git reset --hard <hash>` which discards tracked changes.
_git_lock_unlock() {
  typeset label='' repo_id='' lock_dir='' lock_file='' latest_file=''
  typeset label_arg="${1-}"
  repo_id=$(basename "$(git rev-parse --show-toplevel 2>/dev/null)")
  lock_dir="$ZSH_CACHE_DIR/git-locks"
  latest_file="$lock_dir/${repo_id}-latest"

  [[ -d "$lock_dir" ]] || mkdir -p "$lock_dir"

  if [[ -n "$label_arg" ]]; then
    label="$label_arg"
  elif [[ -f "$latest_file" ]]; then
    label=$(cat "$latest_file")
  else
    printf "❌ No recent git-lock found for %s\n" "$repo_id"
    return 1
  fi

  lock_file="$lock_dir/${repo_id}-${label}.lock"
  if [[ ! -f "$lock_file" ]]; then
    printf "❌ No git-lock named '%s' found for %s\n" "$label" "$repo_id"
    return 1
  fi

  typeset hash='' note='' msg=''
  read -r line < "$lock_file"
  hash=$(print -r -- "$line" | cut -d '#' -f 1 | xargs)
  note=$(print -r -- "$line" | cut -d '#' -f 2- | xargs)
  msg=$(git log -1 --pretty=format:"%s" "$hash" 2>/dev/null)

  printf "🔐 Found [%s:%s] → %s\n" "$repo_id" "$label" "$hash"
  [[ -n "$note" ]] && printf "    # %s\n" "$note"
  [[ -n "$msg" ]] && printf "    commit message: %s\n" "$msg"
  printf "\n"

  _git_lock_confirm "⚠️  Hard reset to [%s]? [y/N] " "$label" || return 1

  git reset --hard "$hash"
  printf "⏪ [%s:%s] Reset to: %s\n" "$repo_id" "$label" "$hash"
}


# _git_lock_list
# List git-locks for the current repository.
# Usage: _git_lock_list
# Notes:
# - Shows label, commit hash, note, timestamp, and commit subject.
# - Marks the per-repo "latest" label.
_git_lock_list() {
  typeset repo_id='' lock_dir='' latest=''
  repo_id=$(basename "$(git rev-parse --show-toplevel 2>/dev/null)")
  lock_dir="$ZSH_CACHE_DIR/git-locks"

  [[ -d "$lock_dir" ]] || {
    printf "📬 No git-locks found for [%s]\n" "$repo_id"
    return 0
  }

  [[ -f "$lock_dir/${repo_id}-latest" ]] && latest=$(cat "$lock_dir/${repo_id}-latest")

  typeset file='' tmp_list=()
  for file in "$lock_dir/${repo_id}-"*.lock; do
    [[ -e "$file" && "$(basename "$file")" != "${repo_id}-latest.lock" ]] || continue
    typeset ts_line='' timestamp='' epoch=''
    ts_line=$(grep "$GIT_LOCK_TIMESTAMP_PATTERN" "$file" 2>/dev/null || true)
    timestamp=${ts_line#timestamp=}
    epoch="$(
      date -j -f "%Y-%m-%d %H:%M:%S" "$timestamp" "+%s" 2>/dev/null \
        || date -d "$timestamp" "+%s" 2>/dev/null
    )"
    [[ -n "$epoch" ]] || epoch=0
    tmp_list+=("$epoch|$file")
  done

  IFS=$'\n' sorted=($(printf '%s\n' "${tmp_list[@]}" | sort -rn))

  if [[ ${#sorted[@]} -eq 0 ]]; then
    printf "📬 No git-locks found for [%s]\n" "$repo_id"
    return 0
  fi

  printf "🔐 git-lock list for [%s]:\n" "$repo_id"
  for item in "${sorted[@]}"; do
    file="${item#*|}"
    typeset name='' hash='' note='' timestamp='' label='' subject='' line=''
    name=$(basename "$file" .lock)
    label=${name#${repo_id}-}
    read -r line < "$file"
    hash=$(print -r -- "$line" | cut -d '#' -f1 | xargs)
    note=$(print -r -- "$line" | cut -d '#' -f2- | xargs)
    timestamp=$(grep "$GIT_LOCK_TIMESTAMP_PATTERN" "$file" | cut -d '=' -f2-)
    subject=$(git log -1 --pretty=%s "$hash" 2>/dev/null)

    printf "\n - 🏷️  tag:     %s%s\n" "$label" \
      "$( [[ "$label" == "$latest" ]] && print '  ⭐ (latest)' )"
    printf "   🧬 commit:  %s\n" "$hash"
    [[ -n "$subject" ]] && printf "   📄 message: %s\n" "$subject"
    [[ -n "$note" ]] && printf "   📝 note:    %s\n" "$note"
    [[ -n "$timestamp" ]] && printf "   📅 time:    %s\n" "$timestamp"
  done
}

# _git_lock_copy <from-label> <to-label>
# Copy a git-lock label (preserves hash/note/timestamp metadata).
# Usage: _git_lock_copy <from-label> <to-label>
# Notes:
# - Prompts before overwriting the target label if it exists.
# - Sets the copied label as the per-repo "latest".
_git_lock_copy() {
  typeset repo_id='' lock_dir='' src_label='' dst_label='' src_file='' dst_file=''
  typeset src_label_arg="${1-}"
  typeset dst_label_arg="${2-}"
  repo_id=$(basename "$(git rev-parse --show-toplevel 2>/dev/null)")
  lock_dir="$ZSH_CACHE_DIR/git-locks"

  [[ -d "$lock_dir" ]] || {
    printf "❌ No git-locks found\n"
    return 1
  }

  src_label=$(_git_lock_resolve_label "$src_label_arg") || {
    printf "❗ Usage: git-lock-copy <source-label> <target-label>\n"
    return 1
  }
  dst_label="$dst_label_arg"
  [[ -z "$dst_label" ]] && {
    printf "❗ Target label is missing\n"
    return 1
  }

  src_file="$lock_dir/${repo_id}-${src_label}.lock"
  dst_file="$lock_dir/${repo_id}-${dst_label}.lock"

  if [[ ! -f "$src_file" ]]; then
    printf "❌ Source git-lock [%s:%s] not found\n" "$repo_id" "$src_label"
    return 1
  fi

  if [[ -f "$dst_file" ]]; then
    _git_lock_confirm "⚠️  Target git-lock [%s:%s] already exists. Overwrite? [y/N] " "$repo_id" "$dst_label" || return 1
  fi

  cp "$src_file" "$dst_file"
  printf "%s\n" "$dst_label" > "$lock_dir/${repo_id}-latest"

  typeset content='' hash='' note='' timestamp='' subject=''
  content=$(<"$src_file")
  hash=$(print -r -- "$content" | sed -n '1p' | cut -d '#' -f1 | xargs)
  note=$(print -r -- "$content" | sed -n '1p' | cut -d '#' -f2- | xargs)
  timestamp=$(print -r -- "$content" | grep "$GIT_LOCK_TIMESTAMP_PATTERN" | cut -d '=' -f2-)
  subject=$(git log -1 --pretty=%s "$hash" 2>/dev/null)

  printf "📋 Copied git-lock [%s:%s] → [%s:%s]\n" "$repo_id" "$src_label" "$repo_id" "$dst_label"
  printf "   🏷️  tag:     %s → %s\n" "$src_label" "$dst_label"
  printf "   🧬 commit:  %s\n" "$hash"
  [[ -n "$subject" ]] && printf "   📄 message: %s\n" "$subject"
  [[ -n "$note" ]] && printf "   📝 note:    %s\n" "$note"
  [[ -n "$timestamp" ]] && printf "   📅 time:    %s\n" "$timestamp"
}

# _git_lock_delete [label]
# Delete a git-lock label for the current repository.
# Usage: _git_lock_delete [label]
# Notes:
# - When label is omitted, it uses the per-repo "latest" label if present.
# - Removes the "latest" marker if you delete the latest label.
_git_lock_delete() {
  typeset repo_id='' lock_dir='' label='' lock_file='' latest_file='' latest_label=''
  typeset label_arg="${1-}"
  repo_id=$(basename "$(git rev-parse --show-toplevel 2>/dev/null)")
  lock_dir="$ZSH_CACHE_DIR/git-locks"
  latest_file="$lock_dir/${repo_id}-latest"

  [[ -d "$lock_dir" ]] || {
    printf "❌ No git-locks found\n"
    return 1
  }

  label=$(_git_lock_resolve_label "$label_arg") || {
    printf "❌ No label provided and no latest git-lock exists\n"
    return 1
  }

  lock_file="$lock_dir/${repo_id}-${label}.lock"
  if [[ ! -f "$lock_file" ]]; then
    printf "❌ git-lock [%s] not found\n" "$label"
    return 1
  fi

  typeset content='' hash='' note='' timestamp='' subject=''
  content=$(<"$lock_file")
  hash=$(print -r -- "$content" | sed -n '1p' | cut -d '#' -f1 | xargs)
  note=$(print -r -- "$content" | sed -n '1p' | cut -d '#' -f2- | xargs)
  timestamp=$(print -r -- "$content" | grep "$GIT_LOCK_TIMESTAMP_PATTERN" | cut -d '=' -f2-)
  subject=$(git log -1 --pretty=%s "$hash" 2>/dev/null)

  printf "🗑️  Candidate for deletion:\n"
  printf "   🏷️  tag:     %s\n" "$label"
  printf "   🧬 commit:  %s\n" "$hash"
  [[ -n "$subject" ]] && printf "   📄 message: %s\n" "$subject"
  [[ -n "$note" ]] && printf "   📝 note:    %s\n" "$note"
  [[ -n "$timestamp" ]] && printf "   📅 time:    %s\n" "$timestamp"
  printf "\n"

  _git_lock_confirm "⚠️  Delete this git-lock? [y/N] " || return 1

  rm -f "$lock_file"
  printf "🗑️  Deleted git-lock [%s:%s]\n" "$repo_id" "$label"

  if [[ -f "$latest_file" ]]; then
    latest_label=$(<"$latest_file")
    if [[ "$label" == "$latest_label" ]]; then
      rm -f "$latest_file"
      printf "🧼 Removed latest marker (was [%s])\n" "$label"
    fi
  fi
}

# _git_lock_diff <label1> <label2> [--no-color]
# Show commit log between two git-lock labels.
# Usage: _git_lock_diff <label1> <label2> [--no-color]
# Notes:
# - Runs: `git log <hash1>..<hash2>`.
_git_lock_diff() {
  emulate -L zsh
  setopt pipe_fail

  typeset repo_id='' lock_dir='' label1='' label2='' file1='' file2='' hash1='' hash2=''
  typeset no_color=false
  typeset -a positional=()

  while [[ $# -gt 0 ]]; do
    typeset arg="${1-}"
    case "$arg" in
      --no-color|no-color)
        no_color=true
        ;;
      --help|-h)
        printf "❗ Usage: git-lock diff <label1> <label2> [--no-color]\n"
        return 0
        ;;
      *)
        positional+=("$arg")
        ;;
    esac
    shift
  done

  if (( ${#positional[@]} > 2 )); then
    printf "❗ Too many labels provided (expected 2)\n"
    printf "❗ Usage: git-lock diff <label1> <label2> [--no-color]\n"
    return 1
  fi

  label1=$(_git_lock_resolve_label "${positional[1]}") || {
    printf "❗ Usage: git-lock diff <label1> <label2> [--no-color]\n"
    return 1
  }
  label2=$(_git_lock_resolve_label "${positional[2]}") || {
    printf "❗ Second label not provided or found\n"
    return 1
  }

  repo_id=$(basename "$(git rev-parse --show-toplevel 2>/dev/null)")
  lock_dir="$ZSH_CACHE_DIR/git-locks"
  file1="$lock_dir/${repo_id}-${label1}.lock"
  file2="$lock_dir/${repo_id}-${label2}.lock"

  [[ -f "$file1" ]] || {
    printf "❌ git-lock [%s] not found for [%s]\n" "$label1" "$repo_id"
    return 1
  }
  [[ -f "$file2" ]] || {
    printf "❌ git-lock [%s] not found for [%s]\n" "$label2" "$repo_id"
    return 1
  }

  hash1=$(sed -n '1p' "$file1" | cut -d '#' -f1 | xargs)
  hash2=$(sed -n '1p' "$file2" | cut -d '#' -f1 | xargs)

  printf "🧮 Comparing commits: [%s:%s] → [%s]\n" "$repo_id" "$label1" "$label2"
  printf "   🔖 %s: %s\n" "$label1" "$hash1"
  printf "   🔖 %s: %s\n" "$label2" "$hash2"
  printf "\n"

  typeset -a log_args=(--oneline --graph --decorate)
  if [[ "$no_color" == true || -n "${NO_COLOR-}" ]]; then
    log_args+=(--color=never)
  fi

  git log "${log_args[@]}" "$hash1..$hash2"
}


# _git_lock_tag <label> <tag-name> [-m <tag-message>] [--push]
# Create an annotated Git tag at the commit recorded by a git-lock label.
# Usage: _git_lock_tag <label> <tag-name> [-m <tag-message>] [--push]
# Notes:
# - Default tag message is the commit subject when `-m` is omitted.
# Safety:
# - `--push` publishes the tag to `origin` and deletes the local tag afterwards.
_git_lock_tag() {
  emulate -L zsh
  setopt pipe_fail

  typeset label='' tag_name='' tag_msg='' do_push=false
  typeset repo_id='' lock_dir='' lock_file='' hash='' timestamp='' line1=''
  typeset -a positional=()

  while [[ $# -gt 0 ]]; do
    typeset arg="${1-}"
    case "$arg" in
      --push)
        do_push=true
        shift ;;
      -m)
        shift
        typeset msg_arg="${1-}"
        tag_msg="$msg_arg"
        shift ;;
      *)
        positional+=("$arg")
        shift ;;
    esac
  done

  if (( ${#positional[@]} != 2 )); then
    printf "❗ Usage: git-lock tag <git-lock-label> <tag-name> [-m <tag-message>] [--push]\n"
    return 1
  fi

  label=$(_git_lock_resolve_label "${positional[1]-}") || {
    printf "❌ git-lock label not provided or not found\n"
    return 1
  }

  tag_name="${positional[2]-}"
  [[ -z "$tag_name" ]] && {
    printf "❗ Usage: git-lock tag <git-lock-label> <tag-name> [-m <tag-message>] [--push]\n"
    return 1
  }

  repo_id=$(basename "$(git rev-parse --show-toplevel 2>/dev/null)")
  lock_dir="$ZSH_CACHE_DIR/git-locks"
  lock_file="$lock_dir/${repo_id}-${label}.lock"

  [[ -f "$lock_file" ]] || {
    printf "❌ git-lock [%s] not found in [%s] for [%s]\n" "$label" "$lock_dir" "$repo_id"
    return 1
  }

  line1=$(sed -n '1p' "$lock_file")
  hash=$(cut -d '#' -f1 <<< "$line1" | xargs)
  timestamp=$(grep "$GIT_LOCK_TIMESTAMP_PATTERN" "$lock_file" | cut -d '=' -f2-)

  [[ -z "$tag_msg" ]] && tag_msg=$(git show -s --format=%s "$hash")

  if git rev-parse "$tag_name" >/dev/null 2>&1; then
    printf "⚠️  Git tag [%s] already exists.\n" "$tag_name"
    _git_lock_confirm "❓ Overwrite it? [y/N] " || return 1
    git tag -d "$tag_name" || {
      printf "❌ Failed to delete existing tag [%s]\n" "$tag_name"
      return 1
    }
  fi

  git tag -a "$tag_name" "$hash" -m "$tag_msg"
  printf "🏷️  Created tag [%s] at commit [%s]\n" "$tag_name" "$hash"
  printf "📝 Message: %s\n" "$tag_msg"

  if $do_push; then
    git push origin "$tag_name"
    printf "🚀 Pushed tag [%s] to origin\n" "$tag_name"
    git tag -d "$tag_name" && printf "🧹 Deleted local tag [%s]\n" "$tag_name"
  fi
}

# git-lock <command> [args...]
# Save/restore repo snapshots via per-repo lock labels.
# Usage: git-lock <lock|unlock|list|copy|delete|diff|tag> [args...]
# Notes:
# - Stores lock files under `$ZSH_CACHE_DIR/git-locks` (per repo).
# - Confirms before destructive operations (e.g., `unlock`, overwriting tags).
# Examples:
#   git-lock lock wip "before refactor"
#   git-lock unlock wip
git-lock() {
  if ! git rev-parse --git-dir > /dev/null 2>&1; then
    printf "❗ Not a Git repository. Run this command inside a Git project.\n"
    return 1
  fi

  typeset cmd="$1"
  if [[ -z "$cmd" || "$cmd" == "help" || "$cmd" == "--help" || "$cmd" == "-h" ]]; then
    printf "%s\n" "Usage: git-lock <command> [args]"
    printf "\n"
    printf "%s\n" "Commands:"
    printf "  %-16s  %s\n" \
      "lock [label] [note] [commit]"  "Save commit hash to lock" \
      "unlock [label]"                "Reset to a saved commit" \
      "list"                          "Show all locks for repo" \
      "copy <from> <to>"              "Duplicate a lock label" \
      "delete [label]"                "Remove a lock" \
      "diff <l1> <l2> [--no-color]"   "Compare commits between two locks" \
      "tag <label> <tag> [-m msg]"    "Create git tag from a lock"
    printf "\n"
    return 0
  fi

  shift

  case "$cmd" in
    lock)    _git_lock "$@" ;;
    unlock)  _git_lock_unlock "$@" ;;
    list)    _git_lock_list "$@" ;;
    copy)    _git_lock_copy "$@" ;;
    delete)  _git_lock_delete "$@" ;;
    diff)    _git_lock_diff "$@" ;;
    tag)     _git_lock_tag "$@" ;;
    *)
      printf "❗ Unknown command: '%s'\n" "$cmd"
      printf "Run 'git-lock help' for usage.\n"
      return 1 ;;
  esac
}
