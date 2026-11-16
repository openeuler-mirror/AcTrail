from __future__ import annotations

import http.client
import json

from .base import UpstreamClient, UpstreamConfig


class OpenAIUpstreamClient(UpstreamClient):
    def _request(
        self,
        connection: http.client.HTTPConnection,
        config: UpstreamConfig,
        request_path: str,
        document: dict[str, object],
        *,
        stream: bool,
    ) -> None:
        headers = {
            "Content-Type": "application/json",
            "Accept": (
                "text/event-stream" if stream else "application/json"
            ),
            "Authorization": f"Bearer {config.api_key}",
        }
        payload = json.dumps(
            document,
            ensure_ascii=False,
            separators=(",", ":"),
        ).encode("utf-8")
        connection.request(
            "POST",
            request_path,
            body=payload,
            headers=headers,
        )
