#!/bin/sh
# Fast path in front of the modeled tmux script (`tmux-model`, same directory).
# A guarded send repeats `has-session` and `show-environment` identity probes
# many times, and starting the Python model for each of them dominates a send
# on a loaded host. Answer the default-state probes here, with exactly the
# model's output and call-log line, and hand every other call, or any state a
# test armed through a flag file, to the model before anything is logged.
root=${0%/*}

simple() {
    for arg in "$@"; do
        case "$arg" in
        '' | *[!A-Za-z0-9_=:.-]*) return 1 ;;
        esac
    done
}

log_call() {
    line=
    sep=
    for arg in "$@"; do
        line="$line$sep\"$arg\""
        sep=', '
    done
    printf '[%s]\n' "$line" >>"$root/calls"
}

case "$1" in
has-session)
    if simple "$@" && [ ! -e "$root/entered" ] && [ ! -e "$root/exit-after-enter" ] &&
        [ ! -e "$root/start-turn" ]; then
        log_call "$@"
        exit 0
    fi
    ;;
show-environment)
    if [ "$#" -eq 4 ] && [ "$2" = -t ] && simple "$@" && [ ! -e "$root/runtime-replaced" ] &&
        [ ! -e "$root/flip-second-runtime-read" ]; then
        case "$4" in
        AGENT_SESSION_ID) value=guarded ;;
        AGENT_SESSION_STATE_DIR) value="$root/state" ;;
        AGENT_SESSION_RUNTIME_ID)
            value=runtime-one
            count=0
            if [ -e "$root/runtime-read-count" ]; then
                read -r count <"$root/runtime-read-count"
            fi
            printf '%s' "$((count + 1))" >"$root/runtime-read-count"
            ;;
        *) value= ;;
        esac
        if [ -n "$value" ]; then
            log_call "$@"
            printf '%s=%s\n' "$4" "$value"
            exit 0
        fi
    fi
    ;;
esac
exec "$root/tmux-model" "$@"
