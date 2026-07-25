from .testing_env import AgentAvailability


class TestingContextSingleton:
    _instance = None

    def __new__(cls, *args, **kwargs):
        if cls._instance is None:
            cls._instance = super(TestingContextSingleton, cls).__new__(cls)
            cls._instance._env_dict = {}
            cls._instance.agent_availability = AgentAvailability()
        return cls._instance

    def check_agent_availability(self, agent_name: str) -> bool:
        return self.agent_availability.check_agent_availability(agent_name)
