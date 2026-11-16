import shlex
import signal
from dataclasses import dataclass, fields


@dataclass(frozen=True)
class OomWorkloadDefaults:
    start_poll_seconds: float = 0.01
    allocation_bytes: int = 1024 * 1024
    touch_stride_bytes: int = 4096


DEFAULTS = OomWorkloadDefaults()


@dataclass(frozen=True)
class CgroupOomDefaults:
    memory_limit_bytes: int = 33554432
    trigger_poll_attempts: int = 300
    vmstat_poll_attempts: int = 50
    poll_seconds: float = 0.1
    sigkill_exit_status: int = 128 + signal.SIGKILL

    def shell_defaults(self) -> str:
        lines = []
        for definition in fields(self):
            name = f"ACTRAIL_OOM_{definition.name.upper()}"
            value = shlex.quote(str(getattr(self, definition.name)))
            lines.append(f'if [ -z "${{{name}:-}}" ]; then {name}={value}; fi')
        return "\n".join(lines)


CGROUP_DEFAULTS = CgroupOomDefaults()


if __name__ == "__main__":
    print(CGROUP_DEFAULTS.shell_defaults())
