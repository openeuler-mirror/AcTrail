#!/usr/bin/env bash
# Build and install the Docker container deployment in one command.
set -euo pipefail

MODULE_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$MODULE_DIR/../.." && pwd)"
DISTRO=openeuler
BASE_IMAGE=''
WORKLOAD_IMAGE=''
BIN_DIR=''
PULL_POLICY=missing
OTEL_ENDPOINT=''
OTEL_ATTRIBUTE_MODE=metadata-only
PRINT_PLAN=0
BUILD_CONTEXT=''

usage() {
    cat <<'EOF'
Usage: sudo -E deploy/container-auto/deploy.sh [options]

Options:
  --distro openeuler|ubuntu  Workload base distribution (default: openeuler)
  --base-image IMAGE         Override the selected distribution's base image
  --image IMAGE              Workload image to build
  --bin-dir DIR              Reuse release binaries instead of building them
  --pull-policy POLICY       missing, always, or never (default: missing)
  --otel-endpoint URL        Also export host spans to an OTLP/HTTP Collector
  --otel-attribute-mode MODE metadata-only or full (default: metadata-only)
  --print-plan               Print resolved inputs without changing the host
  -h, --help                 Show this help
EOF
}

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

cleanup() {
    if [[ -n "$BUILD_CONTEXT" && -d "$BUILD_CONTEXT" ]]; then
        rm -rf -- "$BUILD_CONTEXT"
    fi
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --distro)
            [[ "$#" -ge 2 ]] || { echo "--distro requires a value" >&2; exit 2; }
            DISTRO=$2
            shift 2
            ;;
        --base-image)
            [[ "$#" -ge 2 ]] || { echo "--base-image requires a value" >&2; exit 2; }
            BASE_IMAGE=$2
            shift 2
            ;;
        --image)
            [[ "$#" -ge 2 ]] || { echo "--image requires a value" >&2; exit 2; }
            WORKLOAD_IMAGE=$2
            shift 2
            ;;
        --bin-dir)
            [[ "$#" -ge 2 ]] || { echo "--bin-dir requires a value" >&2; exit 2; }
            BIN_DIR=$2
            shift 2
            ;;
        --pull-policy)
            [[ "$#" -ge 2 ]] || { echo "--pull-policy requires a value" >&2; exit 2; }
            PULL_POLICY=$2
            shift 2
            ;;
        --otel-endpoint)
            [[ "$#" -ge 2 ]] || { echo "--otel-endpoint requires a value" >&2; exit 2; }
            OTEL_ENDPOINT=$2
            shift 2
            ;;
        --otel-attribute-mode)
            [[ "$#" -ge 2 ]] || { echo "--otel-attribute-mode requires a value" >&2; exit 2; }
            OTEL_ATTRIBUTE_MODE=$2
            shift 2
            ;;
        --print-plan)
            PRINT_PLAN=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

case "$DISTRO" in
    openeuler)
        BASE_IMAGE=${BASE_IMAGE:-openeuler/openeuler:24.03-lts-sp3}
        WORKLOAD_IMAGE=${WORKLOAD_IMAGE:-actrail/container-auto:openeuler-24.03}
        ;;
    ubuntu)
        BASE_IMAGE=${BASE_IMAGE:-ubuntu:24.04}
        WORKLOAD_IMAGE=${WORKLOAD_IMAGE:-actrail/container-auto:ubuntu-24.04}
        ;;
    *)
        echo "unsupported distro: $DISTRO" >&2
        exit 2
        ;;
esac

case "$PULL_POLICY" in
    missing|always|never) ;;
    *)
        echo "unsupported pull policy: $PULL_POLICY" >&2
        exit 2
        ;;
esac

case "$OTEL_ATTRIBUTE_MODE" in
    metadata-only|full) ;;
    *)
        echo "unsupported OTLP attribute mode: $OTEL_ATTRIBUTE_MODE" >&2
        exit 2
        ;;
esac
if [[ -z "$OTEL_ENDPOINT" && "$OTEL_ATTRIBUTE_MODE" != metadata-only ]]; then
    echo "--otel-attribute-mode requires --otel-endpoint" >&2
    exit 2
fi
if [[ -n "$OTEL_ENDPOINT" ]]; then
    "$MODULE_DIR/render-otel-http-config.sh" \
        --template "$REPO_ROOT/examples/plugins/builtin/otel-http/otel-http.config.toml" \
        --endpoint "$OTEL_ENDPOINT" \
        --attribute-mode "$OTEL_ATTRIBUTE_MODE" \
        --check
fi

if [[ "$PRINT_PLAN" == "1" ]]; then
    echo "distro=$DISTRO"
    echo "base_image=$BASE_IMAGE"
    echo "workload_image=$WORKLOAD_IMAGE"
    echo "release_source=${BIN_DIR:-build}"
    echo "pull_policy=$PULL_POLICY"
    echo "otel_endpoint=${OTEL_ENDPOINT:-disabled}"
    echo "otel_attribute_mode=$OTEL_ATTRIBUTE_MODE"
    exit 0
fi

[[ "$(id -u)" -eq 0 ]] || fail "run this deployment with sudo -E"

for command_name in awk docker grep install mktemp seq sleep systemctl; do
    command -v "$command_name" >/dev/null 2>&1 \
        || fail "required command is missing: $command_name"
done
docker info >/dev/null 2>&1 || fail "Docker daemon is unavailable"

if [[ -z "$BIN_DIR" ]]; then
    invoking_home="${HOME:-/root}"
    if [[ -n "${SUDO_USER:-}" && "$SUDO_USER" != "root" ]]; then
        resolved_home="$(getent passwd "$SUDO_USER" 2>/dev/null \
            | awk -F: 'NR == 1 { print $6 }')"
        [[ -n "$resolved_home" ]] && invoking_home="$resolved_home"
    fi
    build_path="$invoking_home/.local/bin:$invoking_home/.cargo/bin:$PATH"
    echo "== build and install current AcTrail release =="
    (
        cd "$REPO_ROOT"
        env \
            "CARGO_HOME=${CARGO_HOME:-$invoking_home/.cargo}" \
            "RUSTUP_HOME=${RUSTUP_HOME:-$invoking_home/.rustup}" \
            "PATH=$build_path" \
            ACTRAIL_PLUGIN_DIR=/etc/actrail/plugins \
            ACTRAIL_SKIP_JAVA_AGENT_BUILD=1 \
            bash "$REPO_ROOT/scripts/install-release.sh"
    )
    BIN_DIR="$REPO_ROOT/target/release"
else
    [[ "$BIN_DIR" = /* ]] || BIN_DIR="$(cd -- "$REPO_ROOT" && pwd)/$BIN_DIR"
fi

for artifact in actraild actrailctl actrailviewer \
    libactrail_tls_payload_probe_sync.so; do
    [[ -f "$BIN_DIR/$artifact" ]] \
        || fail "release artifact is missing: $BIN_DIR/$artifact"
done

case "$PULL_POLICY" in
    always)
        docker pull "$BASE_IMAGE"
        ;;
    missing)
        docker image inspect "$BASE_IMAGE" >/dev/null 2>&1 \
            || docker pull "$BASE_IMAGE"
        ;;
    never)
        docker image inspect "$BASE_IMAGE" >/dev/null 2>&1 \
            || fail "base image is unavailable with pull policy never: $BASE_IMAGE"
        ;;
esac

echo "== build $DISTRO workload image =="
BUILD_CONTEXT="$(mktemp -d /tmp/actrail-container-deploy.XXXXXX)"
install -m 0644 "$MODULE_DIR/Dockerfile" "$BUILD_CONTEXT/Dockerfile"
install -m 0755 "$BIN_DIR/actrailctl" "$BUILD_CONTEXT/actrailctl"
install -m 0755 \
    "$BIN_DIR/libactrail_tls_payload_probe_sync.so" \
    "$BUILD_CONTEXT/libactrail_tls_payload_probe_sync.so"
docker build \
    --build-arg "BASE_IMAGE=$BASE_IMAGE" \
    --label org.opencontainers.image.title=actrail-container-auto \
    --label "org.opencontainers.image.base.name=$BASE_IMAGE" \
    -t "$WORKLOAD_IMAGE" \
    "$BUILD_CONTEXT"

case "$DISTRO" in
    openeuler) expected_os_id=openEuler ;;
    ubuntu) expected_os_id=ubuntu ;;
esac
docker run --rm \
    --entrypoint /bin/sh \
    -e "ACTRAIL_EXPECTED_OS_ID=$expected_os_id" \
    "$WORKLOAD_IMAGE" \
    -c 'set -eu
. /etc/os-release
test "$ID" = "$ACTRAIL_EXPECTED_OS_ID"
test -x /usr/local/bin/actrail-container-init
test -x /usr/local/bin/actrailctl
test -x /usr/local/bin/libactrail_tls_payload_probe_sync.so
/usr/local/bin/actrailctl --help >/dev/null'

echo "== install and start AcTrail host deployment =="
install_host_args=("$BIN_DIR")
if [[ -n "$OTEL_ENDPOINT" ]]; then
    install_host_args=(
        --otel-endpoint "$OTEL_ENDPOINT"
        --otel-attribute-mode "$OTEL_ATTRIBUTE_MODE"
        "$BIN_DIR"
    )
fi
"$MODULE_DIR/install-host.sh" "${install_host_args[@]}"

echo "== run an observed workload smoke test =="
if ! smoke_output="$(docker run --rm --user 0:0 \
    --security-opt seccomp=/etc/actrail/seccomp/actrail-notify.json \
    -v /run/actrail:/run/actrail:ro \
    -v /etc/actrail:/etc/actrail:ro \
    "$WORKLOAD_IMAGE" /bin/true 2>&1)"; then
    printf '%s\n' "$smoke_output" >&2
    fail "observed workload smoke test failed"
fi
printf '%s\n' "$smoke_output"
grep -Eq 'trace .* entered Active' <<<"$smoke_output" \
    || fail "observed workload did not create an active trace"

if [[ -n "$OTEL_ENDPOINT" ]]; then
    otel_status=''
    otel_delivered=0
    for _ in $(seq 1 60); do
        otel_status="$(/usr/local/bin/actraild \
            --config /etc/actrail/container-auto.conf \
            plugin status --instance container-auto.otel-http 2>&1 || true)"
        successful_batches="$(awk -F= \
            '$1 == "metric.otel_http.successful_batches" { print $2 }' \
            <<<"$otel_status")"
        last_error="$(awk -F= '$1 == "last_error" { print $2 }' \
            <<<"$otel_status")"
        if [[ "$successful_batches" =~ ^[0-9]+$ ]] \
            && (( successful_batches > 0 )) \
            && [[ "$last_error" == none ]]; then
            otel_delivered=1
            break
        fi
        sleep 0.25
    done
    if [[ "$otel_delivered" != 1 ]]; then
        printf '%s\n' "$otel_status" >&2
        fail "OTLP Collector did not accept the observed workload trace"
    fi
    echo "otel_delivery=verified"
fi

echo "ACTRAIL_CONTAINER_DEPLOYMENT_READY"
echo "distro=$DISTRO"
echo "base_image=$BASE_IMAGE"
echo "workload_image=$WORKLOAD_IMAGE"
echo "host_service=active"
echo "otel_endpoint=${OTEL_ENDPOINT:-disabled}"
cat <<EOF
Run an observed workload with:
  docker run --rm --user 0:0 \\
    --security-opt seccomp=/etc/actrail/seccomp/actrail-notify.json \\
    -v /run/actrail:/run/actrail:ro \\
    -v /etc/actrail:/etc/actrail:ro \\
    $WORKLOAD_IMAGE <agent-command> [args...]
EOF
