import unittest
from tests.v2.regression.resource_metrics_container.case import assert_container_samples


class ContainerSampleAssertions(unittest.TestCase):
    def test_rejects_procfs_fallback_and_wrong_container(self):
        valid = dict(sample_kind="periodic", scope="container", accounting_method="cgroup_v2",
                     accounting_coverage="broader_than_trace", subject="container:abc", memory_current_bytes=8192)
        assert_container_samples([valid], "abc", [8192])
        for change in [dict(accounting_method="procfs"), dict(subject="container:other"),
                       dict(accounting_coverage="exact"), dict(memory_current_bytes=10**9), dict(rss_kb=8)]:
            with self.subTest(change=change), self.assertRaises(AssertionError):
                assert_container_samples([dict(valid, **change)], "abc", [8192])

    def test_requires_actual_samples(self):
        with self.assertRaises(AssertionError):
            assert_container_samples([], "abc", [8192])
