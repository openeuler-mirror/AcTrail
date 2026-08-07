#!/bin/sh
# Runs inside one Kata workload after actrailctl has created the trace.
set -eu

: "${ACTRAIL_XIAOO_INSTANCE:?}"
: "${ACTRAIL_XIAOO_BIN:?}"
: "${ACTRAIL_XIAOO_CONFIG:?}"
: "${ACTRAIL_XIAOO_PROMPT:?}"
: "${ACTRAIL_XIAOO_RESPONSE_MARKER:?}"
: "${ACTRAIL_XIAOO_WRITE_MARKER:?}"
: "${ACTRAIL_XIAOO_TASK_INPUT:?}"
: "${ACTRAIL_XIAOO_COORD_DIR:?}"
: "${ACTRAIL_XIAOO_PROVIDER_SCRIPT:?}"

PROVIDER_PORT="${ACTRAIL_XIAOO_PROVIDER_PORT:-18098}"
PROVIDER_DELAY="${ACTRAIL_XIAOO_PROVIDER_DELAY_SECONDS:-1.0}"
READY_TIMEOUT="${ACTRAIL_XIAOO_READY_TIMEOUT_SECONDS:-60}"
LOCAL_API_KEY="${ACTRAIL_VIRTUAL_XIAOO_API_KEY:-actrail-kata-local-key}"
PROVIDER_PID=""
XIAOO_PID=""

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

case "$READY_TIMEOUT" in
  ''|*[!0-9]*) fail "ACTRAIL_XIAOO_READY_TIMEOUT_SECONDS must be an integer" ;;
esac
[ "$READY_TIMEOUT" -gt 0 ] || fail "ready timeout must be positive"

for path in \
  "$ACTRAIL_XIAOO_BIN" \
  "$ACTRAIL_XIAOO_CONFIG" \
  "$ACTRAIL_XIAOO_TASK_INPUT" \
  "$ACTRAIL_XIAOO_PROVIDER_SCRIPT"; do
  [ -r "$path" ] || fail "required xiaoO asset is unreadable: $path"
done
[ -x "$ACTRAIL_XIAOO_BIN" ] || fail "xiaoO binary is not executable"
command -v python3 >/dev/null 2>&1 \
  || fail "python3 is missing from the virtual-container workload image"

mkdir -p "$ACTRAIL_XIAOO_COORD_DIR"
XIAOO_HOME="$ACTRAIL_XIAOO_COORD_DIR/home"
mkdir -p "$XIAOO_HOME/config" "$XIAOO_HOME/cache"
export HOME="$XIAOO_HOME"
export XDG_CONFIG_HOME="$XIAOO_HOME/config"
export XDG_CACHE_HOME="$XIAOO_HOME/cache"
cd "$ACTRAIL_XIAOO_COORD_DIR"
PROVIDER_STDOUT="$ACTRAIL_XIAOO_COORD_DIR/provider.stdout"
PROVIDER_STDERR="$ACTRAIL_XIAOO_COORD_DIR/provider.stderr"
XIAOO_STDOUT="$ACTRAIL_XIAOO_COORD_DIR/xiaoo.stdout"
TASK_OUTPUT="$ACTRAIL_XIAOO_COORD_DIR/task-output.txt"
PROVIDER_READY="$ACTRAIL_XIAOO_COORD_DIR/provider.ready"
RELEASE_FILE="$ACTRAIL_XIAOO_COORD_DIR/release"
XIAOO_ACTIVE="$ACTRAIL_XIAOO_COORD_DIR/xiaoo.active"

cleanup() {
  rm -f "$XIAOO_ACTIVE"
  if [ -n "$XIAOO_PID" ]; then
    kill "$XIAOO_PID" >/dev/null 2>&1 || true
    wait "$XIAOO_PID" >/dev/null 2>&1 || true
  fi
  if [ -n "$PROVIDER_PID" ]; then
    kill "$PROVIDER_PID" >/dev/null 2>&1 || true
    wait "$PROVIDER_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

cp "$ACTRAIL_XIAOO_TASK_INPUT" "$TASK_OUTPUT"
printf '%s\n' "$ACTRAIL_XIAOO_WRITE_MARKER" >>"$TASK_OUTPUT"
dd if="$TASK_OUTPUT" of=/dev/null bs=4096 count=1 2>/dev/null

python3 -u "$ACTRAIL_XIAOO_PROVIDER_SCRIPT" \
  --mode local-stream \
  --bind-host 127.0.0.1 \
  --bind-port "$PROVIDER_PORT" \
  --local-stream-response-text "$ACTRAIL_XIAOO_RESPONSE_MARKER" \
  --local-stream-reasoning-tokens 3 \
  --response-chunk-delay-seconds "$PROVIDER_DELAY" \
  >"$PROVIDER_STDOUT" 2>"$PROVIDER_STDERR" &
PROVIDER_PID=$!

remaining="$READY_TIMEOUT"
while ! grep -q '^proxy_base_url=' "$PROVIDER_STDOUT" 2>/dev/null; do
  kill -0 "$PROVIDER_PID" >/dev/null 2>&1 || {
    cat "$PROVIDER_STDERR" >&2 || true
    fail "local OpenAI-compatible provider exited before readiness"
  }
  [ "$remaining" -gt 0 ] || fail "local provider readiness timed out"
  remaining=$((remaining - 1))
  sleep 1
done
touch "$PROVIDER_READY"
echo "KATA_XIAOO_PROVIDER_READY instance=$ACTRAIL_XIAOO_INSTANCE"

remaining="$READY_TIMEOUT"
while [ ! -f "$RELEASE_FILE" ]; do
  kill -0 "$PROVIDER_PID" >/dev/null 2>&1 \
    || fail "local provider exited while waiting at the concurrency barrier"
  [ "$remaining" -gt 0 ] || fail "concurrency barrier release timed out"
  remaining=$((remaining - 1))
  sleep 1
done

ACTRAIL_VIRTUAL_XIAOO_API_KEY="$LOCAL_API_KEY" \
  "$ACTRAIL_XIAOO_BIN" --cli run \
    --config "$ACTRAIL_XIAOO_CONFIG" \
    --no-tools \
    --max-turns 1 \
    --prompt "$ACTRAIL_XIAOO_PROMPT" \
    >"$XIAOO_STDOUT" 2>&1 &
XIAOO_PID=$!
touch "$XIAOO_ACTIVE"

set +e
wait "$XIAOO_PID"
xiaoo_rc=$?
set -e
XIAOO_PID=""
rm -f "$XIAOO_ACTIVE"
cat "$XIAOO_STDOUT"
[ "$xiaoo_rc" -eq 0 ] || fail "xiaoO exited with status $xiaoo_rc"
grep -Fq "$ACTRAIL_XIAOO_RESPONSE_MARKER" "$XIAOO_STDOUT" \
  || fail "xiaoO output omitted its provider response marker"

remaining=10
while ! grep -q '^proxy_local_stream ' "$PROVIDER_STDOUT" 2>/dev/null; do
  [ "$remaining" -gt 0 ] || {
    cat "$PROVIDER_STDOUT" >&2 || true
    fail "provider did not record the xiaoO streaming request"
  }
  remaining=$((remaining - 1))
  sleep 1
done

echo "KATA_XIAOO_WORKLOAD_OK instance=$ACTRAIL_XIAOO_INSTANCE"
