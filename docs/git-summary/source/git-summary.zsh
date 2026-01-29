# ────────────────────────────────────────────────────────
# git-summary: author-based contribution report
# Usage: git-summary "2024-01-01" "2024-12-31"
# Supports macOS and Linux with timezone correction based on system settings
# ───────────────────────────────────────────────────────

typeset -r GIT_SUMMARY_OS_DARWIN='Darwin'
typeset -r GIT_SUMMARY_DATE_FMT='%Y-%m-%d'
typeset -g GIT_SUMMARY_DATE_HAS_V=false

if date -v +0d +"%Y-%m-%d" >/dev/null 2>&1; then
  GIT_SUMMARY_DATE_HAS_V=true
fi

# _git_summary_date [format]
# Resolve the current date using the given format; fallback to ISO if empty/invalid.
# Usage: _git_summary_date [format]
_git_summary_date() {
  typeset fmt="${1-}"
  typeset out=''

  if [[ -n "$fmt" ]]; then
    out=$(date +"$fmt" 2>/dev/null)
  fi

  if [[ -z "$out" ]]; then
    out=$(date +"%Y-%m-%d")
  fi

  print -r -- "$out"
  return 0
}

# _git_summary_require_git
# Ensure git is available and we're in a Git repo.
# Usage: _git_summary_require_git
_git_summary_require_git() {
  if ! command -v git >/dev/null 2>&1; then
    printf "❗ git is required but was not found in PATH.\n"
    return 1
  fi

  if ! git rev-parse --git-dir > /dev/null 2>&1; then
    printf "⚠️ Not a Git repository. Run this command inside a Git project.\n"
    return 1
  fi

  return 0
}

# _git_summary_validate_date <YYYY-MM-DD>
# Validate date format and value using BSD/GNU date.
# Usage: _git_summary_validate_date 2024-01-01
_git_summary_validate_date() {
  emulate -L zsh
  typeset input="${1-}"
  typeset fmt="${GIT_SUMMARY_DATE_FMT:-%Y-%m-%d}"
  typeset parsed=''

  if [[ -z "$input" ]]; then
    print "❌ Missing date value."
    return 1
  fi

  if [[ ! "$input" =~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}$' ]]; then
    print "❌ Invalid date format: $input (expected YYYY-MM-DD)."
    return 1
  fi

  if [[ "$GIT_SUMMARY_DATE_HAS_V" == true ]]; then
    parsed=$(date -j -f "$fmt" "$input" +"$fmt" 2>/dev/null) || parsed=''
  else
    parsed=$(date -d "$input" +"$fmt" 2>/dev/null) || parsed=''
  fi

  if [[ -z "$parsed" || "$parsed" != "$input" ]]; then
    print "❌ Invalid date value: $input."
    return 1
  fi

  return 0
}

# _git_summary_validate_range <since> <until>
# Validate that both dates are valid and since <= until.
# Usage: _git_summary_validate_range 2024-01-01 2024-01-31
_git_summary_validate_range() {
  emulate -L zsh
  typeset since="${1-}"
  typeset until="${2-}"

  _git_summary_validate_date "$since" || return 1
  _git_summary_validate_date "$until" || return 1

  if [[ "$since" > "$until" ]]; then
    print "❌ Start date must be on or before end date."
    return 1
  fi

  return 0
}

# ───────────────────────────────────────────────────────
# Aliases and Unalias
# ────────────────────────────────────────────────────────
if command -v safe_unalias >/dev/null; then
  safe_unalias _git_summary
fi

# _git_summary [since] [until]
# Generate a per-author contribution summary over a date range.
# Usage: _git_summary [YYYY-MM-DD] [YYYY-MM-DD]
# Notes:
# - Provide both dates or neither (full history).
# - Filters out common lockfiles from line counts.
_git_summary() {
  emulate -L zsh
  setopt pipe_fail

  _git_summary_require_git || return 1

  typeset since_param="$1"
  typeset until_param="$2"
  typeset log_args=()

  # Validate date parameters: either both empty (full history) or both provided
  if { [[ -n "$since_param" && -z "$until_param" ]] || [[ -z "$since_param" && -n "$until_param" ]] ; }; then
    print "❌ Please provide both start and end dates (YYYY-MM-DD)."
    return 1
  fi

  if [[ -z "$since_param" && -z "$until_param" ]]; then
    log_args=(--no-merges)
  else
    _git_summary_validate_range "$since_param" "$until_param" || return 1
    # Use local calendar boundaries with explicit timezone, so Git parses them in local time.
    typeset tz_raw="$(date +%z)" # e.g., +0800
    typeset since_bound="$since_param 00:00:00 $tz_raw"
    typeset until_bound="$until_param 23:59:59 $tz_raw"
    log_args+=(--since="$since_bound" --until="$until_bound" --no-merges)
  fi

  git log "${log_args[@]}" --pretty=format:"%an <%ae>" |
    sort | uniq | while read -r author; do
      email=$(print -r -- "$author" | grep -oE "<.*>" | tr -d "<>")
      name=$(print -r -- "$author" | sed -E "s/ <.*>//")
      short_email=$(printf "%.40s" "$email")

      log=$(git log "${log_args[@]}" --author="$email" --pretty=format:'%cd' --date=short --numstat)
      filtered=$(print -r -- "$log" | grep -vE '(yarn\.lock|package-lock\.json|pnpm-lock\.yaml|\.lock)$')

      added=$(print -r -- "$filtered" | awk 'NF==3 { add += $1 } END { print add+0 }')
      deleted=$(print -r -- "$filtered" | awk 'NF==3 { del += $2 } END { print del+0 }')
      commits=$(print -r -- "$log" | awk 'NF==1 { c++ } END { print c+0 }')
      first_commit=$(print -r -- "$log" | awk 'NF==1 { date=$1 } END { print date }')
      last_commit=$(print -r -- "$log" | awk 'NF==1 { print $1; exit }')

      printf "%-25s %-40s %8s %8s %8s %8s %12s %12s\n" \
        "$name" "$short_email" "$added" "$deleted" "$((added - deleted))" "$commits" "$first_commit" "$last_commit"
    done | sort -k5 -nr | awk '
      BEGIN {
        printf "%-25s %-40s %8s %8s %8s %8s %12s %12s\n", "Name", "Email", "Added", "Deleted", "Net", "Commits", "First", "Last"
        print  "----------------------------------------------------------------------------------------------------------------------------------------"
      }
      { print }
    '
  return $?
}

# _git_today
# Show a summary of today's commits (local timezone).
# Usage: _git_today
_git_today() {
  typeset today=$(_git_summary_date "$GIT_SUMMARY_DATE_FMT")
  print "\n📅 Git summary for today: $today"
  print
  _git_summary "$today" "$today"
  return $?
}

# _git_yesterday
# Show a summary of yesterday's commits (cross-platform).
# Usage: _git_yesterday
_git_yesterday() {
  typeset fmt="${GIT_SUMMARY_DATE_FMT:-%Y-%m-%d}"
  typeset yesterday=''
  if $GIT_SUMMARY_DATE_HAS_V; then
    yesterday=$(date -v -1d +"$fmt")
  else
    yesterday=$(date -d "yesterday" +"$fmt")
  fi
  print "\n📅 Git summary for yesterday: $yesterday"
  print
  _git_summary "$yesterday" "$yesterday"
  return $?
}

# _git_this_month
# Show a summary from the first day of the month to today.
# Usage: _git_this_month
_git_this_month() {
  typeset today=$(_git_summary_date "$GIT_SUMMARY_DATE_FMT")
  typeset start_date=$(date +"%Y-%m-01")
  print "\n📅 Git summary for this month: $start_date to $today"
  print
  _git_summary "$start_date" "$today"
  return $?
}

# _git_last_month
# Show a summary for the last full month.
# Usage: _git_last_month
_git_last_month() {
  typeset fmt="${GIT_SUMMARY_DATE_FMT:-%Y-%m-%d}"
  typeset start_date='' end_date=''

  if $GIT_SUMMARY_DATE_HAS_V; then
    start_date=$(date -j -v-1m -v1d +"$fmt")
    end_date=$(date -j -v1d -v-1d +"$fmt")
  else
    start_date=$(date -d "$(date +%Y-%m-01) -1 month" +"$fmt")
    end_date=$(date -d "$(date +%Y-%m-01) -1 day" +"$fmt")
  fi

  print "\n📅 Git summary for last month: $start_date to $end_date"
  print
  _git_summary "$start_date" "$end_date"
  return $?
}


# _git_last_week
# Show a summary for the last full week (Monday to Sunday).
# Usage: _git_last_week
_git_last_week() {
  typeset fmt="${GIT_SUMMARY_DATE_FMT:-%Y-%m-%d}"
  typeset CURRENT_DATE='' WEEKDAY='' START_DATE='' END_DATE=''
  CURRENT_DATE=$(date +"$fmt")

  if $GIT_SUMMARY_DATE_HAS_V; then
    WEEKDAY=$(date -j -f "$fmt" "$CURRENT_DATE" +%u)
    END_DATE=$(date -j -f "$fmt" -v -"$WEEKDAY"d "$CURRENT_DATE" +"$fmt")
    START_DATE=$(date -j -f "$fmt" -v -6d "$END_DATE" +"$fmt")
  else
    WEEKDAY=$(date -d "$CURRENT_DATE" +%u)
    END_DATE=$(date -d "$CURRENT_DATE -$WEEKDAY days" +"$fmt")
    START_DATE=$(date -d "$END_DATE -6 days" +"$fmt")
  fi

  print "\n📅 Git summary for last week: $START_DATE to $END_DATE"
  print
  _git_summary "$START_DATE" "$END_DATE"
  return $?
}

# _git_this_week
# Show a summary for this week (Monday to Sunday).
# Usage: _git_this_week
_git_this_week() {
  typeset fmt="${GIT_SUMMARY_DATE_FMT:-%Y-%m-%d}"
  typeset CURRENT_DATE='' WEEKDAY='' START_DATE='' END_DATE=''
  CURRENT_DATE=$(date +"$fmt")

  if $GIT_SUMMARY_DATE_HAS_V; then
    WEEKDAY=$(date -j -f "$fmt" "$CURRENT_DATE" +%u)
    START_DATE=$(date -j -f "$fmt" -v -"$((WEEKDAY - 1))"d "$CURRENT_DATE" +"$fmt")
    END_DATE=$(date -j -f "$fmt" -v +"$((7 - WEEKDAY))"d "$CURRENT_DATE" +"$fmt")
  else
    WEEKDAY=$(date -d "$CURRENT_DATE" +%u)
    START_DATE=$(date -d "$CURRENT_DATE -$((WEEKDAY - 1)) days" +"$fmt")
    END_DATE=$(date -d "$START_DATE +6 days" +"$fmt")
  fi

  print "\n📅 Git summary for this week: $START_DATE to $END_DATE"
  print
  _git_summary "$START_DATE" "$END_DATE"
  return $?
}

# git-summary <preset>|<from> <to>
# Show per-author contribution summary (added/deleted/net/commits) for a date range.
# Usage: git-summary all | <today|yesterday|this-week|last-week|this-month|last-month> | <from> <to>
# Notes:
# - Dates are `YYYY-MM-DD` and interpreted in local timezone boundaries.
git-summary() {
  emulate -L zsh
  typeset cmd="${1-}"
  typeset arg1="${1-}"
  typeset arg2="${2-}"

  case "$cmd" in
    all)
      _git_summary_require_git || return 1
      print "\n📅 Git summary for all commits"
      print
      _git_summary
      return $?
      ;;
    today)
      _git_summary_require_git || return 1
      _git_today
      return $?
      ;;
    yesterday)
      _git_summary_require_git || return 1
      _git_yesterday
      return $?
      ;;
    this-month)
      _git_summary_require_git || return 1
      _git_this_month
      return $?
      ;;
    last-month)
      _git_summary_require_git || return 1
      _git_last_month
      return $?
      ;;
    this-week)
      _git_summary_require_git || return 1
      _git_this_week
      return $?
      ;;
    last-week)
      _git_summary_require_git || return 1
      _git_last_week
      return $?
      ;;
    ""|help|--help|-h)
      printf "%s\n" "Usage: git-summary <command> [args]"
      printf "\n"
      printf "%s\n" "Commands:"
      printf "  %-16s  %s\n" \
        all            "Entire history" \
        today          "Today only" \
        yesterday      "Yesterday only" \
        this-month     "1st to today" \
        last-month     "1st to end of last month" \
        this-week      "This Mon–Sun" \
        last-week      "Last Mon–Sun"
      printf "  %-16s  %s\n" "<from> <to>" "Custom date range (YYYY-MM-DD)"
      printf "\n"
      return 0
      ;;
    *)
      if [[ -n "$arg1" && -n "$arg2" ]]; then
        _git_summary_require_git || return 1
        _git_summary "$arg1" "$arg2"
        return $?
      else
        print "❌ Invalid usage. Try: git-summary help"
        return 1
      fi
      ;;
  esac

  return 0
}
