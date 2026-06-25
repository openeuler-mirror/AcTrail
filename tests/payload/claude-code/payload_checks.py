"""Claude Code E2E payload gates."""

from __future__ import annotations

import re
import time
from pathlib import Path

from common import actrail_command, require_complete_payload_rows_any, run_checked
from config_template import PayloadSource, accepted_tls_payload_sources
from runtime import ClaudeTlsRuntime


def wait_for_llm_payloads(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path | None,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    head: str,
    accepted_sources: list[PayloadSource],
    accepted_tls_sources: list[PayloadSource],
) -> str:
    for _ in range(attempts):
        run_checked(actrail_command(actrailctl, config, "list-traces"), echo=False)
        payloads = run_checked(
            actrail_command(
                actrailviewer,
                config,
                "payloads",
                "--trace-id",
                str(trace_id),
                "--head",
                head,
            ),
            echo=False,
        )
        if payloads_have_required_llm_rows(payloads, accepted_sources, accepted_tls_sources):
            print(payloads, end="")
            return payloads
        time.sleep(sleep_sec)
    detail = ", ".join(f"{source}/{library}" for source, library in accepted_sources)
    raise RuntimeError(f"viewer did not show Claude Code LLM request/response payload rows for {detail}")


def payloads_contain_complete_source(
    payloads: str,
    accepted_sources: list[PayloadSource],
    direction: str,
) -> bool:
    return bool(complete_payload_sources(payloads, accepted_sources, direction))


def payloads_have_required_llm_rows(
    payloads: str,
    accepted_sources: list[PayloadSource],
    accepted_tls_sources: list[PayloadSource],
) -> bool:
    if not payloads_contain_complete_source(payloads, accepted_sources, "outbound"):
        return False
    return tls_response_requirement_satisfied(payloads, accepted_tls_sources)


def tls_response_requirement_satisfied(
    payloads: str,
    accepted_tls_sources: list[PayloadSource],
) -> bool:
    if not accepted_tls_sources:
        return True
    if not payloads_contain_complete_source(payloads, accepted_tls_sources, "outbound"):
        return True
    return payloads_contain_complete_source(payloads, accepted_tls_sources, "inbound")


def complete_payload_sources(
    payloads: str,
    accepted_sources: list[PayloadSource],
    direction: str,
) -> list[PayloadSource]:
    matched: list[PayloadSource] = []
    for line in payloads.splitlines():
        if not re.match(r"^\s*payload-\d+\s+", line):
            continue
        if direction not in line or "Complete" not in line or "success" not in line:
            continue
        for source in accepted_sources:
            boundary, library = source
            if source not in matched and boundary in line and library in line:
                matched.append(source)
    return matched


def require_tls_response_payloads(payloads: str, tls_runtime: ClaudeTlsRuntime | None) -> int:
    return required_tls_response_payload_count(payloads, accepted_tls_payload_sources(tls_runtime))


def required_tls_response_payload_count(
    payloads: str,
    accepted_tls_sources: list[PayloadSource],
) -> int:
    if not accepted_tls_sources:
        return 0
    if not payloads_contain_complete_source(payloads, accepted_tls_sources, "outbound"):
        return 0
    return require_complete_payload_rows_any(payloads, accepted_tls_sources, direction="inbound")


def format_payload_sources(sources: list[PayloadSource]) -> str:
    if not sources:
        return "none"
    return ", ".join(f"{source}/{library}" for source, library in sources)


def payload_texts(
    actrailviewer: Path,
    config: Path | None,
    trace_id: int,
    payloads: str,
    fetch_count: int,
) -> str:
    texts: list[str] = []
    for segment_id in parse_segment_ids(payloads)[:fetch_count]:
        texts.append(
            run_checked(
                actrail_command(
                    actrailviewer,
                    config,
                    "payload",
                    "--trace-id",
                    str(trace_id),
                    "--segment-id",
                    segment_id,
                    "--format",
                    "text",
                ),
                echo=False,
            )
        )
    if not texts:
        raise RuntimeError("payloads output did not contain segment ids")
    return "\n".join(texts)


def parse_segment_ids(payloads: str) -> list[str]:
    ids: list[str] = []
    for line in payloads.splitlines():
        match = re.match(r"^\s*(payload-\d+)\s+", line)
        if match:
            ids.append(match.group(1))
    return ids


def retained_payload_text_bytes(text: str) -> int:
    return len(text.strip().encode("utf-8"))


def require_no_retained_payload_text(text: str) -> None:
    if retained_payload_text_bytes(text) != 0:
        raise RuntimeError("captured Claude LLM payload text was retained")
