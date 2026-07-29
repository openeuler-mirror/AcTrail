#!/usr/bin/env bash
set -euo pipefail

threshold_seconds="${1:-60}"
overrun_seconds="${2:-5}"
ready_path="${3:-}"
release_path="${4:-}"

case "$threshold_seconds:$overrun_seconds" in
    *[!0-9:]* | :* | *:)
        echo "usage: $0 [threshold-seconds] [overrun-seconds]" >&2
        exit 2
        ;;
esac
if [[ -n "$ready_path" && -z "$release_path" ]] ||
    [[ -z "$ready_path" && -n "$release_path" ]]; then
    echo "ready-path and release-path must be provided together" >&2
    exit 2
fi

duration_seconds=$((threshold_seconds + overrun_seconds))
printf 'ACTRAIL_LONG_COMMAND_START threshold_seconds=%s duration_seconds=%s\n' \
    "$threshold_seconds" "$duration_seconds"
if [[ -n "$ready_path" ]]; then
    printf 'ready\n' >"$ready_path"
fi
sleep "$duration_seconds"
if [[ -n "$release_path" ]]; then
    while [[ ! -f "$release_path" ]]; do
        sleep 0.1
    done
fi
printf 'ACTRAIL_LONG_COMMAND_COMPLETE duration_seconds=%s\n' "$duration_seconds"
