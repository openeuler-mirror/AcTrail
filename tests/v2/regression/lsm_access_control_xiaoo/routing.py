from __future__ import annotations

import errno


class BackendSelection:
    """Expected execution mechanisms for real-agent access-control scenarios."""

    MATRIX = {
        "lsm": ("lsm", "lsm", "lsm", "lsm", "lsm"),
        "fanotify_seccomp": ("fanotify", "seccomp", "seccomp", "seccomp", "seccomp"),
        "fanotify_lsm": ("fanotify", "lsm", "lsm", "lsm", "lsm"),
        "mixed_a": ("fanotify", "lsm", "seccomp", "lsm", "seccomp"),
        "mixed_b": ("lsm", "seccomp", "lsm", "seccomp", "lsm"),
    }
    OPERATIONS = ("file.open", "file.mkdir", "file.rmdir", "command.exec", "network.connect")

    def __init__(self, name: str):
        self.name = name
        self.backends = dict(zip(self.OPERATIONS, self.MATRIX[name], strict=True))

    @property
    def seccomp(self) -> bool:
        return "seccomp" in self.backends.values()

    @property
    def fanotify(self) -> bool:
        return self.backends["file.open"] == "fanotify"

    @property
    def capabilities(self) -> list[str]:
        capabilities = []
        for operation, backend in self.backends.items():
            if operation.startswith("file."):
                if backend == "lsm":
                    capability = "enforcement-file-permission-lsm"
                elif operation == "file.open":
                    capability = "enforcement-file-permission-fanotify"
                else:
                    capability = f"enforcement-file-{operation.split('.')[1]}-seccomp"
            elif operation == "command.exec":
                capability = f"enforcement-command-execution-{backend}"
            else:
                capability = f"enforcement-network-connect-{backend}"
            if capability not in capabilities:
                capabilities.append(capability)
        return capabilities

    def require_resources(self, result: dict) -> None:
        fds = result["daemon_fds"]
        if any("fanotify" in fd for fd in fds) != self.fanotify:
            raise AssertionError(f"{self.name}: fanotify resources differ from configured open backend: {fds}")
        if any("seccomp" in fd for fd in fds) != self.seccomp:
            raise AssertionError(f"{self.name}: seccomp resources differ from configured operations: {fds}")
        for process in result["processes"]:
            if not process["threads"] or any(
                thread["seccomp"] != (2 if self.seccomp else 0) for thread in process["threads"]
            ):
                raise AssertionError(f"{self.name}: tracee seccomp differs from configured operations: {process}")
            if not self.seccomp and process["listener_fds"]:
                raise AssertionError(f"{self.name}: unexpected tracee listener: {process}")

    def audit_matches(self, event: dict, rule_id: str, decision: str) -> bool:
        payload = event.get("payload", {})
        if "network" in rule_id:
            operation = "network.connect"
        elif "command" in rule_id or rule_id == "lsm-root":
            operation = "command.exec"
        elif "mkdir" in rule_id:
            operation = "file.mkdir"
        elif "rmdir" in rule_id:
            operation = "file.rmdir"
        else:
            operation = "file.open"
        backend = self.backends[operation]
        if operation == "network.connect" and backend == "seccomp":
            metadata = payload.get("metadata", {})
            return (event.get("collector") == "network-control"
                    and event.get("variant") == "net"
                    and metadata.get("operation") == "connect"
                    and payload.get("result") == (-errno.EPERM if decision == "deny" else None)
                    and metadata.get("rule_id") == rule_id and metadata.get("decision") == decision)
        expected = {"lsm": "bpf-lsm", "fanotify": "fanotify", "seccomp": "seccomp-user-notify"}[backend]
        return (event.get("variant") == "enforcement" and payload.get("backend") == expected
                and payload.get("rule_id") == rule_id and payload.get("decision") == decision
                and payload.get("result") == ("allowed" if decision == "allow" else "denied"))
