#!/usr/bin/env bash
# Build, cache and prepare the openEuler Kata VSOCK deployment from one checkout.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"
KATA_PREFIX="/opt/kata"
BACKEND="stratovirt"
BASE_CONFIG_SOURCE=""
DATA_CONFIG_SOURCE=""
BASE_IMAGE_SOURCE=""
DATA_IMAGE_SOURCE=""
DATA_KERNEL=""
XIAOO=""
WORKLOAD_BASE_IMAGE="docker.io/openeuler/openeuler:24.03-lts"
WORKLOAD_IMAGE="docker.io/library/actrail-openeuler-workload:24.03"
WORKLOAD_IMAGE_ARCHIVE=""
OTEL_ENDPOINT="http://127.0.0.1:14318/v1/traces"
EGRESS_MODE="vsock-bridge"
ENDPOINT_EXPLICIT=0
INSTALL_PACKAGES=1
PACKAGE_MANAGER="${ACTRAIL_PACKAGE_MANAGER:-auto}"
HOST_OS_ID="unknown"
BUILD_WORKLOAD=1
REBUILD_WORKLOAD=0
RUN_TESTS=0
DRY_RUN=0

usage() {
  cat <<'EOF'
Usage:
  sudo -E deploy/virtual-container/host/deploy-openeuler-vsock.sh \
    --base-image-source /path/to/openeuler-kata.image \
    --data-kernel /path/to/bootable-btf-vmlinuz \
    --xiaoo /path/to/xiaoo \
    --run-tests

Required deployment assets:
  --base-image-source FILE   Clean, bootable openEuler Kata Guest image
  --data-kernel FILE         Bootable Kata kernel with BTF/eBPF support

Options:
  --data-image-source FILE   Data Guest source image (default: base image)
  --xiaoo FILE               xiaoO executable; required with --run-tests
  --backend NAME             stratovirt (default) or cloud-hypervisor
  --kata-prefix DIR          Installed Kata 3.32 prefix (default: /opt/kata)
  --base-config-source FILE  Kata base config (default: selected backend config)
  --data-config-source FILE  Kata data config (default: base config)
  --workload-base-image REF  openEuler image used by podman build
  --workload-image REF       Resulting containerd workload image reference
  --workload-image-archive FILE
                             Reusable OCI archive location under local/kata by default
  --otel-endpoint URL        Guest OTLP/HTTP endpoint
  --egress-mode MODE         vsock-bridge (default) or network
  --package-manager NAME     auto (default), dnf, or apt-get
  --skip-packages            Do not install podman and socat at all
  --skip-workload-build      Require the workload image to exist in containerd
  --rebuild-workload         Rebuild and atomically replace the OCI archive
  --run-tests                Run both public virtual-container V2 cases after prepare
  --dry-run                  Print the deployment commands without changing the host
  -h, --help                 Show this help

Cache behavior:
  - podman reuses its layer cache and an existing workload image;
  - containerd reuses the imported workload image;
  - prepare-v2-test-artifacts.py reuses local/kata/artifacts/<digest> when every
    bound input is unchanged;
  - Kata containers/VMs created by --run-tests are intentionally removed.

Image behavior:
  - the clean source image is copied and never modified in place;
  - AcTrail binaries, services and the selected egress configuration are
    injected automatically into content-addressed base/data output images;
  - Kata 3.32.0, the matching clean Guest image, bootable BTF kernel and optional
    xiaoO binary are architecture-bound inputs that must be supplied offline.

This entrypoint prepares a checkout-local acceptance deployment. It does not
install an architecture-specific Kata release, publish a signed Guest image,
or create a Kubernetes RuntimeClass.
EOF
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

print_command() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
}

run() {
  print_command "$@"
  if [[ "$DRY_RUN" == "0" ]]; then
    "$@"
  fi
}

require_file() {
  local label="$1"
  local path="$2"
  [[ -n "$path" ]] || fail "$label is required"
  if [[ "$DRY_RUN" == "0" ]]; then
    [[ "$path" == /* ]] || fail "$label must be an absolute path: $path"
    [[ -f "$path" ]] || fail "$label does not exist: $path"
  fi
}

require_directory() {
  local label="$1"
  local path="$2"
  if [[ "$DRY_RUN" == "0" ]]; then
    [[ "$path" == /* ]] || fail "$label must be an absolute path: $path"
    [[ -d "$path" ]] || fail "$label does not exist: $path"
  fi
}

activate_link() {
  local link_path="$1"
  local target_path="$2"

  if [[ "$DRY_RUN" == "1" ]]; then
    print_command ln -s "$target_path" "$link_path"
    return
  fi
  if [[ -L "$link_path" ]]; then
    [[ "$(readlink -f "$link_path")" == "$(readlink -f "$target_path")" ]] \
      || fail "$link_path points to another Kata installation"
  elif [[ -e "$link_path" ]]; then
    fail "$link_path exists and is not a symlink"
  else
    run ln -s "$target_path" "$link_path"
  fi
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --base-image-source)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      BASE_IMAGE_SOURCE="$2"
      shift 2
      ;;
    --data-image-source)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      DATA_IMAGE_SOURCE="$2"
      shift 2
      ;;
    --data-kernel)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      DATA_KERNEL="$2"
      shift 2
      ;;
    --xiaoo)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      XIAOO="$2"
      shift 2
      ;;
    --backend)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      BACKEND="$2"
      shift 2
      ;;
    --kata-prefix)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      KATA_PREFIX="$2"
      shift 2
      ;;
    --base-config-source)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      BASE_CONFIG_SOURCE="$2"
      shift 2
      ;;
    --data-config-source)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      DATA_CONFIG_SOURCE="$2"
      shift 2
      ;;
    --workload-base-image)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      WORKLOAD_BASE_IMAGE="$2"
      shift 2
      ;;
    --workload-image)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      WORKLOAD_IMAGE="$2"
      shift 2
      ;;
    --workload-image-archive)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      WORKLOAD_IMAGE_ARCHIVE="$2"
      shift 2
      ;;
    --otel-endpoint)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      OTEL_ENDPOINT="$2"
      ENDPOINT_EXPLICIT=1
      shift 2
      ;;
    --egress-mode)
      [[ "$#" -ge 2 ]] || fail "$1 requires a value"
      EGRESS_MODE="$2"
      shift 2
      ;;
    --package-manager)
      [[ $# -ge 2 ]] || fail "--package-manager requires a value"
      PACKAGE_MANAGER=$2
      shift 2
      ;;
    --skip-packages)
      INSTALL_PACKAGES=0
      shift
      ;;
    --skip-workload-build)
      BUILD_WORKLOAD=0
      shift
      ;;
    --rebuild-workload)
      REBUILD_WORKLOAD=1
      shift
      ;;
    --run-tests)
      RUN_TESTS=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
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

case "$BACKEND" in
  stratovirt|cloud-hypervisor) ;;
  *) fail "--backend must be stratovirt or cloud-hypervisor" ;;
esac
case "$EGRESS_MODE" in
  network|vsock-bridge) ;;
  *) fail "--egress-mode must be network or vsock-bridge" ;;
esac
case "$PACKAGE_MANAGER" in
  auto|dnf|apt-get) ;;
  *) fail "--package-manager must be auto, dnf, or apt-get" ;;
esac
if [[ "$EGRESS_MODE" == "network" && "$ENDPOINT_EXPLICIT" == "0" ]]; then
  fail "network egress requires an explicit --otel-endpoint"
fi
if [[ "$BUILD_WORKLOAD" == "0" && "$REBUILD_WORKLOAD" == "1" ]]; then
  fail "--skip-workload-build conflicts with --rebuild-workload"
fi

DATA_IMAGE_SOURCE="${DATA_IMAGE_SOURCE:-$BASE_IMAGE_SOURCE}"
config_name="configuration-stratovirt.toml"
if [[ "$BACKEND" == "cloud-hypervisor" ]]; then
  config_name="configuration-clh.toml"
fi
BASE_CONFIG_SOURCE="${BASE_CONFIG_SOURCE:-$KATA_PREFIX/share/defaults/kata-containers/$config_name}"
DATA_CONFIG_SOURCE="${DATA_CONFIG_SOURCE:-$BASE_CONFIG_SOURCE}"
WORKLOAD_IMAGE_ARCHIVE="${WORKLOAD_IMAGE_ARCHIVE:-$ROOT_DIR/local/kata/workload/actrail-openeuler-workload-24.03.oci.tar}"

require_directory "Kata prefix" "$KATA_PREFIX"
require_file "base Guest image" "$BASE_IMAGE_SOURCE"
require_file "data Guest image" "$DATA_IMAGE_SOURCE"
require_file "data kernel" "$DATA_KERNEL"
require_file "base Kata config" "$BASE_CONFIG_SOURCE"
require_file "data Kata config" "$DATA_CONFIG_SOURCE"
if [[ -n "$XIAOO" ]]; then
  require_file "xiaoO executable" "$XIAOO"
elif [[ "$RUN_TESTS" == "1" ]]; then
  fail "--xiaoo is required with --run-tests"
fi

# The openEuler requirement belongs to the Guest image and the workload image,
# which are arguments. The host only has to provide KVM, a package manager for
# podman/socat, and the offline Kata assets, so record its distribution instead
# of refusing to run on it. Read ID in a subshell: os-release also defines
# NAME/VERSION and must not leak into this script's variables.
HOST_OS_ID="$(
  ( [[ -r /etc/os-release ]] && . /etc/os-release && printf '%s' "${ID:-}" ) \
    2>/dev/null || true
)"
[[ -n "$HOST_OS_ID" ]] || HOST_OS_ID=unknown

if [[ "$DRY_RUN" == "0" ]]; then
  [[ "$(id -u)" == "0" ]] || fail "run with sudo -E"
  [[ -c /dev/kvm && -r /dev/kvm && -w /dev/kvm ]] \
    || fail "readable/writable /dev/kvm is required"
fi

if [[ "$INSTALL_PACKAGES" == "1" ]]; then
  if [[ "$PACKAGE_MANAGER" == "auto" ]]; then
    if command -v dnf >/dev/null 2>&1; then
      PACKAGE_MANAGER=dnf
    elif command -v apt-get >/dev/null 2>&1; then
      PACKAGE_MANAGER=apt-get
    else
      fail "no supported package manager found; install podman and socat, then rerun with --skip-packages"
    fi
  fi
  case "$PACKAGE_MANAGER" in
    dnf)
      run dnf install -y podman socat
      ;;
    apt-get)
      run apt-get update
      run apt-get install -y podman socat
      ;;
  esac
fi

if [[ "$DRY_RUN" == "0" ]]; then
  for command_name in \
    awk bash containerd ctr file getent grep install mktemp modprobe mv \
    podman python3 readlink rm socat systemctl tr; do
    command -v "$command_name" >/dev/null 2>&1 \
      || fail "missing host command: $command_name"
  done
  [[ -f "$KATA_PREFIX/VERSION" ]] \
    || fail "Kata prefix has no VERSION file: $KATA_PREFIX"
  [[ "$(tr -d '[:space:]' <"$KATA_PREFIX/VERSION")" == "3.32.0" ]] \
    || fail "Kata 3.32.0 is required at $KATA_PREFIX"
  [[ -x "$KATA_PREFIX/bin/containerd-shim-kata-v2" ]] \
    || fail "Kata shim is missing under $KATA_PREFIX"
  [[ -x "$KATA_PREFIX/bin/kata-runtime" ]] \
    || fail "kata-runtime is missing under $KATA_PREFIX"
  if [[ "$BACKEND" == "stratovirt" ]]; then
    file "$DATA_KERNEL" | grep -Fq 'boot executable' \
      || fail "StratoVirt data kernel is not a boot executable (use Image/bzImage/vmlinuz, not an uncompressed debug vmlinux): $DATA_KERNEL"
  fi
  if [[ -n "$XIAOO" && ! -x "$XIAOO" ]]; then
    fail "xiaoO is not executable: $XIAOO"
  fi
  [[ "$WORKLOAD_IMAGE_ARCHIVE" == /* ]] \
    || fail "workload image archive must be an absolute path: $WORKLOAD_IMAGE_ARCHIVE"
fi

run systemctl enable --now containerd
run install -d -m 0755 /usr/local/bin
activate_link \
  /usr/local/bin/containerd-shim-kata332-v2 \
  "$KATA_PREFIX/bin/containerd-shim-kata-v2"
activate_link /usr/local/bin/kata-runtime "$KATA_PREFIX/bin/kata-runtime"

if [[ "$EGRESS_MODE" == "vsock-bridge" ]]; then
  if [[ "$BACKEND" == "stratovirt" ]]; then
    run modprobe vhost_vsock
  fi
  run install -D -m 0755 \
    "$ROOT_DIR/deploy/virtual-container/vsock-egress/host-bridge.sh" \
    /usr/local/libexec/actrail-vsock-egress/host-bridge.sh
  run install -D -m 0755 \
    "$ROOT_DIR/deploy/virtual-container/vsock-egress/ch-reconcile.sh" \
    /usr/local/libexec/actrail-vsock-egress/ch-reconcile.sh
  for unit in \
    actrail-vsock-host-stratovirt.service \
    actrail-vsock-host-cloud-hypervisor-reconcile.path \
    actrail-vsock-host-cloud-hypervisor-reconcile.service \
    actrail-vsock-host-cloud-hypervisor@.service; do
    run install -D -m 0644 \
      "$ROOT_DIR/deploy/virtual-container/vsock-egress/systemd/$unit" \
      "/etc/systemd/system/$unit"
  done
  run systemctl daemon-reload
  if [[ "$BACKEND" == "stratovirt" ]]; then
    run systemctl enable --now actrail-vsock-host-stratovirt.service
  else
    run systemctl enable --now \
      actrail-vsock-host-cloud-hypervisor-reconcile.path
  fi
fi

invoking_home="/root"
if [[ -n "${SUDO_USER:-}" && "$SUDO_USER" != "root" ]]; then
  invoking_home="$(getent passwd "$SUDO_USER" | awk -F: 'NR == 1 { print $6 }')"
  [[ -n "$invoking_home" ]] || fail "cannot resolve home for SUDO_USER=$SUDO_USER"
fi
release_environment=(
  "CARGO_HOME=${CARGO_HOME:-$invoking_home/.cargo}"
  "RUSTUP_HOME=${RUSTUP_HOME:-$invoking_home/.rustup}"
  "PATH=${PATH}"
  ACTRAIL_SKIP_JAVA_AGENT_BUILD=1
)
run env "${release_environment[@]}" bash "$ROOT_DIR/scripts/install-release.sh"

archive_arguments=()
if [[ "$BUILD_WORKLOAD" == "1" ]]; then
  containerd_image_exists=0
  podman_image_exists=0
  if [[ "$DRY_RUN" == "0" ]]; then
    if ctr -n default images list --quiet "name==$WORKLOAD_IMAGE" \
      | grep -Fxq "$WORKLOAD_IMAGE"; then
      containerd_image_exists=1
    fi
    if podman image exists "$WORKLOAD_IMAGE"; then
      podman_image_exists=1
    fi
  fi

  if [[ "$REBUILD_WORKLOAD" == "0" && "$containerd_image_exists" == "1" ]]; then
    echo "workload_image_cache=hit runtime=containerd image=$WORKLOAD_IMAGE"
  elif [[ "$REBUILD_WORKLOAD" == "1" || "$podman_image_exists" == "0" ]]; then
    run podman build \
      --format oci \
      --build-arg "BASE_IMAGE=$WORKLOAD_BASE_IMAGE" \
      -f "$ROOT_DIR/deploy/virtual-container/workload/Containerfile.openEuler" \
      -t "$WORKLOAD_IMAGE" \
      "$ROOT_DIR/deploy/virtual-container/workload"
  else
    echo "workload_image_cache=hit runtime=podman image=$WORKLOAD_IMAGE"
  fi

  if [[ "$REBUILD_WORKLOAD" == "0" && "$containerd_image_exists" == "1" \
    && ! -f "$WORKLOAD_IMAGE_ARCHIVE" ]]; then
    echo "workload_archive_cache=not-required image_is_in_containerd"
  elif [[ "$REBUILD_WORKLOAD" == "1" || ! -f "$WORKLOAD_IMAGE_ARCHIVE" ]]; then
    archive_parent="$(dirname "$WORKLOAD_IMAGE_ARCHIVE")"
    run install -d -m 0755 "$archive_parent"
    if [[ "$DRY_RUN" == "1" ]]; then
      print_command podman save --format oci-archive \
        -o "$WORKLOAD_IMAGE_ARCHIVE.tmp" "$WORKLOAD_IMAGE"
      print_command mv "$WORKLOAD_IMAGE_ARCHIVE.tmp" "$WORKLOAD_IMAGE_ARCHIVE"
    else
      archive_staging="$(mktemp "$archive_parent/.workload.XXXXXX.oci.tar")"
      cleanup_archive() {
        local rc=$?
        set +e
        if [[ -n "${archive_staging:-}" && -f "$archive_staging" ]]; then
          rm -f -- "$archive_staging"
        fi
        exit "$rc"
      }
      trap cleanup_archive EXIT INT TERM
      run podman save --format oci-archive \
        -o "$archive_staging" "$WORKLOAD_IMAGE"
      run mv "$archive_staging" "$WORKLOAD_IMAGE_ARCHIVE"
      archive_staging=""
      trap - EXIT INT TERM
    fi
  else
    echo "workload_archive_cache=hit path=$WORKLOAD_IMAGE_ARCHIVE"
  fi
  if [[ "$REBUILD_WORKLOAD" == "1" ]]; then
    run ctr -n default images import "$WORKLOAD_IMAGE_ARCHIVE"
    echo "workload_image_cache=refreshed runtime=containerd image=$WORKLOAD_IMAGE"
  fi
  if [[ "$DRY_RUN" == "1" || -f "$WORKLOAD_IMAGE_ARCHIVE" ]]; then
    archive_arguments=(--workload-image-archive "$WORKLOAD_IMAGE_ARCHIVE")
  fi
elif [[ "$DRY_RUN" == "0" ]]; then
  ctr -n default images list --quiet "name==$WORKLOAD_IMAGE" \
    | grep -Fxq "$WORKLOAD_IMAGE" \
    || fail "workload image is missing from containerd: $WORKLOAD_IMAGE"
fi

prepare_arguments=(
  --backend "$BACKEND"
  --kata-prefix "$KATA_PREFIX"
  --base-config-source "$BASE_CONFIG_SOURCE"
  --data-config-source "$DATA_CONFIG_SOURCE"
  --base-image-source "$BASE_IMAGE_SOURCE"
  --data-image-source "$DATA_IMAGE_SOURCE"
  --data-kernel "$DATA_KERNEL"
  --workload-image "$WORKLOAD_IMAGE"
  --image-pull-policy never
  --otel-endpoint "$OTEL_ENDPOINT"
  --egress-mode "$EGRESS_MODE"
)
if [[ -n "$XIAOO" ]]; then
  prepare_arguments+=(--xiaoo "$XIAOO")
fi
prepare_arguments+=("${archive_arguments[@]}")
run env "PATH=$PATH" python3 \
  "$ROOT_DIR/deploy/virtual-container/host/prepare-v2-test-artifacts.py" \
  "${prepare_arguments[@]}"

if [[ "$RUN_TESTS" == "1" ]]; then
  run env "${release_environment[@]}" \
    "$ROOT_DIR/deploy/virtual-container/host/run-v2-tests.sh" --color never
fi

echo "ACTRAIL_OPENEULER_VSOCK_DEPLOY_READY"
echo "host_os_id=$HOST_OS_ID"
echo "backend=$BACKEND"
echo "egress_mode=$EGRESS_MODE"
echo "otel_endpoint=$OTEL_ENDPOINT"
echo "profile=$ROOT_DIR/local/kata/v2-test-profile.json"
