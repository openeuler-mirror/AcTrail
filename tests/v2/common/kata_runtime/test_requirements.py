from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from tests.v2.common.kata_runtime.requirements import (
    KataContainerRequirements,
    KataMount,
    KataRuntimeProfile,
    PreparePolicy,
    RequirementCheck,
    ResolvedImage,
)


class StaticImage:
    def ensure(self, policy: PreparePolicy) -> ResolvedImage:
        del policy
        return ResolvedImage("example.test/workload:24.09", "sha256:abcd")

    def refresh(self, reason: str) -> ResolvedImage:
        del reason
        return ResolvedImage("example.test/workload:24.09", "sha256:ef01")


class RunningContainer:
    def is_running(self) -> bool:
        return True


class KataContainerRequirementsTest(unittest.TestCase):
    def test_builds_validated_create_spec_from_immutable_requirements(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-requirements.") as raw_dir:
            root = Path(raw_dir)
            runtime_config = root / "configuration.toml"
            runtime_config.write_text("[runtime]\n", encoding="utf-8")
            bundle = root / "workload-bundle"
            bundle.mkdir()
            requirements = KataContainerRequirements(
                profile=KataRuntimeProfile(
                    backend="stratovirt",
                    namespace="default",
                    runtime="io.containerd.kata332.v2",
                    runtime_config=runtime_config,
                    image="example.test/workload:24.09",
                ),
                image=StaticImage(),
                name_prefix="interface",
                command=("/bin/sh", "-c", "sleep 600"),
                mounts=(KataMount(bundle, "/opt/actrail", read_only=True),),
                uid=1000,
                gid=39000,
                labels=(("io.actrail.test.profile", "base"),),
                ready_timeout_seconds=45,
                prepare_policy=PreparePolicy.MISSING,
            )

            requirements.validate_static()
            spec = requirements.create_spec(
                ResolvedImage("example.test/workload:24.09", "sha256:abcd")
            )
            check = requirements.validate_running(RunningContainer())

        self.assertEqual(spec.runtime, "io.containerd.kata332.v2")
        self.assertEqual(spec.runtime_config, runtime_config)
        self.assertEqual(spec.mounts[0].target, "/opt/actrail")
        self.assertEqual((spec.uid, spec.gid), (1000, 39000))
        self.assertEqual(check, RequirementCheck.ready_check())


if __name__ == "__main__":
    unittest.main()
