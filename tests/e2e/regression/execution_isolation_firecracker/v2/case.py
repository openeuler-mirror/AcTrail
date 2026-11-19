"""Run the configured isolation regression scenario."""
from tests.e2e.common.execution_isolation.kata.case import KataExecutionIsolationCase
from tests.e2e.regression.execution_isolation_firecracker.v2.config import FirecrackerExecutionIsolationConfig


class ExecutionIsolationFirecrackerCase(KataExecutionIsolationCase):
    case_id = 'execution_isolation_firecracker'
    description = 'Firecracker coverage through the Kata lifecycle, its hybrid-VSOCK bridge and a real xiaoO sandbox observation path'
    default_enabled = False

    config_type = FirecrackerExecutionIsolationConfig

