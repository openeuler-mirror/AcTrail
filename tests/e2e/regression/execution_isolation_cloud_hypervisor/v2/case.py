"""Run the configured isolation regression scenario."""
from tests.e2e.common.execution_isolation.kata.case import KataExecutionIsolationCase
from tests.e2e.regression.execution_isolation_cloud_hypervisor.v2.config import CloudHypervisorExecutionIsolationConfig


class ExecutionIsolationCloudHypervisorCase(KataExecutionIsolationCase):
    case_id = 'execution_isolation_cloud_hypervisor'
    description = 'Optional Cloud Hypervisor VMM coverage using the Kata lifecycle profile and a real xiaoO sandbox observation path'
    default_enabled = False

    config_type = CloudHypervisorExecutionIsolationConfig

