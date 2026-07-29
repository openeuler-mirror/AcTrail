from __future__ import annotations

import argparse
import os
import sys
from dataclasses import dataclass
from pathlib import Path

from protocol.config import ProtocolConfig
from scenario import ScenarioConfigurationError
from scenario.scenario_generator import ScenarioLoader
from scenario.scenario_generator.config import ScenarioGeneratorConfig
from schedule.config import ScheduleConfig
from server_core.connection.http.config import HTTPConfig
from server_core.connection.https.config import HTTPSConfig
from server_core.config import ServerCoreConfig
from tests.v2.common.utils import colorize


DEFAULT_HTTP_BIND_HOST = "127.0.0.1"
DEFAULT_HTTP_BIND_PORT = 0
DEFAULT_MODEL = "local-maas-test"
DEFAULT_MAX_REQUEST_BYTES = 1_048_576
DEFAULT_MAX_TEMPLATE_BYTES = 1_048_576
DEFAULT_MAX_GENERATOR_DEPTH = 64
DEFAULT_MAX_GENERATOR_NODES = 4096
DEFAULT_RANDOM_SEED = 0
DEFAULT_REQUEST_TIMEOUT_SECONDS = 30.0
DEFAULT_TTFT_MILLISECONDS = 0.0
DEFAULT_TPOT_MILLISECONDS = 0.0
DEFAULT_TLS_OPENSSL_BINARY = "openssl"
DEFAULT_TLS_CERTIFICATE_VALIDITY_DAYS = 1


class ConfigurationError(RuntimeError):
    """Raised when the process cannot start with the supplied configuration."""


@dataclass(frozen=True, slots=True)
class LocalMaaSConfig:
    generator: ScenarioGeneratorConfig
    protocol: ProtocolConfig
    schedule: ScheduleConfig
    server: ServerCoreConfig


class LocalMaaSConfigParser:
    def parse(self) -> LocalMaaSConfig:
        parser = argparse.ArgumentParser(
            description="Run a local MaaS server backed by a lazy scenario generator.",
            formatter_class=argparse.ArgumentDefaultsHelpFormatter,
        )
        self._add_scenario_options(parser)
        self._add_protocol_options(parser)
        self._add_schedule_options(parser)
        self._add_server_options(parser)
        args = parser.parse_args()
        self._require_scenario(parser, args)
        api_key = self._load_api_key(args.api_key_env)
        try:
            generator = ScenarioGeneratorConfig(
                templates_dir=args.templates_dir,
                template_name=args.scenario,
                max_template_bytes=args.max_template_bytes,
                max_depth=args.max_generator_depth,
                max_nodes=args.max_generator_nodes,
                random_seed=args.random_seed,
            )
            return LocalMaaSConfig(
                generator=generator,
                protocol=ProtocolConfig(default_model=args.default_model),
                schedule=ScheduleConfig(
                    ttft_seconds=args.ttft_milliseconds / 1000.0,
                    tpot_seconds=args.tpot_milliseconds / 1000.0,
                ),
                server=ServerCoreConfig(
                    http=HTTPConfig(
                        bind_host=args.http_bind_host,
                        bind_port=args.http_bind_port,
                    ),
                    https=self._https_config(args),
                    max_request_bytes=args.max_request_bytes,
                    request_timeout_seconds=args.request_timeout_seconds,
                    api_key=api_key,
                    log_requests=args.log_requests,
                ),
            )
        except ValueError as error:
            raise ConfigurationError(str(error)) from error

    @staticmethod
    def _add_scenario_options(parser: argparse.ArgumentParser) -> None:
        parser.add_argument(
            "--templates-dir",
            type=Path,
            default=(
                Path(__file__).resolve().parent
                / "scenario"
                / "scenario_generator"
                / "templates"
            ),
            help="directory containing scenario generator JSON templates",
        )
        parser.add_argument(
            "--scenario",
            default=argparse.SUPPRESS,
            help="template path relative to --templates-dir; .json is optional",
        )
        parser.add_argument(
            "--max-template-bytes",
            type=int,
            default=DEFAULT_MAX_TEMPLATE_BYTES,
        )
        parser.add_argument(
            "--max-generator-depth",
            type=int,
            default=DEFAULT_MAX_GENERATOR_DEPTH,
        )
        parser.add_argument(
            "--max-generator-nodes",
            type=int,
            default=DEFAULT_MAX_GENERATOR_NODES,
        )
        parser.add_argument(
            "--random-seed",
            type=int,
            default=DEFAULT_RANDOM_SEED,
            help="default deterministic seed for random generators",
        )
    @staticmethod
    def _add_protocol_options(parser: argparse.ArgumentParser) -> None:
        parser.add_argument("--default-model", default=DEFAULT_MODEL)

    @staticmethod
    def _add_schedule_options(parser: argparse.ArgumentParser) -> None:
        parser.add_argument(
            "--ttft-milliseconds",
            type=float,
            default=DEFAULT_TTFT_MILLISECONDS,
            help="delay before the first SSE frame",
        )
        parser.add_argument(
            "--tpot-milliseconds",
            type=float,
            default=DEFAULT_TPOT_MILLISECONDS,
            help="delay before each SSE frame after the first frame",
        )
    @staticmethod
    def _add_server_options(parser: argparse.ArgumentParser) -> None:
        parser.add_argument("--http-bind-host", default=DEFAULT_HTTP_BIND_HOST)
        parser.add_argument(
            "--http-bind-port",
            type=int,
            default=DEFAULT_HTTP_BIND_PORT,
        )
        parser.add_argument(
            "--https-bind-host",
            default=argparse.SUPPRESS,
            help="HTTPS bind host; defaults to --http-bind-host",
        )
        parser.add_argument(
            "--https-bind-port",
            type=int,
            default=argparse.SUPPRESS,
            help="HTTPS bind port; defaults to ephemeral port 0",
        )
        parser.add_argument(
            "--tls-work-dir",
            type=Path,
            default=argparse.SUPPRESS,
            help="parent directory for the temporary certificate directory",
        )
        parser.add_argument(
            "--tls-openssl-binary",
            default=argparse.SUPPRESS,
            help="OpenSSL executable; defaults to openssl",
        )
        parser.add_argument(
            "--tls-certificate-validity-days",
            type=int,
            default=argparse.SUPPRESS,
            help="temporary certificate validity; defaults to 1 day",
        )
        parser.add_argument(
            "--disable-https",
            action="store_true",
            help="start HTTP only instead of attempting default HTTPS",
        )
        parser.add_argument(
            "--max-request-bytes",
            type=int,
            default=DEFAULT_MAX_REQUEST_BYTES,
        )
        parser.add_argument(
            "--request-timeout-seconds",
            type=float,
            default=DEFAULT_REQUEST_TIMEOUT_SECONDS,
        )
        parser.add_argument(
            "--api-key-env",
            help="environment variable containing the required local API key",
        )
        parser.add_argument(
            "--log-requests",
            action="store_true",
            help="write metadata-only request completion records to stderr",
        )

    @staticmethod
    def _https_config(args: argparse.Namespace) -> HTTPSConfig | None:
        option_names = (
            "https_bind_host",
            "https_bind_port",
            "tls_work_dir",
            "tls_openssl_binary",
            "tls_certificate_validity_days",
        )
        explicitly_configured = any(
            hasattr(args, name) for name in option_names
        )
        if args.disable_https:
            if explicitly_configured:
                raise ValueError(
                    "--disable-https cannot be combined with HTTPS options"
                )
            return None
        return HTTPSConfig(
            bind_host=getattr(
                args,
                "https_bind_host",
                args.http_bind_host,
            ),
            bind_port=getattr(args, "https_bind_port", 0),
            best_effort=not explicitly_configured,
            certificate_work_dir=getattr(args, "tls_work_dir", None),
            openssl_binary=getattr(
                args,
                "tls_openssl_binary",
                DEFAULT_TLS_OPENSSL_BINARY,
            ),
            certificate_validity_days=getattr(
                args,
                "tls_certificate_validity_days",
                DEFAULT_TLS_CERTIFICATE_VALIDITY_DAYS,
            ),
        )

    @staticmethod
    def _require_scenario(
        parser: argparse.ArgumentParser,
        args: argparse.Namespace,
    ) -> None:
        if hasattr(args, "scenario"):
            return
        templates_dir = args.templates_dir.resolve()
        if not templates_dir.is_dir():
            parser.error(
                "--scenario is required; scenario template directory does "
                f"not exist: {templates_dir}"
            )
        try:
            scenarios = ScenarioLoader.available_scenarios(
                templates_dir,
                args.max_template_bytes,
            )
        except ScenarioConfigurationError as error:
            parser.error(str(error))
        if not scenarios:
            parser.error(
                "--scenario is required; no JSON scenarios found in "
                f"{templates_dir}"
            )
        heading = colorize("available scenarios", "cyan", sys.stderr)
        choices = "\n".join(
            f"  {colorize(scenario.scenario_id, 'green', sys.stderr)}\n"
            f"    {scenario.description}"
            for scenario in scenarios
        )
        parser.error(
            f"--scenario is required; {heading}:\n" + choices
        )

    @staticmethod
    def _load_api_key(environment_name: str | None) -> str | None:
        if environment_name is None:
            return None
        api_key = os.environ.get(environment_name)
        if not api_key:
            raise ConfigurationError(
                f"environment variable {environment_name!r} is missing or empty"
            )
        if not api_key.isascii():
            raise ConfigurationError(
                f"environment variable {environment_name!r} must contain an "
                "ASCII API key"
            )
        if any(character in api_key for character in "\r\n"):
            raise ConfigurationError(
                f"environment variable {environment_name!r} cannot contain "
                "line breaks"
            )
        return api_key
