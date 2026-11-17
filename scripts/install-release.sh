#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/install-release.sh [DESTDIR]

Install AcTrail release binaries into DESTDIR and the official plugin packages.
DESTDIR defaults to /usr/local/bin.

Run scripts/install-build-deps.sh --install first. This script refreshes the
release binaries and TLS sync preload runtimes, then copies those artifacts
and the installed-but-disabled otel-jsonl, otel-http, sandbox-resource-alert, file-leakage,
activity-anomaly, alert-forwarding, tool-consecutive-failure-alert, dynamic file-policy,
dynamic command-policy, and dynamic network-policy plugins.

Environment:
  ACTRAIL_SUDO  Privilege command for installing into system directories.
                Defaults to sudo.
  ACTRAIL_PLUGIN_DIR
                Plugin installation root. Defaults to ~/.actrail/plugins for
                the user running this script. Include this absolute directory
                in the plugins.discovery.directory array.
  ACTRAIL_CONFIG_DIR
                System configuration root for the alert proxy and builtin
                forwarding policy. Defaults to /etc/actrail.
  CARGO_TARGET_DIR
                Shared Cargo output directory for binaries, TLS runtimes, and
                WASM plugins. Defaults to target.

The installer also prepares the optional LSM grey module for the running kernel
when matching headers, BTF and pahole are available. The daemon loads it only
when its effective LSM backend and grey mode require it.
EOF
}

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$script_dir/ci/lib/common.sh"

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -gt 1 ]]; then
  usage >&2
  exit 2
fi

dest_dir="${1:-/usr/local/bin}"
plugin_home="${HOME:?HOME is required to resolve the default plugin directory}"
plugin_root="${ACTRAIL_PLUGIN_DIR:-$plugin_home/.actrail/plugins}"
config_root="${ACTRAIL_CONFIG_DIR:-/etc/actrail}"
[[ "$plugin_root" = /* ]] || {
  echo "ACTRAIL_PLUGIN_DIR must be an absolute path: $plugin_root" >&2
  exit 2
}
[[ "$config_root" = /* ]] || {
  echo "ACTRAIL_CONFIG_DIR must be an absolute path: $config_root" >&2
  exit 2
}
otel_jsonl_install_dir="$plugin_root/otel-jsonl"
otel_jsonl_source_dir="$script_dir/../examples/plugins/builtin/otel-jsonl"
otel_http_install_dir="$plugin_root/otel-http"
otel_http_source_dir="$script_dir/../examples/plugins/builtin/otel-http"
sandbox_resource_alert_install_dir="$plugin_root/sandbox-resource-alert"
sandbox_resource_alert_source_dir="$script_dir/../examples/plugins/builtin/sandbox-resource-alert"
alert_forwarding_install_dir="$plugin_root/alert-forwarding"
alert_forwarding_source_dir="$script_dir/../examples/plugins/builtin/alert-forwarding"
alert_forwarding_config_dir="$config_root/plugins/alert-forwarding"
alert_proxy_config_source="$script_dir/../deploy/alert-proxy/actraild-alert-proxy.toml"
file_leakage_install_dir="$plugin_root/file-leakage"
file_leakage_source_dir="$script_dir/../examples/plugins/wit-component/file-leakage"
activity_anomaly_install_dir="$plugin_root/activity-anomaly"
activity_anomaly_source_dir="$script_dir/../examples/plugins/wit-component/activity-anomaly"
file_policy_install_dir="$plugin_root/file-policy-dynamic"
file_policy_source_dir="$script_dir/../examples/plugins/wit-component/file-policy-dynamic"
file_policy_fixture_dir="$file_policy_source_dir/fixture-src"
command_policy_install_dir="$plugin_root/command-policy-dynamic"
command_policy_source_dir="$script_dir/../examples/plugins/wit-component/command-policy-dynamic"
command_policy_fixture_dir="$command_policy_source_dir/fixture-src"
network_policy_install_dir="$plugin_root/network-policy-dynamic"
network_policy_source_dir="$script_dir/../examples/plugins/wit-component/network-policy-dynamic"
network_policy_fixture_dir="$network_policy_source_dir/fixture-src"
target_dir="${CARGO_TARGET_DIR:-target}"
export CARGO_TARGET_DIR="$target_dir"
release_dir="$target_dir/release"
wasm_release_dir="$target_dir/wasm32-wasip2/release"
file_leakage_artifact="$wasm_release_dir/actrail_file_leakage_plugin.wasm"
activity_anomaly_artifact="$wasm_release_dir/actrail_activity_anomaly_plugin.wasm"
file_policy_artifact="$wasm_release_dir/actrail_component_file_policy_dynamic.wasm"
command_policy_artifact="$wasm_release_dir/actrail_component_command_policy_dynamic.wasm"
network_policy_artifact="$wasm_release_dir/actrail_component_network_policy_dynamic.wasm"
tool_consecutive_failure_install_dir="$plugin_root/tool-consecutive-failure-alert"
tool_consecutive_failure_source_dir="$script_dir/../examples/plugins/wasm-legacy/tool-consecutive-failure-alert"
tool_consecutive_failure_artifact="$wasm_release_dir/actrail_tool_consecutive_failure_alert.wasm"
tool_frequent_failure_install_dir="$plugin_root/tool-frequent-failure-alert"
tool_frequent_failure_source_dir="$script_dir/../examples/plugins/wasm-legacy/tool-frequent-failure-alert"
tool_frequent_failure_artifact="$wasm_release_dir/actrail_tool_frequent_failure_alert.wasm"
binaries=(
  actraild
  actrailctl
  actrailcluster
  actrail-sb
  actrail-vsock-gateway
  actraild-alert-proxy
  actraild-grey-proxy
  actrailviewer
  actrailweb
)
runtimes=(
  libactrail_tls_payload_probe_sync.so
  libactrail_tls_payload_probe_sync-musl.so
)

run() {
  "$@"
}

path_needs_privilege() {
  local path="$1"
  [[ "${EUID:-$(id -u)}" -ne 0 ]] || return 1
  if [[ -d "$path" ]]; then
    [[ ! -w "$path" ]]
    return
  fi
  [[ ! -w "$(dirname -- "$path")" ]]
}

install_prefix() {
  local target="$1"
  path_needs_privilege "$target" || return 0
  local configured_sudo="${ACTRAIL_SUDO:-sudo}"
  [[ -n "$configured_sudo" ]] || {
    echo "not root and ACTRAIL_SUDO is empty" >&2
    exit 1
  }

  local -a prefix
  read -r -a prefix <<<"$configured_sudo"
  command -v "${prefix[0]}" >/dev/null 2>&1 || {
    echo "privilege command '${prefix[0]}' is not on PATH" >&2
    exit 1
  }
  printf '%s\n' "${prefix[@]}"
}

mkdir -p "$target_dir"
build_log_dir="$(mktemp -d "$target_dir/build-logs.XXXXXX")"
trap 'rm -r -- "$build_log_dir"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
ci_log INFO 'Starting release build'
if cargo build --release >"$build_log_dir/workspace.log" 2>&1; then
  rm -- "$build_log_dir/workspace.log"
  ci_log PASS 'Release build completed'
else
  code=$?
  ci_log FAIL "Release build failed (exit $code)"
  cat "$build_log_dir/workspace.log" >&2
  rm -- "$build_log_dir/workspace.log"
  exit "$code"
fi

# Skip failed optional components and continue installing the main binaries.
if ! bash "$script_dir/build-tls-sync-runtimes.sh" --musl-only; then
  runtimes=(libactrail_tls_payload_probe_sync.so)
  ci_log SKIP 'musl TLS runtime build failed; runtime installation skipped'
fi

wasm_ready=1
target_libdir="$(rustc --print target-libdir --target wasm32-wasip2)" || {
  wasm_ready=0
  ci_log SKIP 'rustc could not locate wasm32-wasip2 target libraries'
}
if (( wasm_ready )) && ! compgen -G "$target_libdir/libstd-*.rlib" >/dev/null; then
  if ! command -v rustup >/dev/null 2>&1; then
    wasm_ready=0
    ci_log SKIP "wasm32-wasip2 standard library missing in $target_libdir; rustup not found on PATH"
  else
    ci_log INFO 'Installing Rust target: wasm32-wasip2'
    if rustup target add wasm32-wasip2; then
      ci_log PASS 'rustup target add wasm32-wasip2 completed'
    else
      code=$?
      wasm_ready=0
      ci_log SKIP "rustup target add wasm32-wasip2 failed (exit $code); see command output above"
    fi
  fi
fi
plugin_manifests=(
  "$file_leakage_source_dir/Cargo.toml"
  "$activity_anomaly_source_dir/Cargo.toml"
  "$file_policy_fixture_dir/Cargo.toml"
  "$command_policy_fixture_dir/Cargo.toml"
  "$network_policy_fixture_dir/Cargo.toml"
  "$tool_consecutive_failure_source_dir/Cargo.toml"
  "$tool_frequent_failure_source_dir/Cargo.toml"
)
plugin_artifacts=(
  "$file_leakage_artifact"
  "$activity_anomaly_artifact"
  "$file_policy_artifact"
  "$command_policy_artifact"
  "$network_policy_artifact"
  "$tool_consecutive_failure_artifact"
  "$tool_frequent_failure_artifact"
)
mapfile -t binary_install < <(install_prefix "$dest_dir")
mapfile -t plugin_install < <(install_prefix "$plugin_root")
mapfile -t config_install < <(install_prefix "$config_root")

# Prepare the optional module without loading it into an unrelated workload.
# All failures here are optional: auto mode can use ordinary LSM at startup.
kernel_release="$(uname -r)"
kernel_build="${ACTRAIL_KERNEL_BUILD:-/lib/modules/$kernel_release/build}"
kernel_btf="${ACTRAIL_KERNEL_BTF:-/sys/kernel/btf/vmlinux}"
module_dir="/lib/modules/$kernel_release/extra"
if [[ -d "$kernel_build" && -r "$kernel_btf" ]] && command -v pahole >/dev/null 2>&1; then
  ci_log INFO 'Building and installing LSM grey module'
  if bash "$script_dir/../crates/access_control/kernel/build.sh"; then
    mapfile -t module_install < <(install_prefix "$module_dir")
    if "${module_install[@]}" install -d "$module_dir" &&
       "${module_install[@]}" install -m 0644 \
         "$target_dir/kernel/$kernel_release/actrail_lsm_grey.ko" "$module_dir/actrail_lsm_grey.ko" &&
       "${module_install[@]}" depmod -a "$kernel_release"; then
      ci_log PASS "LSM grey module installed for $kernel_release"
    else
      ci_log SKIP 'LSM grey module installation failed'
    fi
  else
    ci_log SKIP 'LSM grey module build failed'
  fi
else
  ci_log SKIP 'LSM grey module: matching headers, kernel BTF or pahole unavailable'
fi

run "${binary_install[@]}" install -d "$dest_dir"
run "${plugin_install[@]}" install -d "$otel_jsonl_install_dir"
run "${plugin_install[@]}" install -d "$otel_http_install_dir"
run "${plugin_install[@]}" install -d "$sandbox_resource_alert_install_dir"
run "${plugin_install[@]}" install -d "$alert_forwarding_install_dir"
run "${config_install[@]}" install -d "$alert_forwarding_config_dir"

for binary in "${binaries[@]}"; do
  source_path="$release_dir/$binary"
  if [[ ! -x "$source_path" ]]; then
    echo "missing executable $source_path; cargo build --release did not produce it" >&2
    exit 1
  fi
  run "${binary_install[@]}" install -m 0755 "$source_path" "$dest_dir/$binary"
done

if [[ ! -e "$config_root/actraild-alert-proxy.toml" ]]; then
  run "${config_install[@]}" install -m 0640 \
    "$alert_proxy_config_source" "$config_root/actraild-alert-proxy.toml"
fi
if [[ ! -e "$alert_forwarding_config_dir/alert-forwarding.config.json" ]]; then
  run "${config_install[@]}" install -m 0640 \
    "$alert_forwarding_source_dir/alert-forwarding.config.json" \
    "$alert_forwarding_config_dir/alert-forwarding.config.json"
fi

for runtime in "${runtimes[@]}"; do
  source_path="$release_dir/$runtime"
  if [[ ! -f "$source_path" ]]; then
    echo "missing runtime $source_path; scripts/build-tls-sync-runtimes.sh did not produce it" >&2
    exit 1
  fi
  run "${binary_install[@]}" install -m 0755 "$source_path" "$dest_dir/$runtime"
done

plugin_done=0
plugin_total=$((4 + ${#plugin_manifests[@]}))

for asset in \
  otel-jsonl.plugin.toml \
  otel-jsonl.config.toml \
  otel-jsonl.config.v1.schema.json; do
  run "${plugin_install[@]}" install -m 0644 \
    "$otel_jsonl_source_dir/$asset" "$otel_jsonl_install_dir/$asset"
done
run "${plugin_install[@]}" rm -f \
  "$otel_jsonl_install_dir/otel-jsonl.plugin-config.v1"
plugin_done=$((plugin_done + 1))
ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: otel-jsonl"
for asset in \
  otel-http.plugin.toml \
  otel-http.config.toml \
  otel-http.config.v1.schema.json; do
  run "${plugin_install[@]}" install -m 0644 \
    "$otel_http_source_dir/$asset" "$otel_http_install_dir/$asset"
done
plugin_done=$((plugin_done + 1))
ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: otel-http"
for asset in \
  sandbox-resource-alert.plugin.toml \
  sandbox-resource-alert.config.json \
  sandbox-resource-alert.config.v1.schema.json; do
  run "${plugin_install[@]}" install -m 0644 \
    "$sandbox_resource_alert_source_dir/$asset" \
    "$sandbox_resource_alert_install_dir/$asset"
done
plugin_done=$((plugin_done + 1))
ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: sandbox-resource-alert"
for asset in \
  alert-forwarding.plugin.toml \
  alert-forwarding.config.json \
  alert-forwarding.config.v1.schema.json; do
  run "${plugin_install[@]}" install -m 0644 \
    "$alert_forwarding_source_dir/$asset" \
    "$alert_forwarding_install_dir/$asset"
done
plugin_done=$((plugin_done + 1))
ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: alert-forwarding"
# Build and install each plugin before processing the next one.
for index in "${!plugin_manifests[@]}"; do
  plugin_done=$((plugin_done + 1))
  artifact="${plugin_artifacts[$index]}"
  if (( ! wasm_ready )); then
    ci_log SKIP "[$plugin_done/$plugin_total] WASM target unavailable: ${artifact##*/}"
    continue
  fi
  build_log="$build_log_dir/${artifact##*/}.log"
  ci_log INFO "[$plugin_done/$plugin_total] Building and installing plugin: ${artifact##*/}"
  if cargo build --release --target wasm32-wasip2 \
      --manifest-path "${plugin_manifests[$index]}" >"$build_log" 2>&1 && [[ -f "$artifact" ]]; then
    rm -- "$build_log"
    case "$index" in
      0)
        run "${plugin_install[@]}" install -d "$file_leakage_install_dir"
        for asset in \
          file-leakage.plugin.toml \
          file-leakage.config.json \
          file-leakage.config.v1.schema.json \
          file-leakage.payload.v1.schema.json; do
          run "${plugin_install[@]}" install -m 0644 \
            "$file_leakage_source_dir/$asset" "$file_leakage_install_dir/$asset"
        done
        run "${plugin_install[@]}" install -m 0644 \
          "$file_leakage_artifact" \
          "$file_leakage_install_dir/actrail_file_leakage_plugin.wasm"
        ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: file-leakage"
        ;;
      1)
        run "${plugin_install[@]}" install -d "$activity_anomaly_install_dir"
        for asset in \
          activity-anomaly.plugin.toml \
          activity-anomaly.config.json \
          activity-anomaly.config.v1.schema.json \
          llm-growth.payload.v1.schema.json \
          command-duration.payload.v1.schema.json; do
          run "${plugin_install[@]}" install -m 0644 \
            "$activity_anomaly_source_dir/$asset" "$activity_anomaly_install_dir/$asset"
        done
        run "${plugin_install[@]}" install -m 0644 \
          "$activity_anomaly_artifact" \
          "$activity_anomaly_install_dir/actrail_activity_anomaly_plugin.wasm"
        ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: activity-anomaly"
        ;;
      2)
        run "${plugin_install[@]}" install -d "$file_policy_install_dir"
        run "${plugin_install[@]}" install -m 0644 \
          "$file_policy_source_dir/plugin.toml" \
          "$file_policy_install_dir/file-policy-dynamic.plugin.toml"
        for asset in \
          file-policy-dynamic.config.json \
          config.schema.json; do
          run "${plugin_install[@]}" install -m 0644 \
            "$file_policy_source_dir/$asset" "$file_policy_install_dir/$asset"
        done
        run "${plugin_install[@]}" install -m 0644 \
          "$file_policy_artifact" \
          "$file_policy_install_dir/component-file-policy-dynamic.wasm"
        ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: file-policy-dynamic"
        ;;
      3)
        run "${plugin_install[@]}" install -d "$command_policy_install_dir"
        run "${plugin_install[@]}" install -m 0644 \
          "$command_policy_source_dir/plugin.toml" \
          "$command_policy_install_dir/command-policy-dynamic.plugin.toml"
        for asset in \
          command-policy-dynamic.config.json \
          config.schema.json; do
          run "${plugin_install[@]}" install -m 0644 \
            "$command_policy_source_dir/$asset" "$command_policy_install_dir/$asset"
        done
        run "${plugin_install[@]}" install -m 0644 \
          "$command_policy_artifact" \
          "$command_policy_install_dir/component-command-policy-dynamic.wasm"
        ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: command-policy-dynamic"
        ;;
      4)
        run "${plugin_install[@]}" install -d "$network_policy_install_dir"
        run "${plugin_install[@]}" install -m 0644 \
          "$network_policy_source_dir/plugin.toml" \
          "$network_policy_install_dir/network-policy-dynamic.plugin.toml"
        for asset in \
          network-policy-dynamic.config.json \
          config.schema.json; do
          run "${plugin_install[@]}" install -m 0644 \
            "$network_policy_source_dir/$asset" "$network_policy_install_dir/$asset"
        done
        run "${plugin_install[@]}" install -m 0644 \
          "$network_policy_artifact" \
          "$network_policy_install_dir/component-network-policy-dynamic.wasm"
        ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: network-policy-dynamic"
        ;;
      5)
        run "${plugin_install[@]}" install -d "$tool_consecutive_failure_install_dir"
        run "${plugin_install[@]}" install -m 0644 \
          "$tool_consecutive_failure_source_dir/plugin.toml" \
          "$tool_consecutive_failure_install_dir/tool-consecutive-failure-alert.plugin.toml"
        run "${plugin_install[@]}" install -m 0644 \
          "$tool_consecutive_failure_source_dir/alert-schema.json" \
          "$tool_consecutive_failure_install_dir/alert-schema.json"
        run "${plugin_install[@]}" install -m 0644 \
          "$tool_consecutive_failure_artifact" \
          "$tool_consecutive_failure_install_dir/actrail_tool_consecutive_failure_alert.wasm"
        ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: tool-consecutive-failure-alert"
        ;;
      6)
        run "${plugin_install[@]}" install -d "$tool_frequent_failure_install_dir"
        run "${plugin_install[@]}" install -m 0644 \
          "$tool_frequent_failure_source_dir/plugin.toml" \
          "$tool_frequent_failure_install_dir/tool-frequent-failure-alert.plugin.toml"
        for asset in \
          frequent-failure-alert-v1.schema.json \
          indeterminate-result-v1.schema.json; do
          run "${plugin_install[@]}" install -m 0644 \
            "$tool_frequent_failure_source_dir/$asset" "$tool_frequent_failure_install_dir/$asset"
        done
        run "${plugin_install[@]}" install -m 0644 \
          "$tool_frequent_failure_artifact" \
          "$tool_frequent_failure_install_dir/actrail_tool_frequent_failure_alert.wasm"
        ci_log PASS "[$plugin_done/$plugin_total] Plugin installed: tool-frequent-failure-alert"
        ;;
    esac
  else
    ci_log SKIP "[$plugin_done/$plugin_total] WASM plugin build failed: ${artifact##*/}"
    cat "$build_log" >&2
    rm -- "$build_log"
    if [[ ! -f "$artifact" ]]; then
      ci_log FAIL "Missing plugin artifact: ${artifact##*/}"
    fi
  fi
done
