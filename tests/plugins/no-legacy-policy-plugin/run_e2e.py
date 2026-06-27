#!/usr/bin/env python3
from __future__ import annotations

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]

REJECTED_TEXT = [
    "plugin_policy_host",
    "policy_plugin_contract",
    "PluginPolicyEngine",
    "PolicyPluginHost",
    "policy-plugin-host",
]

REJECTED_PATHS = [
    ROOT / "crates/adapters/policy/plugin_host",
    ROOT / "crates/contracts/policy/plugin",
]


def run(cmd: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )


def main() -> int:
    for path in REJECTED_PATHS:
        if path.exists():
            raise RuntimeError(f"legacy policy plugin path still exists: {path}")

    files = [
        ROOT / "Cargo.toml",
        ROOT / "crates/apps/daemon/Cargo.toml",
        *list((ROOT / "crates/apps/daemon/src").rglob("*.rs")),
        *list((ROOT / "crates/core/config/src").rglob("*.rs")),
        *list((ROOT / "crates/core/model/src").rglob("*.rs")),
    ]
    for path in files:
        raw = path.read_text(encoding="utf-8")
        for text in REJECTED_TEXT:
            if text in raw:
                raise RuntimeError(f"legacy policy plugin text {text} remains in {path}")

    metadata = run(["cargo", "metadata", "--format-version", "1", "--no-deps"])
    if metadata.returncode != 0:
        raise RuntimeError(f"cargo metadata failed\n{metadata.stdout[-4000:]}")
    for text in ["plugin_policy_host", "policy_plugin_contract"]:
        if text in metadata.stdout:
            raise RuntimeError(f"legacy policy plugin package remains in cargo metadata: {text}")

    print("legacy_policy_plugin_boundary=removed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
