#!/usr/bin/env bash
# Check rustfmt formatting across the Rust workspace.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."
exec cargo fmt --all --check -- --color never
