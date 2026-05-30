"""Small evidence extraction helpers for regression cases."""

from __future__ import annotations

import json
from collections.abc import Iterable


BODY_TEXT_PREVIEW_CHARS = 100


def output_line(output: str, prefix: str) -> str:
    for line in output.splitlines():
        if line.startswith(prefix):
            return line
    return f"missing {prefix}"


def output_lines(output: str, prefixes: tuple[str, ...]) -> str:
    return ", ".join(output_line(output, prefix) for prefix in prefixes)


def evidence_lines(output: str, prefix: str = "evidence.") -> list[str]:
    return [line for line in output.splitlines() if line.startswith(prefix)]


def expected_found_detail(expected: str, found: str | Iterable[str]) -> str:
    return f"\n        - expected: {expected}\n        - found: {found_detail(found)}"


def found_detail(found: str | Iterable[str]) -> str:
    if isinstance(found, str):
        return found
    return fact_list(found)


def fact_list(items: Iterable[str]) -> str:
    return "\n" + "\n".join(f"            - {item}" for item in items)


def evidence_summary_facts(output: str, prefixes: tuple[str, ...]) -> list[str]:
    return [summary_line(output, prefix) for prefix in prefixes]


def line_containing(output: str, fragments: tuple[str, ...]) -> str:
    for line in output.splitlines():
        if all(fragment in line for fragment in fragments):
            return line
    return "missing line containing " + ", ".join(repr(fragment) for fragment in fragments)


def summary_line(output: str, prefix: str) -> str:
    line = output_line(output, prefix)
    if line.startswith("missing "):
        return line
    if prefix.endswith("body_text_json="):
        return format_body_preview(prefix, line)
    key, _, value = line.partition("=")
    if key == "evidence.llm_response":
        return line
    return f"{key} = {value}"


def format_body_preview(prefix: str, line: str) -> str:
    key = prefix.rstrip("=")
    _, _, raw_value = line.partition("=")
    value = decode_json_string(raw_value)
    preview = value[:BODY_TEXT_PREVIEW_CHARS]
    if len(value) > BODY_TEXT_PREVIEW_CHARS:
        preview = f"{preview}... [truncated]"
    return f"{key} = {json.dumps(preview, ensure_ascii=False)}"


def decode_json_string(value: str) -> str:
    try:
        decoded = json.loads(value)
    except json.JSONDecodeError:
        return value
    if isinstance(decoded, str):
        return decoded
    return value
