#!/usr/bin/env bash
# Install the official Kata Containers 3.32.0 release beside distro RPMs.
# The host architecture selects which official archive is accepted.
set -euo pipefail

VERSION="3.32.0"
ARM64_ARCHIVE_SHA256="8736c054d9223974735394f822000823baef509e1c33405ec798240fa9b6e4b5"
AMD64_ARCHIVE_SHA256="1449ecea50bd91fa73a94648db195d18950fe869ba4b1f12d05f55f1fa7c1b01"
ARCHIVE_SHA256=""
ARCH_LABEL=""
ELF_MACHINE=""
PREFIX="/opt/kata-$VERSION"
ACTIVE_LINK="/opt/kata"
LOCAL_BIN="/usr/local/bin"
VERSIONED_RUNTIME="io.containerd.kata332.v2"
VERSIONED_SHIM="containerd-shim-kata332-v2"
ARCHIVE=""
ACTIVATE=1
STAGING=""

usage() {
  cat <<'EOF'
Usage:
  sudo ./install-kata-3.32.sh --archive kata-static-3.32.0-arm64.tar.zst
  sudo ./install-kata-3.32.sh --archive kata-static-3.32.0-amd64.tar.zst

Options:
  --archive FILE  Official Kata 3.32.0 release archive matching the host
                  architecture: arm64 on aarch64, amd64 on x86_64
  --no-activate   Install /opt/kata-3.32.0 without changing runtime symlinks
  -h, --help      Show this help

The distro Kata package under /usr is preserved. Activation creates:
  /opt/kata -> /opt/kata-3.32.0
  /usr/local/bin/containerd-shim-kata-v2 -> /opt/kata/bin/...
  /usr/local/bin/containerd-shim-kata332-v2 -> /opt/kata/bin/...
  /usr/local/bin/kata-runtime -> /opt/kata/bin/...

Use containerd runtime io.containerd.kata332.v2 to avoid a distro-owned
/usr/bin/containerd-shim-kata-v2 taking precedence in the daemon's PATH.
Removing those four symlinks rolls command lookup back to the distro package;
the installer never overwrites /usr/bin binaries.
EOF
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

activate_link() {
  local link_path="$1"
  local target_path="$2"

  if [[ -L "$link_path" ]]; then
    [[ "$(readlink -f "$link_path")" == "$(readlink -f "$target_path")" ]] \
      || fail "$link_path already points to another binary"
  elif [[ -e "$link_path" ]]; then
    fail "$link_path already exists and is not a symlink"
  else
    ln -s "$target_path" "$link_path"
  fi
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --archive)
      [[ "$#" -ge 2 ]] || fail "--archive requires a value"
      ARCHIVE="$2"
      shift 2
      ;;
    --no-activate)
      ACTIVATE=0
      shift
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

[[ "$(id -u)" -eq 0 ]] || fail "run this installer with sudo"
# Each architecture has exactly one accepted official archive. Selecting the
# digest from the host architecture makes an archive built for the other
# architecture fail the integrity check instead of installing binaries the host
# cannot run.
case "$(uname -m)" in
  aarch64)
    ARCH_LABEL="arm64"
    ARCHIVE_SHA256="$ARM64_ARCHIVE_SHA256"
    ELF_MACHINE="ARM aarch64"
    ;;
  x86_64)
    ARCH_LABEL="amd64"
    ARCHIVE_SHA256="$AMD64_ARCHIVE_SHA256"
    ELF_MACHINE="x86-64"
    ;;
  *)
    fail "unsupported host architecture: $(uname -m)"
    ;;
esac
[[ -n "$ARCHIVE" ]] || fail "--archive is required"
[[ -f "$ARCHIVE" ]] || fail "archive not found: $ARCHIVE"

for command_name in awk file grep install ln mkdir mktemp mv readlink rm rmdir sha256sum tar tr; do
  command -v "$command_name" >/dev/null 2>&1 \
    || fail "missing host command: $command_name"
done

ARCHIVE="$(readlink -f "$ARCHIVE")"
actual_sha256="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
[[ "$actual_sha256" == "$ARCHIVE_SHA256" ]] \
  || fail "archive SHA256 mismatch for $ARCH_LABEL: $actual_sha256"

cleanup() {
  local rc=$?
  set +e
  if [[ -n "$STAGING" && "$STAGING" == /opt/.kata-3.32.0.install.* ]]; then
    rm -rf -- "$STAGING"
  fi
  exit "$rc"
}
trap cleanup EXIT INT TERM

if [[ -e "$PREFIX" || -L "$PREFIX" ]]; then
  [[ -f "$PREFIX/VERSION" ]] \
    || fail "existing prefix is not a recognizable Kata install: $PREFIX"
  [[ "$(tr -d '[:space:]' <"$PREFIX/VERSION")" == "$VERSION" ]] \
    || fail "existing prefix has the wrong Kata version: $PREFIX"
  echo "reuse_existing_prefix=$PREFIX"
else
  STAGING="$(mktemp -d "/opt/.kata-3.32.0.install.XXXXXX")"
  tar --zstd -xf "$ARCHIVE" -C "$STAGING"
  extracted="$STAGING/opt/kata"
  [[ -f "$extracted/VERSION" ]] || fail "archive has no Kata VERSION file"
  [[ "$(tr -d '[:space:]' <"$extracted/VERSION")" == "$VERSION" ]] \
    || fail "archive contains an unexpected Kata version"
  [[ -x "$extracted/bin/containerd-shim-kata-v2" ]] \
    || fail "archive has no executable Go shim-v2"
  [[ -x "$extracted/bin/kata-runtime" ]] \
    || fail "archive has no executable kata-runtime"
  [[ -f "$extracted/share/defaults/kata-containers/configuration-stratovirt.toml" ]] \
    || fail "archive has no StratoVirt runtime configuration"
  [[ -x "$extracted/bin/cloud-hypervisor" ]] \
    || fail "archive has no executable Cloud Hypervisor"
  [[ -f "$extracted/share/defaults/kata-containers/configuration-clh.toml" ]] \
    || fail "archive has no Cloud Hypervisor runtime configuration"
  [[ -x "$extracted/bin/firecracker" ]] \
    || fail "archive has no executable Firecracker"
  [[ -x "$extracted/bin/jailer" ]] \
    || fail "archive has no executable Firecracker jailer"
  [[ -f "$extracted/share/defaults/kata-containers/configuration-fc.toml" ]] \
    || fail "archive has no Firecracker runtime configuration"
  [[ -f "$extracted/share/kata-containers/vmlinux.container" ]] \
    || fail "archive has no uncompressed Firecracker guest kernel"
  [[ -f "$extracted/share/kata-containers/kata-containers.img" ]] \
    || fail "archive has no Firecracker guest rootfs image"
  [[ -e "$extracted/share/kata-containers/kata-containers-initrd.img" ]] \
    || fail "archive has no reference Kata initrd"
  file "$extracted/bin/containerd-shim-kata-v2" | grep -Fq "$ELF_MACHINE" \
    || fail "archive shim is not an $ARCH_LABEL executable"
  file "$extracted/bin/firecracker" | grep -Fq "$ELF_MACHINE" \
    || fail "archive Firecracker is not an $ARCH_LABEL executable"
  file "$extracted/bin/jailer" | grep -Fq "$ELF_MACHINE" \
    || fail "archive Firecracker jailer is not an $ARCH_LABEL executable"
  "$extracted/bin/containerd-shim-kata-v2" --version | grep -Fq "version: $VERSION" \
    || fail "archive shim version check failed"
  "$extracted/bin/kata-runtime" version | grep -Fq "kata-runtime  : $VERSION" \
    || fail "archive runtime version check failed"
  mv "$extracted" "$PREFIX"
  rmdir "$STAGING/opt" "$STAGING"
  STAGING=""
fi

[[ -x "$PREFIX/bin/cloud-hypervisor" ]] \
  || fail "installed prefix has no executable Cloud Hypervisor"
[[ -f "$PREFIX/share/defaults/kata-containers/configuration-clh.toml" ]] \
  || fail "installed prefix has no Cloud Hypervisor runtime configuration"
file "$PREFIX/bin/cloud-hypervisor" | grep -Fq "$ELF_MACHINE" \
  || fail "installed Cloud Hypervisor is not an $ARCH_LABEL executable"
[[ -x "$PREFIX/bin/firecracker" ]] \
  || fail "installed prefix has no executable Firecracker"
[[ -x "$PREFIX/bin/jailer" ]] \
  || fail "installed prefix has no executable Firecracker jailer"
[[ -f "$PREFIX/share/defaults/kata-containers/configuration-fc.toml" ]] \
  || fail "installed prefix has no Firecracker runtime configuration"
[[ -f "$PREFIX/share/kata-containers/vmlinux.container" ]] \
  || fail "installed prefix has no uncompressed Firecracker guest kernel"
[[ -f "$PREFIX/share/kata-containers/kata-containers.img" ]] \
  || fail "installed prefix has no Firecracker guest rootfs image"
file "$PREFIX/bin/firecracker" | grep -Fq "$ELF_MACHINE" \
  || fail "installed Firecracker is not an $ARCH_LABEL executable"
file "$PREFIX/bin/jailer" | grep -Fq "$ELF_MACHINE" \
  || fail "installed Firecracker jailer is not an $ARCH_LABEL executable"

if [[ "$ACTIVATE" == "1" ]]; then
  containerd_pid="$(pidof containerd 2>/dev/null | awk '{print $1}')"
  if [[ -n "$containerd_pid" && -r "/proc/$containerd_pid/environ" ]]; then
    containerd_path="$(tr '\0' '\n' <"/proc/$containerd_pid/environ" \
      | awk -F= '$1 == "PATH" {sub(/^PATH=/, ""); print; exit}')"
    case ":$containerd_path:" in
      *:/usr/local/bin:*) ;;
      *)
        fail "running containerd PATH does not include /usr/local/bin: $containerd_path"
        ;;
    esac
  fi

  if [[ -L "$ACTIVE_LINK" ]]; then
    [[ "$(readlink -f "$ACTIVE_LINK")" == "$PREFIX" ]] \
      || fail "$ACTIVE_LINK already points to another installation"
  elif [[ -e "$ACTIVE_LINK" ]]; then
    fail "$ACTIVE_LINK already exists and is not a symlink"
  else
    ln -s "kata-$VERSION" "$ACTIVE_LINK"
  fi

  install -d -m 0755 "$LOCAL_BIN"
  activate_link \
    "$LOCAL_BIN/containerd-shim-kata-v2" \
    "$ACTIVE_LINK/bin/containerd-shim-kata-v2"
  activate_link \
    "$LOCAL_BIN/$VERSIONED_SHIM" \
    "$ACTIVE_LINK/bin/containerd-shim-kata-v2"
  activate_link \
    "$LOCAL_BIN/kata-runtime" \
    "$ACTIVE_LINK/bin/kata-runtime"

fi

trap - EXIT INT TERM

echo "KATA_STATIC_INSTALL_OK"
echo "version=$VERSION"
echo "prefix=$PREFIX"
echo "archive_sha256=$actual_sha256"
echo "activated=$ACTIVATE"
echo "containerd_runtime=$VERSIONED_RUNTIME"
"$PREFIX/bin/containerd-shim-kata-v2" --version
"$PREFIX/bin/kata-runtime" version
