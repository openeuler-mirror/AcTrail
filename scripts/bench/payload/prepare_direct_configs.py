"""Materialize a payload benchmark config directory with the explicit TLS override."""
from __future__ import annotations

import argparse
import shutil
import tomllib
from pathlib import Path

from scripts.bench.overall.runtime.config_patch import ConfigPatch


class DirectBenchmarkConfig:
    def __init__(self, source: Path, output: Path):
        self.source, self.output = source, output

    def write(self):
        self.output.mkdir(parents=True, exist_ok=False)
        for path in self.source.iterdir():
            if path.is_file():
                shutil.copy2(path, self.output / path.name)
        profile = ConfigPatch(self.output / "P.toml")
        overlay = tomllib.loads((Path(__file__).parent / "configs/tls-bpf-copy.toml").read_text())
        profile.values["payload"]["tls"].update(overlay["payload"]["tls"])
        (self.output / "P.toml").write_text("\n".join(
            f"{key} = {ConfigPatch._render(value)}" for key, value in profile.values.items()) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=Path(__file__).parent / "configs")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    DirectBenchmarkConfig(args.source, args.out).write()
