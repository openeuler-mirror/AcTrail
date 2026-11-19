"""Register module-free grey policy startup acceptance."""
import shutil
import subprocess
from pathlib import Path

from tests.e2e.common.core import TestRunDirectories, TestCase, TestResult, TestStatus
from .mode_scenario import ModeScenario
from tests.e2e.common.core import TestOutput


class LsmGreyModesCase(TestCase):
    case_id = "lsm_grey_xiaoo_modes"
    description = "Verify grey startup modes using real Xiaoo with fresh defaults"
    default_enabled = False

    def __init__(self, directories: TestRunDirectories):
        self.directories = directories

    def run(self, context):
        if not shutil.which("modinfo"):
            return TestResult(TestStatus.SKIPPED, "modinfo is required")
        if Path("/sys/module/actrail_lsm_grey").exists() or subprocess.run(
            ["modinfo", "actrail_lsm_grey"], capture_output=True
        ).returncode == 0:
            return TestResult(TestStatus.SKIPPED, "Requires a host without the grey module")
        output = TestOutput(color_mode="never")
        for backend, mode in (("lsm", "auto"), ("lsm", "disabled"),
                              ("lsm", "required"), ("fanotify_seccomp", "required")):
            work = self.directories.work_dir / f"{backend}-{mode}"
            work.mkdir()
            scenario = ModeScenario(work, backend, mode, output, self.directories.bin_dir)
            try:
                scenario.run()
            finally:
                scenario.close()
        return TestResult(TestStatus.PASSED, "Grey startup mode matrix passed")
