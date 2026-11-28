"""Own Host sandbox observation processes and their connection lifecycle."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from tests.e2e.common.process import SubprocessRunner
from ..sandbox_agent import SandboxAgentProfile


@dataclass(frozen=True)
class HostSandboxObservationConfig:
    bin_dir: Path
    work_dir: Path
    command_timeout_seconds: float
    ready_timeout_seconds: float
    terminate_grace_seconds: float
    root_process_name: str
    host_cid: int
    vsock_port: int
    daemon_address: str


class HostSandboxObservation:
    """作用域只拥有 SB 和 gateway，连接和断开由场景显式控制。"""

    def __init__(self, config: HostSandboxObservationConfig):
        self._config = config
        self._runner = SubprocessRunner()
        self._profile = SandboxAgentProfile(
            binary=config.bin_dir / "actrail-sb", work_dir=config.work_dir,
            runner=self._runner, command_timeout_seconds=config.command_timeout_seconds,
        )
        self.sandbox_agent = None
        self.gateway = None
        self.timing = None

    def __enter__(self):
        try:
            self.timing = self._profile.refresh_default_config(self._config.root_process_name)
            self.sandbox_agent = self._runner.start(self._profile.daemon_argv())
            self._profile.wait_ready(self.sandbox_agent, self._config.ready_timeout_seconds)
        except BaseException as error:
            self.__exit__(type(error), error, error.__traceback__)
            raise
        return self

    def connect(self):
        config = self._config
        if self.gateway is not None:
            raise RuntimeError("Sandbox gateway is already connected")
        configured = self._runner.run((
            str(config.bin_dir / "actrail-vsock-gateway"), "init", "--output",
            str(config.work_dir / "gateway.toml"), "--backend", "native",
            "--cid", str(config.host_cid), "--port", str(config.vsock_port),
            "--daemon-address", config.daemon_address,
        ), timeout=config.command_timeout_seconds)
        if configured.returncode != 0:
            raise RuntimeError("Gateway configuration failed: " + configured.diagnostic)
        self.gateway = self._runner.start((
            str(config.bin_dir / "actrail-vsock-gateway"), "--config",
            str(config.work_dir / "gateway.toml"),
        ))
        self.gateway.wait_for_output("gateway ready gateway_id=", timeout=config.ready_timeout_seconds)
        return self._profile.connect(host_cid=config.host_cid, port=config.vsock_port)

    def disconnect_gateway(self) -> None:
        if self.gateway is not None:
            self.gateway.terminate(grace_seconds=self._config.terminate_grace_seconds)
            self.gateway = None

    def reconnect(self):
        self.disconnect_gateway()
        return self.connect()

    def __exit__(self, exception_type, exception, traceback) -> None:
        errors = []
        for name in ("gateway", "sandbox_agent"):
            process = getattr(self, name)
            if process is not None:
                try:
                    stopped = process.terminate(grace_seconds=self._config.terminate_grace_seconds)
                    if exception is not None and stopped.diagnostic:
                        exception.add_note(f"{name} diagnostics: {stopped.diagnostic[-4000:]}")
                except Exception as error:
                    errors.append(f"{name}: {error}")
                finally:
                    setattr(self, name, None)
        if errors:
            message = "\n".join(errors)
            if exception is not None:
                exception.add_note(message)
            else:
                raise RuntimeError(message)
