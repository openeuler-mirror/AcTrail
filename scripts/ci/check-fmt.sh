#!/usr/bin/env bash
# 检查整个 Rust workspace 的 rustfmt 排版。
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."
exec cargo fmt --all --check -- --color never
