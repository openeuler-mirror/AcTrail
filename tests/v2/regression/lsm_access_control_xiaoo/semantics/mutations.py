"""Check actual deletion and attribute effects independently of content access."""
import os
import stat


class FileMutations:
    def __init__(self, root, descriptor):
        self.root = root
        self.descriptor = descriptor
        self.directory = os.open(root / "remove", os.O_RDONLY | os.O_DIRECTORY)

    def run(self, operations):
        for name in ("absolute", "relative"):
            path = self.root / "remove" / name
            # The same directory entry remains readable and writable when deletion is denied.
            with path.open("r+b") as stream:
                assert stream.read() == b"delete me"
                stream.seek(0)
                stream.write(b"writable!")
                stream.flush()
                stream.seek(0)
                assert stream.read() == b"writable!"
            try:
                if name == "absolute":
                    path.unlink()
                else:
                    os.unlink(name, dir_fd=self.directory)
                operations[f"delete-{name}"] = {"allowed": True}
                if path.exists():
                    raise AssertionError(f"unlink retained {path}")
            except OSError as error:
                if not path.exists():
                    raise AssertionError(f"denied unlink removed {path}") from error
                operations[f"delete-{name}"] = {"allowed": False, "errno": error.errno}
        path = self.root / "protected"
        for name in ("chmod", "fchmod", "chown", "fchown"):
            before = self.attributes()
            mode = 0o640 if before[0] == 0o600 else 0o600
            owner = 65534 if before[1] != 65534 else 65533
            try:
                if name == "chmod":
                    os.chmod(path, mode)
                elif name == "fchmod":
                    os.fchmod(self.descriptor, mode)
                elif name == "chown":
                    os.chown(path, owner, owner)
                else:
                    os.fchown(self.descriptor, owner, owner)
                expected = (mode, *before[1:]) if name.endswith("chmod") else (before[0], owner, owner)
                if self.attributes() != expected:
                    raise AssertionError(f"{name}: attributes did not change as requested")
                operations[f"attr-{name}"] = {"allowed": True}
            except OSError as error:
                if self.attributes() != before:
                    raise AssertionError(f"{name}: denied call changed attributes") from error
                operations[f"attr-{name}"] = {"allowed": False, "errno": error.errno}

    def attributes(self):
        info = os.fstat(self.descriptor)
        return stat.S_IMODE(info.st_mode), info.st_uid, info.st_gid

    def close(self):
        os.close(self.directory)
