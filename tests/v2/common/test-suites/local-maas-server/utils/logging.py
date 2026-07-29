from __future__ import annotations

import json
import shlex
import sys
from functools import partial
from typing import TYPE_CHECKING, Mapping

from tests.v2.common.utils import colorize

if TYPE_CHECKING:
    from server_core.connection.interface import ConnectionDescription


class StartupLogger:
    def ready(
        self,
        *,
        connections: Mapping[str, ConnectionDescription],
        scenario: str,
        description: str,
        generator: str,
        infinite: bool,
        warnings: tuple[str, ...],
    ) -> None:
        paint = partial(colorize, stream=sys.stdout)
        lifetime = "infinite" if infinite else "finite"
        lines = [
            paint("Local MaaS server is ready", "green"),
            "",
            paint("Scenario", "cyan"),
            f"  template:    {paint(scenario, 'green')}",
            f"  description: {description}",
            f"  generator:   {paint(generator, 'green')} ({lifetime})",
        ]
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
