"""Register real-agent policy benchmark execution and results."""
import os
import shutil

from tests.e2e.common.core import TestRunDirectories, TestCase, TestResult, TestStatus
from .config import PerformanceConfig
from .benchmark import Benchmark
from .report import Report
from .retained.benchmark import RetainedBenchmark


class LsmPerformanceCase(TestCase):
    case_id = 'lsm_access_control_xiaoo_performance'
    description = 'Measure real Xiaoo TCP policy workloads with seccomp and BPF LSM'
    default_enabled = False

    def __init__(self, directories: TestRunDirectories):
        self.config = PerformanceConfig.from_environment(directories, 'LSM_PERFORMANCE_E2E_')

    def run(self, context):
        if os.geteuid() != 0 or not (self.config.xiaoo or shutil.which('xiaoo')):
            return TestResult(TestStatus.SKIPPED, 'Root and real Xiaoo are required')
        Benchmark(self.config).run(context)
        Report(self.config.output, self.config.output.with_suffix(".md")).run()
        return TestResult(TestStatus.PASSED, 'TCP policy benchmarks completed')


class LsmRetainedPerformanceCase(TestCase):
    case_id = 'lsm_access_control_xiaoo_retained_performance'
    description = 'Measure retained FD and socket operations through real Xiaoo'
    default_enabled = False

    def __init__(self, directories: TestRunDirectories):
        self.config = PerformanceConfig.from_environment(directories, 'LSM_RETAINED_PERFORMANCE_E2E_')

    def run(self, context):
        if os.geteuid() != 0 or not (self.config.xiaoo or shutil.which('xiaoo')):
            return TestResult(TestStatus.SKIPPED, 'Root and real Xiaoo are required')
        RetainedBenchmark(self.config).run(context)
        return TestResult(TestStatus.PASSED, 'Retained operations benchmarks completed')
