from __future__ import annotations

import unittest

from tests.v2.regression.virtual_container.v2.assertions import (
    find_clean_trace,
    parse_summary_counts,
)


class VirtualContainerAssertionsTest(unittest.TestCase):
    def test_finds_trace_by_title_in_machine_readable_viewer_output(self) -> None:
        output = """{
          "traces": [
            {"trace_id": "trace-8", "name": "other",
             "state": "Completed", "health": "Clean"},
            {"trace_id": "trace-9", "name": "kata-combo-abc",
             "state": "Completed", "health": "Clean"}
          ]
        }"""

        trace = find_clean_trace(output, "kata-combo-abc")

        self.assertEqual(trace.trace_id, "trace-9")

    def test_parses_event_counts_without_assuming_trace_id(self) -> None:
        counts = parse_summary_counts(
            "Trace trace-9 title=kata-combo-abc state=Completed health=Clean "
            "profile=kata-guest\n"
            "root_process_id=1 processes=5 events=188 network_events=98 "
            "diagnostics=0\n"
        )

        self.assertEqual(counts.events, 188)
        self.assertEqual(counts.network_events, 98)


if __name__ == "__main__":
    unittest.main()
