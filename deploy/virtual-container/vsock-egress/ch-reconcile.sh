#!/usr/bin/env bash
# Reconcile the per-sandbox Cloud Hypervisor VSOCK bridges with the sandboxes
# that actually exist.
#
# Cloud Hypervisor uses hybrid VSOCK: the Guest connects to CID 2 on a port, and
# the VMM forwards that to "<vm base UDS>_<port>" on the Host. The base UDS lives
# in a per-sandbox directory Kata creates and removes on its own, so a bridge
# cannot be a single node-wide listener the way it is for StratoVirt, and it
# cannot be configured by hand for sandboxes that do not exist yet.
#
# This script is idempotent and makes no assumption about ordering: a bridge may
# start after its sandbox, because the Guest exporter retries.
set -euo pipefail

VM_ROOT="${ACTRAIL_VSOCK_VM_ROOT:-/run/vc/vm}"
UNIT_PREFIX="actrail-vsock-host-cloud-hypervisor"
SYSTEMCTL="${ACTRAIL_VSOCK_SYSTEMCTL:-systemctl}"

declare -A desired=()

# A sandbox counts only when it exposes a Cloud Hypervisor base socket: other
# VMMs and half-created directories must not get a bridge.
if [[ -d "$VM_ROOT" ]]; then
    for base in "$VM_ROOT"/*/clh.sock; do
        [[ -S "$base" ]] || continue
        sandbox_dir="$(dirname -- "$base")"
        sandbox="$(basename -- "$sandbox_dir")"
        [[ -n "$sandbox" ]] || continue
        instance="$(systemd-escape -- "$sandbox")"
        desired["$instance"]=1
    done
fi

for instance in "${!desired[@]}"; do
    "$SYSTEMCTL" start "${UNIT_PREFIX}@${instance}.service" || true
done

# Stop bridges whose sandbox is gone. socat holds its listening socket open even
# after Kata removes the directory, so nothing else would retire the instance.
active_units="$("$SYSTEMCTL" list-units --plain --no-legend --state=active \
    "${UNIT_PREFIX}@*.service" 2>/dev/null | awk '{print $1}')" || active_units=""

while IFS= read -r unit; do
    [[ -n "$unit" ]] || continue
    instance="${unit#"${UNIT_PREFIX}@"}"
    instance="${instance%.service}"
    [[ -n "$instance" ]] || continue
    if [[ -z "${desired[$instance]:-}" ]]; then
        "$SYSTEMCTL" stop "$unit" || true
    fi
done <<<"$active_units"
