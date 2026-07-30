from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from utils.json import StrictJsonDecoder, StrictJsonError

from ..model import ScenarioConfigurationError
from .config import ScenarioGeneratorConfig


@dataclass(frozen=True, slots=True)
class ActionDocument:
    source: Path
    generator: dict[str, object]


class ActionPoolRepository:
    def __init__(self, config: ScenarioGeneratorConfig):
        self._root = config.action_pools_dir.resolve()
        self._max_document_bytes = config.max_template_bytes
        self._max_documents = config.max_nodes
        self._pool_cache: dict[str, tuple[Path, ...]] = {}
        self._document_cache: dict[Path, ActionDocument] = {}

    def load(self, pool_names: tuple[str, ...]) -> tuple[ActionDocument, ...]:
        sources: dict[Path, None] = {}
        for pool_name in pool_names:
            for source in self._sources(pool_name):
                sources[source] = None
                if len(sources) > self._max_documents:
                    raise ScenarioConfigurationError(
                        "action_pool selects more files than the configured "
                        f"generator node limit of {self._max_documents}"
                    )
        return tuple(self._document(source) for source in sources)

    def _sources(self, pool_name: str) -> tuple[Path, ...]:
        cached = self._pool_cache.get(pool_name)
        if cached is not None:
            return cached
        if not self._root.is_dir():
            raise ScenarioConfigurationError(
                f"action pool directory does not exist: {self._root}"
            )
        relative = Path(pool_name)
        if relative.is_absolute():
            raise ScenarioConfigurationError(
                f"action pool path must be relative: {pool_name}"
            )
        pool_root = (self._root / relative).resolve()
        try:
            pool_root.relative_to(self._root)
        except ValueError as error:
            raise ScenarioConfigurationError(
                f"action pool path must stay inside its directory: {pool_name}"
            ) from error
        if not pool_root.is_dir():
            raise ScenarioConfigurationError(
                f"action pool does not exist: {pool_name}"
            )

        sources = tuple(self._walk_json(pool_root))
        if not sources:
            raise ScenarioConfigurationError(
                f"action pool contains no JSON actions: {pool_name}"
            )
        self._pool_cache[pool_name] = sources
        return sources

    def _walk_json(self, pool_root: Path):
        discovered = 0
        for directory, directory_names, file_names in os.walk(
            pool_root,
            followlinks=False,
        ):
            directory_names.sort()
            for file_name in sorted(file_names):
                if not file_name.endswith(".json"):
                    continue
                source = (Path(directory) / file_name).resolve()
                try:
                    source.relative_to(self._root)
                except ValueError as error:
                    raise ScenarioConfigurationError(
                        f"action file must stay inside its directory: {source}"
                    ) from error
                if not source.is_file():
                    continue
                discovered += 1
                if discovered > self._max_documents:
                    raise ScenarioConfigurationError(
                        f"action pool contains more than "
                        f"{self._max_documents} JSON files: {pool_root}"
                    )
                yield source

    def _document(self, source: Path) -> ActionDocument:
        cached = self._document_cache.get(source)
        if cached is not None:
            return cached
        with source.open("rb") as action_file:
            raw_document = action_file.read(self._max_document_bytes + 1)
        if len(raw_document) > self._max_document_bytes:
            raise ScenarioConfigurationError(
                f"action exceeds the {self._max_document_bytes}-byte startup "
                f"limit: {source}"
            )
        try:
            document = StrictJsonDecoder().decode_utf8(raw_document)
        except StrictJsonError as error:
            raise ScenarioConfigurationError(
                f"invalid action JSON: {source}: {error}"
            ) from error
        if not isinstance(document, dict):
            raise ScenarioConfigurationError(
                f"action root must be a JSON object: {source}"
            )
        description = document.get("description")
        if not isinstance(description, str) or not description.strip():
            raise ScenarioConfigurationError(
                "$.description must be a non-empty English string: "
                f"{source}"
            )
        generator = dict(document)
        del generator["description"]
        action = ActionDocument(source=source, generator=generator)
        self._document_cache[source] = action
        return action
