"""Guest fixture defaults exported when the host prepares the bundle."""

from __future__ import annotations

import argparse
import shlex
from dataclasses import dataclass, fields
from pathlib import Path


@dataclass(frozen=True)
class GuestConstants:
    ACTRAIL_BUNDLE: str = "/actrail"
    ACTRAIL_WORK_BASE: str = "/tmp/actrail-e2e"
    ACTRAIL_SETTLE_SECONDS: int = 3
    ACTRAIL_READY_ATTEMPTS: int = 80
    ACTRAIL_READY_POLL_SECONDS: float = 0.25
    ACTRAIL_TLS_SERVER_HOLD_SECONDS: int = 6
    ACTRAIL_TLS_SERVER_SETTLE_SECONDS: int = 1
    ACTRAIL_TLS_ONLY_PORT: int = 4433
    ACTRAIL_EBPF_ONLY_PORT: int = 4434
    ACTRAIL_COMBO_PORT: int = 4435
    ACTRAIL_CERT_RSA_BITS: int = 2048
    ACTRAIL_CERT_DAYS: int = 1
    REQUIRE_EBPF: int = 0

    def shell_defaults(self) -> str:
        lines = ["# Guest fixture defaults generated from constants.py."]
        for definition in fields(self):
            name = definition.name
            value = shlex.quote(str(getattr(self, name)))
            lines.append(f'if [ -z "${{{name}:-}}" ]; then {name}={value}; fi')
        return "\n".join(lines) + "\n"


CASE_CONSTANTS = GuestConstants()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    arguments.output.write_text(CASE_CONSTANTS.shell_defaults(), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
