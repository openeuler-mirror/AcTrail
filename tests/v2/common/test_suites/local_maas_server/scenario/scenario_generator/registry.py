"""Scenario discovery: templates dir resolution and candidate listing."""

from __future__ import annotations

import os
from pathlib import Path

from ..model import ScenarioMeta
from .loader import ScenarioLoader


TEMPLATES_DIR_ENV = "LOCAL_MAAS_TEMPLATES_DIR"
# matches the replay server CLI default (config.DEFAULT_MAX_TEMPLATE_BYTES)
DEFAULT_MAX_TEMPLATE_BYTES = 1_048_576
DEFAULT_TEMPLATES_DIR = Path(__file__).resolve().parent / "templates"


class ScenarioRegistry:
    """Single source for where scenarios live and what is available.

    The templates directory resolves from ``LOCAL_MAAS_TEMPLATES_DIR`` when
    set, otherwise relative to this package's own location. Replay server and
    external tooling (e.g. the overall benchmark) share this resolution so
    listing and loading always agree.
    """

    def __init__(
        self,
        templates_dir: Path = DEFAULT_TEMPLATES_DIR,
        max_template_bytes: int = DEFAULT_MAX_TEMPLATE_BYTES,
    ):
        self._templates_dir = templates_dir
        self._max_template_bytes = max_template_bytes

    @staticmethod
    def resolve_templates_dir() -> Path:
        override = os.environ.get(TEMPLATES_DIR_ENV)
        if override:
            return Path(override)
        return DEFAULT_TEMPLATES_DIR

    @classmethod
    def from_environment(cls) -> ScenarioRegistry:
        return cls(cls.resolve_templates_dir())

    def available_scenarios(self) -> tuple[ScenarioMeta, ...]:
        scenarios = ScenarioLoader.available_scenarios(
            self._templates_dir,
            self._max_template_bytes,
        )
        return tuple(
            sorted(
                scenarios,
                key=lambda scenario: (
                    0 if scenario.scenario_id.startswith("recorded/") else 1,
                    scenario.scenario_id,
                ),
            )
        )

    def scenario_meta(self, scenario_id: str) -> ScenarioMeta:
        return ScenarioLoader.load_meta(
            self._templates_dir,
            self._max_template_bytes,
            scenario_id,
        )
