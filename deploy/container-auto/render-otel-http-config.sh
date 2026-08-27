#!/usr/bin/env bash
# Render the host-side OTLP/HTTP exporter config for the Docker deployment.
set -euo pipefail

TEMPLATE=''
OUTPUT=''
ENDPOINT=''
ATTRIBUTE_MODE=metadata-only
CHECK_ONLY=0
TEMP_PATH=''

usage() {
    cat <<'EOF'
Usage: render-otel-http-config.sh --template FILE --output FILE --endpoint URL [options]

Options:
  --attribute-mode MODE  metadata-only or full (default: metadata-only)
  --check                Validate without writing an output file
  -h, --help             Show this help
EOF
}

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

cleanup() {
    if [[ -n "$TEMP_PATH" && -f "$TEMP_PATH" ]]; then
        rm -f -- "$TEMP_PATH"
    fi
}
trap cleanup EXIT

while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --template)
            [[ "$#" -ge 2 ]] || fail "--template requires a value"
            TEMPLATE=$2
            shift 2
            ;;
        --output)
            [[ "$#" -ge 2 ]] || fail "--output requires a value"
            OUTPUT=$2
            shift 2
            ;;
        --endpoint)
            [[ "$#" -ge 2 ]] || fail "--endpoint requires a value"
            ENDPOINT=$2
            shift 2
            ;;
        --attribute-mode)
            [[ "$#" -ge 2 ]] || fail "--attribute-mode requires a value"
            ATTRIBUTE_MODE=$2
            shift 2
            ;;
        --check)
            CHECK_ONLY=1
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

[[ -f "$TEMPLATE" ]] || fail "template does not exist: $TEMPLATE"
if [[ "$CHECK_ONLY" != 1 ]]; then
    [[ -n "$OUTPUT" ]] || fail "--output is required"
    [[ -d "$(dirname -- "$OUTPUT")" ]] \
        || fail "output directory does not exist: $(dirname -- "$OUTPUT")"
fi

case "$ENDPOINT" in
    http://*|https://*) ;;
    *) fail "--endpoint must start with http:// or https://" ;;
esac
if [[ "$ENDPOINT" == *'"'* || "$ENDPOINT" == *'\'* \
    || "$ENDPOINT" =~ [[:space:]] ]]; then
    fail "--endpoint contains characters unsafe for TOML"
fi
case "$ENDPOINT" in
    *'?'*|*'#'*) fail "--endpoint must not contain a query or fragment" ;;
esac

remainder=${ENDPOINT#*://}
authority=${remainder%%/*}
[[ -n "$authority" && "$authority" != "$remainder" ]] \
    || fail "--endpoint must contain a host and /v1/traces path"
[[ "$authority" != :* ]] || fail "--endpoint must contain a host"
[[ "$authority" != *@* ]] \
    || fail "--endpoint must not contain userinfo"
[[ "/${remainder#*/}" == */v1/traces ]] \
    || fail "--endpoint path must end in /v1/traces"

normalized_endpoint=$(printf '%s' "$ENDPOINT" | tr '[:upper:]' '[:lower:]')
case "$normalized_endpoint" in
    *collector_host*|*collector-host*|*placeholder*|*replace_me*|*replace-me*|*change_me*|*change-me*)
        fail "--endpoint must be a concrete Collector address"
        ;;
esac

case "$ATTRIBUTE_MODE" in
    metadata-only|full) ;;
    *) fail "--attribute-mode must be metadata-only or full" ;;
esac

case "$ENDPOINT" in
    https://*) allow_insecure=false ;;
    *) allow_insecure=true ;;
esac

if [[ "$CHECK_ONLY" == 1 ]]; then
    exit 0
fi

TEMP_PATH=$(mktemp "${OUTPUT}.tmp.XXXXXX")
if ! awk \
    -v endpoint="$ENDPOINT" \
    -v allow_insecure="$allow_insecure" \
    -v attribute_mode="$ATTRIBUTE_MODE" '
    /^[[:space:]]*endpoint[[:space:]]*=/ {
        endpoint_count++
        print "endpoint = \"" endpoint "\""
        next
    }
    /^[[:space:]]*allow_insecure[[:space:]]*=/ {
        insecure_count++
        print "allow_insecure = " allow_insecure
        next
    }
    /^[[:space:]]*attribute_mode[[:space:]]*=/ {
        attribute_count++
        print "attribute_mode = \"" attribute_mode "\""
        next
    }
    { print }
    END {
        if (endpoint_count != 1 || insecure_count != 1 || attribute_count != 1) {
            exit 42
        }
    }
' "$TEMPLATE" >"$TEMP_PATH"; then
    fail "otel-http template must contain exactly one endpoint, allow_insecure, and attribute_mode assignment"
fi

install -m 0640 -- "$TEMP_PATH" "$OUTPUT"
rm -f -- "$TEMP_PATH"
TEMP_PATH=''
