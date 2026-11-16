from __future__ import annotations

from pathlib import Path
from typing import Any

from utils.json import StrictJsonDecoder, StrictJsonError

from ..model import (
    ScenarioConfigurationError,
    ScenarioDefinition,
    ScenarioMeta,
)
from .config import ScenarioGeneratorConfig
from .factory import ScenarioGeneratorFactory


class ScenarioLoader:
    """Load scenarios from ``*.meta.json`` plus lazily loaded sequence data.

    Listing only reads meta files. Selecting a scenario loads its sequence:
    recorded scenarios stream ``*.tool.jsonl`` / ``*.message.jsonl`` rounds on
    demand, other generator types read their ``*.seq.json`` document.
    """

    def __init__(
        self,
        config: ScenarioGeneratorConfig,
        supported_protocols: frozenset[str],
    ):
        self._config = config
        self._supported_protocols = supported_protocols

    def load(self) -> ScenarioDefinition:
        scenario_id = self._config.template_name
        meta = self.load_meta(
            self._config.templates_dir,
            self._config.max_template_bytes,
            scenario_id,
        )
        factory = ScenarioGeneratorFactory(
            self._config,
            self._supported_protocols,
        )
        if meta.generator_type == "recorded":
            generator = factory.create_recorded(
                self._resolve_source_path(meta.tool_source),
                self._resolve_source_path(meta.message_source),
            )
            source = self._resolve_source_path(meta.tool_source)
        else:
            source = self._resolve_source_path(meta.sequence)
            document = self._read_document(
                source,
                self._config.max_template_bytes,
            )
            generator = factory.create(document)
        return ScenarioDefinition(
            scenario_id=scenario_id,
            description=meta.description,
            generator=generator,
            source=source,
        )

    @classmethod
    def load_meta(
        cls,
        templates_dir: Path,
        max_template_bytes: int,
        scenario_id: str,
    ) -> ScenarioMeta:
        root = templates_dir.resolve()
        if not root.is_dir():
            raise ScenarioConfigurationError(
                f"scenario template directory does not exist: {root}"
            )
        meta_path = cls._resolve_within(root, f"{scenario_id}.meta.json")
        if not meta_path.is_file():
            raise ScenarioConfigurationError(
                f"scenario meta file does not exist: {meta_path}"
            )
        document = cls._read_document(meta_path, max_template_bytes)
        return cls._parse_meta(scenario_id, document)

    @classmethod
    def available_scenarios(
        cls,
        templates_dir: Path,
        max_template_bytes: int,
    ) -> tuple[ScenarioMeta, ...]:
        root = templates_dir.resolve()
        if not root.is_dir():
            raise ScenarioConfigurationError(
                f"scenario template directory does not exist: {root}"
            )
        metas = []
        for source in sorted(root.rglob("*.meta.json")):
            if not source.is_file():
                continue
            relative = source.relative_to(root).as_posix()
            scenario_id = relative.removesuffix(".meta.json")
            document = cls._read_document(source, max_template_bytes)
            metas.append(cls._parse_meta(scenario_id, document))
        return tuple(metas)

    def _resolve_source_path(self, relative: str) -> Path:
        root = self._config.templates_dir.resolve()
        source = self._resolve_within(root, relative)
        if not source.is_file():
            raise ScenarioConfigurationError(
                f"scenario source does not exist: {source}"
            )
        return source

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
    def _resolve_within(root: Path, relative: str) -> Path:
        source = (root / relative).resolve()
        try:
            source.relative_to(root)
        except ValueError as error:
            raise ScenarioConfigurationError(
                f"scenario source must stay inside its directory: {relative}"
            ) from error
        return source

    @staticmethod
    def _parse_meta(scenario_id: str, document: object) -> ScenarioMeta:
        if not isinstance(document, dict):
            raise ScenarioConfigurationError(
                f"scenario meta root must be a JSON object: {scenario_id}"
            )
        description = document.get("description")
        if not isinstance(description, str) or not description.strip():
            raise ScenarioConfigurationError(
                f"$.description must be a non-empty string: {scenario_id}"
            )
        generator_type = document.get("type")
        if not isinstance(generator_type, str) or not generator_type:
            raise ScenarioConfigurationError(
                f"$.type must be a non-empty string: {scenario_id}"
            )
        meta = ScenarioMeta(
            scenario_id=scenario_id,
            description=description.strip(),
            generator_type=generator_type,
            infinite=_optional_bool(document.get("infinite")),
            tool_source=_optional_string(document.get("tool_source")),
            message_source=_optional_string(document.get("message_source")),
            sequence=_optional_string(document.get("sequence")),
            rounds=_optional_int(document.get("rounds")),
            tool_rounds=_optional_int(document.get("tool_rounds")),
            message_rounds=_optional_int(document.get("message_rounds")),
            tools=tuple(
                tool
                for tool in document.get("tools", ())
                if isinstance(tool, str)
            ),
        )
        if meta.generator_type == "recorded":
            if not meta.tool_source or not meta.message_source:
                raise ScenarioConfigurationError(
                    f"recorded scenario meta needs tool_source and "
                    f"message_source: {scenario_id}"
                )
        elif not meta.sequence:
            raise ScenarioConfigurationError(
                f"scenario meta needs a sequence reference: {scenario_id}"
            )
        return meta


def _optional_int(value: object) -> int | None:
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return value


def _optional_bool(value: object) -> bool | None:
    if not isinstance(value, bool):
        return None
    return value


def _optional_string(value: object) -> str | None:
    if not isinstance(value, str) or not value:
        return None
    return value
