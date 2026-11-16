from __future__ import annotations

import hashlib
import json
import time
from dataclasses import dataclass
from pathlib import Path

from scenario.scenario_generator import ScenarioGeneratorConfig, ScenarioLoader
from utils.json import StrictJsonDecoder, StrictJsonError

from .parser import RecordedResponse


class RecordFinalizeError(RuntimeError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True, slots=True)
class FinalizeResult:
    scenario_id: str
    scenario_file: Path
    responses: int


class RecordStore:
    def __init__(
        self,
        recordings_dir: Path,
        *,
        templates_dir: Path,
        supported_protocols: frozenset[str],
        max_template_bytes: int,
        max_depth: int,
        max_nodes: int,
        random_seed: int,
    ):
        self._dir = recordings_dir
        self._templates_dir = templates_dir
        self._supported_protocols = supported_protocols
        self._max_template_bytes = max_template_bytes
        self._max_depth = max_depth
        self._max_nodes = max_nodes
        self._random_seed = random_seed

    def create_cache(self, session_id: str) -> Path:
        path = self._cache_path(session_id)
        try:
            path.touch(mode=0o600, exist_ok=False)
        except FileExistsError as error:
            raise RecordFinalizeError(
                "cache_exists",
                f"recording cache already exists: {path}",
            ) from error
        return path

    def append(
        self,
        cache_path: Path,
        recorded: RecordedResponse,
    ) -> None:
        document = recorded.to_cache_document()
        with cache_path.open("a", encoding="utf-8") as cache_file:
            cache_file.write(
                json.dumps(
                    document,
                    ensure_ascii=False,
                    separators=(",", ":"),
                )
                + "\n"
            )
            cache_file.flush()

    def finalize(
        self,
        session_id: str,
        cache_path: Path,
        scenario_id: str,
    ) -> FinalizeResult:
        records = self._read_records(cache_path)
        if not records:
            raise RecordFinalizeError(
                "empty_recording",
                f"session {session_id} has no recorded responses",
            )
        artifact_id = self._artifact_id(scenario_id, cache_path)
        playable_id = f"recorded/{artifact_id}"
        recorded_dir = self._templates_dir / "recorded"
        recorded_dir.mkdir(parents=True, exist_ok=True)
        tool_file = recorded_dir / f"{artifact_id}.tool.jsonl"
        message_file = recorded_dir / f"{artifact_id}.message.jsonl"
        meta_file = recorded_dir / f"{artifact_id}.meta.json"
        if any(
            path.exists()
            for path in (tool_file, message_file, meta_file)
        ):
            raise RecordFinalizeError(
                "scenario_exists",
                f"recorded scenario already exists: {playable_id}",
            )
        tool_nodes = [
            self._response_node(record)
            for record in records
            if self._has_tool_call(record)
        ]
        message_nodes = [
            self._response_node(record)
            for record in records
            if not self._has_tool_call(record)
        ]
        self._write_jsonl(tool_file, tool_nodes)
        self._write_jsonl(message_file, message_nodes)
        meta_file.write_text(
            json.dumps(
                {
                    "name": playable_id,
                    "description": (
                        f"Recorded scenario {playable_id} "
                        f"from session {session_id}"
                    ),
                    "type": "recorded",
                    "infinite": False,
                    "tool_source": (
                        f"recorded/{artifact_id}.tool.jsonl"
                    ),
                    "message_source": (
                        f"recorded/{artifact_id}.message.jsonl"
                    ),
                    "rounds": len(tool_nodes) + len(message_nodes),
                    "tool_rounds": len(tool_nodes),
                    "message_rounds": len(message_nodes),
                    "tools": self._round_tool_names(tool_nodes),
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        self._validate(playable_id)
        try:
            cache_path.unlink()
        except OSError as error:
            for path in (meta_file, tool_file, message_file):
                path.unlink(missing_ok=True)
            raise RecordFinalizeError(
                "cache_cleanup_failed",
                f"cannot remove recording cache {cache_path}: {error}",
            ) from error
        return FinalizeResult(
            scenario_id=playable_id,
            scenario_file=meta_file,
            responses=len(records),
        )

    @staticmethod
    def _write_jsonl(
        path: Path,
        nodes: list[dict[str, object]],
    ) -> None:
        with path.open("w", encoding="utf-8") as node_file:
            for node in nodes:
                node_file.write(
                    json.dumps(
                        node,
                        ensure_ascii=False,
                        separators=(",", ":"),
                    )
                    + "\n"
                )

    @staticmethod
    def _round_tool_names(
        tool_nodes: list[dict[str, object]],
    ) -> list[str]:
        names: set[str] = set()
        for node in tool_nodes:
            response = node.get("response")
            if not isinstance(response, dict):
                continue
            for block in response.get("blocks", ()):
                if (
                    isinstance(block, dict)
                    and block.get("type") == "tool_call"
                    and isinstance(block.get("name"), str)
                ):
                    names.add(block["name"])
        return sorted(names)

    @staticmethod
    def _has_tool_call(record: dict[str, object]) -> bool:
        blocks = record.get("blocks")
        if not isinstance(blocks, list):
            return False
        return any(
            isinstance(block, dict)
            and block.get("type") == "tool_call"
            for block in blocks
        )

    @staticmethod
    def _artifact_id(scenario_id: str, cache_path: Path) -> str:
        try:
            digest = hashlib.sha256(
                cache_path.read_bytes()
            ).hexdigest()
        except OSError:
            digest = hashlib.sha256(
                str(cache_path).encode("utf-8")
            ).hexdigest()
        suffix = f"{time.strftime('%Y%m%d%H%M%S')}-{digest[:5]}"
        return f"{scenario_id}-{suffix}"

    def _cache_path(self, session_id: str) -> Path:
        return self._dir / f"{session_id}.cache.jsonl"

    def _read_records(
        self, cache_path: Path
    ) -> list[dict[str, object]]:
        records: list[dict[str, object]] = []
        try:
            with cache_path.open("rb") as cache_file:
                for line in cache_file:
                    if not line.strip():
                        continue
                    try:
                        document = StrictJsonDecoder().decode_utf8(line)
                    except StrictJsonError as error:
                        raise RecordFinalizeError(
                            "invalid_cache",
                            f"invalid cache line in {cache_path}: {error}",
                        ) from error
                    if not isinstance(document, dict):
                        raise RecordFinalizeError(
                            "invalid_cache",
                            f"cache line is not an object: {cache_path}",
                        )
                    records.append(document)
        except OSError as error:
            raise RecordFinalizeError(
                "cache_unreadable",
                f"cannot read recording cache {cache_path}: {error}",
            ) from error
        return records

    @staticmethod
    def _response_node(
        record: dict[str, object],
    ) -> dict[str, object]:
        expect: dict[str, object] = {}
        protocol = record.get("protocol")
        if isinstance(protocol, str) and protocol:
            expect["protocol"] = protocol
        stream = record.get("stream")
        if isinstance(stream, bool):
            expect["stream"] = stream
        response: dict[str, object] = {
            "blocks": record.get("blocks"),
            "stop": record.get("stop"),
            "usage": {"output_tokens": record.get("output_tokens", 0)},
        }
        model = record.get("model")
        if isinstance(model, str) and model:
            response["model"] = model
        return {
            "type": "response",
            "expect": expect,
            "response": response,
        }

    def _validate(self, scenario_id: str) -> None:
        config = ScenarioGeneratorConfig(
            templates_dir=self._templates_dir,
            action_pools_dir=self._templates_dir / "action_pools",
            template_name=scenario_id,
            max_template_bytes=self._max_template_bytes,
            max_depth=self._max_depth,
            max_nodes=self._max_nodes,
            random_seed=self._random_seed,
        )
        try:
            ScenarioLoader(
                config,
                self._supported_protocols,
            ).load()
        except Exception as error:
            source.unlink(missing_ok=True)
            raise RecordFinalizeError(
                "invalid_recorded_scenario",
                f"recorded scenario failed validation: {error}",
            ) from error
