#!/usr/bin/env python3
"""End-to-end verification of local_maas_server recording mode.

The test starts:
  1. a probe upstream that speaks the OpenAI-compatible API and logs every
     received request body;
  2. the local MaaS server in recording mode;
  3. after finalize, the local MaaS server in normal playback mode with the
     recorded scenario;
  4. when upstream credentials exist in the environment, one transport-mode
     round against a real MaaS (DeepSeek or LOCAL_MAAS_UPSTREAM_*).

It then asserts the full loop: session creation, API key enforcement, tool
whitelist pruning, direct/SSE forwarding, canonical tool normalization in the
cache, finalize, replay of the recorded scenario, and real-upstream probing.
"""


from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from test.e2e_support import (  # noqa: E402
    MaaSServerProcess,
    ProbeUpstream,
    REPO,
    _free_port,
)

class RecordE2E:
    def __init__(self, workdir: Path):
        self.workdir = workdir
        self.recordings_dir = workdir / "recordings"
        self.upstream = ProbeUpstream()
        self.record_server: MaaSServerProcess | None = None
        self.transport_server: MaaSServerProcess | None = None
        self.playback_server: MaaSServerProcess | None = None
        self.session: dict[str, Any] | None = None
        self.scenario_id: str | None = None

    def run(self) -> None:
        upstream_origin = self.upstream.start()
        record_port = _free_port()
        playback_port = _free_port()
        self.record_server = MaaSServerProcess(
            [
                "record",
                "--disable-https",
                "--http-bind-port",
                str(record_port),
                "--recordings-dir",
                str(self.recordings_dir),
                "--templates-dir",
                str(self.workdir / "templates"),
            ],
            workdir=REPO,
        )
        self.record_server.wait_ready()
        self._assert_session_lifecycle(upstream_origin)
        self._assert_session_without_upstream()
        self._assert_forwarding()
        self._assert_transport_mode(upstream_origin)
        self._assert_real_upstream()
        self._assert_cache_and_finalize()
        self.playback_server = MaaSServerProcess(
            [
                "replay",
                "--disable-https",
                "--http-bind-port",
                str(playback_port),
                "--templates-dir",
                str(self.workdir / "templates"),
                "--scenario",
                str(self.scenario_id),
            ],
            workdir=REPO,
        )
        self.playback_server.wait_ready()
        self._assert_replay()

    def _assert_session_lifecycle(self, upstream_origin: str) -> None:
        assert self.record_server is not None
        status, body = self.record_server.request(
            "POST",
            "/record/sessions",
            document={
                "tools": ["run_command"],
                "upstream": {
                    "base_url": upstream_origin,
                    "api_key": "probe-key",
                },
            },
        )
        assert status == 201, (status, body)
        self.session = json.loads(body)
        assert self.session["api_key"]
        assert self.session["state"] == "open"

        status, body = self.record_server.request(
            "POST",
            "/v1/chat/completions",
            document={
                "model": "local-maas-test",
                "messages": [],
                "stream": False,
                "tools": [
                    {
                        "type": "function",
                        "function": {"name": "run_command", "parameters": {}},
                    }
                ],
            },
        )
        assert status == 401, (status, body)

        status, body = self.record_server.request(
            "GET",
            "/record/sessions",
        )
        assert status == 200, (status, body)
        sessions = json.loads(body)["sessions"]
        assert len(sessions) == 1

    def _assert_session_without_upstream(self) -> None:
        has_creds = bool(os.environ.get("DEEPSEEK_API_KEY")) or bool(
            os.environ.get("LOCAL_MAAS_UPSTREAM_URL")
            and os.environ.get("LOCAL_MAAS_UPSTREAM_API_KEY")
        )
        if not has_creds:
            print(
                "SESSION_WITHOUT_UPSTREAM_SKIPPED: "
                "no upstream credentials in the environment"
            )
            return
        assert self.record_server is not None
        status, body = self.record_server.request(
            "POST",
            "/record/sessions",
            document={"tools": ["read_file"]},
        )
        assert status == 201, (status, body)
        session = json.loads(body)
        expected_base = os.environ.get(
            "LOCAL_MAAS_UPSTREAM_URL"
        ) or "https://api.deepseek.com"
        assert session["upstream"]["base_url"] == expected_base, session
        if not os.environ.get("LOCAL_MAAS_UPSTREAM_URL"):
            assert session["upstream"].get("model"), session
        print("SESSION_WITHOUT_UPSTREAM_OK")

    def _assert_forwarding(self) -> None:
        assert self.record_server is not None
        assert self.session is not None
        api_key = self.session["api_key"]
        tools = [
            {
                "type": "function",
                "function": {
                    "name": name,
                    "parameters": {"type": "object", "properties": {}},
                },
            }
            for name in ("run_command", "read_file")
        ]
        direct_document = {
            "model": "local-maas-test",
            "messages": [],
            "stream": False,
            "tools": tools,
        }
        status, body = self.record_server.request(
            "POST",
            "/v1/chat/completions",
            document=direct_document,
            api_key=api_key,
        )
        assert status == 200, (status, body)
        parsed = json.loads(body)
        message = parsed["choices"][0]["message"]
        assert "run_command" in json.dumps(message["tool_calls"])
        assert message["content"] == "I will run the scripted local check."

        stream_document = {
            "model": "local-maas-test",
            "messages": [],
            "stream": True,
            "tools": tools,
        }
        status, body = self.record_server.request(
            "POST",
            "/v1/chat/completions",
            document=stream_document,
            api_key=api_key,
        )
        assert status == 200, (status, body)
        assert b"[DONE]" in body
        assert b"reasoning_content" in body
        assert b"run_command" in body

        assert self.upstream.tool_names_by_request() == [
            ["run_command"],
            ["run_command"],
        ], self.upstream.tool_names_by_request()

    def _assert_transport_mode(self, upstream_origin: str) -> None:
        transport_config = self.workdir / "transport.json"
        transport_config.write_text(
            json.dumps(
                {
                    "base_url": upstream_origin,
                    "api_key": "probe-key",
                },
                ensure_ascii=False,
            ),
            encoding="utf-8",
        )
        self.transport_server = MaaSServerProcess(
            [
                "transport",
                "--disable-https",
                "--http-bind-port",
                str(_free_port()),
                "--transport-config",
                str(transport_config),
            ],
            workdir=REPO,
        )
        self.transport_server.wait_ready()
        status, body = self.transport_server.request(
            "GET",
            "/record/sessions",
        )
        assert status == 404, (status, body)
        status, body = self.transport_server.request(
            "POST",
            "/v1/chat/completions",
            document={
                "model": "local-maas-test",
                "messages": [],
                "stream": False,
                "tools": [
                    {
                        "type": "function",
                        "function": {
                            "name": name,
                            "parameters": {
                                "type": "object",
                                "properties": {},
                            },
                        },
                    }
                    for name in ("run_command", "read_file")
                ],
            },
        )
        assert status == 200, (status, body)
        assert b"I will run the scripted local check." in body
        last_request = self.upstream.requests[-1]
        tool_names = [
            tool["function"]["name"]
            for tool in last_request["tools"]
            if isinstance(tool, dict)
        ]
        assert tool_names == ["run_command", "read_file"], tool_names
        self.transport_server.stop()
        self.transport_server = None

    def _assert_real_upstream(self) -> None:
        has_deepseek = bool(os.environ.get("DEEPSEEK_API_KEY"))
        has_custom = bool(
            os.environ.get("LOCAL_MAAS_UPSTREAM_URL")
            and os.environ.get("LOCAL_MAAS_UPSTREAM_API_KEY")
        )
        if not has_deepseek and not has_custom:
            print(
                "REAL_MAAS_SKIPPED: no upstream credentials "
                "in the environment"
            )
            return
        expected_base = os.environ.get(
            "LOCAL_MAAS_UPSTREAM_URL"
        ) or "https://api.deepseek.com"
        request_model = os.environ.get("LOCAL_MAAS_UPSTREAM_MODEL")
        if not request_model:
            request_model = "deepseek-chat"
        server = MaaSServerProcess(
            [
                "transport",
                "--disable-https",
                "--http-bind-port",
                str(_free_port()),
            ],
            workdir=REPO,
        )
        try:
            server.wait_ready(timeout_seconds=60.0)
            status, body = server.request("GET", "/healthz")
            assert status == 200, (status, body)
            health = json.loads(body)
            assert health["mode"] == "transport", health
            assert health["upstream"] == expected_base, health
            if has_deepseek and not has_custom:
                assert health.get("model"), health
            status, body = server.request(
                "POST",
                "/v1/chat/completions",
                document={
                    "model": request_model,
                    "messages": [
                        {
                            "role": "user",
                            "content": (
                                "Reply with exactly REAL_MAAS_E2E_OK"
                            ),
                        }
                    ],
                    "stream": False,
                    "max_tokens": 2048,
                    "temperature": 0,
                },
                timeout=60.0,
            )
            assert status == 200, (status, body[:500])
            parsed = json.loads(body)
            choices = parsed.get("choices", [])
            assert choices, parsed
            content = choices[0].get("message", {}).get("content")
            assert isinstance(content, str) and content.strip(), parsed
            print("REAL_MAAS_ROUND_OK")
        finally:
            server.stop()

    def _assert_cache_and_finalize(self) -> None:
        assert self.record_server is not None
        assert self.session is not None
        cache_file = Path(self.session["cache_file"])
        assert cache_file.is_file()
        lines = cache_file.read_text(encoding="utf-8").splitlines()
        assert len(lines) == 2, lines
        first = json.loads(lines[0])
        assert first["protocol"] == "openai"
        assert first["stop"] == "tool_call"
        tool_blocks = [
            block
            for block in first["blocks"]
            if block["type"] == "tool_call"
        ]
        assert len(tool_blocks) == 1
        assert tool_blocks[0]["name"] == "bash", tool_blocks
        assert tool_blocks[0]["arguments"] == {
            "command": "printf ok"
        }, tool_blocks

        status, body = self.record_server.request(
            "POST",
            f"/record/sessions/{self.session['session_id']}/finalize",
            document={"scenario_id": "recorded-e2e"},
        )
        assert status == 200, (status, body)
        finalized = json.loads(body)
        assert finalized["responses"] == 2
        scenario_file = Path(finalized["scenario_file"])
        assert scenario_file.is_file()
        assert scenario_file.parent.name == "recorded", scenario_file
        assert scenario_file.name.endswith(".meta.json"), scenario_file
        assert finalized["scenario_id"].startswith("recorded/"), finalized
        assert not cache_file.exists(), cache_file
        scenario_document = json.loads(
            scenario_file.read_text(encoding="utf-8")
        )
        assert scenario_document["type"] == "recorded", scenario_document
        assert scenario_document["rounds"] == finalized["responses"]
        tool_file = (
            scenario_file.parent
            / Path(scenario_document["tool_source"]).name
        )
        message_file = (
            scenario_file.parent
            / Path(scenario_document["message_source"]).name
        )
        assert tool_file.is_file(), tool_file
        assert message_file.is_file(), message_file

        def count_lines(path: Path) -> int:
            return len(
                [
                    line
                    for line in path.read_text(
                        encoding="utf-8"
                    ).splitlines()
                    if line.strip()
                ]
            )

        assert count_lines(tool_file) == scenario_document["tool_rounds"]
        assert (
            count_lines(message_file)
            == scenario_document["message_rounds"]
        )
        self.scenario_id = finalized["scenario_id"]

    def _assert_replay(self) -> None:
        assert self.playback_server is not None
        tools = [
            {
                "type": "function",
                "function": {
                    "name": "run_command",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "cmd": {
                                "type": "string",
                                "description": "command to run",
                            }
                        },
                        "required": ["cmd"],
                    },
                },
            }
        ]
        status, body = self.playback_server.request(
            "POST",
            "/v1/chat/completions",
            document={
                "model": "local-maas-test",
                "messages": [],
                "stream": False,
                "tools": tools,
            },
        )
        assert status == 200, (status, body)
        parsed = json.loads(body)
        message = parsed["choices"][0]["message"]
        assert message["content"] == "I will run the scripted local check."
        assert message["reasoning_content"]
        tool_call = message["tool_calls"][0]["function"]
        assert tool_call["name"] == "run_command", tool_call
        assert json.loads(tool_call["arguments"]) == {
            "cmd": "printf ok"
        }, tool_call

        status, body = self.playback_server.request(
            "POST",
            "/v1/chat/completions",
            document={
                "model": "local-maas-test",
                "messages": [],
                "stream": True,
                "tools": tools,
            },
        )
        assert status == 200, (status, body)
        assert b"[DONE]" in body
        assert b"I will run the scripted local check." in body

    def cleanup(self) -> None:
        for server in (
            self.transport_server,
            self.playback_server,
            self.record_server,
        ):
            if server is not None:
                server.stop()
        self.upstream.stop()


def main() -> int:
    workdir = Path(tempfile.mkdtemp(prefix="local-maas-record-e2e-"))
    runner = RecordE2E(workdir)
    try:
        runner.run()
    finally:
        runner.cleanup()
        shutil.rmtree(workdir, ignore_errors=True)
    print("LOCAL_MAAS_RECORD_E2E_OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
