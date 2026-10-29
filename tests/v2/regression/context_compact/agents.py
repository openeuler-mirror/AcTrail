from __future__ import annotations

import os

from tests.v2.common.testing_env import AgentBinaryDiscovery, default_claude_model

from .config import ContextCompactConfig
from .scenario import AgentSpec


class CompactAgentCatalog:
    """Resolve candidates through the shared real-agent discovery chain."""

    def __init__(self, config: ContextCompactConfig):
        self._discovery = AgentBinaryDiscovery(config.repo)

    def resolve(self, name: str) -> AgentSpec | None:
        variable = f"{name.upper()}_E2E_BINARY"
        binary = self._discovery.resolve(variable, name)
        if binary is None:
            if os.environ.get(variable):
                raise ValueError(f"{variable} is not executable: {os.environ[variable]}")
            return None
        binary = binary.resolve()
        environment = self._discovery.environment(binary)
        if name == "claude":
            return AgentSpec(
                name, binary, environment, None,
                arguments=(
                    "--model", default_claude_model(),
                    "--safe-mode", "--permission-mode", "dontAsk",
                    "--tools", "Bash",
                    "--allowedTools", "Bash",
                ),
            )
        model = None
        configured_model = os.environ.get("OPENCODE_E2E_MODEL")
        if configured_model:
            provider, separator, model_name = configured_model.partition("/")
            if not separator or not provider or not model_name:
                raise ValueError("OPENCODE_E2E_MODEL must be provider/model")
            model = {"providerID": provider, "modelID": model_name}
        return AgentSpec(name, binary, environment, model)
