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
OPENCODE_BRIDGE_PID=""

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
  if [ -n "$OPENCODE_BRIDGE_PID" ]; then
    kill "$OPENCODE_BRIDGE_PID" >/dev/null 2>&1 || true
    wait "$OPENCODE_BRIDGE_PID" >/dev/null 2>&1 || true
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

if [ -n "${ACTRAIL_OPENCODE_FREE_MODEL:-}" ]; then
  : "${ACTRAIL_OPENCODE_PROMPT:?}"
  : "${ACTRAIL_OPENCODE_RESPONSE_MARKER:?}"
  : "${ACTRAIL_OPENCODE_GUEST_BRIDGE:?}"
  : "${ACTRAIL_OPENCODE_PROXY_PORT:?}"
  : "${ACTRAIL_OPENCODE_VSOCK_PORT:?}"
  case "$ACTRAIL_OPENCODE_FREE_MODEL" in
    opencode/*-free) ;;
    *) fail "OpenCode smoke only permits opencode/*-free models" ;;
  esac
  command -v opencode >/dev/null 2>&1 \
    || fail "opencode is missing from the virtual-container workload image"
  command -v timeout >/dev/null 2>&1 \
    || fail "timeout is missing from the virtual-container workload image"
  "$ACTRAIL_OPENCODE_GUEST_BRIDGE" \
    --listen-port "$ACTRAIL_OPENCODE_PROXY_PORT" \
    --vsock-port "$ACTRAIL_OPENCODE_VSOCK_PORT" \
    >"$ACTRAIL_XIAOO_COORD_DIR/opencode-bridge.stdout" \
    2>"$ACTRAIL_XIAOO_COORD_DIR/opencode-bridge.stderr" &
  OPENCODE_BRIDGE_PID=$!
  sleep 1
  kill -0 "$OPENCODE_BRIDGE_PID" >/dev/null 2>&1 \
    || fail "OpenCode Guest VSOCK bridge exited before readiness"
  OPENCODE_HOME="$ACTRAIL_XIAOO_COORD_DIR/opencode-home"
  install -d -m 0700 \
    "$OPENCODE_HOME" \
    "$OPENCODE_HOME/config" \
    "$OPENCODE_HOME/data" \
    "$OPENCODE_HOME/cache/opencode"
  [ -d /opt/opencode-bootstrap/node_modules/@opencode-ai/plugin ] \
    || fail "OpenCode plugin bootstrap cache is missing from the workload image"
  cp -a /opt/opencode-bootstrap/. "$OPENCODE_HOME/config/opencode/"
  [ -s /opt/opencode-bootstrap-cache/models.json ] \
    || fail "OpenCode model bootstrap cache is missing from the workload image"
  cp /opt/opencode-bootstrap-cache/models.json \
    "$OPENCODE_HOME/cache/opencode/models.json"
  OPENCODE_STDOUT="$ACTRAIL_XIAOO_COORD_DIR/opencode.stdout"
  set +e
  HOME="$OPENCODE_HOME" \
    XDG_CONFIG_HOME="$OPENCODE_HOME/config" \
    XDG_DATA_HOME="$OPENCODE_HOME/data" \
    XDG_CACHE_HOME="$OPENCODE_HOME/cache" \
    HTTP_PROXY="http://127.0.0.1:$ACTRAIL_OPENCODE_PROXY_PORT" \
    HTTPS_PROXY="http://127.0.0.1:$ACTRAIL_OPENCODE_PROXY_PORT" \
    http_proxy="http://127.0.0.1:$ACTRAIL_OPENCODE_PROXY_PORT" \
    https_proxy="http://127.0.0.1:$ACTRAIL_OPENCODE_PROXY_PORT" \
    NO_COLOR=1 \
    timeout 180 opencode run --pure \
      --model "$ACTRAIL_OPENCODE_FREE_MODEL" \
      "$ACTRAIL_OPENCODE_PROMPT" \
      </dev/null \
      >"$OPENCODE_STDOUT" 2>&1
  opencode_rc=$?
  set -e
  cat "$OPENCODE_STDOUT"
  if [ "$opencode_rc" -ne 0 ]; then
    echo "OpenCode Guest bridge stdout:" >&2
    cat "$ACTRAIL_XIAOO_COORD_DIR/opencode-bridge.stdout" >&2 || true
    echo "OpenCode Guest bridge stderr:" >&2
    cat "$ACTRAIL_XIAOO_COORD_DIR/opencode-bridge.stderr" >&2 || true
    fail "OpenCode free-model smoke exited with status $opencode_rc"
  fi
  grep -Fq "$ACTRAIL_OPENCODE_RESPONSE_MARKER" "$OPENCODE_STDOUT" \
    || fail "OpenCode output omitted its response marker"
  echo "KATA_OPENCODE_FREE_OK instance=$ACTRAIL_XIAOO_INSTANCE"
fi

echo "KATA_XIAOO_WORKLOAD_OK instance=$ACTRAIL_XIAOO_INSTANCE"
