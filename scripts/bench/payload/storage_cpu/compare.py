"""Compare fixed-work storage CPU measurements using each run's bare baseline."""

import argparse
import json
import statistics
from pathlib import Path


class StorageComparison:
    def __init__(self, paths):
        self.paths = paths
        self.runs = {label: json.loads((path / "results.json").read_text())
                     for label, path in paths.items()}

    def write(self, output):
        reference = self.runs["sqlite"]
        reference_samples = [s for s in reference["samples"] if s["phase"] == "measured"]
        if not reference_samples:
            raise ValueError("sqlite: no measured workload")
        reference_workload = reference_samples[0]["workload_result"]
        rows = []
        for label, run in self.runs.items():
            if run["storage_backend"] != label:
                raise ValueError(f"{label}: storage backend does not match input label")
            if run["binary_commit"] != reference["binary_commit"]:
                raise ValueError(f"{label}: binary commits differ")
            if run["status"] != "passed" or run.get("diagnostic_perf"):
                raise ValueError(f"{label}: incomplete or profiled run")
            if run["settings"] != reference["settings"]:
                raise ValueError(f"{label}: workload settings differ")
            if run["agent_kind"] != reference["agent_kind"]:
                raise ValueError(f"{label}: agent differs")
            samples = [s for s in run["samples"] if s["phase"] == "measured"]
            for mode in ("0", "P", "C"):
                selected = [s for s in samples if s["mode"] == mode]
                if len(selected) != 3 or any(s["status"] != "passed" for s in selected):
                    raise ValueError(f"{label}/{mode}: requires three successful samples")
                if any(s["workload_result"] != reference_workload for s in selected):
                    raise ValueError(f"{label}/{mode}: actual workload differs")
                row = {"run": label, "mode": mode}
                for key in ("task_cpu_ms", "daemon_cpu_ms", "total_cpu_ms", "wall_ms"):
                    row[key] = statistics.mean(s[key] for s in selected)
                    row[f"{key}_range"] = [min(s[key] for s in selected), max(s[key] for s in selected)]
                bare = statistics.mean(s["task_cpu_ms"] for s in samples if s["mode"] == "0")
                if bare <= 0:
                    raise ValueError(f"{label}: bare CPU must be positive")
                row["bare_cpu_ms"] = bare
                row["task_extra_cpu_ms"] = row["task_cpu_ms"] - bare
                row["task_extra_pct_of_bare"] = row["task_extra_cpu_ms"] / bare * 100
                row["daemon_pct_of_bare"] = row["daemon_cpu_ms"] / bare * 100
                row["extra_cpu_ms"] = row["total_cpu_ms"] - bare
                row["extra_pct_of_bare"] = row["extra_cpu_ms"] / bare * 100
                rows.append(row)
        output.mkdir(parents=True, exist_ok=True)
        document = {"inputs": {k: str(v.resolve()) for k, v in self.paths.items()},
                    "settings": reference["settings"], "workload": reference_workload,
                    "means": rows}
        (output / "comparison.json").write_text(json.dumps(document, indent=2) + "\n")
        lines = ["# Storage CPU comparison", "",
                 f"Real {reference['agent_kind']} HTTPS workload; warmups excluded, three measured samples per mode. "
                 "CPU values are milliseconds. Extra CPU = task + daemon − the same run's bare task CPU. "
                 "Daemon measurement ends at trace finalization. MaaS and controller CPU are excluded.", "",
                 "All percentages use the same backend run's bare task CPU as denominator. "
                 "Task increment = task − bare; total extra = task increment + daemon.", "",
                 "| Backend | Mode | Bare CPU ms | Task CPU ms | Daemon CPU ms | Task increment / bare | Daemon / bare | Total extra / bare | Wall ms |",
                 "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
        for row in rows:
            values = [f"{row[k]:.1f}" for k in ("bare_cpu_ms", "task_cpu_ms", "daemon_cpu_ms")]
            lines.append(f"| {row['run']} | {row['mode']} | " + " | ".join(values)
                         + f" | {row['task_extra_pct_of_bare']:+.1f}% | {row['daemon_pct_of_bare']:.1f}%"
                         + f" | {row['extra_pct_of_bare']:.1f}% | {row['wall_ms']:.1f} |")
        lines += ["", "Positive increments mean additional CPU relative to bare. "
                  "Three samples describe this workload and do not establish statistical significance."]
        lines += ["", "Per-sample values, ranges, effective configurations, build records and coverage evidence are retained in the input directories and comparison.json."]
        (output / "comparison.md").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for label in ("sqlite", "noop", "out"):
        parser.add_argument(f"--{label}", type=Path, required=True)
    args = parser.parse_args()
    StorageComparison({label: getattr(args, label) for label in ("sqlite", "noop")}).write(args.out)
