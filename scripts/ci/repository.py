#!/usr/bin/env python3
"""为 CI 检查提供 Git 工作区文件清单和统一诊断。"""

from pathlib import Path
import subprocess


class Repository:
    def __init__(self):
        self.root = Path(__file__).resolve().parents[2]
        self.errors = 0

    def files(self, suffix):
        # 读取已跟踪及未忽略的新文件，避免扫描 target 等构建产物。
        result = subprocess.run(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
            cwd=self.root, check=True, stdout=subprocess.PIPE,
        )
        for name in sorted(set(result.stdout.decode().split("\0"))):
            path = self.root / name
            if name.endswith(suffix) and path.is_file():
                yield path

    def report(self, path, line, message):
        self.errors += 1
        print(f"`{path.relative_to(self.root)}`:{line}: {message}")

    def finish(self, label, count):
        print(f"{'FAIL' if self.errors else 'PASS'} {label}: {count} files, {self.errors} errors")
        return int(bool(self.errors))
