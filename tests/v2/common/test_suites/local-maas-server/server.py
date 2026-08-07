#!/usr/bin/env python3
from __future__ import annotations

import atexit
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[5]))

from config import ConfigurationError, parse_cli_args
from scenario import ScenarioConfigurationError
from server_core.connection.interface import ConnectionStartupError
from server_core.server import LocalMaaSServer
from utils import ExitSignalWaiter

_STARTUP_ERRORS = (
    ConfigurationError,
    ConnectionStartupError,
    OSError,
    ScenarioConfigurationError,
    ValueError,
)


def main() -> int:
    try:
        config = parse_cli_args()
        server = LocalMaaSServer(config)
    except _STARTUP_ERRORS as error:
        print(
            f"local_maas_startup_error={error}",
            file=sys.stderr,
            flush=True,
        )
        return 2

    exit_waiter = ExitSignalWaiter()
    cleanup = server.stop
    atexit.register(cleanup)
    try:
        exit_waiter.install()
        try:
            server.start(silent=False)
        except _STARTUP_ERRORS as error:
            print(
                f"local_maas_startup_error={error}",
                file=sys.stderr,
                flush=True,
            )
            return 2
        exit_waiter.wait()
    finally:
        cleanup()
        atexit.unregister(cleanup)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
