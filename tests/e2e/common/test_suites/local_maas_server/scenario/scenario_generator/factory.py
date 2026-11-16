from __future__ import annotations

from pathlib import Path
from typing import Any

from ..model import (
    RequestExpectation,
    ResponseBlock,
    ResponseSpec,
    ResponseTemplate,
    ScenarioConfigurationError,
    ToolCall,
    UsageDelta,
)
from .action_pool_repository import ActionPoolRepository
from .config import ScenarioGeneratorConfig
from .impl import (
    ActionPoolGenerator,
    LoopGenerator,
    RandomGenerator,
    RecordedGenerator,
    ResponseGenerator,
    SequentialGenerator,
)
from .interface import ScenarioGenerator


class ScenarioGeneratorFactory:
    _RESPONSE_KEYS = frozenset({"type", "expect", "response"})
    _SEQUENTIAL_KEYS = frozenset({"type", "generators"})
    _LOOP_KEYS = frozenset({"type", "generator", "count"})
    _RANDOM_KEYS = frozenset({"type", "generators", "count", "seed"})
    _ACTION_POOL_KEYS = frozenset(
        {"type", "pools", "selection", "count", "seed"}
    )
    _EXPECTATION_KEYS = frozenset({"protocol", "stream", "model"})
    _RESPONSE_SPEC_KEYS = frozenset({"model", "blocks", "stop", "usage"})
    _USAGE_KEYS = frozenset({"output_tokens"})
    _TEXT_BLOCK_KEYS = frozenset({"type", "text", "chunks"})
    _TOOL_BLOCK_KEYS = frozenset({"type", "name", "arguments"})
    _STOP_REASONS = frozenset({"complete", "tool_call", "length"})

    def __init__(
        self,
        config: ScenarioGeneratorConfig,
        supported_protocols: frozenset[str],
    ):
        self._config = config
        self._supported_protocols = supported_protocols
        self._action_pools = ActionPoolRepository(config)
        self._nodes_created = 0

    def create(self, document: object) -> ScenarioGenerator:
        self._nodes_created = 0
        return self._create(
            document,
            "$",
            depth=1,
            allow_action_pool=True,
        )

    def _create(
        self,
        value: object,
        path: str,
        depth: int,
        allow_action_pool: bool,
    ) -> ScenarioGenerator:
        if depth > self._config.max_depth:
            raise ScenarioConfigurationError(
                f"{path} exceeds the configured generator depth limit of "
                f"{self._config.max_depth}"
            )
        self._nodes_created += 1
        if self._nodes_created > self._config.max_nodes:
            raise ScenarioConfigurationError(
                f"{path} exceeds the configured generator node limit of "
                f"{self._config.max_nodes}"
            )

        node = self._require_object(value, path)
        generator_type = self._require_non_empty_string(
            node.get("type"), f"{path}.type"
        )
        if generator_type == "response":
            return self._create_response(node, path)
        if generator_type == "sequential":
            return self._create_sequential(
                node, path, depth, allow_action_pool
            )
        if generator_type == "loop":
            return self._create_loop(node, path, depth, allow_action_pool)
        if generator_type == "random":
            return self._create_random(node, path, depth, allow_action_pool)
        if generator_type == "action_pool":
            if not allow_action_pool:
                raise ScenarioConfigurationError(
                    f"{path}.type cannot reference action_pool from an action"
                )
            return self._create_action_pool(node, path, depth)
        raise ScenarioConfigurationError(
            f"{path}.type must be response, sequential, loop, random, "
            "or action_pool"
        )

    def _create_response(
        self, node: dict[str, Any], path: str
    ) -> ResponseGenerator:
        self._reject_unknown_keys(node, self._RESPONSE_KEYS, path)
        return ResponseGenerator(
            ResponseTemplate(
                source_path=path,
                expectation=self._parse_expectation(
                    node.get("expect", {}), f"{path}.expect"
                ),
                response=self._parse_response(
                    node.get("response"), f"{path}.response"
                ),
            )
        )

    def create_recorded(
        self,
        tool_source: Path,
        message_source: Path,
    ) -> RecordedGenerator:
        for source in (tool_source, message_source):
            if not source.is_file():
                raise ScenarioConfigurationError(
                    f"recorded rounds file does not exist: {source}"
                )
        return RecordedGenerator(
            tool_source=tool_source,
            message_source=message_source,
            node_parser=self._parse_recorded_node,
            loop_exhausted_messages=self._config.loop_exhausted_messages,
            lazy_load_size=self._config.lazy_load_size,
        )

    def _parse_recorded_node(
        self,
        node: object,
        path: str,
    ) -> ResponseTemplate:
        node_object = self._require_object(node, path)
        self._reject_unknown_keys(
            node_object, self._RESPONSE_KEYS, path
        )
        return ResponseTemplate(
            source_path=path,
            expectation=self._parse_expectation(
                node_object.get("expect", {}),
                f"{path}.expect",
            ),
            response=self._parse_response(
                node_object.get("response"),
                f"{path}.response",
            ),
        )

    def _create_sequential(
        self,
        node: dict[str, Any],
        path: str,
        depth: int,
        allow_action_pool: bool,
    ) -> SequentialGenerator:
        self._reject_unknown_keys(node, self._SEQUENTIAL_KEYS, path)
        generators = self._parse_children(
            node.get("generators"),
            path,
            depth,
            allow_action_pool,
        )
        for index, generator in enumerate(generators[:-1]):
            if generator.is_infinite:
                raise ScenarioConfigurationError(
                    f"{path}.generators[{index}] is infinite, making later "
                    f"generators unreachable"
                )
        return SequentialGenerator(generators)

    def _create_loop(
        self,
        node: dict[str, Any],
        path: str,
        depth: int,
        allow_action_pool: bool,
    ) -> LoopGenerator:
        self._reject_unknown_keys(node, self._LOOP_KEYS, path)
        count = self._parse_optional_count(node, path)
        generator = self._create(
            node.get("generator"),
            f"{path}.generator",
            depth + 1,
            allow_action_pool,
        )
        if generator.is_infinite:
            raise ScenarioConfigurationError(
                f"{path}.generator must be finite so each loop iteration can finish"
            )
        return LoopGenerator(generator=generator, count=count)

    def _create_random(
        self,
        node: dict[str, Any],
        path: str,
        depth: int,
        allow_action_pool: bool,
    ) -> RandomGenerator:
        self._reject_unknown_keys(node, self._RANDOM_KEYS, path)
        count = self._parse_optional_count(node, path)
        seed = node.get("seed", self._config.random_seed)
        if isinstance(seed, bool) or not isinstance(seed, int):
            raise ScenarioConfigurationError(f"{path}.seed must be an integer")
        generators = self._parse_children(
            node.get("generators"),
            path,
            depth,
            allow_action_pool,
        )
        for index, generator in enumerate(generators):
            if generator.is_infinite:
                raise ScenarioConfigurationError(
                    f"{path}.generators[{index}] must be finite"
                )
        return RandomGenerator(
            generators=generators,
            count=count,
            seed=seed,
            node_path=path,
        )

    def _create_action_pool(
        self,
        node: dict[str, Any],
        path: str,
        depth: int,
    ) -> ActionPoolGenerator:
        self._reject_unknown_keys(node, self._ACTION_POOL_KEYS, path)
        raw_pools = node.get("pools")
        if not isinstance(raw_pools, list) or not raw_pools:
            raise ScenarioConfigurationError(
                f"{path}.pools must be a non-empty array"
            )
        pools = tuple(
            self._require_non_empty_string(
                pool,
                f"{path}.pools[{index}]",
            )
            for index, pool in enumerate(raw_pools)
        )
        if len(pools) != len(set(pools)):
            raise ScenarioConfigurationError(
                f"{path}.pools must not contain duplicates"
            )
        selection = self._require_non_empty_string(
            node.get("selection", "random"),
            f"{path}.selection",
        )
        if selection not in {"random", "sequential"}:
            raise ScenarioConfigurationError(
                f"{path}.selection must be random or sequential"
            )
        seed = node.get("seed", self._config.random_seed)
        if isinstance(seed, bool) or not isinstance(seed, int):
            raise ScenarioConfigurationError(f"{path}.seed must be an integer")

        action_root = self._config.action_pools_dir.resolve()
        actions = tuple(
            self._create(
                action.generator,
                (
                    "@action_pools/"
                    + action.source.relative_to(action_root).as_posix()
                ),
                depth + 1,
                allow_action_pool=False,
            )
            for action in self._action_pools.load(pools)
        )
        for index, action in enumerate(actions):
            if action.is_infinite:
                raise ScenarioConfigurationError(
                    f"{path} selected infinite action {index}"
                )
        return ActionPoolGenerator(
            actions=actions,
            selection=selection,
            count=self._parse_optional_count(node, path),
            seed=seed,
            node_path=path,
        )

    def _parse_children(
        self,
        value: object,
        path: str,
        depth: int,
        allow_action_pool: bool,
    ) -> tuple[ScenarioGenerator, ...]:
        if not isinstance(value, list) or not value:
            raise ScenarioConfigurationError(
                f"{path}.generators must be a non-empty array"
            )
        return tuple(
            self._create(
                child,
                f"{path}.generators[{index}]",
                depth + 1,
                allow_action_pool,
            )
            for index, child in enumerate(value)
        )

    def _parse_expectation(
        self, value: object, path: str
    ) -> RequestExpectation:
        expectation = self._require_object(value, path)
        self._reject_unknown_keys(expectation, self._EXPECTATION_KEYS, path)
        protocol = expectation.get("protocol")
        if protocol is not None:
            protocol = self._require_non_empty_string(
                protocol, f"{path}.protocol"
            )
            if protocol not in self._supported_protocols:
                supported = ", ".join(sorted(self._supported_protocols))
                raise ScenarioConfigurationError(
                    f"{path}.protocol must be one of: {supported}"
                )
        stream = expectation.get("stream")
        if stream is not None and not isinstance(stream, bool):
            raise ScenarioConfigurationError(f"{path}.stream must be a boolean")
        model = expectation.get("model")
        if model is not None:
            model = self._require_non_empty_string(model, f"{path}.model")
        return RequestExpectation(
            protocol=protocol,
            stream=stream,
            model=model,
        )

    def _parse_response(self, value: object, path: str) -> ResponseSpec:
        response = self._require_object(value, path)
        self._reject_unknown_keys(response, self._RESPONSE_SPEC_KEYS, path)
        model = response.get("model")
        if model is not None:
            model = self._require_non_empty_string(model, f"{path}.model")

        raw_blocks = response.get("blocks")
        if not isinstance(raw_blocks, list) or not raw_blocks:
            raise ScenarioConfigurationError(
                f"{path}.blocks must be a non-empty array"
            )
        blocks = tuple(
            self._parse_block(block, f"{path}.blocks[{index}]")
            for index, block in enumerate(raw_blocks)
        )

        has_tool_call = any(block.kind == "tool_call" for block in blocks)
        raw_stop = response.get("stop")
        if raw_stop is None:
            stop = "tool_call" if has_tool_call else "complete"
        else:
            stop = self._require_non_empty_string(raw_stop, f"{path}.stop")
            if stop not in self._STOP_REASONS:
                supported = ", ".join(sorted(self._STOP_REASONS))
                raise ScenarioConfigurationError(
                    f"{path}.stop must be one of: {supported}"
                )
        if has_tool_call != (stop == "tool_call"):
            raise ScenarioConfigurationError(
                f"{path}.stop and tool_call blocks must agree"
            )

        return ResponseSpec(
            model=model,
            blocks=blocks,
            stop=stop,
            usage_delta=self._parse_usage(
                response.get("usage", {}), f"{path}.usage"
            ),
        )

    def _parse_block(self, value: object, path: str) -> ResponseBlock:
        block = self._require_object(value, path)
        kind = self._require_non_empty_string(block.get("type"), f"{path}.type")
        if kind == "tool_call":
            self._reject_unknown_keys(block, self._TOOL_BLOCK_KEYS, path)
            return ResponseBlock(
                kind=kind,
                fragments=(),
                tool_call=ToolCall(
                    name=self._require_non_empty_string(
                        block.get("name"), f"{path}.name"
                    ),
                    arguments=self._require_object(
                        block.get("arguments"), f"{path}.arguments"
                    ),
                ),
            )
        if kind not in {"reasoning", "message"}:
            raise ScenarioConfigurationError(
                f"{path}.type must be reasoning, message, or tool_call"
            )
        self._reject_unknown_keys(block, self._TEXT_BLOCK_KEYS, path)
        return ResponseBlock(
            kind=kind,
            fragments=self._parse_fragments(block, path),
            tool_call=None,
        )

    def _parse_fragments(
        self, block: dict[str, Any], path: str
    ) -> tuple[str, ...]:
        has_text = "text" in block
        has_chunks = "chunks" in block
        if has_text == has_chunks:
            raise ScenarioConfigurationError(
                f"{path} must contain exactly one of text or chunks"
            )
        if has_text:
            return (
                self._require_non_empty_string(block["text"], f"{path}.text"),
            )
        chunks = block["chunks"]
        if not isinstance(chunks, list) or not chunks:
            raise ScenarioConfigurationError(
                f"{path}.chunks must be a non-empty array"
            )
        return tuple(
            self._require_non_empty_string(chunk, f"{path}.chunks[{index}]")
            for index, chunk in enumerate(chunks)
        )

    def _parse_usage(self, value: object, path: str) -> UsageDelta:
        usage = self._require_object(value, path)
        self._reject_unknown_keys(usage, self._USAGE_KEYS, path)
        return UsageDelta(
            output_tokens=self._non_negative_integer(
                usage.get("output_tokens", 0), f"{path}.output_tokens"
            ),
        )

    @staticmethod
    def _parse_optional_count(node: dict[str, Any], path: str) -> int | None:
        if "count" not in node:
            return None
        count = node["count"]
        if isinstance(count, bool) or not isinstance(count, int) or count <= 0:
            raise ScenarioConfigurationError(
                f"{path}.count must be a positive integer when present"
            )
        return count

    @staticmethod
    def _require_object(value: object, path: str) -> dict[str, Any]:
        if not isinstance(value, dict):
            raise ScenarioConfigurationError(f"{path} must be an object")
        return value

    @staticmethod
    def _require_non_empty_string(value: object, path: str) -> str:
        if not isinstance(value, str) or not value:
            raise ScenarioConfigurationError(
                f"{path} must be a non-empty string"
            )
        return value

    @staticmethod
    def _non_negative_integer(value: object, path: str) -> int:
        if isinstance(value, bool) or not isinstance(value, int) or value < 0:
            raise ScenarioConfigurationError(
                f"{path} must be a non-negative integer"
            )
        return value

    @staticmethod
    def _reject_unknown_keys(
        value: dict[str, Any], allowed: frozenset[str], path: str
    ) -> None:
        unknown = sorted(value.keys() - allowed)
        if unknown:
            raise ScenarioConfigurationError(
                f"{path} contains unknown fields: {', '.join(unknown)}"
            )
