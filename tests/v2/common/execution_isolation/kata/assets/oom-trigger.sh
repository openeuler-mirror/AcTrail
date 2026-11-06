#!/bin/sh
set -eu

CGROUP_ROOT=/sys/fs/cgroup
GROUP="$CGROUP_ROOT/actrail-e2e-oom"
START_FILE=/tmp/actrail-e2e-oom.start
TRIGGER_PID=""
OOM_KILL_MARKER="${ACTRAIL_EXECUTION_ISOLATION_OOM_KILL_MARKER:?}"

cleanup() {
  rm -f "$START_FILE"
  if [ -n "$TRIGGER_PID" ] && kill -0 "$TRIGGER_PID" 2>/dev/null; then
    kill -KILL "$TRIGGER_PID" 2>/dev/null || true
  fi
  if [ -f "$GROUP/cgroup.kill" ]; then
    echo 1 > "$GROUP/cgroup.kill" 2>/dev/null || true
  fi
  rmdir "$GROUP" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

[ -f "$CGROUP_ROOT/cgroup.controllers" ] \
  || { echo "cgroup v2 is unavailable" >&2; exit 70; }
[ -r /proc/vmstat ] || { echo "/proc/vmstat is unreadable" >&2; exit 71; }
grep -qw memory "$CGROUP_ROOT/cgroup.controllers" \
  || { echo "cgroup memory controller is unavailable" >&2; exit 72; }
if ! grep -qw memory "$CGROUP_ROOT/cgroup.subtree_control"; then
  echo +memory > "$CGROUP_ROOT/cgroup.subtree_control"
fi

mkdir "$GROUP"
[ -f "$GROUP/memory.max" ] \
  || { echo "cgroup memory controller is unavailable" >&2; exit 73; }
echo "$ACTRAIL_OOM_MEMORY_LIMIT_BYTES" > "$GROUP/memory.max"
if [ -f "$GROUP/memory.swap.max" ]; then
  echo 0 > "$GROUP/memory.swap.max"
fi
if [ -f "$GROUP/memory.oom.group" ]; then
  echo 1 > "$GROUP/memory.oom.group"
fi

before_vmstat="$(awk '$1 == "oom_kill" { print $2 }' /proc/vmstat)"
case "$before_vmstat" in
  ''|*[!0-9]*) echo "invalid oom_kill baseline" >&2; exit 74 ;;
esac

rm -f "$START_FILE"
(
  while [ ! -f "$START_FILE" ]; do
    sleep "$ACTRAIL_OOM_POLL_SECONDS"
  done
  exec awk 'BEGIN {
    value = "0123456789abcdef"
    while (1) value = value value
  }'
) &
TRIGGER_PID=$!
echo "$TRIGGER_PID" > "$GROUP/cgroup.procs"
touch "$START_FILE"

remaining="$ACTRAIL_OOM_KILL_ATTEMPTS"
while kill -0 "$TRIGGER_PID" 2>/dev/null; do
  [ "$remaining" -gt 0 ] \
    || { echo "OOM trigger was not killed before the configured deadline" >&2; exit 75; }
  remaining=$((remaining - 1))
  sleep "$ACTRAIL_OOM_POLL_SECONDS"
done
wait "$TRIGGER_PID" 2>/dev/null || true
TRIGGER_PID=""

events_oom_kill="$(awk '$1 == "oom_kill" { print $2 }' "$GROUP/memory.events")"
case "$events_oom_kill" in
  ''|*[!0-9]*) echo "invalid cgroup oom_kill result" >&2; exit 76 ;;
esac
[ "$events_oom_kill" -gt 0 ] \
  || { echo "cgroup did not report an OOM kill" >&2; exit 77; }

remaining="$ACTRAIL_OOM_VMSTAT_ATTEMPTS"
while [ "$remaining" -gt 0 ]; do
  after_vmstat="$(awk '$1 == "oom_kill" { print $2 }' /proc/vmstat)"
  if [ "$after_vmstat" -gt "$before_vmstat" ]; then
    echo "$OOM_KILL_MARKER before=$before_vmstat after=$after_vmstat"
    exit 0
  fi
  remaining=$((remaining - 1))
  sleep "$ACTRAIL_OOM_POLL_SECONDS"
done

echo "Guest vmstat oom_kill did not increase" >&2
exit 78
