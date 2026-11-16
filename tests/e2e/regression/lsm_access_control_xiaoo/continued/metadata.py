"""Change real file metadata through paths and retained descriptors."""

import os
import stat


class MetadataOperations:
    def __init__(self, root, descriptors):
        self.root = root
        self.descriptors = descriptors

    def run(self, phase, results):
        mode = {"initial": 0o640, "deny": 0o604, "restore": 0o600}[phase]
        for role, descriptor in self.descriptors.items():
            for name, target, action in (
                ("chmod", mode ^ 0o020, lambda: os.chmod(self.root / role, mode ^ 0o020)),
                ("fchmod", mode, lambda: os.fchmod(descriptor, mode)),
            ):
                before = stat.S_IMODE(os.fstat(descriptor).st_mode)
                try:
                    action()
                    observed = {"allowed": True}
                except OSError as error:
                    observed = {"allowed": False, "errno": error.errno}
                actual = stat.S_IMODE(os.fstat(descriptor).st_mode)
                if actual != (target if observed["allowed"] else before):
                    raise AssertionError(f"{name} produced unexpected mode: {actual:o}")
                results[f"{role}-{name}"] = observed
        owner = {"initial": 65534, "deny": 65533, "restore": os.getuid()}[phase]
        for role, descriptor in self.descriptors.items():
            for name, target, action in (
                ("chown", owner ^ 1, lambda: os.chown(self.root / role, owner ^ 1, owner ^ 1)),
                ("fchown", owner, lambda: os.fchown(descriptor, owner, owner)),
            ):
                before = os.fstat(descriptor)
                try:
                    action()
                    observed = {"allowed": True}
                except OSError as error:
                    observed = {"allowed": False, "errno": error.errno}
                actual = os.fstat(descriptor)
                expected = (target, target) if observed["allowed"] else (before.st_uid, before.st_gid)
                if (actual.st_uid, actual.st_gid) != expected:
                    raise AssertionError(f"{name} produced unexpected ownership")
                results[f"{role}-{name}"] = observed
        descriptor = os.memfd_create("actrail-metadata")
        try:
            os.fchmod(descriptor, mode)
            if stat.S_IMODE(os.fstat(descriptor).st_mode) != mode:
                raise AssertionError("memfd chmod was affected by path policy")
            os.fchown(descriptor, owner, owner)
            actual = os.fstat(descriptor)
            if (actual.st_uid, actual.st_gid) != (owner, owner):
                raise AssertionError("memfd chown was affected by path policy")
        finally:
            os.close(descriptor)
