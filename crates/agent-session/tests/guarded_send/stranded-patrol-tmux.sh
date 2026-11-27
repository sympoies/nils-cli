#!/bin/sh
# Patrol view of the same single-line Claude composer used by the send fixture.
root=${0%/*}

case "$1" in
list-windows)
    printf 'modeled-guarded|0\n'
    ;;
has-session)
    exit 0
    ;;
show-environment)
    printf 'AGENT_SESSION_ID=guarded\nAGENT_SESSION_STATE_DIR=%s/state\nAGENT_SESSION_RUNTIME_ID=runtime-one\n' "$root"
    ;;
capture-pane)
    [ ! -e "$root/capture-fails" ] || exit 1
    [ -f "$root/composer" ] || exit 1
    IFS= read -r composer <"$root/composer"
    printf '────────────────────────────\n❯ %s\n────────────────────────────\n  status\n' "$composer"
    ;;
*)
    exit 1
    ;;
esac
