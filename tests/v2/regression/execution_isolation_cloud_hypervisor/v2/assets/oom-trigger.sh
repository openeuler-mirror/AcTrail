#!/bin/sh
set -eu

CGROUP_ROOT=/sys/fs/cgroup
GROUP="$CGROUP_ROOT/actrail-e2e-oom"
START_FILE=/run/actrail-execution/oom.start
TRIGGER=/opt/actrail-execution/oom_trigger.py
TRIGGER_PID=""

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
[ -x "$TRIGGER" ] || { echo "OOM trigger is unavailable" >&2; exit 72; }

mkdir "$GROUP"
[ -f "$GROUP/memory.max" ] \
  || { echo "cgroup memory controller is unavailable" >&2; exit 73; }
echo 33554432 > "$GROUP/memory.max"
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
python3 "$TRIGGER" "$START_FILE" &
TRIGGER_PID=$!
echo "$TRIGGER_PID" > "$GROUP/cgroup.procs"
touch "$START_FILE"

remaining=300
while kill -0 "$TRIGGER_PID" 2>/dev/null; do
  [ "$remaining" -gt 0 ] \
    || { echo "OOM trigger was not killed within 30 seconds" >&2; exit 75; }
  remaining=$((remaining - 1))
  sleep 0.1
done
wait "$TRIGGER_PID" 2>/dev/null || true
TRIGGER_PID=""

events_oom_kill="$(awk '$1 == "oom_kill" { print $2 }' "$GROUP/memory.events")"
case "$events_oom_kill" in
  ''|*[!0-9]*) echo "invalid cgroup oom_kill result" >&2; exit 76 ;;
esac
[ "$events_oom_kill" -gt 0 ] \
  || { echo "cgroup did not report an OOM kill" >&2; exit 77; }

remaining=50
while [ "$remaining" -gt 0 ]; do
  after_vmstat="$(awk '$1 == "oom_kill" { print $2 }' /proc/vmstat)"
  if [ "$after_vmstat" -gt "$before_vmstat" ]; then
    echo "ACTRAIL_CLOUD_HYPERVISOR_OOM_KILL_OK before=$before_vmstat after=$after_vmstat"
    exit 0
  fi
  remaining=$((remaining - 1))
  sleep 0.1
done

echo "Guest vmstat oom_kill did not increase" >&2
exit 78
