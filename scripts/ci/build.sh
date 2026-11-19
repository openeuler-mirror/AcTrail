#!/usr/bin/env bash
# Build and install release artifacts with shared Cargo network settings.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd -- "$script_dir/../.."
source "$script_dir/lib/common.sh"
export CARGO_NET_RETRY=10
export CARGO_HTTP_TIMEOUT=120

# Use Rust mirrors on the same domain as the package repository.
export RUSTUP_DIST_SERVER="${RUSTUP_DIST_SERVER:-https://repo.huaweicloud.com/rustup}"
export RUSTUP_UPDATE_ROOT="${RUSTUP_UPDATE_ROOT:-https://repo.huaweicloud.com/rustup/rustup}"
cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
if ! command -v rustup >/dev/null 2>&1; then
  if [[ -x "$cargo_bin/rustup" ]]; then
    export PATH="$cargo_bin:$PATH"
    ci_log INFO "Using rustup from $cargo_bin"
  else
    # Keep the current compiler first on PATH if installation fails.
    export PATH="$(dirname -- "$(command -v rustc)"):$PATH"
    rust_host="$(rustc -vV | sed -n 's/^host: //p')"
    rust_version="$(rustc --version | awk '{print $2}')"
    ci_log INFO "Installing rustup and Rust $rust_version from $RUSTUP_UPDATE_ROOT"
    if rustup_install_dir="$(mktemp -d)"; then
      rustup_installer="$rustup_install_dir/rustup-init"
      if curl --fail --silent --show-error --location \
          --connect-timeout "${ACTRAIL_CI_CONNECT_TIMEOUT:-10}" \
          --max-time "${ACTRAIL_CI_DOWNLOAD_TIMEOUT:-120}" \
          "$RUSTUP_UPDATE_ROOT/dist/$rust_host/rustup-init" -o "$rustup_installer" &&
          chmod +x "$rustup_installer" &&
          "$rustup_installer" -y --no-modify-path --profile minimal \
            --default-toolchain "$rust_version" --component rustfmt; then
        export PATH="$cargo_bin:$PATH"
        export RUSTUP_TOOLCHAIN="$rust_version"
        ci_log PASS "Installed rustup with Rust $rust_version"
      else
        code=$?
        ci_log WARN "rustup installation failed (exit $code); continuing with the existing compiler"
      fi
      rm -r -- "$rustup_install_dir"
    else
      ci_log WARN 'Cannot create rustup installer directory; continuing with the existing compiler'
    fi
  fi
fi

exec bash "$script_dir/../install-release.sh" /usr/local/bin
