"""Verify real startup scaling evidence and report absolute CPU and adjacent slopes."""
from __future__ import annotations

import argparse
import collections
import hashlib
import json
import sqlite3
import statistics
import tomllib
from pathlib import Path


class ScalingSummary:
    METRICS = ('task_cpu_ms', 'daemon_cpu_ms', 'total_cpu_ms', 'wall_ms')
    MODES = ('bare', 'tls-sync', 'bpf-copy')
    SIZES = (1000, 2000, 3000)

    def __init__(self, directory):
        self.out = directory.resolve()
        self.groups = collections.defaultdict(list)
        self.validations = []
        self.profiles = {}

    @staticmethod
    def normalized_config(value, directory):
        if isinstance(value, dict):
            return {key: ScalingSummary.normalized_config(item, directory) for key, item in value.items()}
        if isinstance(value, list):
            return [ScalingSummary.normalized_config(item, directory) for item in value]
        return value.replace(str(directory), '@JOB') if isinstance(value, str) else value

    def read_job(self, job):
        directory = self.out / job['directory']
        report = json.loads((directory / 'results.json').read_text())
        if job['status'] != 'passed' or report['status'] != 'passed' or report.get('diagnostic_perf'):
            raise RuntimeError('unsuccessful or profiled job')
        with (directory / 'workload').open('rb') as stream:
            if hashlib.file_digest(stream, 'sha256').hexdigest() != job['driver_sha256']:
                raise RuntimeError('driver hash changed')
        settings = report['settings']
        count = job['count']
        if (settings['warmups'], settings['rounds'], settings['task_iterations'],
            settings['fork_operations'], settings['exec_operations']) != (1, 3, 1, count, count):
            raise RuntimeError('fixed workload parameters mismatch')
        modes = ['0', 'P'] if job['backend'] == 'sync' else ['P']
        if report['modes'] != modes or report['workloads'] != ['fork', 'exec']:
            raise RuntimeError('unexpected workload/mode matrix')
        expected_samples = {(mode, kind, phase, round_) for mode in modes for kind in ('fork', 'exec')
                            for phase, rounds in (('warmup', (1,)), ('measured', (1, 2, 3))) for round_ in rounds}
        actual_samples = [(s['mode'], s['workload'], s['phase'], s['round']) for s in report['samples']]
        if set(actual_samples) != expected_samples or len(actual_samples) != len(expected_samples):
            raise RuntimeError('missing, duplicated or unexpected sample')
        profile = tomllib.loads((directory / 'configs/P.resolved.toml').read_text())
        backend = profile['payload']['tls']['capture_backend']
        if backend != ('tls-sync' if job['backend'] == 'sync' else 'bpf-copy'):
            raise RuntimeError('effective TLS backend mismatch')
        if profile['payload']['tls']['max_segment_bytes'] != 65535 or profile['payload']['tls']['max_operation_bytes'] != 65535:
            raise RuntimeError('both backends require the same explicit 65535 byte limits')
        normalized = self.normalized_config(profile, directory)
        normalized['payload']['tls']['capture_backend'] = '@BACKEND'
        self.profiles[count, job['backend']] = normalized
        with sqlite3.connect(f"file:{directory / 'runtime-P/data/actrail.sqlite'}?mode=ro", uri=True) as db:
            for sample in report['samples']:
                if sample['status'] != 'passed':
                    raise RuntimeError('failed sample')
                kind, mode, phase, round_ = sample['workload'], sample['mode'], sample['phase'], sample['round']
                stdout = directory / f'{mode}-{kind}-{phase}-{round_}/stdout.log'
                records = [json.loads(line) for line in stdout.read_text().splitlines() if line.startswith('{"kind":')]
                if len(records) != 1 or records[0]['kind'] != kind or records[0]['operations'] != count:
                    raise RuntimeError('driver did not complete the expected successful child count')
                if mode == 'P':
                    traces = sample['collection']['traces']
                    if len(traces) != 1:
                        raise RuntimeError('expected exactly one trace per observed sample')
                    trace = traces[0][0]
                    state = db.execute('SELECT lifecycle_state,health FROM traces WHERE trace_id=?', (trace,)).fetchone()
                    if state not in (('completed', 'clean'), ('exited', 'clean')):
                        raise RuntimeError(f'trace not clean and finalized: {state}')
                    events = db.execute('SELECT COUNT(*) FROM events WHERE trace_id=? AND kind_code=0', (trace,)).fetchone()[0]
                    if events < count:
                        raise RuntimeError('insufficient process event coverage')
                    self.validations.append(dict(directory=job['directory'], trace_id=trace,
                        workload=kind, phase=phase, actual_children=count, process_events=events))
                if phase == 'measured':
                    self.groups[(kind, 'bare' if mode == '0' else backend, count)].append(sample)

    @staticmethod
    def difference(before, after):
        return dict(before=before, after=after, delta=after - before,
                    percent_of_before=(after - before) / before * 100 if before else None)

    def run(self):
        manifest = json.loads((self.out / 'manifest.json').read_text())
        jobs = manifest['jobs']
        if manifest['status'] != 'passed' or {(j['backend'], j['count']) for j in jobs} != {
            (backend, count) for backend in ('sync', 'direct') for count in self.SIZES} or len(jobs) != 6:
            raise RuntimeError('six successful fixed-size jobs are required')
        if len({job['driver_sha256'] for job in jobs}) != 1:
            raise RuntimeError('drivers differ')
        for job in jobs:
            self.read_job(job)
        for count in self.SIZES:
            if self.profiles[count, 'sync'] != self.profiles[count, 'direct']:
                raise RuntimeError(f'effective profiles differ beyond isolation paths and backend for N={count}')
        expected = {(kind, mode, count) for kind in ('fork', 'exec') for mode in self.MODES for count in self.SIZES}
        if set(self.groups) != expected or any(len(samples) != 3 for samples in self.groups.values()):
            raise RuntimeError('each group requires three measured samples')
        rows, index = [], {}
        for key, samples in sorted(self.groups.items()):
            row = dict(workload=key[0], mode=key[1], children=key[2])
            for metric in self.METRICS:
                values = [sample[metric] for sample in samples]
                row[metric] = dict(mean=statistics.mean(values), minimum=min(values), maximum=max(values))
            rows.append(row)
            index[key] = row
        comparisons, slopes = [], []
        for kind in ('fork', 'exec'):
            for count in self.SIZES:
                for before, after in (('bare', 'tls-sync'), ('bare', 'bpf-copy'), ('tls-sync', 'bpf-copy')):
                    comparisons.append(dict(workload=kind, children=count, before_mode=before, after_mode=after,
                        metrics={metric: self.difference(index[kind, before, count][metric]['mean'],
                            index[kind, after, count][metric]['mean']) for metric in self.METRICS}))
            for low, high in ((1000, 2000), (2000, 3000)):
                for mode, baseline in [('bare', None), ('tls-sync', None), ('bpf-copy', None),
                                       ('tls-sync', 'bare'), ('bpf-copy', 'bare'), ('bpf-copy', 'tls-sync')]:
                    values = {}
                    for metric in self.METRICS:
                        delta = index[kind, mode, high][metric]['mean'] - index[kind, mode, low][metric]['mean']
                        if baseline:
                            delta -= index[kind, baseline, high][metric]['mean'] - index[kind, baseline, low][metric]['mean']
                        values[metric] = delta / (high - low)
                    slopes.append(dict(workload=kind, mode=mode, subtract=baseline, low=low, high=high,
                                       ms_per_additional_child=values))
        result = dict(status='passed', measured_samples=54, warmups=18, rows=rows,
                      comparisons=comparisons, adjacent_slopes=slopes, database_validation=self.validations)
        (self.out / 'scaling-summary.json').write_text(json.dumps(result, indent=2) + '\n')
        self.markdown(result)

    def markdown(self, result):
        lines = ['# TLS process startup scaling', '',
            'Native dynamic ELF driver; sequential fork or fork+exec of the same driver. Exec parent and child each perform one integer iteration. No business TLS traffic. Each mean has three measurements after one warmup.', '',
            'CPU and wall units are ms. Task is wait4 user+system including reaped descendants; observed task includes ctl launch. Daemon includes trace finalization, excludes daemon startup/shutdown. Event counts are coverage checks, not exact successful exec counts; driver completion establishes N successful children.', '',
            '| Workload | Backend | N | Task mean [min,max] | Daemon mean [min,max] | Total mean [min,max] | Wall mean [min,max] |',
            '|---|---|---:|---:|---:|---:|---:|']
        for row in result['rows']:
            values = [f"{row[m]['mean']:.2f} [{row[m]['minimum']:.2f},{row[m]['maximum']:.2f}]" for m in self.METRICS]
            lines.append(f"| {row['workload']} | {row['mode']} | {row['children']} | " + ' | '.join(values) + ' |')
        lines += ['', 'Changes use the stated before backend as denominator; a zero baseline has no percentage.', '',
                  '| Workload | N | Before → after | Task Δ (%) | Daemon Δ (%) | Total Δ (%) |', '|---|---:|---|---:|---:|---:|']
        for row in result['comparisons']:
            values = []
            for metric in self.METRICS[:3]:
                value = row['metrics'][metric]
                percent = 'n/a' if value['percent_of_before'] is None else f"{value['percent_of_before']:+.2f}%"
                values.append(f"{value['delta']:+.2f} ({percent})")
            lines.append(f"| {row['workload']} | {row['children']} | {row['before_mode']} → {row['after_mode']} | " + ' | '.join(values) + ' |')
        lines += ['', 'Adjacent growth is (high-size value − low-size value) / additional children, in ms/child. Subtracted rows apply the same difference to both backends before dividing.', '',
                  '| Workload | Quantity | N interval | Task | Daemon | Total | Wall |', '|---|---|---|---:|---:|---:|---:|']
        for row in result['adjacent_slopes']:
            label = row['mode'] + (f" minus {row['subtract']}" if row['subtract'] else '')
            values = [f"{row['ms_per_additional_child'][metric]:+.6f}" for metric in self.METRICS]
            lines.append(f"| {row['workload']} | {label} | {row['low']}→{row['high']} | " + ' | '.join(values) + ' |')
        lines += ['', 'All 18 warmups and 54 measured samples require actual driver completion; all 48 observed traces require clean finalization and process-event coverage. Concurrent daemon identities remain in each raw report. No historical sample is mixed into this comparison.', '']
        (self.out / 'scaling-summary.md').write_text('\n'.join(lines))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    ScalingSummary(parser.parse_args().out).run()
