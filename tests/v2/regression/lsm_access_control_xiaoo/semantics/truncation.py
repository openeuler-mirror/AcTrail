"""Verify pathname, retained-FD and open-time truncation effects."""
import os


class TruncationOperations:
    def __init__(self, root):
        self.path = root / "truncate"
        self.descriptor = os.open(self.path, os.O_RDWR)

    def run(self, operations):
        for name, action in (("extend", lambda: os.ftruncate(self.descriptor, 8192)),
                             ("path", lambda: os.truncate(self.path, 2000)),
                             ("fd", lambda: os.ftruncate(self.descriptor, 1000)),
                             ("open-readonly", lambda: self.open_truncate(os.O_RDONLY)),
                             ("open", lambda: self.open_truncate(os.O_WRONLY))):
            before = self.contents()
            try:
                action()
                length = {"extend": 8192, "path": 2000, "fd": 1000, "open-readonly": 0, "open": 0}[name]
                expected = before[:length] + b"\0" * max(0, length - len(before))
                if self.contents() != expected or os.fstat(self.descriptor).st_size != length:
                    raise AssertionError(f"{name}: truncation contents differ")
                operations[f"truncate-{name}"] = {"allowed": True}
            except OSError as error:
                if self.contents() != before or os.fstat(self.descriptor).st_size != len(before):
                    raise AssertionError(f"{name}: denied truncation changed contents") from error
                operations[f"truncate-{name}"] = {"allowed": False, "errno": error.errno}

    def open_truncate(self, mode):
        descriptor = os.open(self.path, mode | os.O_TRUNC)
        os.close(descriptor)

    def contents(self):
        return os.pread(self.descriptor, os.fstat(self.descriptor).st_size, 0)

    def close(self):
        os.close(self.descriptor)
