#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/install-build-deps.sh [--check|--install] [OPTIONS]

Check or install host dependencies needed to build AcTrail from source.

Modes:
  --check                         Check only; install nothing.
  --install                       Check, then install missing dependencies without prompting.
  no mode                         Check, then ask for confirmation before installing missing dependencies.

Options:
  --package-manager dnf|apt-get   Override package manager detection.
  --skip-frontend-deps            Skip Node.js/npm and actrailweb npm dependencies.
  -h, --help                      Show this help.

Environment:
  ACTRAIL_PACKAGE_MANAGER         auto, dnf, or apt-get. Defaults to auto.
  ACTRAIL_ASSUME_YES              1 passes noninteractive flags to dnf/apt-get. Defaults to 1.
  ACTRAIL_APT_UPDATE              1 runs apt-get update before apt-get install. Defaults to 1.
  ACTRAIL_FRONTEND_NPM_CI         1 checks/runs npm ci for actrailweb. Defaults to 1.
  ACTRAIL_DNF_PACKAGES            Space-separated dnf package list.
  ACTRAIL_APT_PACKAGES            Space-separated apt-get package list.
  ACTRAIL_DNF_FRONTEND_PACKAGES   Space-separated dnf package list for Node.js/npm.
  ACTRAIL_APT_FRONTEND_PACKAGES   Space-separated apt-get package list for Node.js/npm.
  ACTRAIL_MUSL_LINKER             Path/name of musl C linker for TLS sync musl runtime.
  ACTRAIL_TLS_SYNC_PREBUILT_MUSL_RUNTIME
                                  Prebuilt musl TLS sync runtime to use instead of local musl build.
  ACTRAIL_MIN_NODE_MAJOR          Minimum Node.js major version. Defaults to 18.
  ACTRAIL_SUDO                    Privilege command for non-root users. Defaults to sudo.

Default native packages:
  dnf:     clang llvm elfutils-devel zlib-devel pkgconf-pkg-config openssl-devel
  apt-get: clang llvm libelf-dev zlib1g-dev pkg-config libssl-dev musl-tools

Default frontend packages:
  dnf:     nodejs npm
  apt-get: nodejs npm

Rust is checked, not installed. Install Rust/Cargo with a toolchain that satisfies
the workspace rust-version in Cargo.toml before running cargo build --release.
EOF
}

fail() {
  echo "error: $*" >&2
  exit 1
}

bool_enabled() {
  case "$1" in
    1 | true | yes | on) return 0 ;;
    0 | false | no | off) return 1 ;;
    *) fail "invalid boolean value '$1'" ;;
  esac
}

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"

mode=prompt
package_manager="${ACTRAIL_PACKAGE_MANAGER:-auto}"
frontend_npm_ci="${ACTRAIL_FRONTEND_NPM_CI:-1}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)
      [[ "$mode" == prompt ]] || fail "choose only one mode"
      mode=check
      shift
      ;;
    --install)
      [[ "$mode" == prompt ]] || fail "choose only one mode"
      mode=install
      shift
      ;;
    --package-manager)
      [[ $# -ge 2 ]] || fail "--package-manager requires dnf or apt-get"
      package_manager="$2"
      shift 2
      ;;
    --skip-frontend-deps)
      frontend_npm_ci=0
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument '$1'"
      ;;
  esac
done

detect_package_manager() {
  case "$package_manager" in
    auto)
      if command -v dnf >/dev/null 2>&1; then
        echo dnf
      elif command -v apt-get >/dev/null 2>&1; then
        echo apt-get
      else
        fail "no supported package manager found; set ACTRAIL_PACKAGE_MANAGER to dnf or apt-get"
      fi
      ;;
    dnf | apt-get)
      command -v "$package_manager" >/dev/null 2>&1 ||
        fail "selected package manager '$package_manager' is not on PATH"
      echo "$package_manager"
      ;;
    *)
      fail "unsupported package manager '$package_manager'; expected auto, dnf, or apt-get"
      ;;
  esac
}

package_list() {
  case "$1" in
    dnf)
      printf '%s\n' "${ACTRAIL_DNF_PACKAGES:-clang llvm elfutils-devel zlib-devel pkgconf-pkg-config openssl-devel}"
      ;;
    apt-get)
      printf '%s\n' "${ACTRAIL_APT_PACKAGES:-clang llvm libelf-dev zlib1g-dev pkg-config libssl-dev musl-tools}"
      ;;
    *)
      fail "internal error: unsupported package manager '$1'"
      ;;
  esac
}

frontend_package_list() {
  case "$1" in
    dnf)
      printf '%s\n' "${ACTRAIL_DNF_FRONTEND_PACKAGES:-nodejs npm}"
      ;;
    apt-get)
      printf '%s\n' "${ACTRAIL_APT_FRONTEND_PACKAGES:-nodejs npm}"
      ;;
    *)
      fail "internal error: unsupported package manager '$1'"
      ;;
  esac
}

package_installed() {
  local manager="$1"
  local package="$2"

  case "$manager" in
    dnf)
      rpm -q "$package" >/dev/null 2>&1
      ;;
    apt-get)
      dpkg-query -W -f='${Status}' "$package" 2>/dev/null | grep -qx 'install ok installed'
      ;;
    *)
      fail "internal error: unsupported package manager '$manager'"
      ;;
  esac
}

sudo_prefix() {
  if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
    return 0
  fi

  local configured_sudo="${ACTRAIL_SUDO:-sudo}"
  [[ -n "$configured_sudo" ]] || fail "not root and ACTRAIL_SUDO is empty"

  local -a prefix
  read -r -a prefix <<<"$configured_sudo"
  command -v "${prefix[0]}" >/dev/null 2>&1 ||
    fail "privilege command '${prefix[0]}' is not on PATH"
  printf '%s\n' "${prefix[@]}"
}

install_packages() {
  local manager="$1"
  local package_source="${2:-native}"
  local assume_yes="${ACTRAIL_ASSUME_YES:-1}"
  local -a packages command prefix

  case "$package_source" in
    native)
      read -r -a packages <<<"$(package_list "$manager")"
      ;;
    frontend)
      read -r -a packages <<<"$(frontend_package_list "$manager")"
      ;;
    *)
      fail "internal error: unsupported package source '$package_source'"
      ;;
  esac
  [[ "${#packages[@]}" -gt 0 ]] || return 0

  mapfile -t prefix < <(sudo_prefix)

  case "$manager" in
    dnf)
      command=("${prefix[@]}" dnf install)
      if bool_enabled "$assume_yes"; then
        command+=(-y)
      fi
      command+=("${packages[@]}")
      run "${command[@]}"
      ;;
    apt-get)
      if bool_enabled "${ACTRAIL_APT_UPDATE:-1}"; then
        run "${prefix[@]}" apt-get update
      fi
      command=("${prefix[@]}" apt-get install)
      if bool_enabled "$assume_yes"; then
        command+=(-y)
      fi
      command+=("${packages[@]}")
      run "${command[@]}"
      ;;
    *)
      fail "internal error: unsupported package manager '$manager'"
      ;;
  esac
}

required_rust_version() {
  local version
  version="$(awk -F '"' '/rust-version[[:space:]]*=/{ print $2; exit }' "$repo_root/Cargo.toml")"
  [[ -n "$version" ]] || fail "missing workspace rust-version in Cargo.toml"
  echo "$version"
}

numeric_part() {
  local part="$1"
  part="${part%%[^0-9]*}"
  echo "${part:-0}"
}

version_ge() {
  local have="$1"
  local required="$2"
  local -a have_parts required_parts
  IFS='.' read -r -a have_parts <<<"$have"
  IFS='.' read -r -a required_parts <<<"$required"

  for index in 0 1 2; do
    local have_part required_part
    have_part="$(numeric_part "${have_parts[$index]:-0}")"
    required_part="$(numeric_part "${required_parts[$index]:-0}")"
    if ((have_part > required_part)); then
      return 0
    fi
    if ((have_part < required_part)); then
      return 1
    fi
  done
  return 0
}

check_packages() {
  local manager="$1"
  local ok=0
  local -a packages
  read -r -a packages <<<"$(package_list "$manager")"

  for package in "${packages[@]}"; do
    if package_installed "$manager" "$package"; then
      printf 'ok: package %s\n' "$package"
    else
      printf 'missing: package %s\n' "$package" >&2
      ok=1
    fi
  done

  return "$ok"
}

check_rust() {
  local required rustc_version
  required="$(required_rust_version)"

  if ! command -v cargo >/dev/null 2>&1; then
    echo 'missing: cargo' >&2
    return 1
  fi
  if ! command -v rustc >/dev/null 2>&1; then
    echo 'missing: rustc' >&2
    return 1
  fi

  rustc_version="$(rustc --version | awk '{ print $2 }')"
  if ! version_ge "$rustc_version" "$required"; then
    printf 'missing: rustc >= %s, found %s\n' "$required" "$rustc_version" >&2
    return 1
  fi

  printf 'ok: rustc %s >= %s\n' "$rustc_version" "$required"
}

musl_target_for_host() {
  local host
  host="$(rustc -vV | awk '/^host:/ { print $2 }')"
  case "${ACTRAIL_MUSL_TARGET:-$host}" in
    x86_64-unknown-linux-gnu | x86_64-unknown-linux-musl)
      echo x86_64-unknown-linux-musl
      ;;
    aarch64-unknown-linux-gnu | aarch64-unknown-linux-musl)
      echo aarch64-unknown-linux-musl
      ;;
    *)
      return 1
      ;;
  esac
}

check_musl_runtime_builder() {
  local musl_target linker

  if [[ -n "${ACTRAIL_TLS_SYNC_PREBUILT_MUSL_RUNTIME:-}" ]]; then
    if [[ -f "$ACTRAIL_TLS_SYNC_PREBUILT_MUSL_RUNTIME" ]]; then
      printf 'ok: prebuilt musl TLS sync runtime %s\n' "$ACTRAIL_TLS_SYNC_PREBUILT_MUSL_RUNTIME"
      return 0
    fi
    printf 'missing: prebuilt musl TLS sync runtime %s\n' "$ACTRAIL_TLS_SYNC_PREBUILT_MUSL_RUNTIME" >&2
    return 1
  fi

  musl_target="$(musl_target_for_host)" || {
    echo 'missing: supported musl Rust target for this host' >&2
    return 1
  }
  if [[ -n "${ACTRAIL_MUSL_LINKER:-}" ]]; then
    if command -v "$ACTRAIL_MUSL_LINKER" >/dev/null 2>&1 || [[ -x "$ACTRAIL_MUSL_LINKER" ]]; then
      printf 'ok: musl linker %s for %s\n' "$ACTRAIL_MUSL_LINKER" "$musl_target"
      return 0
    fi
    printf 'missing: ACTRAIL_MUSL_LINKER=%s\n' "$ACTRAIL_MUSL_LINKER" >&2
    return 1
  fi

  case "$musl_target" in
    x86_64-unknown-linux-musl)
      for linker in x86_64-linux-musl-gcc musl-gcc; do
        if command -v "$linker" >/dev/null 2>&1; then
          printf 'ok: musl linker %s for %s\n' "$linker" "$musl_target"
          return 0
        fi
      done
      echo 'missing: musl linker for x86_64-unknown-linux-musl (searched x86_64-linux-musl-gcc musl-gcc)' >&2
      ;;
    aarch64-unknown-linux-musl)
      for linker in aarch64-linux-musl-gcc musl-gcc; do
        if command -v "$linker" >/dev/null 2>&1; then
          printf 'ok: musl linker %s for %s\n' "$linker" "$musl_target"
          return 0
        fi
      done
      echo 'missing: musl linker for aarch64-unknown-linux-musl (searched aarch64-linux-musl-gcc musl-gcc)' >&2
      ;;
  esac
  echo 'set ACTRAIL_MUSL_LINKER or ACTRAIL_TLS_SYNC_PREBUILT_MUSL_RUNTIME; some dnf repos do not package musl-gcc' >&2
  return 1
}

check_frontend() {
  local frontend_dir="$repo_root/crates/apps/web/frontend"
  local min_major node_version node_major

  if ! command -v node >/dev/null 2>&1; then
    echo 'missing: node' >&2
    return 1
  fi
  if ! command -v npm >/dev/null 2>&1; then
    echo 'missing: npm' >&2
    return 1
  fi

  min_major="${ACTRAIL_MIN_NODE_MAJOR:-18}"
  node_version="$(node --version)"
  node_major="${node_version#v}"
  node_major="${node_major%%.*}"
  if [[ ! "$node_major" =~ ^[0-9]+$ ]]; then
    printf 'missing: cannot parse Node.js version %s\n' "$node_version" >&2
    return 1
  fi
  if ((node_major < min_major)); then
    printf 'missing: Node.js major >= %s, found %s\n' "$min_major" "$node_version" >&2
    return 1
  fi
  if [[ ! -d "$frontend_dir/node_modules" ]]; then
    printf 'missing: %s/node_modules\n' "$frontend_dir" >&2
    return 1
  fi

  printf 'ok: node %s >= major %s\n' "$node_version" "$min_major"
  printf 'ok: actrailweb frontend node_modules\n'
}

check_all() {
  local manager="$1"
  local ok=0

  check_packages "$manager" || ok=1
  check_rust || ok=1
  check_musl_runtime_builder || ok=1
  if bool_enabled "$frontend_npm_ci"; then
    check_frontend || ok=1
  fi

  return "$ok"
}

confirm_install() {
  if [[ ! -t 0 ]]; then
    fail "dependencies are missing and confirmation requires a terminal; rerun with --install"
  fi

  local reply
  printf 'Install missing AcTrail build dependencies now? [y/N] '
  read -r reply
  case "$reply" in
    y | Y | yes | YES) ;;
    *) fail "installation cancelled" ;;
  esac
}

install_frontend_deps() {
  local frontend_dir="$repo_root/crates/apps/web/frontend"
  if ! command -v node >/dev/null 2>&1 || ! command -v npm >/dev/null 2>&1; then
    install_packages "$1" frontend
  fi
  [[ -f "$frontend_dir/package-lock.json" ]] ||
    fail "missing $frontend_dir/package-lock.json"
  run npm ci --prefix "$frontend_dir"
}

install_deps() {
  local manager="$1"
  install_packages "$manager"
  if bool_enabled "$frontend_npm_ci"; then
    install_frontend_deps "$manager"
  fi
}

manager="$(detect_package_manager)"
printf 'AcTrail build dependency check using %s\n' "$manager"

if check_all "$manager"; then
  printf 'AcTrail build dependencies are ready.\n'
  exit 0
fi

if [[ "$mode" == check ]]; then
  fail "AcTrail build dependencies are missing"
fi

if [[ "$mode" == prompt ]]; then
  confirm_install
fi

install_deps "$manager"

printf 'Rechecking AcTrail build dependencies\n'
check_all "$manager" || fail "AcTrail build dependencies are still missing"
printf 'AcTrail build dependencies are ready.\n'
