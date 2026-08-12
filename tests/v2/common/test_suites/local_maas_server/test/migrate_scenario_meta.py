#!/usr/bin/env python3
"""One-time migration: split scenario files into meta + sequence files.

For every standalone scenario under the templates directory (excluding the
action_pools resource directory) this writes:

  recorded:  <id>.meta.json + <id>.tool.jsonl + <id>.message.jsonl
  other:     <id>.meta.json + <id>.seq.json

and deletes the original combined ``<id>.json``. Idempotent on the new layout.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[6]))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from test.e2e_support import SERVER_DIR  # noqa: E402

from scenario.scenario_generator import (  # noqa: E402
    ScenarioGeneratorConfig,
    ScenarioGeneratorFactory,
)
from scenario.scenario_generator.stats import scenario_round_stats  # noqa: E402


SUPPORTED_PROTOCOLS = frozenset({"openai", "anthropic"})
TEMPLATES_DIR = SERVER_DIR / "scenario" / "scenario_generator" / "templates"
ACTION_POOLS_DIR = TEMPLATES_DIR / "action_pools"


def _write_jsonl(path: Path, nodes: list[dict[str, object]]) -> None:
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


def _tool_names(tool_nodes: list[dict[str, object]]) -> list[str]:
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


def main() -> int:
    config = ScenarioGeneratorConfig(
        templates_dir=TEMPLATES_DIR,
        action_pools_dir=ACTION_POOLS_DIR,
        template_name="unused",
        max_template_bytes=1_048_576,
        max_depth=64,
        max_nodes=4096,
        random_seed=0,
    )
    factory = ScenarioGeneratorFactory(config, SUPPORTED_PROTOCOLS)
    migrated: list[tuple[str, str]] = []
    for source in sorted(TEMPLATES_DIR.rglob("*.json")):
        if not source.is_file():
            continue
        if ACTION_POOLS_DIR in source.parents:
            continue
        relative = source.relative_to(TEMPLATES_DIR).as_posix()
        if relative.endswith((".meta.json", ".seq.json")):
            continue
        base = relative.removesuffix(".json")
        document = json.loads(source.read_text(encoding="utf-8"))
        generator_type = document.get("type")
        meta: dict[str, object] = {
            "name": base,
            "description": document.get("description", ""),
            "type": generator_type,
        }
        if generator_type == "recorded":
            tool_nodes = document.get("tool", [])
            message_nodes = document.get("message", [])
            _write_jsonl(
                source.with_suffix(".tool.jsonl"),
                tool_nodes,
            )
            _write_jsonl(
                source.with_suffix(".message.jsonl"),
                message_nodes,
            )
            meta.update(
                {
                    "infinite": False,
                    "tool_source": f"{base}.tool.jsonl",
                    "message_source": f"{base}.message.jsonl",
                    "rounds": len(tool_nodes) + len(message_nodes),
                    "tool_rounds": len(tool_nodes),
                    "message_rounds": len(message_nodes),
                    "tools": _tool_names(tool_nodes),
                }
            )
        else:
            sequence = {
                key: value
                for key, value in document.items()
                if key != "description"
            }
            generator = factory.create(sequence)
            stats = scenario_round_stats(generator)
            meta.update(
                {
                    "infinite": generator.is_infinite,
                    "sequence": f"{base}.seq.json",
                    "rounds": stats.rounds,
                    "tool_rounds": stats.tool_rounds,
                    "message_rounds": stats.message_rounds,
                    "tools": list(stats.tools),
                }
            )
            source.with_suffix(".seq.json").write_text(
                json.dumps(sequence, ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8",
            )
        source.with_suffix(".meta.json").write_text(
            json.dumps(meta, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        source.unlink()
        migrated.append((relative, str(generator_type)))
    refreshed = _refresh_generator_metas(factory)
    print(f"migrated {len(migrated)} scenarios")
    for relative, kind in migrated:
        print(f"  {relative} -> {kind}")
    print(f"refreshed {len(refreshed)} generator metas")
    return 0


def _refresh_generator_metas(
    factory: ScenarioGeneratorFactory,
) -> list[str]:
    refreshed: list[str] = []
    for meta_path in sorted(TEMPLATES_DIR.rglob("*.meta.json")):
        document = json.loads(meta_path.read_text(encoding="utf-8"))
        if document.get("type") == "recorded":
            continue
        base = meta_path.name.removesuffix(".meta.json")
        seq_path = meta_path.parent / f"{base}.seq.json"
        generator = factory.create(
            json.loads(seq_path.read_text(encoding="utf-8"))
        )
        stats = scenario_round_stats(generator)
        document.update(
            {
                "infinite": generator.is_infinite,
                "rounds": stats.rounds,
                "tool_rounds": stats.tool_rounds,
                "message_rounds": stats.message_rounds,
                "tools": list(stats.tools),
            }
        )
        meta_path.write_text(
            json.dumps(document, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        refreshed.append(meta_path.name)
    return refreshed


if __name__ == "__main__":
    raise SystemExit(main())
