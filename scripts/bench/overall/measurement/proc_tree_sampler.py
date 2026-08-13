"""Lightweight Linux process-tree resource sampling."""

from __future__ import annotations

import os


_PAGE_KB = os.sysconf("SC_PAGE_SIZE") / 1024
_CLK_TCK = os.sysconf("SC_CLK_TCK")


class ProcTreeSampler:
    def __init__(self, root_pid: int):
        self._root = root_pid

    def sample(self) -> tuple[float, float, int]:
        cpu_seconds = 0.0
        rss_kb = 0.0
        count = 0
        for entry in os.listdir("/proc"):
            if not entry.isdigit():
                continue
            pid = int(entry)
            if not self._is_descendant(pid):
                continue
            stat = self._read_stat(pid)
            if stat is None:
                continue
            cpu_seconds += (stat["utime"] + stat["stime"]) / _CLK_TCK
            rss_kb += stat["rss"] * _PAGE_KB
            count += 1
        return cpu_seconds, rss_kb, count

    def _is_descendant(self, pid: int) -> bool:
        current = pid
        seen: set[int] = set()
        while current not in seen:
            if current == self._root:
                return True
            seen.add(current)
            stat = self._read_stat(current)
            if stat is None:
                return False
            current = stat["ppid"]
        return False

    @staticmethod
    def _read_stat(pid: int) -> dict[str, int] | None:
        try:
            with open(f"/proc/{pid}/stat", "rb") as stat_file:
                raw = stat_file.read()
        except OSError:
            return None
        name_end = raw.rfind(b")")
        if name_end < 0:
            return None
        tail = raw[name_end + 1 :].split()
        if len(tail) < 22:
            return None
        try:
            return {
                "ppid": int(tail[1]),
                "utime": int(tail[11]),
                "stime": int(tail[12]),
                "rss": int(tail[21]),
            }
        except ValueError:
            return None
