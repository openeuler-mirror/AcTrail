from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class ScenarioGeneratorConfig:
    templates_dir: Path
    template_name: str
    max_template_bytes: int
    max_depth: int
    max_nodes: int
    random_seed: int

    def __post_init__(self) -> None:
        if not self.template_name:
            raise ValueError("template_name must be non-empty")
        if self.max_template_bytes <= 0:
            raise ValueError("max_template_bytes must be positive")
        if self.max_depth <= 0:
            raise ValueError("max_depth must be positive")
        if self.max_nodes <= 0:
            raise ValueError("max_nodes must be positive")
