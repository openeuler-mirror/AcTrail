"""Exercise reused resource buffers with real files and inspect their audits."""
import errno
import os


class FileResources:
    capacity = 252

    def __init__(self, root):
        self.root = root
        self.short = root / "s"
        self.long = root / ("long-" + "x" * (240 - len(str(root)) - 6))
        self.overflow = root / "overflow" / ("x" * 160) / ("y" * 160)
        if not len(str(self.short)) < len(str(self.long)) < self.capacity:
            raise ValueError("resource regression requires a shorter work directory")

    def prepare(self):
        self.overflow.parent.mkdir(parents=True)
        for path in (self.long, self.short, self.overflow):
            path.write_bytes(b"resource")

    def run(self):
        results = []
        # The same task alternates long, short, failed and successful resolution.
        for path in (self.long, self.short, self.overflow, self.short):
            try:
                descriptor = os.open(path, os.O_RDONLY)
                os.close(descriptor)
                results.append({"path": str(path), "errno": 0})
            except OSError as error:
                results.append({"path": str(path), "errno": error.errno})
        return results

    def verify(self, results):
        paths = (self.long, self.short, self.overflow, self.short)
        if len(results) != len(paths):
            raise AssertionError(f"resource operations missing: {results}")
        for path, observed in zip(paths, results):
            expected = errno.EACCES if path == self.overflow else 0
            if observed != {"path": str(path), "errno": expected}:
                raise AssertionError(f"unexpected resource access: {observed}")

    def verify_audit(self, events):
        payloads = [event.get("payload", {}) for event in events]
        for path in (self.long, self.short):
            matches = [payload for payload in payloads
                       if payload.get("operation") == "open" and payload.get("path") == str(path)]
            if not matches or any(payload.get("decision") != "allow" for payload in matches):
                raise AssertionError(f"missing successful resource audit: {path}")
        failures = [payload for payload in payloads if payload.get("operation") == "open"
                    and payload.get("metadata", {}).get("resource_error") == "-36"]
        if not failures or any(payload.get("path") is not None for payload in failures):
            raise AssertionError(f"failed resolution retained resource bytes: {failures}")
