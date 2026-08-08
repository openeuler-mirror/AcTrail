from pathlib import Path

from .agent_availability import AgentAvailability
from .agent_discovery import AgentBinaryDiscovery, default_claude_model


def default_codex_model(repo: Path) -> str | None:
    return AgentBinaryDiscovery(repo).default_codex_model()
