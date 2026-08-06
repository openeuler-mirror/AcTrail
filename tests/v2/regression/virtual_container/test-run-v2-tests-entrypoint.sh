#!/usr/bin/env bash
# Verify that the public V2 entrypoint explains checkout-local preparation.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
V2_RUNNER="$ROOT_DIR/deploy/virtual-container/host/run-v2-tests.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

TEMP_ROOT="${TMPDIR:-/tmp}"
WORK_DIR="$(mktemp -d "${TEMP_ROOT%/}/actrail-v2-entrypoint.XXXXXX")"
trap 'rm -rf -- "$WORK_DIR"' EXIT
FAKE_REPO="$WORK_DIR/repo"
FAKE_BIN="$WORK_DIR/bin"
ROOT_FAKE_BIN="$WORK_DIR/root-bin"
FAKE_HOME="$WORK_DIR/home"
PYTHON_BIN="$(command -v python3)"

install -d \
  "$FAKE_REPO/deploy/virtual-container/host" \
  "$FAKE_REPO/tests/v2/regression" \
  "$FAKE_BIN" \
  "$ROOT_FAKE_BIN" \
  "$FAKE_HOME"
install -m 0755 "$V2_RUNNER" \
  "$FAKE_REPO/deploy/virtual-container/host/run-v2-tests.sh"

cat >"$FAKE_REPO/tests/v2/regression/test_all.py" <<'PY'
import os
from pathlib import Path

Path(os.environ["ACTRAIL_ENTRYPOINT_TEST_RUNNER_MARKER"]).touch()
raise SystemExit(86)
PY

cat >"$FAKE_BIN/sudo" <<EOF
#!/usr/bin/env bash
touch "$WORK_DIR/sudo-called"
exit 86
EOF
chmod 0755 "$FAKE_BIN/sudo"

cat >"$FAKE_BIN/id" <<'EOF'
#!/usr/bin/env bash
if [[ "${1:-}" == "-u" ]]; then
  echo 1000
  exit 0
fi
exec /usr/bin/id "$@"
EOF
chmod 0755 "$FAKE_BIN/id"

cat >"$ROOT_FAKE_BIN/id" <<'EOF'
#!/usr/bin/env bash
if [[ "${1:-}" == "-u" ]]; then
  echo 0
  exit 0
fi
exec /usr/bin/id "$@"
EOF
chmod 0755 "$ROOT_FAKE_BIN/id"

set +e
contracts_output="$({
  HOME="$FAKE_HOME" \
  PATH="$FAKE_BIN:/usr/bin:/bin" \
  VIRTUAL_CONTAINER_E2E_SCOPE=contracts \
    "$FAKE_REPO/deploy/virtual-container/host/run-v2-tests.sh" \
      --case virtual_container \
      --color never
} 2>&1)"
contracts_rc=$?
set -e

[[ "$contracts_rc" -eq 86 ]] \
  || fail "contracts scope did not reach the runner: rc=$contracts_rc output=$contracts_output"
[[ -e "$WORK_DIR/sudo-called" ]] \
  || fail "contracts scope with no profile did not request runner privileges"
rm -f "$WORK_DIR/sudo-called"

set +e
root_output="$({
  ACTRAIL_ENTRYPOINT_TEST_RUNNER_MARKER="$WORK_DIR/runner-called" \
  HOME="$FAKE_HOME" \
  PATH="$ROOT_FAKE_BIN:$(dirname "$PYTHON_BIN"):/usr/bin:/bin" \
  VIRTUAL_CONTAINER_E2E_SCOPE=contracts \
    "$FAKE_REPO/deploy/virtual-container/host/run-v2-tests.sh" \
      --case virtual_container \
      --color never
} 2>&1)"
root_rc=$?
set -e

[[ "$root_rc" -eq 86 ]] \
  || fail "root contracts scope did not reach the runner: rc=$root_rc output=$root_output"
[[ -e "$WORK_DIR/runner-called" ]] \
  || fail "root contracts scope did not execute the runner"
rm -f "$WORK_DIR/runner-called"

set +e
missing_output="$({
  HOME="$FAKE_HOME" \
  PATH="$FAKE_BIN:/usr/bin:/bin" \
  VIRTUAL_CONTAINER_E2E_SCOPE=all \
    "$FAKE_REPO/deploy/virtual-container/host/run-v2-tests.sh" \
      --color never
} 2>&1)"
missing_rc=$?
set -e

[[ "$missing_rc" -eq 2 ]] \
  || fail "missing profile exited $missing_rc instead of 2: $missing_output"
[[ ! -e "$WORK_DIR/sudo-called" ]] \
  || fail "missing profile reached sudo instead of failing before execution"
grep -Fq 'missing machine-local V2 profile' <<<"$missing_output" \
  || fail "missing profile diagnostic is absent"
grep -Fq 'local/kata/v2-test-profile.json' <<<"$missing_output" \
  || fail "missing profile diagnostic omits the expected path"
grep -Fq 'prepare-v2-test-artifacts.py' <<<"$missing_output" \
  || fail "missing profile diagnostic omits the preparation command"
grep -Fq 'same checkout' <<<"$missing_output" \
  || fail "missing profile diagnostic omits the checkout-local requirement"

echo "RUN_V2_TESTS_ENTRYPOINT_TEST_OK"
