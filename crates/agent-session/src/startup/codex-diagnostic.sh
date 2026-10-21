collect_startup_diagnostic() {
  if (umask 077; tail -c 16384 > "$startup_diagnostic_buffer"); then
    runtime_exit="$(cat "$runtime_exit_status" 2>/dev/null)"
    if { [ "$(cat "$startup_stage" 2>/dev/null)" != initial_connection ] ||
         { [ -n "$runtime_exit" ] && [ "$runtime_exit" != 0 ]; }; } &&
       [ -s "$startup_diagnostic_buffer" ]; then
      mv -f "$startup_diagnostic_buffer" "$startup_diagnostic" 2>/dev/null ||
        rm -f -- "$startup_diagnostic_buffer"
    else
      rm -f -- "$startup_diagnostic_buffer"
    fi
    if [ "$runtime_exit" = 0 ] &&
       [ "$(cat "$startup_stage" 2>/dev/null)" = initial_connection ] &&
       [ ! -f "${startup_failure:-}" ]; then
      rm -f -- "$runtime_exit_status"
    fi
  else
    rm -f -- "$startup_diagnostic_buffer"
  fi
}