#!/bin/sh
# Bounded synthetic single-line Claude composer; no provider or real pane.
root=${0%/*}
log_call() {
    line= sep=
    for arg in "$@"; do
        line="$line$sep\"$arg\""
        sep=', '
    done
    printf '[%s]\n' "$line" >>"$root/calls"
}
read_composer() {
    text=
    IFS= read -r text <"$root/composer" || :
}
case "$1" in
capture-pane)
    log_call "$@"
    read_composer
    printf '────────────────────────────\n❯ '
    if [ -n "$text" ]; then printf '%s' "$text";
    else printf '\033[2mTry "edit <filepath> to..."\033[22m'; fi
    printf '\n────────────────────────────\n  status\n'
    exit 0
    ;;
display-message)
    log_call "$@"
    read_composer
    cursor=${#text}
    if [ -f "$root/cursor" ]; then read -r cursor <"$root/cursor" || :; fi
    printf '%s 1\n' "$((2 + cursor))"
    exit 0
    ;;
show-environment)
    if [ -f "$root/runtime-replaced" ] && [ "$#" -eq 4 ]; then
        log_call "$@"
        case "$4" in
        AGENT_SESSION_ID) value=guarded ;;
        AGENT_SESSION_RUNTIME_ID) value=runtime-replaced ;;
        AGENT_SESSION_STATE_DIR) value="$root/state" ;;
        *) exit 1 ;;
        esac
        printf '%s=%s\n' "$4" "$value"
        exit 0
    fi
    ;;
esac
exec "$root/tmux-default" "$@"
