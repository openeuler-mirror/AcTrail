from pathlib import Path
from typing import Mapping

from .output import TestOutput
from .testing_env import AgentAvailability


class TestingContextSingleton:
    _instance = None

    def __new__(cls, *args, **kwargs):
        if cls._instance is None:
            cls._instance = super(TestingContextSingleton, cls).__new__(cls)
            cls._instance._env_dict = {}
            cls._instance.agent_availability = AgentAvailability()
            cls._instance.output = TestOutput()
        return cls._instance

    def check_agent_availability(
        self,
        agent_name: str,
        binary: Path | str | None = None,
        environment: Mapping[str, str] | None = None,
    ) -> bool:
        return self.agent_availability.check_agent_availability(
            agent_name,
            binary,
            environment,
        )
