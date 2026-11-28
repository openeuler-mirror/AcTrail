"""Run the configured isolation regression scenario."""
from tests.e2e.common.execution_isolation.kata.case import KataExecutionIsolationCase
from tests.e2e.regression.execution_isolation_stratovirt.v2.config import StratoVirtExecutionIsolationConfig


class ExecutionIsolationStratovirtCase(KataExecutionIsolationCase):
    case_id = 'execution_isolation_stratovirt'
    description = 'Optional StratoVirt VMM coverage using native AF_VSOCK, the Kata lifecycle profile and a real xiaoO sandbox observation path'
    default_enabled = False

    config_type = StratoVirtExecutionIsolationConfig

