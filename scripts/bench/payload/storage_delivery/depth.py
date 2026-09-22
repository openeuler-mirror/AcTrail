"""Read observation depth only from the daemon owned by this acceptance run."""

import json
import os
import subprocess
import sys
from pathlib import Path


class ObservationDepthProbe:
    def __init__(self, daemon_pid: int, binary: Path, agent_config: Path):
        self.daemon_pid = daemon_pid
        self.binary = binary.resolve()
        self.agent_config = os.fsencode(agent_config)
        owned = set()
        for fdinfo in Path(f"/proc/{daemon_pid}/fdinfo").iterdir():
            try:
                for line in fdinfo.read_text().splitlines():
                    if line.startswith("map_id:"):
                        owned.add(int(line.split()[1]))
            except FileNotFoundError:
                continue
        maps = self.command("map", "show")
        selected = [row for row in maps if row["id"] in owned
                    and row["name"] == "process_observation_depths"[:15]
                    and row["bytes_key"] == 4 and row["bytes_value"] == 16]
        if len(selected) != 1:
            raise RuntimeError(f"expected one owned depth map: {selected}")
        self.map_id = selected[0]["id"]

    @staticmethod
    def command(*args):
        result = subprocess.run(["bpftool", "-j", *args], capture_output=True,
                                text=True, timeout=5, check=True)
        return json.loads(result.stdout)

    def sample(self):
        for process in Path("/proc").iterdir():
            if not process.name.isdigit():
                continue
            try:
                if (process / "exe").resolve(strict=True) != self.binary:
                    continue
                args = (process / "cmdline").read_bytes().split(b"\0")
                if not any(key == b"--config" and value == self.agent_config
                           for key, value in zip(args, args[1:])):
                    continue
                before = (process / "stat").read_text().rsplit(")", 1)[1].split()[19]
                pid = int(process.name)
                key = pid.to_bytes(4, sys.byteorder)
                rows = self.command("map", "dump", "id", str(self.map_id))
                for row in rows:
                    if bytes(int(byte, 16) for byte in row["key"]) != key:
                        continue
                    value = bytes(int(byte, 16) for byte in row["value"])
                    depth = int.from_bytes(value[8:12], sys.byteorder, signed=True)
                    generation = int.from_bytes(value[:8], sys.byteorder)
                    after = (process / "stat").read_text().rsplit(")", 1)[1].split()[19]
                    # Collector kernel_start_time uses boot ns when available,
                    # otherwise the root launch observation's /proc clock ticks.
                    ticks = generation * os.sysconf("SC_CLK_TCK") // 1_000_000_000
                    if before != after or (ticks != int(before) and generation != int(before)):
                        raise RuntimeError("depth map does not match live agent generation")
                    if depth == 0:
                        return dict(daemon_pid=self.daemon_pid, map_id=self.map_id,
                                    agent_pid=pid, start_ticks=before,
                                    generation=generation,
                                    generation_encoding=("proc_ticks" if generation == int(before)
                                                         else "boot_ns"), depth=depth)
            except (FileNotFoundError, ProcessLookupError):
                continue
        return None
