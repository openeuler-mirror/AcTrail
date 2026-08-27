from __future__ import annotations

import unittest

from tests.v2.common.execution_isolation.sandbox_agent import SandboxAgentTiming


class SandboxAgentTimingTest(unittest.TestCase):
    def test_root_discovery_wait_covers_two_refresh_intervals(self) -> None:
        timing = SandboxAgentTiming(
            io_poll_seconds=0.5,
            resource_poll_seconds=1.0,
            sender_io_timeout_seconds=2.0,
            reconnect_interval_seconds=1.0,
            root_refresh_seconds=1.5,
        )

        self.assertEqual(timing.minimum_root_discovery_settle_seconds, 3.25)


if __name__ == "__main__":
    unittest.main()
