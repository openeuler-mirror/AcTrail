from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path


@dataclass(frozen=True)
class LocalMaaSDefaults:
    http_bind_host: str = "127.0.0.1"
    http_bind_port: int = 0
    https_bind_port: int = 0
    model: str = "local-maas-test"
    max_request_bytes: int = 1_048_576
    max_template_bytes: int = 1_048_576
    max_generator_depth: int = 64
    max_generator_nodes: int = 4096
    random_seed: int = 0
    request_timeout_seconds: float = 30.0
    ttft_milliseconds: float = 0.0
    tpot_milliseconds: float = 0.0
    tls_openssl_binary: str = "openssl"
    tls_certificate_validity_days: int = 1
    scenario_generator_dir: Path = field(default_factory=lambda: Path(__file__).resolve().parent / "scenario" / "scenario_generator")
    recordings_dir: Path = field(default_factory=lambda: Path(__file__).resolve().parent / "recordings")
    loop_exhausted_messages: bool = True
    lazy_load_size: int = 0
    statistics_round_cap: int = 100_000
    allow_reuse_address: bool = True
    daemon_threads: bool = True
    silent_start: bool = False
    default_upstream_url: str = "https://api.deepseek.com"
    record_scenario_name: str = "record"


DEFAULTS = LocalMaaSDefaults()
