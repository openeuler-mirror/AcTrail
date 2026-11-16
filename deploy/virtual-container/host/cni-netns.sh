#!/usr/bin/env bash
# Create or remove one CNI-configured network namespace for a Kata test VM.
#
# `ctr run` starts a Kata sandbox without any Guest network. Kubernetes instead
# has the CRI configure a network namespace before the sandbox exists, and Kata
# then mirrors that interface into the Guest. This helper reproduces that order
# for the V2 acceptance run, so `--egress-mode network` can be verified on a
# single node:
#
#   VIRTUAL_CONTAINER_E2E_NETWORK_NAMESPACE_HELPER=<this script>
#   ACTRAIL_TEST_CNI_CONFIG=/etc/actrail-cni-test/net.json
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: cni-netns.sh add|del NAME

Environment:
  ACTRAIL_TEST_CNI_CONFIG  CNI network configuration with one plugin (required)
  ACTRAIL_TEST_CNI_PATH    Plugin search path (default: /opt/cni/bin:/usr/libexec/cni)
  ACTRAIL_TEST_CNI_IFNAME  Interface name inside the namespace (default: eth0)

`add` prints the namespace path on its last stdout line.
USAGE
}

fail() {
  echo "cni-netns: $*" >&2
  exit 2
}

[[ "$#" -eq 2 ]] || { usage >&2; exit 2; }
ACTION="$1"
NAME="$2"
[[ "$NAME" =~ ^[A-Za-z0-9][A-Za-z0-9_.-]{0,62}$ ]] \
  || fail "unsafe namespace name: $NAME"

CNI_CONFIG="${ACTRAIL_TEST_CNI_CONFIG:-}"
[[ -n "$CNI_CONFIG" ]] || fail "ACTRAIL_TEST_CNI_CONFIG is required"
[[ -f "$CNI_CONFIG" ]] || fail "CNI configuration does not exist: $CNI_CONFIG"
CNI_SEARCH_PATH="${ACTRAIL_TEST_CNI_PATH:-/opt/cni/bin:/usr/libexec/cni}"
NETNS_PATH="/run/netns/$NAME"

plugin_type="$(
  python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["type"])' \
    "$CNI_CONFIG" 2>/dev/null
)" || fail "CNI configuration has no single-plugin \"type\": $CNI_CONFIG"
[[ "$plugin_type" =~ ^[A-Za-z0-9_.-]+$ ]] \
  || fail "unsafe CNI plugin type: $plugin_type"
plugin=""
IFS=: read -r -a plugin_dirs <<<"$CNI_SEARCH_PATH"
for directory in "${plugin_dirs[@]}"; do
  if [[ -x "$directory/$plugin_type" ]]; then
    plugin="$directory/$plugin_type"
    break
  fi
done
[[ -n "$plugin" ]] || fail "CNI plugin $plugin_type not found in $CNI_SEARCH_PATH"

run_plugin() {
  CNI_COMMAND="$1" \
  CNI_CONTAINERID="$NAME" \
  CNI_NETNS="$NETNS_PATH" \
  CNI_IFNAME="${ACTRAIL_TEST_CNI_IFNAME:-eth0}" \
  CNI_PATH="$CNI_SEARCH_PATH" \
    "$plugin" <"$CNI_CONFIG"
}

case "$ACTION" in
  add)
    ip netns add "$NAME"
    if ! run_plugin ADD >/dev/null; then
      ip netns del "$NAME" || true
      fail "CNI ADD failed for $NAME"
    fi
    echo "$NETNS_PATH"
    ;;
  del)
    # Both steps are best effort so a half-created namespace is still removed.
    run_plugin DEL >/dev/null 2>&1 || true
    ip netns del "$NAME" 2>/dev/null || true
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
