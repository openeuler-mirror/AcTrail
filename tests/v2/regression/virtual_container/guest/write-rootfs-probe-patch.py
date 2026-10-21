#!/usr/bin/env python3
"""Generate the paths used to probe an installed Guest configuration offline."""

from __future__ import annotations

import argparse
from pathlib import Path
import sys
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parents[5]))

from tests.v2.common.patch_builder import PatchBuilder


class GuestRootfsProbePatch:
    def __init__(self, rootfs: Path, bundle: Path) -> None:
        self._rootfs = rootfs.resolve()
        self._bundle = bundle.resolve()

    def save(self, output: Path) -> None:
        with (self._rootfs / "etc/actrail/operator.conf").open("rb") as source:
            installed = tomllib.load(source)
        plugins = installed["plugins"]["startup"]["load"]
        for plugin in plugins:
            for field in ("manifest", "plugin_config"):
                plugin[field] = str(self._rootfs / plugin[field].lstrip("/"))
        PatchBuilder().with_values({
            "payload": {"tls": {
                "sync_runtime_library_path": str(
                    self._bundle / "libactrail_tls_payload_probe_sync.so"
                ),
            }},
            "plugins": {"startup": {"load": plugins}},
        }).build().save(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rootfs", type=Path, required=True)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    GuestRootfsProbePatch(args.rootfs, args.bundle).save(args.output)
