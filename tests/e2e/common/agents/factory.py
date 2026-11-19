"""Construct agents from the internal concrete-adapter registry."""

from importlib import import_module
from pkgutil import walk_packages
from random import choice
from pathlib import Path
from typing import TYPE_CHECKING

from .base import Agent, _AgentMeta

if TYPE_CHECKING:
    from tests.e2e.common.runner import TestingContextSingleton


class _AgentFactoryMeta(type):
    def __getattr__(cls, name: str) -> type[Agent]:
        """Resolve calls such as AgentFactory.xiaoo() through the registry."""
        cls._load_agents()
        try:
            return _AgentMeta.registry[name]
        except KeyError:
            raise AttributeError(f"No agent registered as {name!r}") from None


class AgentFactory(metaclass=_AgentFactoryMeta):
    """Create registered agents and pass constructor arguments through."""

    _loaded = False

    @classmethod
    def create(cls, kind: str, **kwargs) -> Agent:
        """Construct a fresh instance of a named registered adapter."""
        return getattr(cls, kind)(**kwargs)

    @classmethod
    def auto(
        cls,
        repo: Path,
        test_context: "TestingContextSingleton",
        *,
        kinds: tuple[str, ...] | None = None,
    ) -> Agent | None:
        """Return the first available Agent using the run's cached checks."""
        from tests.e2e.common.testing_env import AgentBinaryDiscovery
        from tests.e2e.common.testing_env.constants import DEFAULTS

        discovery = AgentBinaryDiscovery(repo)
        candidates = DEFAULTS.candidates
        if kinds is not None:
            by_kind = {candidate[0]: candidate for candidate in candidates}
            candidates = tuple(by_kind[kind] for kind in kinds)
        for kind, variable, executable in candidates:
            binary = discovery.resolve(variable, executable)
            if binary is None:
                continue
            environment = (
                discovery.codex_environment(binary)
                if kind == "codex"
                else discovery.environment(binary)
            )
            test_context.report_progress("Check Agent availability", f"checking {kind} availability")
            if test_context.check_agent_availability(kind, binary, environment):
                return cls.create(kind, binary=binary, environment=environment)
        return None

    @classmethod
    def _load_agents(cls) -> None:
        """Import this package's adapters once so their classes are registered."""
        if cls._loaded:
            return
        package = import_module(__package__)
        for module in walk_packages(package.__path__, package.__name__ + "."):
            import_module(module.name)
        cls._loaded = True

    @staticmethod
    def random(*args, **kwargs) -> Agent:
        """Create an instance of a randomly selected registered agent class."""
        AgentFactory._load_agents()
        candidates = tuple(_AgentMeta.registry.values())
        if not candidates:
            raise LookupError("No agents registered")
        return choice(candidates)(*args, **kwargs)
