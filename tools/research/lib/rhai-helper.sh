# RESEARCH-ONLY. The supervised `mcp-rhai` helpers of a stack (director, narrator, steward, ask
# relay): sourced by tools/research/mcp-e2e and tools/research/mcp-rhai-helper-check, not executed.
#
# A helper never outlives what it serves (tools/research/mcp-rhai/src/daemon.rs). Its supervisor
# loop runs while the panel and the script that started it run; mcp-rhai itself stops (exit 4) when
# its supervisor is gone (--exit-with-parent) or the panel has not answered for $RHAI_GIVE_UP
# seconds (--give-up), and holds $RHAI_PIDDIR/<name>.pid, so a second copy is refused (exit 5) and
# rhai_helpers_stop finds every helper even after its supervisor was killed. Found 2026-11-23:
# helpers ran 6.5 h under pid 1 after their stack had stopped.
#
# Needs: RHAI (the mcp-rhai binary), RHAI_PIDDIR (a directory the stack owns).

# rhai_helper NAME LOG PANEL_PID URL SCRIPT [VAR=value ...]: start one supervised helper in the
# background; its supervisor's pid is in $! as after any `&`.
rhai_helper() {
  local name="$1" log="$2" panel="$3" url="$4" script="$5" owner="$$"
  shift 5
  ( while kill -0 "$panel" 2>/dev/null && kill -0 "$owner" 2>/dev/null; do
      env "$@" "$RHAI" --panel "$url" --exit-with-parent --give-up "${RHAI_GIVE_UP:-20}" \
        --pidfile "$RHAI_PIDDIR/$name.pid" "$script" >> "$log" 2>&1
      [[ $? -eq 5 ]] && break   # another copy holds the pidfile: never two of one helper
      sleep 3
    done ) &
}

# rhai_helpers_stop: stop every helper named by a pidfile in $RHAI_PIDDIR.
rhai_helpers_stop() {
  local f p
  for f in "$RHAI_PIDDIR"/*.pid; do
    [[ -f "$f" ]] || continue
    p="$(cat "$f" 2>/dev/null)"
    [[ "$p" =~ ^[0-9]+$ ]] && kill -TERM "$p" 2>/dev/null || true
  done
}
