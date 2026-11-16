#!/usr/bin/env bash
# Runs the Btrfs backend end-to-end tests on a loopback Btrfs filesystem.
# Run as root. The tests are built as ACTRAIL_BUILD_USER (default: SUDO_USER), so cargo never
# writes to that user's toolchain or target directory as root.
set -euo pipefail

case_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$case_dir/../../../../.." && pwd)
build_user=${ACTRAIL_BUILD_USER:-${SUDO_USER:?set ACTRAIL_BUILD_USER to the user that owns the Rust toolchain}}
runtime_dir=$(mktemp -d /var/tmp/actrail-btrfs-backend.XXXXXX)
mount_root="$runtime_dir/mnt"
workspace_root="$mount_root/workspaces"
workspace="$workspace_root/main"
snapshot_root="$mount_root/.actrail"
config="$runtime_dir/backend-config.json"

cleanup() {
    if umount "$mount_root" 2>/dev/null || ! mountpoint -q "$mount_root"; then
        rm -rf "$runtime_dir"
    fi
}
trap cleanup EXIT

test_binary=$(
    runuser -u "$build_user" -- bash -lc \
        "cd '$repo_dir' && cargo test -p actrail_ckpt_fs_btrfs --test real_backend --no-run --message-format=json" |
        python3 -c '
import json, sys
for line in sys.stdin:
    if line.startswith("{"):
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message["target"]["name"] == "real_backend" and message.get("executable"):
            print(message["executable"])
'
)

modprobe btrfs
truncate -s 1G "$runtime_dir/filesystem.img"
mkfs.btrfs -q "$runtime_dir/filesystem.img"
mkdir -p "$mount_root"
mount -o loop "$runtime_dir/filesystem.img" "$mount_root"
mkdir -p "$workspace_root" "$snapshot_root"
btrfs subvolume create "$workspace" >/dev/null
# Connect refuses a workspace with nested subvolumes; tests create theirs after connecting.

cat >"$config" <<EOF
{
  "btrfs_path": "$(command -v btrfs)",
  "findmnt_path": "$(command -v findmnt)",
  "copy_path": "$(command -v cp)",
  "snapshot_root": "$snapshot_root",
  "allowed_workspace_roots": ["$workspace_root"]
}
EOF

ACTRAIL_BTRFS_TEST_CONFIG="$config" ACTRAIL_BTRFS_TEST_ROOT="$workspace" \
    "$test_binary" --ignored --test-threads=1
