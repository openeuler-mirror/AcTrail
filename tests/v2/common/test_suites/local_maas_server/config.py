from __future__ import annotations

import argparse
import os
import sys
from dataclasses import dataclass
from pathlib import Path

from protocol.config import ProtocolConfig
from record.config import RecordConfig
from scenario import ScenarioConfigurationError
from scenario.model import ScenarioMeta
from scenario.scenario_generator import ScenarioRegistry
from scenario.scenario_generator.config import ScenarioGeneratorConfig
from scenario.scenario_generator.registry import DEFAULT_MAX_TEMPLATE_BYTES
from scenario.tool_alias import ToolAliasConfig
from schedule.config import ScheduleConfig
from server_core.connection.http.config import HTTPConfig
from server_core.connection.https.config import HTTPSConfig
from server_core.config import ServerCoreConfig
from tests.v2.common.utils import colorize
from transport import TransportConfig
from transport.upstream import UpstreamConfig
from utils.json import StrictJsonDecoder, StrictJsonError


DEFAULT_HTTP_BIND_HOST = "127.0.0.1"
DEFAULT_HTTP_BIND_PORT = 0
DEFAULT_MODEL = "local-maas-test"
DEFAULT_MAX_REQUEST_BYTES = 1_048_576
DEFAULT_MAX_GENERATOR_DEPTH = 64
DEFAULT_MAX_GENERATOR_NODES = 4096
DEFAULT_RANDOM_SEED = 0
DEFAULT_REQUEST_TIMEOUT_SECONDS = 30.0
DEFAULT_TTFT_MILLISECONDS = 0.0
DEFAULT_TPOT_MILLISECONDS = 0.0
DEFAULT_TLS_OPENSSL_BINARY = "openssl"
DEFAULT_TLS_CERTIFICATE_VALIDITY_DAYS = 1
DEFAULT_SCENARIO_GENERATOR_DIR = (
    Path(__file__).resolve().parent
    / "scenario"
    / "scenario_generator"
)
DEFAULT_RECORDINGS_DIR = Path(__file__).resolve().parent / "recordings"


class ConfigurationError(RuntimeError):
    """Raised when the process cannot start with the supplied configuration."""


@dataclass(frozen=True, slots=True)
class LocalMaaSConfig:
    mode: str
    generator: ScenarioGeneratorConfig
    tool_alias: ToolAliasConfig
    protocol: ProtocolConfig
    schedule: ScheduleConfig
    server: ServerCoreConfig
    record: RecordConfig
    transport: TransportConfig | None

    @classmethod
    def parse_from(cls, args: argparse.Namespace) -> LocalMaaSConfig:
        mode = args.command
        transport_config = cls._transport_config(mode, args)
        api_key = (
            cls._load_api_key(args.api_key_env)
            if hasattr(args, "api_key_env")
            else None
        )
        try:
            return cls(
                mode=mode,
                generator=ScenarioGeneratorConfig(
                    templates_dir=getattr(
                        args,
                        "templates_dir",
                        ScenarioRegistry.resolve_templates_dir(),
                    ),
                    action_pools_dir=getattr(
                        args,
                        "action_pools_dir",
                        DEFAULT_SCENARIO_GENERATOR_DIR / "action_pools",
                    ),
                    template_name=getattr(args, "scenario", "record"),
                    max_template_bytes=getattr(
                        args,
                        "max_template_bytes",
                        DEFAULT_MAX_TEMPLATE_BYTES,
                    ),
                    max_depth=getattr(
                        args,
                        "max_generator_depth",
                        DEFAULT_MAX_GENERATOR_DEPTH,
                    ),
                    max_nodes=getattr(
                        args,
                        "max_generator_nodes",
                        DEFAULT_MAX_GENERATOR_NODES,
                    ),
                    random_seed=getattr(
                        args,
                        "random_seed",
                        DEFAULT_RANDOM_SEED,
                    ),
                    loop_exhausted_messages=getattr(
                        args,
                        "loop_exhausted_messages",
                        True,
                    ),
                    lazy_load_size=getattr(
                        args,
                        "lazy_load_size",
                        0,
                    ),
                ),
                tool_alias=ToolAliasConfig.default(),
                protocol=ProtocolConfig(default_model=args.default_model),
                schedule=ScheduleConfig(
                    ttft_seconds=(
                        getattr(
                            args,
                            "ttft_milliseconds",
                            DEFAULT_TTFT_MILLISECONDS,
                        )
                        / 1000.0
                    ),
                    tpot_seconds=(
                        getattr(
                            args,
                            "tpot_milliseconds",
                            DEFAULT_TPOT_MILLISECONDS,
                        )
                        / 1000.0
                    ),
                ),
                server=ServerCoreConfig(
                    http=HTTPConfig(
                        bind_host=args.http_bind_host,
                        bind_port=args.http_bind_port,
                    ),
                    https=cls._https_config(args),
                    max_request_bytes=args.max_request_bytes,
                    request_timeout_seconds=args.request_timeout_seconds,
                    api_key=api_key,
                    log_requests=args.log_requests,
                ),
                record=RecordConfig(
                    record=(mode == "record"),
                    recordings_dir=getattr(
                        args,
                        "recordings_dir",
                        DEFAULT_RECORDINGS_DIR,
                    ),
                ),
                transport=transport_config,
            )
        except ValueError as error:
            raise ConfigurationError(str(error)) from error

    @classmethod
    def _transport_config(
        cls,
        mode: str,
        args: argparse.Namespace,
    ) -> TransportConfig | None:
        if mode == "transport" and hasattr(args, "transport_config"):
            return cls._load_transport_config(args.transport_config)
        return None

    @staticmethod
    def _load_transport_config(source: Path) -> TransportConfig:
        try:
            raw = source.read_bytes()
            document = StrictJsonDecoder().decode_utf8(raw)
        except (OSError, StrictJsonError) as error:
            raise ConfigurationError(
                f"invalid transport config: {error}"
            ) from error
        if not isinstance(document, dict):
            raise ConfigurationError(
                "transport config must be a JSON object"
            )
        unknown = sorted(document.keys() - {"base_url", "api_key", "model"})
        if unknown:
            raise ConfigurationError(
                "transport config contains unknown fields: "
                + ", ".join(unknown)
            )
        base_url = document.get("base_url")
        api_key_value = document.get("api_key")
        model = document.get("model")
        if not isinstance(base_url, str) or not base_url:
            raise ConfigurationError(
                "transport config base_url must be a non-empty string"
            )
        if not isinstance(api_key_value, str) or not api_key_value:
            raise ConfigurationError(
                "transport config api_key must be a non-empty string"
            )
        if model is not None and (
            not isinstance(model, str) or not model
        ):
            raise ConfigurationError(
                "transport config model must be a non-empty string"
            )
        try:
            upstream = UpstreamConfig(
                base_url=base_url,
                api_key=api_key_value,
                model=model,
            )
        except ValueError as error:
            raise ConfigurationError(str(error)) from error
        return TransportConfig(upstream=upstream)

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


class LocalMaaSConfigParser:
    """Subcommand parser: common server options plus per-mode arguments."""

    def parse_args(self) -> argparse.Namespace:
        parser = self._build_parser()
        args = parser.parse_args()
        self._maybe_list_scenarios(parser, args)
        self._require_scenario(parser, args)
        return args

    @staticmethod
    def _build_parser() -> argparse.ArgumentParser:
        common = argparse.ArgumentParser(add_help=False)
        LocalMaaSConfigParser._add_common_options(common)
        parser = argparse.ArgumentParser(
            prog="server.py",
            description="Run a local MaaS server: replay, transport, or record.",
            formatter_class=argparse.ArgumentDefaultsHelpFormatter,
        )
        subparsers = parser.add_subparsers(
            dest="command",
            required=True,
            metavar="COMMAND",
        )
        replay = subparsers.add_parser(
            "replay",
            parents=[common],
            help="replay a scenario against a local agent",
            formatter_class=argparse.ArgumentDefaultsHelpFormatter,
        )
        LocalMaaSConfigParser._add_replay_options(replay)
        transport = subparsers.add_parser(
            "transport",
            parents=[common],
            help="transparent proxy to a real upstream MaaS",
            formatter_class=argparse.ArgumentDefaultsHelpFormatter,
        )
        LocalMaaSConfigParser._add_transport_options(transport)
        record = subparsers.add_parser(
            "record",
            parents=[common],
            help="forward to a real upstream and record a scenario",
            formatter_class=argparse.ArgumentDefaultsHelpFormatter,
        )
        LocalMaaSConfigParser._add_record_options(record)
        return parser

    @staticmethod
    def _add_common_options(parser: argparse.ArgumentParser) -> None:
        parser.add_argument(
            "--default-model",
            default=DEFAULT_MODEL,
            help="model name used when a response does not specify one",
        )
        parser.add_argument(
            "--http-bind-host",
            default=DEFAULT_HTTP_BIND_HOST,
            help="host to bind the HTTP listener",
        )
        parser.add_argument(
            "--http-bind-port",
            type=int,
            default=DEFAULT_HTTP_BIND_PORT,
            help="TCP port for the HTTP listener (0 = ephemeral)",
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
            help="maximum accepted MaaS request body size in bytes",
        )
        parser.add_argument(
            "--request-timeout-seconds",
            type=float,
            default=DEFAULT_REQUEST_TIMEOUT_SECONDS,
            help="timeout for reading a complete MaaS request",
        )
        parser.add_argument(
            "--log-requests",
            action="store_true",
            help="write metadata-only request completion records to stderr",
        )

    @staticmethod
    def _add_replay_options(parser: argparse.ArgumentParser) -> None:
        parser.add_argument(
            "--templates-dir",
            type=Path,
            default=ScenarioRegistry.resolve_templates_dir(),
            help=(
                "directory containing scenario generator JSON templates; "
                "defaults to $LOCAL_MAAS_TEMPLATES_DIR or the bundled "
                "templates directory"
            ),
        )
        parser.add_argument(
            "--action-pools-dir",
            type=Path,
            default=DEFAULT_SCENARIO_GENERATOR_DIR / "action_pools",
            help="directory containing reusable action pool JSON generators",
        )
        parser.add_argument(
            "--scenario",
            default=argparse.SUPPRESS,
            help="template path relative to --templates-dir; .json is optional",
        )
        parser.add_argument(
            "--list-scenarios",
            action="store_true",
            help="list available scenarios and exit",
        )
        parser.add_argument(
            "--max-template-bytes",
            type=int,
            default=DEFAULT_MAX_TEMPLATE_BYTES,
            help="maximum scenario meta/sequence file size in bytes",
        )
        parser.add_argument(
            "--max-generator-depth",
            type=int,
            default=DEFAULT_MAX_GENERATOR_DEPTH,
            help="maximum nested generator depth",
        )
        parser.add_argument(
            "--max-generator-nodes",
            type=int,
            default=DEFAULT_MAX_GENERATOR_NODES,
            help="maximum generator nodes per scenario",
        )
        parser.add_argument(
            "--random-seed",
            type=int,
            default=DEFAULT_RANDOM_SEED,
            help="default deterministic seed for random generators",
        )
        parser.add_argument(
            "--loop-exhausted-messages",
            action=argparse.BooleanOptionalAction,
            default=True,
            help=(
                "replay recorded message rounds from the start once the "
                "message queue is exhausted"
            ),
        )
        parser.add_argument(
            "--lazy-load-size",
            type=int,
            default=0,
            help=(
                "recorded rounds read batch: 0 reads the whole file eagerly, "
                "N>0 streams N lines at a time"
            ),
        )
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
        parser.add_argument(
            "--api-key-env",
            help="environment variable containing the required local API key",
        )

    @staticmethod
    def _add_transport_options(parser: argparse.ArgumentParser) -> None:
        parser.add_argument(
            "--transport-config",
            type=Path,
            default=argparse.SUPPRESS,
            help=(
                "JSON file with the upstream MaaS config; when omitted the "
                "upstream resolves from the environment"
            ),
        )
        parser.add_argument(
            "--api-key-env",
            help="environment variable containing the required local API key",
        )

    @staticmethod
    def _add_record_options(parser: argparse.ArgumentParser) -> None:
        parser.add_argument(
            "--templates-dir",
            type=Path,
            default=ScenarioRegistry.resolve_templates_dir(),
            help=(
                "directory for finalized recorded scenarios; defaults to "
                "$LOCAL_MAAS_TEMPLATES_DIR or the bundled templates directory"
            ),
        )
        parser.add_argument(
            "--max-template-bytes",
            type=int,
            default=DEFAULT_MAX_TEMPLATE_BYTES,
            help="maximum scenario meta/sequence file size in bytes",
        )
        parser.add_argument(
            "--max-generator-depth",
            type=int,
            default=DEFAULT_MAX_GENERATOR_DEPTH,
            help="maximum nested generator depth",
        )
        parser.add_argument(
            "--max-generator-nodes",
            type=int,
            default=DEFAULT_MAX_GENERATOR_NODES,
            help="maximum generator nodes per scenario",
        )
        parser.add_argument(
            "--random-seed",
            type=int,
            default=DEFAULT_RANDOM_SEED,
            help="default deterministic seed for scenario validation",
        )
        parser.add_argument(
            "--recordings-dir",
            type=Path,
            default=DEFAULT_RECORDINGS_DIR,
            help="directory for recording caches and finalized scenarios",
        )

    @staticmethod
    def _require_scenario(
        parser: argparse.ArgumentParser,
        args: argparse.Namespace,
    ) -> None:
        if args.command != "replay" or hasattr(args, "scenario"):
            return
        if hasattr(args, "scenario"):
            return
        scenarios = LocalMaaSConfigParser._load_scenarios(parser, args)
        heading = colorize("available scenarios", "cyan", sys.stderr)
        choices = []
        for scenario in scenarios:
            choices.append(
                f"  {colorize(scenario.scenario_id, 'green', sys.stderr)}"
            )
            meta = LocalMaaSConfigParser._scenario_meta_text(scenario)
            if meta:
                choices.append(f"    {meta}")
            choices.append(f"    {scenario.description}")
        parser.error(
            f"--scenario is required; {heading}:\n" + "\n".join(choices)
        )

    @staticmethod
    def _maybe_list_scenarios(
        parser: argparse.ArgumentParser,
        args: argparse.Namespace,
    ) -> None:
        if not getattr(args, "list_scenarios", False):
            return
        scenarios = LocalMaaSConfigParser._load_scenarios(parser, args)
        print(colorize("available scenarios", "cyan", sys.stdout))
        for scenario in scenarios:
            print(
                f"  {colorize(scenario.scenario_id, 'green', sys.stdout)}"
            )
            meta = LocalMaaSConfigParser._scenario_meta_text(scenario)
            if meta:
                print(f"    {meta}")
            print(f"    {scenario.description}")
        raise SystemExit(0)

    @staticmethod
    def _load_scenarios(
        parser: argparse.ArgumentParser,
        args: argparse.Namespace,
    ) -> tuple[ScenarioMeta, ...]:
        templates_dir = args.templates_dir.resolve()
        if not templates_dir.is_dir():
            parser.error(
                f"scenario template directory does not exist: {templates_dir}"
            )
        try:
            scenarios = ScenarioRegistry(
                templates_dir,
                args.max_template_bytes,
            ).available_scenarios()
        except ScenarioConfigurationError as error:
            parser.error(str(error))
        if not scenarios:
            parser.error(f"no JSON scenarios found in {templates_dir}")
        return scenarios

    @staticmethod
    def _scenario_meta_text(scenario: ScenarioMeta) -> str:
        if scenario.rounds is None and not scenario.generator_type:
            return ""

        def fmt(value: int | None) -> str:
            return "inf" if value is None else str(value)

        parts = [
            f"rounds={fmt(scenario.rounds)}",
            (
                f"(tool={fmt(scenario.tool_rounds)}, "
                f"message={fmt(scenario.message_rounds)})"
            ),
        ]
        if scenario.tools:
            parts.append(f"tools=[{','.join(scenario.tools)}]")
        return " ".join(parts)

def parse_cli_args() -> LocalMaaSConfig:
    args = LocalMaaSConfigParser().parse_args()
    return LocalMaaSConfig.parse_from(args)
