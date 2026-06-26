"""Payload viewer checks."""

from __future__ import annotations

import re
import time
from pathlib import Path

from .config import actrail_command, run_checked


def wait_for_payloads(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path | None,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    head: str,
    required_fragments: list[str],
) -> str:
    for _ in range(attempts):
        run_checked(actrail_command(actrailctl, config, "list-traces"), echo=False)
        output = run_checked(
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
        if all(fragment in output for fragment in required_fragments):
            print(output, end="", flush=True)
            return output
        time.sleep(sleep_sec)
    raise RuntimeError("viewer payload output missed required fragments")


def wait_for_payloads_any(
    actrailctl: Path,
    actrailviewer: Path,
    config: Path | None,
    trace_id: int,
    attempts: int,
    sleep_sec: float,
    head: str,
    required_options: list[list[str]],
) -> str:
    for _ in range(attempts):
        run_checked(actrail_command(actrailctl, config, "list-traces"), echo=False)
        output = run_checked(
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
        if any(all(fragment in output for fragment in option) for option in required_options):
            print(output, end="", flush=True)
            return output
        time.sleep(sleep_sec)
    raise RuntimeError("viewer payload output missed every accepted fragment set")


def require_complete_payload_rows(payloads: str, source: str, library: str) -> int:
    count = 0
    for line in payloads.splitlines():
        if not re.match(r"^\s*payload-\d+\s+", line):
            continue
        if source not in line or library not in line:
            continue
        if "Truncated" in line or "success" not in line:
            raise RuntimeError(f"payload row is not complete/successful: {line}")
        count += 1
    if count == 0:
        raise RuntimeError(f"no {source} {library} payload rows found")
    return count


def require_complete_payload_rows_any(
    payloads: str,
    accepted: list[tuple[str, str]],
    direction: str | None = None,
) -> int:
    count = 0
    incomplete = 0
    for line in payloads.splitlines():
        if not re.match(r"^\s*payload-\d+\s+", line):
            continue
        if direction is not None and direction not in line:
            continue
        if not any(source in line and library in line for source, library in accepted):
            continue
        if "Truncated" in line or "success" not in line:
            incomplete += 1
            continue
        count += 1
    if count == 0:
        detail = ", ".join(f"{source} {library}" for source, library in accepted)
        direction_text = f" {direction}" if direction is not None else ""
        raise RuntimeError(
            f"no complete accepted{direction_text} payload rows found: {detail}; "
            f"incomplete_matching_rows={incomplete}"
        )
    return count
