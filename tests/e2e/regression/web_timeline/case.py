"""Register browser acceptance scenarios with the common regression runner."""
from dataclasses import dataclass, replace
import os
from pathlib import Path
import shutil

from tests.e2e.common.core import TestCase, TestRunDirectories, TestResult, TestStatus
from .constants import CASE_CONSTANTS as C
from .acceptance import Acceptance
from .network.acceptance import NetworkAcceptance
from .cluster_acceptance import ClusterAcceptance


@dataclass(frozen=True)
class WebTimelineConfig:
    work: Path
    bin_dir: Path
    agent: Path
    agent_timeout: int
    port: int
    endpoint: str
    playwright: str
    chromium: str
    source: Path | None = None

    @classmethod
    def from_environment(cls, directories: TestRunDirectories):
        """Read browser and agent settings shared by all Web scenarios."""
        prefix = 'WEB_TIMELINE_E2E_'
        source = os.environ.get(prefix + 'SOURCE')
        return cls(
            work=directories.work_dir / 'acceptance',
            bin_dir=directories.bin_dir,
            agent=Path(os.environ.get(prefix + 'AGENT', shutil.which('xiaoo') or C.agent_path)),
            agent_timeout=int(os.environ.get(prefix + 'AGENT_TIMEOUT', C.agent_timeout_seconds)),
            port=int(os.environ.get(prefix + 'PORT', C.port)),
            endpoint=os.environ.get(prefix + 'ENDPOINT', C.endpoint),
            playwright=os.environ.get(prefix + 'PLAYWRIGHT', C.playwright_path),
            chromium=os.environ.get(prefix + 'CHROMIUM', C.chromium_path),
            source=Path(source) if source else None,
        )


class WebTimelineCase(TestCase):
    case_id = 'web_timeline'
    description = 'Capture real agent activity and verify Web timeline and content in Chromium'
    default_enabled = False
    acceptance = Acceptance


    def __init__(self, directories: TestRunDirectories) -> None:
        config = WebTimelineConfig.from_environment(directories)
        self.config = config

    def missing_dependency(self):
        if not shutil.which('node'):
            return 'Node.js is required for browser acceptance'
        for label, path in (('XiaoO', self.config.agent), ('Playwright', self.config.playwright),
                            ('Chromium', self.config.chromium)):
            if not Path(path).is_file():
                return f'{label} is unavailable: {path}'
        return None

    def run(self, test_context):
        if reason := self.missing_dependency():
            return TestResult(TestStatus.SKIPPED, reason)
        self.acceptance(self.config).run()
        return TestResult(TestStatus.PASSED, self.description)


class WebTimelineNetworkCase(WebTimelineCase):
    case_id = 'web_timeline_network'
    description = 'Capture a real agent HTTP tool request and verify its Web view'
    acceptance = NetworkAcceptance


class WebTimelineClusterCase(WebTimelineCase):
    case_id = 'web_timeline_cluster'
    description = 'Capture real agent traces and verify cluster import and browser navigation'

    def run(self, test_context):
        if reason := self.missing_dependency():
            return TestResult(TestStatus.SKIPPED, reason)
        # Generate the source in this case's own work directory when none is supplied.
        source = self.config.source
        if source is None:
            source = self.config.work.parent / 'source'
            Acceptance(replace(self.config, work=source)).run()
        ClusterAcceptance(replace(self.config, source=source)).run()
        return TestResult(TestStatus.PASSED, self.description)
