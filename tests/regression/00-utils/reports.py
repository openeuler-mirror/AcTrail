"""Regression report writers."""

from __future__ import annotations

import json
from datetime import datetime
from pathlib import Path

from model import CaseResult, check_status_counts, check_status_summary


def write_reports(output_dir: Path, results: list[CaseResult]) -> tuple[Path, Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    markdown = output_dir / "report.md"
    machine = output_dir / "report.json"
    generated_at = datetime.now().isoformat(timespec="seconds")
    markdown.write_text(markdown_report(results, generated_at), encoding="utf-8")
    machine.write_text(json.dumps(json_report(results, generated_at), ensure_ascii=False, indent=2), encoding="utf-8")
    return markdown, machine


def markdown_report(results: list[CaseResult], generated_at: str) -> str:
    lines = [
        "# AcTrail Regression Report",
        "",
        f"Generated: {generated_at}",
        "",
        "| Case | Status | Check Summary | Duration | Checks |",
        "| --- | --- | --- | ---: | ---: |",
    ]
    for result in results:
        lines.append(
            f"| `{result.case_id}` | {result.status} | {check_status_summary(result)} | "
            f"{result.duration_seconds:.2f}s | {len(result.checks)} |"
        )
    for result in results:
        lines.extend(case_markdown(result))
    lines.append("")
    return "\n".join(lines)


def case_markdown(result: CaseResult) -> list[str]:
    lines = [
        "",
        f"## {result.case_id}: {result.title}",
        "",
        f"- Status: `{result.status}`",
        f"- Check Summary: `{check_status_summary(result)}`",
        f"- Duration: `{result.duration_seconds:.2f}s`",
    ]
    if result.command:
        lines.append(f"- Command: `{' '.join(result.command)}`")
    if result.report_paths:
        lines.append("- Artifacts: " + ", ".join(f"`{path}`" for path in result.report_paths))
    if result.checks:
        lines.extend(["", "Checks:"])
        for check in result.checks:
            detail = f": {check.detail}" if check.detail else ""
            lines.append(f"- `{check.status}` {check.name}{detail}")
            if check.evidence:
                lines.append(f"  - Reason: {check.evidence}")
    if result.stdout_tail:
        lines.extend(["", "Stdout tail:", "", "```text", result.stdout_tail.rstrip(), "```"])
    if result.stderr_tail:
        lines.extend(["", "Stderr tail:", "", "```text", result.stderr_tail.rstrip(), "```"])
    return lines


def json_report(results: list[CaseResult], generated_at: str) -> dict:
    return {
        "generated_at": generated_at,
        "cases": [
            {
                "id": result.case_id,
                "title": result.title,
                "status": result.status,
                "check_summary": check_status_summary(result),
                "check_counts": check_status_counts(result),
                "duration_seconds": result.duration_seconds,
                "command": result.command,
                "checks": [
                    {
                        "name": check.name,
                        "status": check.status,
                        "detail": check.detail,
                        "evidence": check.evidence,
                    }
                    for check in result.checks
                ],
                "stdout_tail": result.stdout_tail,
                "stderr_tail": result.stderr_tail,
                "report_paths": result.report_paths,
            }
            for result in results
        ],
    }
