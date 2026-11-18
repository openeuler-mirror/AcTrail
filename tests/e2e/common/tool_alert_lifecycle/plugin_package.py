from __future__ import annotations

import os
import shutil
import tomllib
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ToolAlertPluginPackage:
    repo: Path
    work_dir: Path
    source_manifest: Path

    @property
    def manifest(self) -> Path:
        return self.work_dir / "source-plugin" / self.source_manifest.name

    def prepare(self) -> None:
        with self.source_manifest.open("rb") as stream:
            manifest = tomllib.load(stream)

        target_dir = self.repo / (os.environ.get("CARGO_TARGET_DIR") or "target")
        artifact = manifest["runtime"]["wasm"]["artifact_path"]
        sources = [
            self.source_manifest,
            target_dir / "wasm32-wasip2" / "release" / artifact,
        ]
        sources.extend(
            self.source_manifest.with_name(alert["payload_schema_ref"])
            for alert in manifest["outputs"]["alerts"].values()
        )
        missing = [str(source) for source in sources if not source.is_file()]
        if missing:
            raise FileNotFoundError(
                f"tool alert plugin package files missing at {', '.join(missing)}"
            )

        directory = self.manifest.parent
        directory.mkdir(exist_ok=True)
        for source in sources:
            shutil.copyfile(source, directory / source.name)
