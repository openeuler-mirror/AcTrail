#!/bin/sh
set -eu

: "${ACTRAIL_HOST_NAMED_ROOT:?}"
: "${ACTRAIL_HOST_REAL_XIAOO:?}"
: "${ACTRAIL_HOST_XIAOO_CONFIG:?}"
: "${ACTRAIL_HOST_PROVIDER_SCRIPT:?}"
: "${ACTRAIL_HOST_COORD_DIR:?}"
: "${ACTRAIL_HOST_TASK_FILE:?}"
: "${ACTRAIL_HOST_OOM_COMMAND:?}"

provider_port="${ACTRAIL_HOST_PROVIDER_PORT:?}"
ready_timeout="${ACTRAIL_HOST_READY_TIMEOUT_SECONDS:?}"
api_key="${ACTRAIL_HOST_API_KEY:?}"
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
  --bind-host "${ACTRAIL_HOST_PROVIDER_HOST:?}" \
  --bind-port "$provider_port" \
  --local-stream-response-text ACTRAIL_HOST_XIAOO_OK \
  --local-stream-reasoning-tokens "${ACTRAIL_HOST_REASONING_TOKENS:?}" \
  --local-tool-command "head -c ${ACTRAIL_HOST_READ_BYTES:?} \"$ACTRAIL_HOST_TASK_FILE\" > \"$read_output\"" \
  --local-tool-command "dd if=/dev/zero of=\"$write_output\" bs=${ACTRAIL_HOST_WRITE_BLOCK_BYTES:?} count=${ACTRAIL_HOST_WRITE_BLOCKS:?}" \
  --local-tool-command "$ACTRAIL_HOST_OOM_COMMAND" \
  >"$provider_stdout" 2>"$provider_stderr" &
provider_pid=$!

remaining="$ready_timeout"
while ! grep -q '^proxy_base_url=' "$provider_stdout" 2>/dev/null; do
  kill -0 "$provider_pid" >/dev/null 2>&1 || {
    cat "$provider_stderr" >&2 || true
    fail "local provider exited before readiness"
  }
  [ "$remaining" -gt 0 ] || fail "local provider readiness timed out"
  remaining=$((remaining - ACTRAIL_HOST_POLL_SECONDS))
  sleep "${ACTRAIL_HOST_POLL_SECONDS:?}"
done
touch "$coord/provider.ready"

remaining="$ready_timeout"
while [ ! -f "$coord/workload.release" ]; do
  kill -0 "$provider_pid" >/dev/null 2>&1 \
    || fail "local provider exited while waiting for workload release"
  [ "$remaining" -gt 0 ] || fail "workload release timed out"
  remaining=$((remaining - ACTRAIL_HOST_POLL_SECONDS))
  sleep "${ACTRAIL_HOST_POLL_SECONDS:?}"
done

set +e
ACTRAIL_VIRTUAL_XIAOO_API_KEY="$api_key" \
ACTRAIL_HOST_REAL_XIAOO="$ACTRAIL_HOST_REAL_XIAOO" \
  "$ACTRAIL_HOST_NAMED_ROOT" --cli run \
    --config "$ACTRAIL_HOST_XIAOO_CONFIG" \
    --tools bash \
    --max-turns "${ACTRAIL_HOST_MAX_TURNS:?}" \
    --prompt "Use the requested Bash tools, then reply ACTRAIL_HOST_XIAOO_OK." \
    >"$xiaoo_stdout" 2>&1
xiaoo_rc=$?
set -e

cat "$xiaoo_stdout"
[ "$xiaoo_rc" -eq 0 ] || fail "xiaoO exited with status $xiaoo_rc"
grep -Fq ACTRAIL_HOST_XIAOO_OK "$xiaoo_stdout" \
  || fail "xiaoO output omitted response marker"
[ "$(wc -c < "$read_output")" -eq "$ACTRAIL_HOST_READ_BYTES" ] \
  || fail "xiaoO did not complete the read tool call"
[ "$(wc -c < "$write_output")" -eq "$((ACTRAIL_HOST_WRITE_BLOCK_BYTES * ACTRAIL_HOST_WRITE_BLOCKS))" ] \
  || fail "xiaoO did not complete the write tool call"
grep -q '^proxy_local_stream turn=tool-call-1 ' "$provider_stdout" \
  || fail "provider did not observe the first xiaoO tool turn"
grep -q '^proxy_local_stream turn=tool-call-2 ' "$provider_stdout" \
  || fail "provider did not observe the second xiaoO tool turn"
grep -q '^proxy_local_stream turn=tool-call-3 ' "$provider_stdout" \
  || fail "provider did not observe the third xiaoO tool turn"
[ -f "$coord/oom.completed" ] \
  || fail "xiaoO did not complete the isolated OOM tool call"

echo ACTRAIL_HOST_XIAOO_WORKLOAD_OK
