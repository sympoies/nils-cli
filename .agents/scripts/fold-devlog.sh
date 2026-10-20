#!/usr/bin/env bash
# Prepare only: the maintainer delivers these files through a reviewed PR.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
branch="$(git branch --show-current)"
if [[ ! "$branch" =~ ^chore/devlog-fold-[a-z0-9][a-z0-9-]*$ ]]; then
  echo 'error: use a managed chore/devlog-fold-<date> worktree' >&2
  exit 2
fi
if [[ -n "$(git status --porcelain=v1 --untracked-files=all)" ]]; then
  echo 'error: fold preparation requires a clean worktree' >&2
  exit 2
fi
source "$repo_root/scripts/dev-env.sh"
# Use the same cutoff on contributor machines and CI runners.
export TZ=UTC
devlog check --base HEAD
devlog fold
devlog check --base HEAD
printf '%s\n' 'Review the diff, commit as chore(devlog): fold accepted fragments, then deliver through the normal PR checks and review.'
