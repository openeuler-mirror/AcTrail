from __future__ import annotations

import json
import shutil
import subprocess
from typing import Any
from urllib.parse import quote, urlencode


class PluginWebApi:
    """Small JSON client for the actrailweb plugin control endpoints."""

    def __init__(self, base_url: str, timeout_seconds: int):
        self._base_url = base_url.rstrip("/")
        self._timeout_seconds = timeout_seconds
        self._curl = shutil.which("curl")
        if self._curl is None:
            raise RuntimeError("curl executable not found in PATH")

    def catalog(self) -> dict[str, Any]:
        return self._request("GET", "/api/plugins/catalog")

    def load(
        self,
        package: str,
        instance_id: str,
        grants: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        body: dict[str, Any] = {"instance_id": instance_id}
        if grants is not None:
            body["grants"] = grants
        return self._request(
            "POST",
            "/api/plugins/catalog/load",
            query={"package": package},
            body=body,
        )

    def runtime(self) -> dict[str, Any]:
        return self._request("GET", "/api/plugins/runtime")

    def config(self, instance_id: str) -> dict[str, Any]:
        return self._request(
            "GET",
            "/api/plugins/runtime/config",
            query={"instance_id": instance_id},
        )

    def validate_config(
        self,
        instance_id: str,
        config: dict[str, Any],
    ) -> dict[str, Any]:
        return self._request(
            "POST",
            "/api/plugins/runtime/config/validate",
            query={"instance_id": instance_id},
            body={"config": config},
        )

    def update_config(
        self,
        instance_id: str,
        config: dict[str, Any],
    ) -> dict[str, Any]:
        return self._request(
            "POST",
            "/api/plugins/runtime/config",
            query={"instance_id": instance_id},
            body={"config": config},
        )

    def unload(self, instance_id: str) -> dict[str, Any]:
        return self._request(
            "POST",
            "/api/plugins/runtime/unload",
            query={"instance_id": instance_id},
        )

    def command(self, instance_id: str, argv: list[str]) -> dict[str, Any]:
        return self._request(
            "POST",
            "/api/plugins/runtime/command",
            query={"instance_id": instance_id},
            body={"argv": argv},
        )

    def alerts(self, trace_id: int) -> dict[str, Any]:
        return self._request("GET", f"/api/traces/{trace_id}/alerts")

    def llm_request_content(
        self,
        trace_id: int,
        action_id: str,
        *,
        max_bytes: int,
    ) -> dict[str, Any]:
        return self._request(
            "GET",
            f"/api/traces/{trace_id}/actions/"
            f"{quote(action_id, safe='')}/content/llm-request",
            query={"max_bytes": str(max_bytes)},
        )

    def llm_request_lineage(
        self,
        trace_id: int,
        action_id: str,
    ) -> dict[str, Any]:
        return self._request(
            "GET",
            f"/api/traces/{trace_id}/actions/"
            f"{quote(action_id, safe='')}/lineage/llm-request",
        )

    def llm_request_trajectory(
        self,
        trace_id: int,
        trajectory_id: str,
    ) -> dict[str, Any]:
        return self._request(
            "GET",
            f"/api/traces/{trace_id}/llm-trajectories/"
            f"{quote(trajectory_id, safe='')}",
        )

    def _request(
        self,
        method: str,
        path: str,
        *,
        query: dict[str, str] | None = None,
        body: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        url = self._base_url + path
        if query:
            url += "?" + urlencode(query)
        command = [
            self._curl,
            "--fail",
            "--silent",
            "--show-error",
            "--noproxy",
            "*",
            "--request",
            method,
            "--max-time",
            str(self._timeout_seconds),
        ]
        if body is not None:
            command.extend(
                [
                    "--header",
                    "Content-Type: application/json",
                    "--data-binary",
                    json.dumps(body),
                ]
            )
        command.append(url)
        try:
            completed = subprocess.run(
                command,
                capture_output=True,
                text=True,
                timeout=self._timeout_seconds + 1,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise RuntimeError(f"{method} {url} failed: {error}") from error
        if completed.returncode != 0:
            raise RuntimeError(
                f"{method} {url} failed with curl {completed.returncode}: "
                f"{completed.stderr[-2000:]}"
            )
        raw = completed.stdout
        try:
            value = json.loads(raw)
        except json.JSONDecodeError as error:
            raise RuntimeError(
                f"{method} {url} returned invalid JSON: {raw[:1000]}"
            ) from error
        if not isinstance(value, dict):
            raise RuntimeError(f"{method} {url} returned non-object JSON")
        return value
