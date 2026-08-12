"""opencode agent backend for the overall replay benchmark.

Writes a project-local ``opencode.json`` pointing an OpenAI-compatible provider
("bench") at the local replay server, then runs ``opencode run`` headlessly.
"""

from __future__ import annotations

import json
from pathlib import Path

from .. import AgentBackend, resolve_binary


MODEL = "deepseek-v4-flash"


def prepare(work_dir: Path, replay_port: int) -> None:
    config = {
        "provider": {
            "bench": {
                "npm": "@ai-sdk/openai-compatible",
                "options": {
                    "baseURL": f"http://127.0.0.1:{replay_port}/v1",
                    "apiKey": "bench",
                },
                "models": {
                    MODEL: {"name": MODEL},
                },
            }
        }
    }
    (work_dir / "opencode.json").write_text(
        json.dumps(config, indent=2) + "\n",
        encoding="utf-8",
    )


def build_command(
    binary: Path | None,
    replay_port: int,
    prompt: str,
    max_turns: int,
) -> list[str]:
    del replay_port, max_turns  # opencode uses the config file for the endpoint
    if binary is None:
        raise SystemExit("opencode not found; pass --opencode explicitly")
    return [
        str(binary),
        "run",
        "--model",
        f"bench/{MODEL}",
        "--auto",
        prompt,
    ]


def backend(configured_binary: str | None) -> AgentBackend:
    binary = resolve_binary(configured_binary, "opencode")
    return AgentBackend(
        name="opencode",
        binary=binary,
        command=lambda port, prompt, turns: build_command(
            binary,
            port,
            prompt,
            turns,
        ),
        prepare=prepare,
        run_cwd=lambda work_dir: work_dir,
        case_timeout_seconds=150.0,
    )
