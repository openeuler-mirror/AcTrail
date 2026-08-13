from __future__ import annotations

import json
import os
import urllib.request

from .config import TransportConfig
from .upstream import UpstreamConfig


class UpstreamResolutionError(RuntimeError):
    """Raised when transport mode cannot resolve an upstream at startup."""


class TransportUpstreamResolver:
    """Resolve the transport upstream from explicit config, environment
    variables, or a DeepSeek API key probe."""

    def __init__(self, request_timeout_seconds: float):
        self._timeout = request_timeout_seconds

    def resolve(
        self,
        explicit: TransportConfig | None,
        context: str = "transport mode",
    ) -> TransportConfig:
        if explicit is not None:
            return explicit
        configured = self._from_environment()
        if configured is not None:
            return configured
        deepseek = self._from_deepseek_environment()
        if deepseek is not None:
            return deepseek
        raise UpstreamResolutionError(
            f"{context} requires an upstream: --transport-config, "
            "LOCAL_MAAS_UPSTREAM_URL/LOCAL_MAAS_UPSTREAM_API_KEY/"
            "LOCAL_MAAS_PROTOCOL, or DEEPSEEK_API_KEY"
        )

    def _from_environment(self) -> TransportConfig | None:
        url = os.environ.get("LOCAL_MAAS_UPSTREAM_URL")
        api_key = os.environ.get("LOCAL_MAAS_UPSTREAM_API_KEY")
        protocol = os.environ.get("LOCAL_MAAS_PROTOCOL")
        model = os.environ.get("LOCAL_MAAS_UPSTREAM_MODEL")
        if (
            url is None
            and api_key is None
            and protocol is None
            and model is None
        ):
            return None
        if not url or not api_key:
            raise UpstreamResolutionError(
                "LOCAL_MAAS_UPSTREAM_URL and LOCAL_MAAS_UPSTREAM_API_KEY "
                "must both be set when configuring a transport upstream"
            )
        if protocol not in (None, "", "openai"):
            raise UpstreamResolutionError(
                f"unsupported LOCAL_MAAS_PROTOCOL {protocol!r}; "
                "only openai is supported"
            )
        try:
            upstream = UpstreamConfig(
                base_url=url,
                api_key=api_key,
                model=model or None,
            )
        except ValueError as error:
            raise UpstreamResolutionError(str(error)) from error
        return TransportConfig(upstream=upstream)

    def _from_deepseek_environment(self) -> TransportConfig | None:
        api_key = os.environ.get("DEEPSEEK_API_KEY")
        if not api_key:
            return None
        base_url = "https://api.deepseek.com"
        models = self._fetch_models(base_url, api_key)
        model = models[0] if models else None
        try:
            upstream = UpstreamConfig(
                base_url=base_url,
                api_key=api_key,
                model=model,
            )
        except ValueError as error:
            raise UpstreamResolutionError(str(error)) from error
        return TransportConfig(upstream=upstream)

    def _fetch_models(self, base_url: str, api_key: str) -> list[str]:
        request = urllib.request.Request(
            base_url + "/models",
            headers={
                "Authorization": f"Bearer {api_key}",
                "Accept": "application/json",
            },
        )
        try:
            with urllib.request.urlopen(
                request, timeout=self._timeout
            ) as response:
                document = json.loads(response.read().decode("utf-8"))
        except (OSError, ValueError) as error:
            raise UpstreamResolutionError(
                f"upstream models probe failed for {base_url}: {error}"
            ) from error
        if not isinstance(document, dict):
            raise UpstreamResolutionError(
                f"upstream models probe returned a non-object: {base_url}"
            )
        data = document.get("data")
        if not isinstance(data, list):
            return []
        return [
            item["id"]
            for item in data
            if isinstance(item, dict)
            and isinstance(item.get("id"), str)
        ]
