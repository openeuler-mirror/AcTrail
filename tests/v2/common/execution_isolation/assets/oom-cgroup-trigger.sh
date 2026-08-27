#!/bin/sh
set -eu

: "${ACTRAIL_HOST_OOM_TRIGGER:?}"
: "${ACTRAIL_HOST_COORD_DIR:?}"

start_file="$ACTRAIL_HOST_COORD_DIR/oom.start"
pid_file="$ACTRAIL_HOST_COORD_DIR/oom.pid"
group_file="$ACTRAIL_HOST_COORD_DIR/oom.cgroup"
trigger_pid=""
group=""
mode=""

cleanup() {
  status=$?
  trap - EXIT INT TERM
  rm -f "$start_file"
  if [ -n "$trigger_pid" ]; then
    if kill -0 "$trigger_pid" 2>/dev/null; then
      kill -KILL "$trigger_pid" 2>/dev/null || true
    fi
    wait "$trigger_pid" 2>/dev/null || true
    trigger_pid=""
  fi
  if [ -n "$group" ] && [ -d "$group" ]; then
    if ! rmdir "$group" 2>/dev/null; then
      echo "failed to remove test OOM cgroup: $group" >&2
      status=80
    fi
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

active_swap=0
if [ ! -r /proc/swaps ]; then
  echo "host swap state is unavailable" >&2
  exit 79
fi
if awk 'NR > 1 { found = 1 } END { exit(found ? 0 : 1) }' /proc/swaps; then
  active_swap=1
fi

if [ -f /sys/fs/cgroup/cgroup.controllers ]; then
  mode=v2
  group="/sys/fs/cgroup/actrail-host-oom-$$"
  mkdir "$group"
  printf '%s\n' "$group" > "$group_file"
  echo 33554432 > "$group/memory.max"
  if [ -f "$group/memory.swap.max" ]; then
    echo 0 > "$group/memory.swap.max"
  elif [ "$active_swap" -eq 1 ]; then
    echo "active host swap cannot be bounded by the test cgroup" >&2
    exit 79
  fi
elif [ -f /sys/fs/cgroup/memory/memory.limit_in_bytes ]; then
  mode=v1
  group="/sys/fs/cgroup/memory/actrail-host-oom-$$"
  mkdir "$group"
  printf '%s\n' "$group" > "$group_file"
  echo 33554432 > "$group/memory.limit_in_bytes"
  if [ -f "$group/memory.memsw.limit_in_bytes" ]; then
    echo 33554432 > "$group/memory.memsw.limit_in_bytes"
  elif [ "$active_swap" -eq 1 ]; then
    echo "active host swap cannot be bounded by the test cgroup" >&2
    exit 79
  fi
else
  echo "memory cgroup controller is unavailable" >&2
  exit 70
fi

before="$(awk '$1 == "oom_kill" { print $2 }' /proc/vmstat)"
case "$before" in
  ''|*[!0-9]*) echo "invalid oom_kill baseline" >&2; exit 71 ;;
esac

rm -f "$start_file" "$pid_file"
python3 "$ACTRAIL_HOST_OOM_TRIGGER" "$start_file" &
trigger_pid=$!
printf '%s\n' "$trigger_pid" > "$pid_file"
if [ "$mode" = v2 ]; then
  echo "$trigger_pid" > "$group/cgroup.procs"
  cgroup_oom_before="$(awk '$1 == "oom_kill" { print $2 }' "$group/memory.events")"
else
  echo "$trigger_pid" > "$group/tasks"
  cgroup_oom_before="$(awk '$1 == "oom_kill" { print $2 }' "$group/memory.oom_control")"
fi
case "$cgroup_oom_before" in
  ''|*[!0-9]*) echo "invalid cgroup oom_kill baseline" >&2; exit 74 ;;
esac
released_at_ms="$(python3 -c 'import time; print(time.time_ns() // 1000000)')"
case "$released_at_ms" in
  ''|*[!0-9]*) echo "invalid OOM release timestamp" >&2; exit 78 ;;
esac
touch "$start_file"

remaining=300
while kill -0 "$trigger_pid" 2>/dev/null; do
  [ "$remaining" -gt 0 ] || {
    echo "OOM trigger was not killed within 30 seconds" >&2
    exit 72
  }
  remaining=$((remaining - 1))
  sleep 0.1
done
set +e
wait "$trigger_pid" 2>/dev/null
wait_status=$?
set -e
trigger_pid=""
if [ "$wait_status" -ne 137 ]; then
  echo "OOM trigger exited with status $wait_status instead of SIGKILL" >&2
  exit 75
fi

if [ "$mode" = v2 ]; then
  cgroup_oom_after="$(awk '$1 == "oom_kill" { print $2 }' "$group/memory.events")"
else
  cgroup_oom_after="$(awk '$1 == "oom_kill" { print $2 }' "$group/memory.oom_control")"
fi
case "$cgroup_oom_after" in
  ''|*[!0-9]*) echo "invalid cgroup oom_kill result" >&2; exit 76 ;;
esac
if [ "$cgroup_oom_after" -le "$cgroup_oom_before" ]; then
  echo "test cgroup did not report an OOM kill" >&2
  exit 77
fi

remaining=50
while [ "$remaining" -gt 0 ]; do
  after="$(awk '$1 == "oom_kill" { print $2 }' /proc/vmstat)"
  if [ "$after" -gt "$before" ]; then
    touch "$ACTRAIL_HOST_COORD_DIR/oom.completed"
    echo "ACTRAIL_HOST_OOM_KILL_OK pid=$(cat "$pid_file") before=$before after=$after cgroup_before=$cgroup_oom_before cgroup_after=$cgroup_oom_after released_at_ms=$released_at_ms"
    exit 0
  fi
  remaining=$((remaining - 1))
  sleep 0.1
done

echo "vmstat oom_kill did not increase" >&2
exit 73
