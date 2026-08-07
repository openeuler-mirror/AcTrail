from __future__ import annotations

from pathlib import Path

from utils.json import StrictJsonDecoder, StrictJsonError

from ..model import (
    ScenarioConfigurationError,
    ScenarioDefinition,
    ScenarioSummary,
)
from .config import ScenarioGeneratorConfig
from .factory import ScenarioGeneratorFactory


class ScenarioLoader:
    def __init__(
        self,
        config: ScenarioGeneratorConfig,
        supported_protocols: frozenset[str],
    ):
        self._config = config
        self._supported_protocols = supported_protocols

    def load(self) -> ScenarioDefinition:
        source, scenario_id = self._resolve_source()
        document = self._read_document(
            source,
            self._config.max_template_bytes,
        )
        description, generator_document = self._split_document(
            document,
            source,
        )
        generator = ScenarioGeneratorFactory(
            self._config,
            self._supported_protocols,
        ).create(generator_document)
        return ScenarioDefinition(
            scenario_id=scenario_id,
            description=description,
            generator=generator,
            source=source,
        )

    @classmethod
    def available_scenarios(
        cls,
        templates_dir: Path,
        max_template_bytes: int,
    ) -> tuple[ScenarioSummary, ...]:
        root = templates_dir.resolve()
        if not root.is_dir():
            raise ScenarioConfigurationError(
                f"scenario template directory does not exist: {root}"
            )
        summaries = []
        for source in sorted(root.rglob("*.json")):
            if not source.is_file():
                continue
            document = cls._read_document(source, max_template_bytes)
            description, _generator_document = cls._split_document(
                document,
                source,
            )
            summaries.append(
                ScenarioSummary(
                    scenario_id=(
                        source.relative_to(root).with_suffix("").as_posix()
                    ),
                    description=description,
                )
            )
        return tuple(summaries)

    @staticmethod
    def _read_document(source: Path, max_template_bytes: int) -> object:
        with source.open("rb") as template_file:
            raw_document = template_file.read(max_template_bytes + 1)
        if len(raw_document) > max_template_bytes:
            raise ScenarioConfigurationError(
                f"scenario exceeds the {max_template_bytes}-byte startup "
                f"limit: {source}"
            )
        try:
            return StrictJsonDecoder().decode_utf8(raw_document)
        except StrictJsonError as error:
            raise ScenarioConfigurationError(
                f"invalid scenario JSON: {source}: {error}"
            ) from error

    @staticmethod
    def _split_document(
        document: object,
        source: Path,
    ) -> tuple[str, dict[str, object]]:
        if not isinstance(document, dict):
            raise ScenarioConfigurationError(
                f"scenario root must be a JSON object: {source}"
            )
        description = document.get("description")
        if not isinstance(description, str) or not description.strip():
            raise ScenarioConfigurationError(
                "$.description must be a non-empty English string: "
                f"{source}"
            )
        generator_document = dict(document)
        del generator_document["description"]
        return description.strip(), generator_document

    def _resolve_source(self) -> tuple[Path, str]:
        root = self._config.templates_dir.resolve()
        if not root.is_dir():
            raise ScenarioConfigurationError(
                f"scenario template directory does not exist: {root}"
            )
        relative = Path(self._config.template_name)
        if relative.is_absolute():
            raise ScenarioConfigurationError(
                "scenario template name must be relative"
            )
        if relative.suffix == "":
            relative = relative.with_suffix(".json")
        source = (root / relative).resolve()
        try:
            normalized = source.relative_to(root)
        except ValueError as error:
            raise ScenarioConfigurationError(
                f"scenario template must stay inside its directory: "
                f"{self._config.template_name}"
            ) from error
        if not source.is_file():
            raise ScenarioConfigurationError(
                f"scenario template does not exist: {source}"
            )
        return source, normalized.with_suffix("").as_posix()
