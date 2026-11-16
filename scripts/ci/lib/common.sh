#!/usr/bin/env bash
# CI 公共工具：依赖检查、控制台格式和失败原因提取。

export LC_ALL=C

# 检查调用方列出的命令，输出所有缺失项并返回失败。
check_dependency() {
    local tool missing=0
    for tool in "$@"; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            printf 'FAIL [DEPENDENCY]: Required command not found: %s\n' "$tool" >&2
            missing=1
        fi
    done
    return "$missing"
}

ci_title() {
    printf '\n================================================================\n  %s\n================================================================\n' "$*" >&2
}

ci_section() {
    printf '\n--- %s ---\n' "$*" >&2
}

ci_result() {
    local step="$1" code="$2"
    if (( code == 0 )); then
        printf '  [PASS] %s\n' "$step" >&2
    else
        printf '  [FAIL] %s (exit %s)\n' "$step" "$code" >&2
    fi
}

print_indented() {
    local line
    while IFS= read -r line || [[ -n "$line" ]]; do
        printf '  %s\n' "$line" >&2
    done
}

# 优先取首条 FAIL 行并去掉标记，否则取首个非空行。
extract_reason() {
    local line stripped first_nonempty=''
    while IFS= read -r line || [[ -n "$line" ]]; do
        stripped="${line#"${line%%[![:space:]]*}"}"
        if [[ -z "$first_nonempty" && -n "$stripped" ]]; then
            first_nonempty="$stripped"
        fi
        case "$stripped" in
            FAIL[[:space:]]*|FAIL:*)
                printf '%s\n' "$stripped" |
                    sed -E 's/^FAIL[[:space:]]*(\[[^]]*\][[:space:]]*)?[:-]?[[:space:]]*//'
                return 0
                ;;
        esac
    done
    printf '%s\n' "${first_nonempty:-(No detailed output)}"
}
