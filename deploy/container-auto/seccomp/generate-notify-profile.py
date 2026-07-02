#!/usr/bin/env python3
"""Generate AcTrail's seccomp-notify profile from a pinned Moby baseline."""

from __future__ import annotations

import argparse
import hashlib
import json
import urllib.request
from pathlib import Path
from typing import Any


SOURCE_URL = (
    "https://raw.githubusercontent.com/moby/profiles/"
    "refs/tags/seccomp/v0.2.1/seccomp/default.json"
)
SOURCE_SHA256 = "536529b665dd0972c37bfb569f5d4ac8a53592e7b00752bc39ff063ca9864c74"
DEFAULT_OUTPUT = Path(__file__).with_name("actrail-notify.json")


def source_bytes(path: Path | None) -> bytes:
    if path is not None:
        return path.read_bytes()
    with urllib.request.urlopen(SOURCE_URL, timeout=30) as response:
        return response.read()


def verify_source(raw: bytes) -> None:
    actual = hashlib.sha256(raw).hexdigest()
    if actual != SOURCE_SHA256:
        raise SystemExit(
            f"Moby profile SHA-256 mismatch: expected {SOURCE_SHA256}, got {actual}"
        )


def generate(document: dict[str, Any]) -> dict[str, Any]:
    syscalls = document.get("syscalls")
    if not isinstance(syscalls, list):
        raise SystemExit("Moby profile has no syscall rule list")

    removed = 0
    for rule in syscalls:
        names = rule.get("names")
        includes = rule.get("includes", {})
        caps = includes.get("caps", []) if isinstance(includes, dict) else []
        if (
            isinstance(names, list)
            and "pidfd_getfd" in names
            and "CAP_SYS_PTRACE" in caps
        ):
            names.remove("pidfd_getfd")
            removed += 1

    if removed != 1:
        raise SystemExit(
            f"expected one CAP_SYS_PTRACE-gated pidfd_getfd rule, found {removed}"
        )

    # Rule order is not semantically significant for these ALLOW entries, but
    # keeping the AcTrail delta next to the broad baseline allowlist makes
    # review and regeneration straightforward.
    syscalls.insert(
        1,
        {
            "names": ["pidfd_getfd"],
            "action": "SCMP_ACT_ALLOW",
        },
    )
    return document


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--source",
        type=Path,
        help="read the pinned Moby profile from this file instead of downloading it",
    )
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()

    raw = source_bytes(args.source)
    verify_source(raw)
    document = generate(json.loads(raw))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, indent=2, sort_keys=False) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
