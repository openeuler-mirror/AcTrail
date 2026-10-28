from __future__ import annotations

import base64
import json
import os
import re
import signal
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core.loopback_port import resolve_test_port

from .config import ContextCompactConfig
from .scenario import AgentRun, AgentSpec, ConversationScenario


class OpenCodeSessionDriver:
    """Drive real prompts and native compaction in one collected server process."""

    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")
    def __init__(
        self,
        config: ContextCompactConfig,
        runtime: ActrailRuntime,
        spec: AgentSpec,
        scenario: ConversationScenario,
    ):
        self._config = config
        self._runtime = runtime
        self._spec = spec
        self._scenario = scenario
        self._directory = config.work_dir / scenario.name
        self._log_path = self._directory / "opencode.log"
        self._opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self._process: subprocess.Popen[bytes] | None = None
        self._base_url = ""
        self._session_id: str | None = None

    def run(self) -> AgentRun:
        self._directory.mkdir(parents=True, exist_ok=True)
        workspace = self._directory / "workspace"
        workspace.mkdir(exist_ok=True)
        port = resolve_test_port("CONTEXT_COMPACT_E2E_OPENCODE_PORT")
        self._base_url = f"http://127.0.0.1:{port}"
        with self._log_path.open("wb") as log:
            try:
                self._process = subprocess.Popen(
                    self._runtime.control_command(
                        "launch", "--", self._spec.binary,
                        "serve", "--hostname", "127.0.0.1", "--port", str(port),
                    ),
                    cwd=workspace,
                    env=self._spec.environment,
                    stdin=subprocess.DEVNULL,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    start_new_session=True,
                )
                trace_id = self._wait_for_startup()
                session = self._request("POST", "/session", {
                    "title": f"AcTrail context compaction {self._scenario.name}",
                    "permission": [{
                        "permission": "bash", "pattern": "*", "action": "allow",
                    }],
                })
                self._session_id = session["id"]
                model = self._spec.model
                for index, prompt in enumerate(self._scenario.prompts):
                    if index == self._scenario.compact_turn_index:
                        if model is None:
                            raise RuntimeError("first response has no provider/model for compact")
                        reply = self._compact(model)
                    else:
                        body: dict[str, Any] = {
                            "parts": [{"type": "text", "text": prompt}],
                        }
                        if model is not None:
                            body["model"] = model
                        reply = self._request("POST", self._session_path("message"), body)
                        self._require_completed_reply(reply)
                        info = reply["info"]
                        model = {"providerID": info["providerID"], "modelID": info["modelID"]}
                    (self._directory / f"turn-{index}.json").write_text(
                        json.dumps(reply, ensure_ascii=False, indent=2), encoding="utf-8",
                    )
                return AgentRun(trace_id)
            finally:
                self._close()

    def _wait_for_startup(self) -> int:
        deadline = time.monotonic() + self._config.startup_timeout_seconds
        last_error: Exception | None = None
        while time.monotonic() < deadline:
            if self._process is not None and self._process.poll() is not None:
                raise RuntimeError(f"OpenCode serve exited during startup: {self._log_tail()}")
            trace_ids = set(self._TRACE_PATTERN.findall(self._log_path.read_text(errors="replace")))
            if len(trace_ids) > 1:
                raise RuntimeError(f"OpenCode launch reported multiple trace IDs: {trace_ids}")
            try:
                health = self._request(
                    "GET", "/global/health",
                    timeout_seconds=min(
                        self._config.command_timeout_seconds,
                        max(deadline - time.monotonic(), 0.001),
                    ),
                )
                if health.get("healthy") is True and len(trace_ids) == 1:
                    return int(next(iter(trace_ids)))
            except urllib.error.HTTPError:
                raise
            except (urllib.error.URLError, TimeoutError) as error:
                last_error = error
            time.sleep(self._config.drain_interval_seconds)
        raise TimeoutError(f"OpenCode serve startup timed out: {last_error}; {self._log_tail()}")

    def _compact(self, model: dict[str, str]) -> bool:
        result = self._request("POST", self._session_path("summarize"), {**model, "auto": False})
        if result is not True:
            raise RuntimeError("native OpenCode summarize did not succeed")
        return result

    def _request(
        self, method: str, path: str, body: Any = None,
        *, timeout_seconds: float | None = None,
    ) -> Any:
        headers = {"Content-Type": "application/json"}
        password = self._spec.environment.get("OPENCODE_SERVER_PASSWORD")
        if password:
            username = self._spec.environment.get("OPENCODE_SERVER_USERNAME", "opencode")
            token = base64.b64encode(f"{username}:{password}".encode()).decode()
            headers["Authorization"] = f"Basic {token}"
        request = urllib.request.Request(
            self._base_url + path,
            data=json.dumps(body).encode() if body is not None else None,
            headers=headers, method=method,
        )
        with self._opener.open(
            request,
            timeout=timeout_seconds or self._config.turn_timeout_seconds,
        ) as response:
            return json.load(response)

    def _session_path(self, suffix: str) -> str:
        if self._session_id is None:
            raise RuntimeError("OpenCode session has not been created")
        return f"/session/{self._session_id}/{suffix}"

    @staticmethod
    def _require_completed_reply(reply: dict[str, Any]) -> None:
        info = reply.get("info", {})
        if info.get("error") or not info.get("time", {}).get("completed") or not info.get("finish"):
            raise AssertionError(f"OpenCode reply did not complete successfully: {info}")

    def _log_tail(self) -> str:
        return self._log_path.read_text(errors="replace")[-4000:]

    def _close(self) -> None:
        process = self._process
        if process is None:
            return
        try:
            if self._session_id is not None and process.poll() is None:
                self._request(
                    "DELETE", f"/session/{self._session_id}",
                    timeout_seconds=self._config.shutdown_timeout_seconds,
                )
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=self._config.shutdown_timeout_seconds)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=self._config.shutdown_timeout_seconds)
