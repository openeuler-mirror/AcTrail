"""Agent interfaces shared by end-to-end scenarios."""

from .base import Agent, AgentResult
from .factory import AgentFactory

__all__ = ["Agent", "AgentFactory", "AgentResult"]
