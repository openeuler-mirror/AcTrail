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


def tls_response_evidence_facts(
    payloads: str,
    accepted_tls_sources: list[PayloadSource],
    response_count: int,
) -> list[str]:
    outbound_tls = complete_payload_sources(payloads, accepted_tls_sources, "outbound")
    inbound_tls = complete_payload_sources(payloads, accepted_tls_sources, "inbound")
    return [
        f"accepted_tls_sources={format_payload_sources(accepted_tls_sources)}",
        f"outbound_tls_sources={format_payload_sources(outbound_tls)}",
        f"inbound_tls_sources={format_payload_sources(inbound_tls)}",
        f"tls_response_required={bool(outbound_tls)}",
        f"captured_response_payload_segments={response_count}",
    ]


def format_payload_sources(sources: list[PayloadSource]) -> str:
    if not sources:
        return "none"
    return ", ".join(f"{source}/{library}" for source, library in sources)


def payload_source_selection_selftest() -> list[str]:
    accepted_tls_sources = [("TlsUserSpace", "boringssl")]
    accepted_sources = [*accepted_tls_sources, ("Syscall", "socket-syscall")]
    http_payloads = "payload-1 trace-1 Syscall socket-syscall outbound Complete success\n"
    https_payloads = (
        "payload-1 trace-1 TlsUserSpace boringssl outbound Complete success\n"
        "payload-2 trace-1 TlsUserSpace boringssl inbound Complete success\n"
    )
    missing_tls_response = "payload-1 trace-1 TlsUserSpace boringssl outbound Complete success\n"
    if not payloads_have_required_llm_rows(http_payloads, accepted_sources, accepted_tls_sources):
        raise RuntimeError("plain HTTP socket payload was treated as requiring a TLS response")
    if required_tls_response_payload_count(http_payloads, accepted_tls_sources) != 0:
        raise RuntimeError("plain HTTP socket payload unexpectedly counted a TLS response")
    if not payloads_have_required_llm_rows(https_payloads, accepted_sources, accepted_tls_sources):
        raise RuntimeError("TLS payload with inbound response did not satisfy TLS response gate")
    if required_tls_response_payload_count(https_payloads, accepted_tls_sources) != 1:
        raise RuntimeError("TLS payload response count was not one")
    if payloads_have_required_llm_rows(missing_tls_response, accepted_sources, accepted_tls_sources):
        raise RuntimeError("TLS outbound payload without inbound TLS response passed the gate")
    return [
        "plain_http_outbound_source=Syscall/socket-syscall",
        "plain_http_tls_response_required=false",
        "https_outbound_source=TlsUserSpace/boringssl",
        "https_tls_response_required=true",
        "https_missing_inbound_tls_response_passes=false",
    ]


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


def require_non_empty_payload_text(text: str) -> None:
    if not text:
        raise RuntimeError("captured payload text was empty")
