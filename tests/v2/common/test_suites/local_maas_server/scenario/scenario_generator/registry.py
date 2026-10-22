"""Scenario discovery: templates dir resolution and candidate listing."""

from __future__ import annotations

import os
from pathlib import Path

from ..model import ScenarioMeta
from .loader import ScenarioLoader

from tests.v2.common.test_suites.local_maas_server.constants import DEFAULTS


TEMPLATES_DIR_ENV = "LOCAL_MAAS_TEMPLATES_DIR"
DEFAULT_TEMPLATES_DIR = DEFAULTS.scenario_generator_dir / "templates"


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
        max_template_bytes: int = DEFAULTS.max_template_bytes,
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
