gate=$1; broker_gate=$2; heartbeat=$3; capability=$4; incarnation=$5; generation=$6; broker_bin=$7; shift 7
done_file="${heartbeat}.done.$$"
umask 077
while [ ! -f "$broker_gate" ]; do sleep 0.01; done
"$broker_bin" --state-dir "$AGENT_SESSION_STATE_DIR" broker heartbeat --session "$AGENT_SESSION_ID" --incarnation "$incarnation" --generation "$generation" --capability-file "$capability" --format json >/dev/null 2>&1 &
broker_pid=$!
while [ ! -f "$gate" ]; do sleep 0.01; done
# Keep only a bounded stderr tail, without changing the provider's foreground
# terminal or retaining its stdout. Kernel pipes bound in-flight memory.
startup_dir="$AGENT_SESSION_STATE_DIR/sessions/$AGENT_SESSION_ID"
stderr_input="$startup_dir/.startup-stderr.input"
stderr_tail="$startup_dir/.startup-stderr.tail"
stderr_log="$startup_dir/.startup-stderr.log"
rm -f "$stderr_input" "$stderr_tail"
invoke() { :; "$@"; status=$?; }
if [ -d "$startup_dir" ] && mkfifo "$stderr_input" "$stderr_tail"; then
  tail -c 16384 < "$stderr_tail" > "$stderr_log" &
  collector=$!
  tee "$stderr_tail" < "$stderr_input" >&2 &
  forwarding=$!
  invoke "$@" 2> "$stderr_input"
  # The provider is terminal even while a descendant keeps stderr open.
  if [ ! -f "$startup_dir/.runtime-exit-status" ]; then
    printf '%s\n' "$status" > "$startup_dir/.runtime-exit-status"
  fi
  # A descendant can keep stderr open. Do not wait indefinitely for its EOF.
  drain_attempts=0
  while kill -0 "$forwarding" 2>/dev/null && [ "$drain_attempts" -lt 20 ]; do
    sleep 0.05
    drain_attempts=$((drain_attempts + 1))
  done
  kill "$forwarding" 2>/dev/null || true
  wait "$forwarding" 2>/dev/null || true
  wait "$collector" 2>/dev/null || true
  rm -f "$stderr_input" "$stderr_tail"
  if [ "$status" = 0 ] && [ "$(cat "$startup_dir/.startup-stage" 2>/dev/null)" = initial_connection ]; then
    rm -f "$stderr_log"
  fi
else
  invoke "$@"
fi
printf '%s\n' "$status" > "$done_file"
kill "$broker_pid" >/dev/null 2>&1 || true
wait "$broker_pid" >/dev/null 2>&1 || true
"$broker_bin" --state-dir "$AGENT_SESSION_STATE_DIR" broker stop --exit-code "$status" --session "$AGENT_SESSION_ID" --capability-file "$capability" --format json >/dev/null 2>&1 || "$broker_bin" --state-dir "$AGENT_SESSION_STATE_DIR" broker stop --session "$AGENT_SESSION_ID" --capability-file "$capability" --format json >/dev/null 2>&1 || true
rm -f "$done_file" "$capability" "$broker_gate" "$gate"
exit "$status"
