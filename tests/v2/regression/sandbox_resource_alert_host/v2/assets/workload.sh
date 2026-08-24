#!/bin/sh
set -eu

: "${ACTRAIL_HOST_NAMED_ROOT:?}"
: "${ACTRAIL_HOST_REAL_XIAOO:?}"
: "${ACTRAIL_HOST_XIAOO_CONFIG:?}"
: "${ACTRAIL_HOST_PROVIDER_SCRIPT:?}"
: "${ACTRAIL_HOST_COORD_DIR:?}"
: "${ACTRAIL_HOST_TASK_FILE:?}"

provider_port="${ACTRAIL_HOST_PROVIDER_PORT:-18098}"
ready_timeout="${ACTRAIL_HOST_READY_TIMEOUT_SECONDS:-90}"
api_key="${ACTRAIL_HOST_API_KEY:-actrail-host-local-key}"
provider_pid=""

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

cleanup() {
  if [ -n "$provider_pid" ]; then
    kill "$provider_pid" >/dev/null 2>&1 || true
    wait "$provider_pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

coord="$ACTRAIL_HOST_COORD_DIR"
mkdir -p "$coord/home/config" "$coord/home/cache"
export HOME="$coord/home"
export XDG_CONFIG_HOME="$coord/home/config"
export XDG_CACHE_HOME="$coord/home/cache"

provider_stdout="$coord/provider.stdout"
provider_stderr="$coord/provider.stderr"
xiaoo_stdout="$coord/xiaoo.stdout"
read_output="$coord/agent-read.bin"
write_output="$coord/agent-write.bin"

python3 -u "$ACTRAIL_HOST_PROVIDER_SCRIPT" \
  --mode local-stream \
  --bind-host 127.0.0.1 \
  --bind-port "$provider_port" \
  --local-stream-response-text ACTRAIL_HOST_XIAOO_OK \
  --local-stream-reasoning-tokens 3 \
  --local-tool-command "head -c 1048576 \"$ACTRAIL_HOST_TASK_FILE\" > \"$read_output\"" \
  --local-tool-command "dd if=/dev/zero of=\"$write_output\" bs=1048576 count=4" \
  >"$provider_stdout" 2>"$provider_stderr" &
provider_pid=$!

remaining="$ready_timeout"
while ! grep -q '^proxy_base_url=' "$provider_stdout" 2>/dev/null; do
  kill -0 "$provider_pid" >/dev/null 2>&1 || {
    cat "$provider_stderr" >&2 || true
    fail "local provider exited before readiness"
  }
  [ "$remaining" -gt 0 ] || fail "local provider readiness timed out"
  remaining=$((remaining - 1))
  sleep 1
done
touch "$coord/provider.ready"

remaining="$ready_timeout"
while [ ! -f "$coord/workload.release" ]; do
  kill -0 "$provider_pid" >/dev/null 2>&1 \
    || fail "local provider exited while waiting for workload release"
  [ "$remaining" -gt 0 ] || fail "workload release timed out"
  remaining=$((remaining - 1))
  sleep 1
done

set +e
ACTRAIL_VIRTUAL_XIAOO_API_KEY="$api_key" \
ACTRAIL_HOST_REAL_XIAOO="$ACTRAIL_HOST_REAL_XIAOO" \
  "$ACTRAIL_HOST_NAMED_ROOT" --cli run \
    --config "$ACTRAIL_HOST_XIAOO_CONFIG" \
    --tools bash \
    --max-turns 4 \
    --prompt "Use the requested Bash tools, then reply ACTRAIL_HOST_XIAOO_OK." \
    >"$xiaoo_stdout" 2>&1
xiaoo_rc=$?
set -e

cat "$xiaoo_stdout"
[ "$xiaoo_rc" -eq 0 ] || fail "xiaoO exited with status $xiaoo_rc"
grep -Fq ACTRAIL_HOST_XIAOO_OK "$xiaoo_stdout" \
  || fail "xiaoO output omitted response marker"
[ "$(wc -c < "$read_output")" -eq 1048576 ] \
  || fail "xiaoO did not complete the read tool call"
[ "$(wc -c < "$write_output")" -eq 4194304 ] \
  || fail "xiaoO did not complete the write tool call"
grep -q '^proxy_local_stream turn=tool-call-1 ' "$provider_stdout" \
  || fail "provider did not observe the first xiaoO tool turn"
grep -q '^proxy_local_stream turn=tool-call-2 ' "$provider_stdout" \
  || fail "provider did not observe the second xiaoO tool turn"

echo ACTRAIL_HOST_XIAOO_WORKLOAD_OK
