"""Claude Code E2E config rendering and TLS runtime selection."""

from __future__ import annotations

import os
from pathlib import Path

from runtime import ClaudeTlsRuntime, resolve_claude_tls_runtime

PayloadSource = tuple[str, str]

TLS_ENABLED_PLACEHOLDER = "__CLAUDE_TLS_ENABLED__"
TLS_BINARY_PLACEHOLDER = "__CLAUDE_TLS_BINARY__"
TLS_RESOLVER_PLACEHOLDER = "__CLAUDE_TLS_RESOLVER__"
TLS_LIBRARY_PLACEHOLDER = "__CLAUDE_TLS_LIBRARY__"
TLS_PATTERN_PLACEHOLDER = "__CLAUDE_TLS_PATTERN_PATH__"
SECCOMP_NOTIFY_PLACEHOLDER = "__CLAUDE_SECCOMP_NOTIFY_ENABLED__"
TLS_REQUIRED_CAPABILITY_PLACEHOLDER = "__CLAUDE_TLS_REQUIRED_CAPABILITY__"


def resolve_optional_claude_tls_runtime(values: dict[str, str]) -> ClaudeTlsRuntime | None:
    try:
        runtime = resolve_claude_tls_runtime(values)
    except Exception as error:
        if os.environ.get("CLAUDE_TLS_BINARY"):
            raise
        print(f"claude_tls_runtime=disabled {error}")
        return None
    print(f"claude_tls_runtime={runtime.detail}")
    return runtime


def accepted_payload_sources(
    tls_runtime: ClaudeTlsRuntime | None,
) -> list[PayloadSource]:
    sources = [("Syscall", "socket-syscall")]
    if tls_runtime is not None:
        sources.insert(0, ("TlsUserSpace", tls_runtime.library))
    return sources


def accepted_tls_payload_sources(
    tls_runtime: ClaudeTlsRuntime | None,
) -> list[PayloadSource]:
    if tls_runtime is None:
        return []
    return [("TlsUserSpace", tls_runtime.library)]


def write_resolved_operator_config(
    template_path: Path,
    output_path: Path,
    tls_runtime: ClaudeTlsRuntime | None,
) -> None:
    raw = template_path.read_text(encoding="utf-8")
    if tls_runtime is None:
        replacements = {
            TLS_ENABLED_PLACEHOLDER: "false",
            TLS_BINARY_PLACEHOLDER: "disabled",
            TLS_RESOLVER_PLACEHOLDER: "openssl-symbols",
            TLS_LIBRARY_PLACEHOLDER: "openssl",
            TLS_PATTERN_PLACEHOLDER: "disabled",
            SECCOMP_NOTIFY_PLACEHOLDER: "true",
            TLS_REQUIRED_CAPABILITY_PLACEHOLDER: "# tls-plaintext-payload disabled",
        }
    else:
        replacements = {
            TLS_ENABLED_PLACEHOLDER: "true",
            TLS_BINARY_PLACEHOLDER: str(tls_runtime.binary),
            TLS_RESOLVER_PLACEHOLDER: tls_runtime.resolver,
            TLS_LIBRARY_PLACEHOLDER: tls_runtime.library,
            TLS_PATTERN_PLACEHOLDER: tls_runtime.pattern_path,
            SECCOMP_NOTIFY_PLACEHOLDER: "true",
            TLS_REQUIRED_CAPABILITY_PLACEHOLDER: "required_capability = tls-plaintext-payload",
        }
    for placeholder, value in replacements.items():
        if placeholder not in raw:
            if placeholder == TLS_REQUIRED_CAPABILITY_PLACEHOLDER:
                continue
            raise RuntimeError(f"{template_path} does not contain {placeholder}")
        raw = raw.replace(placeholder, value)
    output_path.write_text(raw, encoding="utf-8")
