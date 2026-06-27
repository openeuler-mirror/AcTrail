#!/usr/bin/env python3
"""Update changelog/yyyy-mm.md from upstream master commits."""

from __future__ import annotations

import argparse
import datetime as dt
import os
import re
import subprocess
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path


UPSTREAM_REMOTE_URL = "git@gitcode.com:openeuler/AcTrail.git"
UPSTREAM_BRANCH = "master"
UPSTREAM_REF = "refs/remotes/actrail_changelog/master"
CHANGELOG_DIR = "changelog"
STATE_PATH = "changelog/.master-changelog-state"
AUTO_START = "<!-- actrail-master-changelog:start -->"
AUTO_END = "<!-- actrail-master-changelog:end -->"

CATEGORY_TITLES = {
    "added": "新增",
    "fixed": "修复",
    "changed": "变更",
    "performance": "性能",
    "docs": "文档",
    "tests": "测试",
    "build": "构建",
    "chore": "维护",
}

CATEGORY_ORDER = [
    "added",
    "fixed",
    "changed",
    "performance",
    "docs",
    "tests",
    "build",
    "chore",
]


@dataclass(frozen=True)
class Commit:
    sha: str
    short_sha: str
    subject: str
    date: str
    month: str
    category: str


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def run_git(args: list[str], *, check: bool = True) -> str:
    command = ["git", *args]
    result = subprocess.run(
        command,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if check and result.returncode != 0:
        stderr = result.stderr.strip()
        fail(f"{' '.join(command)} failed: {stderr}")
    return result.stdout


def repo_root() -> Path:
    root = run_git(["rev-parse", "--show-toplevel"]).strip()
    if not root:
        fail("not inside a git repository")
    return Path(root)


def ensure_upstream_ref(fetch: bool) -> None:
    if fetch:
        run_git(
            [
                "fetch",
                "--no-tags",
                UPSTREAM_REMOTE_URL,
                f"+refs/heads/{UPSTREAM_BRANCH}:{UPSTREAM_REF}",
            ]
        )
        return

    result = subprocess.run(
        ["git", "rev-parse", "--verify", UPSTREAM_REF],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if result.returncode != 0:
        fail(f"{UPSTREAM_REF} is missing; rerun without --no-fetch")


def read_state(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}

    state: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if "=" not in stripped:
            fail(f"invalid state line in {path}: {line}")
        key, value = stripped.split("=", 1)
        state[key.strip()] = value.strip()
    return state


def write_state(path: Path, last_commit: str, dry_run: bool) -> None:
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()
    content = "\n".join(
        [
            f"source_url={UPSTREAM_REMOTE_URL}",
            f"branch={UPSTREAM_BRANCH}",
            f"source_ref={UPSTREAM_REF}",
            f"last_commit={last_commit}",
            f"updated_at={now}",
            "",
        ]
    )
    if dry_run:
        print(f"--- {path} (dry-run) ---")
        print(content, end="")
        return

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def resolve_target(target: str | None) -> str:
    ref = target or UPSTREAM_REF
    return run_git(["rev-parse", "--verify", ref]).strip()


def validate_commit_exists(commit: str) -> str:
    return run_git(["rev-parse", "--verify", f"{commit}^{{commit}}"]).strip()


def is_ancestor(ancestor: str, descendant: str) -> bool:
    result = subprocess.run(
        ["git", "merge-base", "--is-ancestor", ancestor, descendant],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.returncode == 0


def commit_range(state: dict[str, str], since: str | None, target: str) -> list[str]:
    if since:
        since_sha = validate_commit_exists(since)
        if not is_ancestor(since_sha, target):
            fail(f"--since commit {since_sha} is not an ancestor of target {target}")
        revspec = f"{since_sha}..{target}"
        return run_git(["rev-list", "--first-parent", "--reverse", revspec]).splitlines()

    last_commit = state.get("last_commit", "")
    if last_commit:
        last_sha = validate_commit_exists(last_commit)
        if not is_ancestor(last_sha, target):
            fail(
                f"state last_commit {last_sha} is not an ancestor of upstream "
                f"master {target}; inspect {STATE_PATH}"
            )
        return run_git(
            ["rev-list", "--first-parent", "--reverse", f"{last_sha}..{target}"]
        ).splitlines()

    return run_git(["rev-list", "--first-parent", "--reverse", target]).splitlines()


def first_parent_history(target: str) -> list[str]:
    return run_git(["rev-list", "--first-parent", "--reverse", target]).splitlines()


def commit_subject(sha: str) -> str:
    subject = run_git(["show", "-s", "--format=%s", sha]).strip()
    subject = re.sub(r"\s+", " ", subject)
    return subject or "(no subject)"


def commit_date(sha: str) -> str:
    return run_git(["show", "-s", "--format=%cs", sha]).strip()


def classify(subject: str) -> str:
    text = subject.lower()
    normalized = re.sub(r"^[!#]\d+\s+", "", text).strip()

    if re.match(r"^(feat|feature)(\(.+\))?!?:", normalized) or normalized.startswith(
        ("add ", "added ", "新增")
    ):
        return "added"
    if re.match(r"^fix(\(.+\))?!?:", normalized) or normalized.startswith(
        ("fix ", "fixed ", "修复")
    ):
        return "fixed"
    if re.match(r"^perf(\(.+\))?!?:", normalized) or "performance" in normalized:
        return "performance"
    if re.match(r"^docs?(\(.+\))?!?:", normalized) or normalized.startswith(
        ("doc ", "docs ", "readme", "文档")
    ):
        return "docs"
    if re.match(r"^test(s)?(\(.+\))?!?:", normalized) or normalized.startswith(
        ("test ", "tests ", "e2e ")
    ):
        return "tests"
    if re.match(r"^(build|ci|release)(\(.+\))?!?:", normalized) or normalized.startswith(
        ("build ", "ci ", "package ")
    ):
        return "build"
    if re.match(r"^refactor(\(.+\))?!?:", normalized) or normalized.startswith(
        (
            "update ",
            "change ",
            "convert ",
            "rework ",
            "integrate ",
            "refactor ",
            "improve ",
            "support ",
            "prepare ",
        )
    ):
        return "changed"
    return "chore"


def load_commits(shas: list[str]) -> list[Commit]:
    commits: list[Commit] = []
    for sha in shas:
        subject = commit_subject(sha)
        date = commit_date(sha)
        commits.append(
            Commit(
                sha=sha,
                short_sha=sha[:7],
                subject=subject,
                date=date,
                month=date[:7],
                category=classify(subject),
            )
        )
    return commits


def strip_existing_auto_block(content: str) -> tuple[str, str]:
    start = content.find(AUTO_START)
    end = content.find(AUTO_END)
    if start == -1 and end == -1:
        return content.rstrip(), ""
    if start == -1 or end == -1 or end < start:
        fail("invalid changelog auto block markers")
    before = content[:start].rstrip()
    after = content[end + len(AUTO_END) :].strip()
    return before, after


def render_month(month: str, commits: list[Commit], existing: str) -> str:
    before, after = strip_existing_auto_block(existing)
    if not before:
        before = f"# {month} 变更记录"

    by_day: dict[str, list[Commit]] = defaultdict(list)
    for commit in commits:
        by_day[commit.date].append(commit)

    lines: list[str] = [before, "", AUTO_START]
    for day in sorted(by_day, reverse=True):
        lines.extend(["", f"## {day}"])
        day_commits = by_day[day]
        for category in CATEGORY_ORDER:
            entries = [c for c in day_commits if c.category == category]
            if not entries:
                continue
            lines.extend(["", f"### {CATEGORY_TITLES[category]}"])
            for entry in entries:
                lines.append(f"- {entry.subject} (`{entry.short_sha}`)")
    lines.extend(["", AUTO_END])
    if after:
        lines.extend(["", after])
    return "\n".join(lines).rstrip() + "\n"


def update_changelog_files(root: Path, commits: list[Commit], dry_run: bool) -> None:
    by_month: dict[str, list[Commit]] = defaultdict(list)
    for commit in commits:
        by_month[commit.month].append(commit)

    changelog_dir = root / CHANGELOG_DIR
    for month in sorted(by_month):
        path = changelog_dir / f"{month}.md"
        existing = path.read_text(encoding="utf-8") if path.exists() else ""
        content = render_month(month, by_month[month], existing)
        if dry_run:
            print(f"--- {path} (dry-run) ---")
            print(content, end="")
            continue
        changelog_dir.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Fetch gitcode.com/openEuler/AcTrail.git master and update "
            "changelog/yyyy-mm.md from commits not yet recorded in state."
        )
    )
    parser.add_argument(
        "--since",
        help=(
            "Start after this commit instead of changelog state. The commit itself "
            "is excluded."
        ),
    )
    parser.add_argument(
        "--to",
        help=(
            f"Target commit/ref. Defaults to {UPSTREAM_REF} after fetching upstream master."
        ),
    )
    parser.add_argument(
        "--no-fetch",
        action="store_true",
        help=f"Do not fetch upstream master; use existing {UPSTREAM_REF}.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print generated files and state without writing them.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = repo_root()
    os.chdir(root)

    ensure_upstream_ref(fetch=not args.no_fetch)
    state_file = root / STATE_PATH
    state = read_state(state_file)
    target = resolve_target(args.to)
    new_shas = commit_range(state, args.since, target)
    if not new_shas:
        print(f"changelog is already up to date for upstream {UPSTREAM_BRANCH}")
        return 0

    shas = new_shas if args.since else first_parent_history(target)
    commits = load_commits(shas)
    update_changelog_files(root, commits, dry_run=args.dry_run)
    write_state(state_file, last_commit=target, dry_run=args.dry_run)
    print(
        f"processed {len(new_shas)} new commit(s) from upstream "
        f"{UPSTREAM_BRANCH} through {target[:12]}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
