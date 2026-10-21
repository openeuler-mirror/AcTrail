from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class LocalMaaSTestDefaults:
    model: str = "deepseek-v4-flash"
    request_model: str = "deepseek-chat"
    tools_xiaoo: str = "file_read,glob,grep"
    tools_opencode: str = "read,glob,grep"
    agent: str = "xiaoo"
    max_turns: int = 35
    agent_poll_seconds: float = 15.0
    agent_timeout_seconds: float = 3600.0
    agent_stop_timeout_seconds: float = 10.0
    server_ready_timeout_seconds: float = 20.0
    server_ready_poll_seconds: float = 0.1
    server_stop_timeout_seconds: float = 5.0
    http_timeout_seconds: float = 10.0
    upstream_timeout_seconds: float = 60.0
    upstream_ready_timeout_seconds: float = 60.0
    upstream_max_tokens: int = 2048
    upstream_temperature: float = 0.0
    upstream_stream: bool = False
    loopback_host: str = "127.0.0.1"
    ephemeral_port: int = 0
    healthcheck_timeout_seconds: float = 1.0


DEFAULTS = LocalMaaSTestDefaults()
