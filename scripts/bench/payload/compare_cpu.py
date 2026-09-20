"""Compare two complete fixed-work payload benchmark reports."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


class CpuComparison:
    def __init__(self, before, after, output):
        self.before, self.after, self.output = before, after, output

    @staticmethod
    def read(directory):
        report = json.loads((directory / "results.json").read_text())
        if report["status"] != "passed" or report.get("diagnostic_perf"):
            raise ValueError(f"not a successful unsampled benchmark: {directory}")
        for sample in report["samples"]:
            if sample["status"] != "passed":
                raise ValueError("failed workload sample")
        if report["settings"]["warmups"] != 1 or report["settings"]["rounds"] != 3:
            raise ValueError("comparison requires one warmup and three measured rounds")
        return report

    def run(self):
        before, after = self.read(self.before), self.read(self.after)
        if before["settings"] != after["settings"] or before["modes"] != after["modes"]:
            raise ValueError("workload settings or modes differ")
        indexes = [{(r["workload"], r["mode"]): r for r in report["summary"]}
                   for report in (before, after)]
        if indexes[0].keys() != indexes[1].keys():
            raise ValueError("comparison cells differ")
        metrics = ("task_cpu_ms", "daemon_cpu_ms", "total_cpu_ms", "wall_ms")
        rows = []
        for key in sorted(indexes[0]):
            row = dict(workload=key[0], mode=key[1])
            for metric in metrics:
                old, new = indexes[0][key][metric], indexes[1][key][metric]
                row[metric] = dict(before=old, after=new, delta=new-old,
                                   change_pct=(new-old)/old*100 if old else None)
                for label, report in (("before", before), ("after", after)):
                    values = [s[metric] for s in report["samples"]
                              if (s["workload"], s["mode"]) == key and s["phase"] == "measured"]
                    if len(values) != 3:
                        raise ValueError(f"expected three measured samples: {label}/{key}")
                    row[metric][label + "_range"] = [min(values), max(values)]
            for label, index in zip(("before", "after"), indexes):
                baseline = index[(key[0], "0")]["task_cpu_ms"]
                extra = index[key]["total_cpu_ms"] - baseline
                row[label + "_bare"] = dict(task_cpu_ms=baseline, extra_total_cpu_ms=extra,
                                             overhead_pct=extra / baseline * 100)
            rows.append(row)
        self.output.mkdir(parents=True, exist_ok=True)
        result = dict(before=str(self.before), after=str(self.after), settings=before["settings"], rows=rows)
        (self.output / "cpu-comparison.json").write_text(json.dumps(result, indent=2) + "\n")
        lines = ["# TLS backend CPU comparison", "", "CPU units are milliseconds. Each value is the mean of three measured runs after one warmup. Task includes reaped descendants; daemon includes trace finalization. Percent change uses the same-mode old value as denominator. These runs have no perf sampler.", "",
                 "| Workload | Mode | Task before → after | Daemon before → after | Total before → after | Total change | Wall before → after |",
                 "| --- | --- | ---: | ---: | ---: | ---: | ---: |"]
        for row in rows:
            values = {m: f"{row[m]['before']:.1f} → {row[m]['after']:.1f}" for m in metrics}
            total = row["total_cpu_ms"]
            lines.append(f"| {row['workload']} | {row['mode']} | {values['task_cpu_ms']} | {values['daemon_cpu_ms']} | {values['total_cpu_ms']} | {total['delta']:+.1f} ({total['change_pct']:+.1f}%) | {values['wall_ms']} |")
        lines += ["", "Collection overhead relative to each version's own bare run:", "",
                  "| Workload | Mode | Old extra CPU | New extra CPU | Old overhead | New overhead |",
                  "| --- | --- | ---: | ---: | ---: | ---: |"]
        for row in rows:
            if row["mode"] == "0":
                continue
            old, new = row["before_bare"], row["after_bare"]
            lines.append(f"| {row['workload']} | {row['mode']} | {old['extra_total_cpu_ms']:.1f} | {new['extra_total_cpu_ms']:.1f} | {old['overhead_pct']:+.1f}% | {new['overhead_pct']:+.1f}% |")
        lines += ["", "Measured minimum–maximum (milliseconds):", "",
                  "| Workload | Mode | Version | Task range | Daemon range | Total range | Wall range |",
                  "| --- | --- | --- | ---: | ---: | ---: | ---: |"]
        for row in rows:
            for label in ("before", "after"):
                values = " | ".join(f"{row[m][label + '_range'][0]:.1f}–{row[m][label + '_range'][1]:.1f}" for m in metrics)
                lines.append(f"| {row['workload']} | {row['mode']} | {label} | {values} |")
        lines += ["", "The C backend remains tls-sync. Any C change is a control observation, not attributable to switching P to bpf-copy. No long-request or truncated-capture CPU benefit is established by a short-content workload.", ""]
        (self.output / "cpu-comparison.md").write_text("\n".join(lines))
        print(self.output / "cpu-comparison.md")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    CpuComparison(args.before, args.after, args.out).run()
