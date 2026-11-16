"""Default runtime parameters for this regression scenario."""

from dataclasses import dataclass, field
from pathlib import Path
import tempfile


@dataclass(frozen=True)
class HttpLlmSourcesConstants:
    assertion_protocol: str = 'http1'
    require_llm_require_connection_reuse: bool = True
    bin_dir: str = 'target/release'
    runs: int = 2
    max_turns: int = 4
    min_http_errors: int = 1
    timeout: int = 180
    content_max_bytes: int = 8388608
    exit_timeout: int = 5
    marker_random_bytes: int = 8
    enter_poll_seconds: float = 0.1
    protocol: str = 'http2'
    protocol_timeout_seconds: int = 240
    relay_upstream_url: str = 'https://chatgpt.com/backend-api/codex'
    relay_timeout_seconds: int = 30
    relay_header_limit_bytes: int = 65536
    relay_max_connections: int = 16
    relay_read_size_bytes: int = 65536
    relay_cleanup_timeout_seconds: int = 5
    retain_transport: bool = False
    io_poll_seconds: float = 0.25
    loopback_host: str = '127.0.0.1'
    output_parent: Path = field(default_factory=lambda: Path(tempfile.gettempdir()))


CASE_CONSTANTS = HttpLlmSourcesConstants()
