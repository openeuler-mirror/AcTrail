#!/bin/sh
# Shared helpers for the V2 in-guest Kata fixtures.
set -u

ACTRAIL_BUNDLE="${ACTRAIL_BUNDLE:-/actrail}"
ACTRAIL_WORK_BASE="${ACTRAIL_WORK_BASE:-/tmp/actrail-e2e}"
ACTRAIL_SETTLE_SECONDS="${ACTRAIL_SETTLE_SECONDS:-3}"

ACTRAILD="${ACTRAILD:-$ACTRAIL_BUNDLE/actraild}"
ACTRAILCTL="${ACTRAILCTL:-$ACTRAIL_BUNDLE/actrailctl}"
ACTRAILVIEWER="${ACTRAILVIEWER:-$ACTRAIL_BUNDLE/actrailviewer}"
TLS_PAYLOAD_SYNC_LIBRARY="${TLS_PAYLOAD_SYNC_LIBRARY:-${TLS_SYNC_LIBRARY:-$ACTRAIL_BUNDLE/libactrail_tls_payload_probe_sync.so}}"
TLS_SYNC_LIBRARY="$TLS_PAYLOAD_SYNC_LIBRARY"

if [ -f "$ACTRAIL_BUNDLE/openssl.cnf" ]; then
  export OPENSSL_CONF="${OPENSSL_CONF:-$ACTRAIL_BUNDLE/openssl.cnf}"
fi
export TLS_PAYLOAD_SYNC_LIBRARY
if [ -d "$ACTRAIL_BUNDLE/lib" ]; then
  export LD_LIBRARY_PATH="$ACTRAIL_BUNDLE/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

need_exe() {
  [ -x "$1" ] || fail "required executable missing or not executable: $1"
}

actrail_need_binaries() {
  need_exe "$ACTRAILD"
  need_exe "$ACTRAILCTL"
  need_exe "$ACTRAILVIEWER"
  [ -f "$TLS_SYNC_LIBRARY" ] || fail "TLS sync library missing: $TLS_SYNC_LIBRARY"
}

openssl_bin() {
  if [ -n "${ACTRAIL_OPENSSL:-}" ]; then
    printf '%s\n' "$ACTRAIL_OPENSSL"
  elif [ -x /usr/bin/openssl ]; then
    printf '%s\n' /usr/bin/openssl
  elif command -v openssl >/dev/null 2>&1; then
    command -v openssl
  elif [ -x "$ACTRAIL_BUNDLE/openssl" ]; then
    printf '%s\n' "$ACTRAIL_BUNDLE/openssl"
  else
    fail "openssl executable not found; install openssl in the guest image or bundle ACTRAIL_OPENSSL"
  fi
}

prepare_workdir() {
  name="$1"
  case "$name" in
    *[!A-Za-z0-9._-]*|'') fail "unsafe workdir name: $name" ;;
  esac
  work="$ACTRAIL_WORK_BASE/$name"
  rm -rf "$work"
  mkdir -p "$work"
  printf '%s\n' "$work"
}

mount_guest_observation_fs() {
  mkdir -p /sys/kernel/tracing /sys/fs/bpf 2>/dev/null || true
  mount -t tracefs tracefs /sys/kernel/tracing 2>/dev/null || true
  mount -t bpf bpf /sys/fs/bpf 2>/dev/null || true
}

config_value() {
  key="$1"
  file="$2"
  awk -F= -v key="$key" '
    $1 ~ "^[[:space:]]*" key "[[:space:]]*$" {
      value=$2
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
      gsub(/^"|"$/, "", value)
      print value
      exit
    }
  ' "$file"
}

payload_tls_enabled() {
  file="$1"
  awk '
    /^\[payload\.tls\]/ { in_tls = 1; next }
    /^\[/ { in_tls = 0 }
    in_tls && $1 == "enabled" {
      print $3
      exit
    }
  ' "$file"
}

start_daemon() {
  config="$1"
  [ -f "$config" ] || fail "config missing: $config"
  "$ACTRAILD" --config "$config" stop >/dev/null 2>&1 || true
  socket_path="$(config_value socket_path "$config")"
  tls_socket_path="$(config_value sync_event_socket_path "$config")"
  pid_file="$(config_value pid_file "$config")"
  sqlite_path="$(config_value path "$config" | head -1)"
  [ -n "$socket_path" ] || fail "control socket path missing in $config"
  mkdir -p "$(dirname "$socket_path")"
  [ -n "$tls_socket_path" ] && mkdir -p "$(dirname "$tls_socket_path")"
  [ -n "$pid_file" ] && mkdir -p "$(dirname "$pid_file")"
  [ -n "$sqlite_path" ] && mkdir -p "$(dirname "$sqlite_path")"
  rm -f "$socket_path" "$tls_socket_path" "$pid_file"
  "$ACTRAILD" --config "$config" start

  i=0
  while [ "$i" -lt 80 ]; do
    [ -S "$socket_path" ] && break
    sleep 0.25
    i=$((i + 1))
  done
  [ -S "$socket_path" ] || fail "control socket did not appear: $socket_path"
  if [ "$(payload_tls_enabled "$config")" = "true" ]; then
    i=0
    while [ "$i" -lt 80 ]; do
      [ -S "$tls_socket_path" ] && break
      sleep 0.25
      i=$((i + 1))
    done
    [ -S "$tls_socket_path" ] || fail "tls-sync socket did not appear: $tls_socket_path"
  fi
}

stop_daemon() {
  config="$1"
  "$ACTRAILD" --config "$config" stop >/dev/null 2>&1 || true
}

generate_tls_cert() {
  openssl="$1"
  work="$2"
  "$openssl" req -x509 -newkey rsa:2048 -keyout "$work/key.pem" -out "$work/cert.pem" \
    -days 1 -nodes -subj /CN=localhost >"$work/openssl-cert.log" 2>&1 \
    || { cat "$work/openssl-cert.log"; fail "openssl cert generation failed"; }
}

start_tls_server() {
  openssl="$1"
  work="$2"
  port="$3"
  (printf "SERVER_REPLY_9931\n"; sleep 6) | "$openssl" s_server \
    -accept "$port" -cert "$work/cert.pem" -key "$work/key.pem" -quiet \
    >"$work/srv.out" 2>"$work/srv.err" &
  printf '%s\n' "$!"
}

kill_pid() {
  pid="$1"
  [ -n "$pid" ] || return 0
  kill "$pid" >/dev/null 2>&1 || true
  wait "$pid" >/dev/null 2>&1 || true
}

trace_from_launch_or_viewer() {
  config="$1"
  launch_out="$2"
  awk '/^trace .* entered Active/ { print $2; exit }' "$launch_out"
  "$ACTRAILVIEWER" --config "$config" traces 2>/dev/null | awk '/^trace/ { print $1; exit }'
}

first_trace_id() {
  config="$1"
  launch_out="$2"
  trace_id="$(trace_from_launch_or_viewer "$config" "$launch_out" | awk 'NF { print; exit }')"
  [ -n "$trace_id" ] || fail "no trace id found"
  printf '%s\n' "$trace_id"
}

summary_value() {
  key="$1"
  sed -n "s/.* $key=\([0-9][0-9]*\).*/\1/p" | head -1
}

assert_contains() {
  file="$1"
  pattern="$2"
  grep -q "$pattern" "$file" || {
    echo "-- missing pattern: $pattern" >&2
    echo "-- file: $file" >&2
    cat "$file" >&2
    exit 1
  }
}

assert_not_contains() {
  file="$1"
  pattern="$2"
  if grep -q "$pattern" "$file"; then
    echo "-- unexpected pattern: $pattern" >&2
    echo "-- file: $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

assert_file_equals() {
  file="$1"
  expected="$2"
  actual="$(cat "$file")"
  [ "$actual" = "$expected" ] || {
    echo "-- unexpected file content" >&2
    echo "-- file: $file" >&2
    echo "-- expected: $expected" >&2
    echo "-- actual: $actual" >&2
    cat "$file" >&2
    exit 1
  }
}

assert_tls_payload_contents() {
  config="$1"
  trace_id="$2"
  payloads_file="$3"
  work="$4"

  outbound_segment="$(awk '/TlsUserSpace/ && /SSL_write/ { print $1; exit }' "$payloads_file")"
  inbound_segment="$(awk '/TlsUserSpace/ && /SSL_read/ { print $1; exit }' "$payloads_file")"
  [ -n "$outbound_segment" ] || fail "outbound TLS payload segment not found"
  [ -n "$inbound_segment" ] || fail "inbound TLS payload segment not found"

  "$ACTRAILVIEWER" --config "$config" payload --trace-id "$trace_id" \
    --segment-id "$outbound_segment" --format hex >"$work/payload-outbound.hex" 2>&1
  "$ACTRAILVIEWER" --config "$config" payload --trace-id "$trace_id" \
    --segment-id "$inbound_segment" --format hex >"$work/payload-inbound.hex" 2>&1

  assert_file_equals "$work/payload-outbound.hex" \
    "434c49454e545f5345435245545f4d41524b45525f373738380a"
  assert_file_equals "$work/payload-inbound.hex" \
    "5345525645525f5245504c595f393933310a"
}
