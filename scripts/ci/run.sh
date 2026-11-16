#!/usr/bin/env bash
# 按统一文本格式汇报 CI 步骤、执行结果和失败详情。
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/common.sh
source "$script_dir/lib/common.sh"
cd -- "$script_dir/../.."

for arg in "$@"; do
    case "$arg" in
        -h|--help)
            printf 'Usage: bash scripts/ci/run.sh [-h|--help]\n' >&2
            printf 'Run all CI checks. Exit codes: 0 success, 1 check failure, 2 invalid arguments.\n' >&2
            exit 0
            ;;
        *)
            printf 'Unknown argument: %s (usage: bash scripts/ci/run.sh [-h|--help])\n' "$arg" >&2
            exit 2
            ;;
    esac
done

check_dependency bash git python3 cargo rustfmt dirname mktemp rm sed

# 进度和结果统一输出到 stderr，子脚本输出单独捕获。
exec 1>&2
steps=(
    'Module layout|scripts/ci/check-module-layout.sh'
    'Test hygiene|scripts/ci/check-tests-hygiene.py'
    'Rust formatting|scripts/ci/check-fmt.sh'
    'Documentation|scripts/ci/check-docs.py'
)
log_dir="$(mktemp -d "${TMPDIR:-/tmp}/actrail-ci-logs.XXXXXX")"
# 入口途中退出时清理目录。完成失败汇总后取消清理，以保留失败日志。
trap 'rm -r -- "$log_dir"' EXIT
failed_steps=()
failed_reasons=()
failed_commands=()
failed_logs=()
step_no=0
printf 'CI: %s checks\n' "${#steps[@]}"

# 成功只显示结果，失败显示完整详情并继续执行其余检查。
for step in "${steps[@]}"; do
    name="${step%%|*}"
    script="${step#*|}"
    case "$script" in
        *.py) interpreter=python3 ;;
        *.sh) interpreter=bash ;;
    esac
    command=("$interpreter" "$script")
    step_no=$((step_no + 1))
    output="$log_dir/step-$step_no.log"
    ci_title "$name"
    if "${command[@]}" > "$output" 2>&1; then
        ci_result "$name" 0
        rm -- "$output"
    else
        code=$?
        ci_result "$name" "$code"
        failed_steps+=("$name")
        failed_reasons+=("$(extract_reason < "$output")")
        printf -v rerun '%q ' "${command[@]}"
        failed_commands+=("${rerun% }")
        failed_logs+=("$output")
        ci_section "Failure details: $name"
        print_indented < "$output"
    fi
done

ci_title 'CI result'
if (( ${#failed_steps[@]} == 0 )); then
    printf '  [PASS] All checks passed\n'
    exit 0
fi

printf '  [FAIL] %s check(s) failed:\n' "${#failed_steps[@]}"
for ((i = 0; i < ${#failed_steps[@]}; i++)); do
    printf '        - %s\n' "${failed_steps[$i]}"
    printf '            Reason: %s\n' "${failed_reasons[$i]}"
    printf '            Rerun: %s\n' "${failed_commands[$i]}"
    if [[ -s "${failed_logs[$i]}" ]]; then
        printf '            Log: %s\n' "${failed_logs[$i]}"
    fi
done
trap - EXIT
exit 1
