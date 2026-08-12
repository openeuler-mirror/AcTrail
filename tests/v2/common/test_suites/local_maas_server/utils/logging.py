from __future__ import annotations

import json
import shlex
import sys
from functools import partial
from typing import TYPE_CHECKING, Mapping

from tests.v2.common.utils import colorize

if TYPE_CHECKING:
    from server_core.connection.interface import ConnectionDescription
    from server_core.help import HelpMessage


class StartupLogger:
    def ready(
        self,
        *,
        help: HelpMessage,
        connections: Mapping[str, ConnectionDescription],
        warnings: tuple[str, ...],
    ) -> None:
        paint = partial(colorize, stream=sys.stdout)
        lines: list[str] = [paint("Local MaaS server is ready", "green")]
        first_connection = next(iter(connections.values()), None)
        origin = first_connection.origin if first_connection else ""
        for title, section_lines in help.iter_sections(origin):
            lines.extend(["", paint(title, "cyan")])
            lines.extend(f"  {line}" for line in section_lines)
        client_environment = None
        for index, connection in enumerate(connections.values(), start=1):
            lines.extend(
                [
                    "",
                    paint(f"Listener {index}", "cyan"),
                    f"  service:    {paint(connection.service, 'green')}",
                    "  listen:     "
                    f"{paint(f'{connection.host}:{connection.port}', 'green')}",
                    f"  origin:     {paint(connection.origin, 'green')}",
                ]
            )
            if connection.ca_cert_file is not None:
                lines.append(
                    "  ca bundle:  "
                    f"{paint(connection.ca_cert_file, 'green')}"
                )
                client_environment = (
                    "SSL_CERT_FILE="
                    f"{shlex.quote(connection.ca_cert_file)}"
                    " <command>"
                )
            lines.extend(["", paint("  REST APIs:", "cyan")])
            current_service = None
            for route in connection.rest_apis:
                if route.service != current_service:
                    current_service = route.service
                    lines.append(f"    {paint(current_service, 'cyan')}")
                lines.append(
                    f"      {paint(f'{route.method:<5}', 'green')} "
                    f"{route.path}"
                )
        if warnings:
            lines.extend(["", paint("Warnings", "yellow")])
            lines.extend(
                f"  {paint(warning, 'yellow')}" for warning in warnings
            )
        lines.extend(["", paint("Press Ctrl+C to stop.", "yellow")])
        if client_environment is not None:
            lines.extend(
                [
                    "",
                    paint(
                        "Please run with the Local MaaS CA:",
                        "yellow",
                    )
                    + " "
                    + paint(client_environment, "green"),
                ]
            )
        print("\n".join(lines), flush=True)


class RequestLogger:
    def __init__(self, enabled: bool):
        self._enabled = enabled

    def completed(
        self,
        *,
        protocol: str,
        template_path: str,
        stream: bool,
        status: int,
    ) -> None:
        if not self._enabled:
            return
        print(
            json.dumps(
                {
                    "event": "local_maas_request",
                    "protocol": protocol,
                    "template": template_path,
                    "stream": stream,
                    "status": status,
                },
                ensure_ascii=False,
                separators=(",", ":"),
            ),
            file=sys.stderr,
            flush=True,
        )
