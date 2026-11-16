#!/usr/bin/env bash
# CI helpers for dependency checks, console output, and failure details.

export LC_ALL=C

# Use ANSI 16 colors on terminals; NO_COLOR disables and CLICOLOR_FORCE enables color.
ci_log() {
    local level="$1" color
    shift
    case "$level" in
        PASS) color=92 ;;
        FAIL) color=91 ;;
        SKIP|WARN) color=93 ;;
        *) color=96 ;;
    esac
    if [[ -z "${NO_COLOR:-}" ]] &&
       { [[ "${CLICOLOR_FORCE:-0}" != 0 ]] ||
         [[ -t 2 && -n "${TERM:-}" && "$TERM" != dumb ]]; }; then
        printf '\033[%sm[%s]\033[0m %s\n' "$color" "$level" "$*" >&2
    else
        printf '[%s] %s\n' "$level" "$*" >&2
    fi
}

# Report all missing commands and return failure if any are missing.
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
        ci_log PASS "$step"
    else
        ci_log FAIL "$step (exit $code)"
    fi
}

print_indented() {
    local line
    while IFS= read -r line || [[ -n "$line" ]]; do
        printf '  %s\n' "$line" >&2
    done
}

# Return the first FAIL line without its prefix, or the first nonempty line.
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
