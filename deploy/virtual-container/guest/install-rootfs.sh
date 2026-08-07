#!/usr/bin/env bash
# Install the AcTrail guest service into an unpacked or mounted Kata rootfs.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=otel-endpoint.sh
source "$SCRIPT_DIR/otel-endpoint.sh"
ROOTFS=""
BUNDLE=""
CONFIG="$SCRIPT_DIR/operator.conf"
UNIT="$SCRIPT_DIR/actraild.service"
TMPFILES="$SCRIPT_DIR/actrail-tmpfiles.conf"
INTERFACE_DROP_IN="$SCRIPT_DIR/systemd/workload-interface/kata-agent.service.d/10-actrail-workload-interface.conf"
OTEL_ENDPOINT=""
STARTUP_DEPENDENCY="optional"
WITH_VIEWER=0
SOCKET_GID=39000
MIN_FREE_BYTES=$((32 * 1024 * 1024))

usage() {
  cat <<'EOF'
Usage: install-rootfs.sh --rootfs DIR --bundle DIR --otel-endpoint URL [options]

Options:
  --config FILE                Guest operator config (default: guest/operator.conf)
  --unit FILE                  Guest systemd unit (default: guest/actraild.service)
  --otel-endpoint URL          Guest-reachable OTLP/HTTP traces URL (required)
  --startup-dependency POLICY  optional or required (default: optional)
  --with-viewer                Also install actrailviewer (omitted by default)
  --socket-gid GID             Numeric GID shared with workloads (default: 39000)
  --min-free-mib N             Free-space reserve after installation (default: 32)
  -h, --help                   Show this help

The rootfs must contain systemd, kata-containers.target and kata-agent.service.
The script never accepts / as --rootfs and verifies bundle checksums, ELF
architecture and the target rootfs GLIBC level before writing files.

The OTLP endpoint must use http:// or https:// and name the Collector address
reachable from inside the Guest. Guest 127.0.0.1 is the Guest itself, not the
host. Its path must end in /v1/traces; query strings, fragments and placeholder
endpoints are rejected instead of being installed.

The startup dependency controls only whether kata-agent requires a ready
actraild service. It does not select Agent observation failure behavior.
EOF
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --rootfs)
      [[ "$#" -ge 2 ]] || fail "--rootfs requires a value"
      ROOTFS="$2"
      shift 2
      ;;
    --bundle)
      [[ "$#" -ge 2 ]] || fail "--bundle requires a value"
      BUNDLE="$2"
      shift 2
      ;;
    --config)
      [[ "$#" -ge 2 ]] || fail "--config requires a value"
      CONFIG="$2"
      shift 2
      ;;
    --unit)
      [[ "$#" -ge 2 ]] || fail "--unit requires a value"
      UNIT="$2"
      shift 2
      ;;
    --otel-endpoint)
      [[ "$#" -ge 2 ]] || fail "--otel-endpoint requires a value"
      OTEL_ENDPOINT="$2"
      shift 2
      ;;
    --startup-dependency)
      [[ "$#" -ge 2 ]] || fail "--startup-dependency requires a value"
      STARTUP_DEPENDENCY="$2"
      shift 2
      ;;
    --with-viewer)
      WITH_VIEWER=1
      shift
      ;;
    --socket-gid)
      [[ "$#" -ge 2 ]] || fail "--socket-gid requires a value"
      [[ "$2" =~ ^[0-9]+$ ]] || fail "--socket-gid must be an integer"
      SOCKET_GID="$((10#$2))"
      shift 2
      ;;
    --min-free-mib)
      [[ "$#" -ge 2 ]] || fail "--min-free-mib requires a value"
      [[ "$2" =~ ^[0-9]+$ ]] || fail "--min-free-mib must be an integer"
      MIN_FREE_BYTES=$((10#$2 * 1024 * 1024))
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

for command_name in awk basename chmod df find grep install ln mktemp readelf realpath rm sed sha256sum sort stat tail; do
  command -v "$command_name" >/dev/null 2>&1 || fail "missing command: $command_name"
done

[[ -n "$ROOTFS" ]] || fail "--rootfs is required"
[[ -n "$BUNDLE" ]] || fail "--bundle is required"
actrail_validate_guest_otel_endpoint "$OTEL_ENDPOINT" \
  || fail "$ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
[[ -d "$ROOTFS" ]] || fail "rootfs is not a directory: $ROOTFS"
[[ -d "$BUNDLE" ]] || fail "bundle is not a directory: $BUNDLE"
[[ -f "$CONFIG" ]] || fail "config not found: $CONFIG"
[[ -f "$UNIT" ]] || fail "unit not found: $UNIT"
[[ -f "$TMPFILES" ]] || fail "tmpfiles config not found: $TMPFILES"
[[ -f "$INTERFACE_DROP_IN" ]] \
  || fail "workload-interface drop-in not found: $INTERFACE_DROP_IN"
case "$STARTUP_DEPENDENCY" in
  optional|required) ;;
  *) fail "--startup-dependency must be optional or required" ;;
esac
(( SOCKET_GID > 0 && SOCKET_GID <= 2147483647 )) \
  || fail "--socket-gid must be between 1 and 2147483647"

ROOTFS="$(realpath "$ROOTFS")"
BUNDLE="$(realpath "$BUNDLE")"
CONFIG="$(realpath "$CONFIG")"
UNIT="$(realpath "$UNIT")"
TMPFILES="$(realpath "$TMPFILES")"
INTERFACE_DROP_IN="$(realpath "$INTERFACE_DROP_IN")"
[[ "$ROOTFS" != "/" ]] || fail "refusing to install into /; pass an offline rootfs"

rootfs_target() {
  local relative="${1#/}"
  local target=""
  target="$(realpath -m "$ROOTFS/$relative")"
  case "$target" in
    "$ROOTFS"|"$ROOTFS"/*)
      printf '%s\n' "$target"
      ;;
    *)
      fail "rootfs path escapes through a symbolic link: /$relative -> $target"
      ;;
  esac
}

elf_machine() {
  readelf -h "$1" 2>/dev/null \
    | awk -F: '/^[[:space:]]*Machine:/ {
        sub(/^[[:space:]]+/, "", $2)
        print $2
        exit
      }'
}

elf_max_glibc() {
  readelf --version-info "$1" 2>/dev/null \
    | grep -o 'GLIBC_[0-9][0-9.]*' \
    | sed 's/^GLIBC_//' \
    | sort -Vu \
    | tail -1
}

version_gt() {
  local left="$1"
  local right="$2"
  [[ "$left" != "$right" ]] \
    && [[ "$(printf '%s\n%s\n' "$left" "$right" | sort -V | tail -1)" == "$left" ]]
}

manifest="$BUNDLE/MANIFEST.sha256"
[[ -f "$manifest" ]] || fail "bundle manifest not found: $manifest"
awk '
  NF < 2 || $2 !~ /^\.\// || $2 ~ /(^|\/)\.\.(\/|$)/ { exit 1 }
' "$manifest" || fail "bundle manifest contains an unsafe path"
(
  cd "$BUNDLE"
  sha256sum --strict --check MANIFEST.sha256 >/dev/null
) || fail "bundle checksum verification failed"

required_bundle_files=(
  actraild
  actrailctl
  libactrail_tls_payload_probe_sync.so
  BUNDLE-INFO
  plugins/otel-http/otel-http.plugin.toml
  plugins/otel-http/otel-http.config.toml
  plugins/otel-http/otel-http.config.v1.schema.json
)
if [[ "$WITH_VIEWER" == "1" ]]; then
  required_bundle_files+=(actrailviewer)
fi
for relative in "${required_bundle_files[@]}"; do
  [[ -f "$BUNDLE/$relative" ]] || fail "bundle file missing: $relative"
done
[[ -d "$BUNDLE/lib" ]] || fail "bundle library directory missing: $BUNDLE/lib"

systemd_binary=""
for relative in /usr/lib/systemd/systemd /lib/systemd/systemd; do
  candidate="$(rootfs_target "$relative" 2>/dev/null || true)"
  if [[ -n "$candidate" && -f "$candidate" ]]; then
    systemd_binary="$candidate"
    break
  fi
done
[[ -n "$systemd_binary" ]] || fail "target rootfs does not contain a safe systemd binary"

unit_dir="$(rootfs_target /usr/lib/systemd/system)"
[[ -f "$unit_dir/kata-containers.target" ]] \
  || fail "target rootfs is missing kata-containers.target"
[[ -f "$unit_dir/kata-agent.service" ]] \
  || fail "target rootfs is missing kata-agent.service"

bundle_machine="$(elf_machine "$BUNDLE/actraild")"
rootfs_machine="$(elf_machine "$systemd_binary")"
[[ -n "$bundle_machine" && -n "$rootfs_machine" ]] \
  || fail "cannot determine bundle or rootfs ELF architecture"
[[ "$bundle_machine" == "$rootfs_machine" ]] \
  || fail "architecture mismatch: bundle='$bundle_machine' rootfs='$rootfs_machine'"

lib_search_roots=()
for relative in /usr/lib /usr/lib64 /lib /lib64; do
  candidate="$(rootfs_target "$relative" 2>/dev/null || true)"
  [[ -n "$candidate" && -d "$candidate" ]] && lib_search_roots+=("$candidate")
done
[[ "${#lib_search_roots[@]}" -gt 0 ]] || fail "target rootfs has no library directories"

target_libc=""
while IFS= read -r candidate; do
  [[ "$(elf_machine "$candidate")" == "$bundle_machine" ]] || continue
  target_libc="$candidate"
  break
done < <(find "${lib_search_roots[@]}" -xdev -type f -name libc.so.6 -print 2>/dev/null | LC_ALL=C sort -u)
[[ -n "$target_libc" ]] || fail "cannot find target libc.so.6 for $bundle_machine"
target_glibc="$(elf_max_glibc "$target_libc" || true)"
[[ -n "$target_glibc" ]] || fail "cannot determine target GLIBC level from $target_libc"

elf_inputs=(
  "$BUNDLE/actraild"
  "$BUNDLE/actrailctl"
  "$BUNDLE/libactrail_tls_payload_probe_sync.so"
)
if [[ "$WITH_VIEWER" == "1" ]]; then
  elf_inputs+=("$BUNDLE/actrailviewer")
fi
while IFS= read -r library; do
  elf_inputs+=("$library")
done < <(find "$BUNDLE/lib" -maxdepth 1 -type f -print | LC_ALL=C sort)

required_glibc=""
install_bytes=0
for artifact in "${elf_inputs[@]}"; do
  artifact_machine="$(elf_machine "$artifact")"
  [[ "$artifact_machine" == "$bundle_machine" ]] \
    || fail "ELF architecture mismatch in bundle: $artifact"
  artifact_glibc="$(elf_max_glibc "$artifact" || true)"
  if [[ -n "$artifact_glibc" ]] \
    && { [[ -z "$required_glibc" ]] || version_gt "$artifact_glibc" "$required_glibc"; }; then
    required_glibc="$artifact_glibc"
  fi
  install_bytes=$((install_bytes + $(stat -c '%s' "$artifact")))
done
if [[ -n "$required_glibc" ]] && version_gt "$required_glibc" "$target_glibc"; then
  fail "bundle requires GLIBC_$required_glibc but rootfs provides GLIBC_$target_glibc"
fi

available_bytes="$(df -PB1 "$ROOTFS" | awk 'NR == 2 { print $4 }')"
[[ "$available_bytes" =~ ^[0-9]+$ ]] || fail "cannot determine rootfs free space"
if (( install_bytes + MIN_FREE_BYTES > available_bytes )); then
  fail "insufficient rootfs space: need $install_bytes bytes plus $MIN_FREE_BYTES reserve, have $available_bytes"
fi

grep -Fqx 'sync_runtime_library_path = "/usr/local/lib/actrail/libactrail_tls_payload_probe_sync.so"' "$CONFIG" \
  || fail "config must point sync_runtime_library_path at the guest installation path"
grep -Fqx 'socket_path = "/dev/actrail/control.sock"' "$CONFIG" \
  || fail "config must place the control socket in the workload interface directory"
grep -Fqx 'sync_event_socket_path = "/dev/actrail/tls-sync.sock"' "$CONFIG" \
  || fail "config must place the TLS socket in the workload interface directory"
grep -Fq 'ExecStart=/usr/local/bin/actraild ' "$UNIT" \
  || fail "unit does not use the guest actraild installation path"
grep -Fqx \
  'ExecStartPre=/usr/bin/systemd-tmpfiles --create --prefix=/dev/actrail' \
  "$UNIT" \
  || fail "unit does not materialize its own workload interface"
grep -Fq '/usr/bin/touch /run/actrail/ready' "$UNIT" \
  || fail "unit does not publish the guest readiness marker"
grep -Fqx 'WantedBy=multi-user.target kata-containers.target' "$UNIT" \
  || fail "unit is not enabled for both VM and Kata targets"
grep -Fqx 'Group=actrail' "$UNIT" \
  || fail "unit must use the dedicated actrail socket group"
grep -Fqx 'd /dev/actrail 0750 root actrail -' "$TMPFILES" \
  || fail "tmpfiles config must create the workload interface before kata-agent"
grep -Fqx \
  'ExecStartPre=/usr/bin/systemd-tmpfiles --create --prefix=/dev/actrail' \
  "$INTERFACE_DROP_IN" \
  || fail "kata-agent drop-in must materialize the workload interface"

bin_dir="$(rootfs_target /usr/local/bin)"
private_lib_dir="$(rootfs_target /usr/local/lib/actrail)"
config_dir="$(rootfs_target /etc/actrail)"
plugin_config_dir="$(rootfs_target /etc/actrail/plugins/otel-http)"
share_dir="$(rootfs_target /usr/share/actrail)"
plugin_manifest_dir="$(rootfs_target /usr/share/actrail/plugins/otel-http)"
tmpfiles_dir="$(rootfs_target /usr/lib/tmpfiles.d)"
group_file="$(rootfs_target /etc/group)"
[[ -f "$group_file" ]] || fail "target rootfs is missing /etc/group"

existing_group_gid="$(
  awk -F: '$1 == "actrail" { print $3; exit }' "$group_file"
)"
if [[ -n "$existing_group_gid" ]]; then
  [[ "$existing_group_gid" =~ ^[0-9]+$ ]] \
    || fail "target rootfs has an invalid actrail group entry"
  [[ "$existing_group_gid" == "$SOCKET_GID" ]] \
    || fail "target rootfs actrail group uses GID $existing_group_gid, expected $SOCKET_GID"
elif awk -F: -v gid="$SOCKET_GID" '$3 == gid { found = 1 } END { exit !found }' "$group_file"; then
  conflicting_group="$(
    awk -F: -v gid="$SOCKET_GID" '$3 == gid { print $1; exit }' "$group_file"
  )"
  fail "target rootfs GID $SOCKET_GID is already used by group $conflicting_group"
else
  printf 'actrail:x:%s:\n' "$SOCKET_GID" >>"$group_file"
fi

install -d -m 0755 \
  "$bin_dir" \
  "$private_lib_dir" \
  "$config_dir" \
  "$plugin_config_dir" \
  "$share_dir" \
  "$plugin_manifest_dir" \
  "$tmpfiles_dir" \
  "$unit_dir"
actrail_write_guest_otel_endpoint_config \
  "$BUNDLE/plugins/otel-http/otel-http.config.toml" \
  "$plugin_config_dir/otel-http.config.toml" \
  "$OTEL_ENDPOINT" \
  || fail "$ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
install -m 0755 "$BUNDLE/actraild" "$bin_dir/actraild"
install -m 0755 "$BUNDLE/actrailctl" "$bin_dir/actrailctl"
install -m 0755 "$BUNDLE/libactrail_tls_payload_probe_sync.so" \
  "$private_lib_dir/libactrail_tls_payload_probe_sync.so"
if [[ "$WITH_VIEWER" == "1" ]]; then
  install -m 0755 "$BUNDLE/actrailviewer" "$bin_dir/actrailviewer"
fi
while IFS= read -r library; do
  install -m 0644 "$library" "$private_lib_dir/$(basename "$library")"
done < <(find "$BUNDLE/lib" -maxdepth 1 -type f -print | LC_ALL=C sort)
install -m 0640 "$CONFIG" "$config_dir/operator.conf"
install -m 0644 \
  "$BUNDLE/plugins/otel-http/otel-http.plugin.toml" \
  "$BUNDLE/plugins/otel-http/otel-http.config.v1.schema.json" \
  "$plugin_manifest_dir/"
install -m 0644 "$UNIT" "$unit_dir/actraild.service"
install -m 0644 "$TMPFILES" "$tmpfiles_dir/actrail.conf"

for target in kata-containers.target multi-user.target; do
  wants_dir="$unit_dir/$target.wants"
  install -d -m 0755 "$wants_dir"
  ln -sfn ../actraild.service "$wants_dir/actraild.service"
done

dependency_dir="$unit_dir/kata-agent.service.d"
interface_target="$dependency_dir/10-actrail-workload-interface.conf"
install -d -m 0755 "$dependency_dir"
install -m 0644 "$INTERFACE_DROP_IN" "$interface_target"

required_source="$SCRIPT_DIR/systemd/required/kata-agent.service.d/20-actrail-required.conf"
required_target="$dependency_dir/20-actrail-required.conf"
if [[ "$STARTUP_DEPENDENCY" == "required" ]]; then
  [[ -f "$required_source" ]] \
    || fail "required dependency drop-in not found: $required_source"
  install -m 0644 "$required_source" "$required_target"
elif [[ -e "$required_target" || -L "$required_target" ]]; then
  rm -f "$required_target"
fi

install_info="$share_dir/guest-install-info"
{
  printf 'format=1\n'
  printf 'guest_startup_dependency=%s\n' "$STARTUP_DEPENDENCY"
  printf 'workload_socket_group=actrail\n'
  printf 'workload_socket_gid=%s\n' "$SOCKET_GID"
  printf 'bundle_machine=%s\n' "$bundle_machine"
  printf 'bundle_required_glibc=%s\n' "${required_glibc:-none}"
  printf 'rootfs_glibc=%s\n' "$target_glibc"
  printf 'viewer_installed=%s\n' "$WITH_VIEWER"
} >"$install_info"
chmod 0644 "$install_info"

workload_interface="$share_dir/workload-interface"
{
  printf 'format=1\n'
  printf 'guest_socket_source=/dev/actrail\n'
  printf 'workload_socket_target=/run/actrail\n'
  printf 'socket_group=actrail\n'
  printf 'socket_gid=%s\n' "$SOCKET_GID"
  printf 'socket_mode=0660\n'
  printf 'socket_directory_mode=0750\n'
  printf 'tool_target=/opt/actrail\n'
} >"$workload_interface"
chmod 0644 "$workload_interface"

echo "ACTRAIL_GUEST_ROOTFS_INSTALLED"
echo "rootfs=$ROOTFS"
echo "guest_startup_dependency=$STARTUP_DEPENDENCY"
echo "bundle_machine=$bundle_machine"
echo "bundle_required_glibc=${required_glibc:-none}"
echo "rootfs_glibc=$target_glibc"
echo "viewer_installed=$WITH_VIEWER"
echo "workload_socket_gid=$SOCKET_GID"
echo "otel_endpoint_configured=true"
