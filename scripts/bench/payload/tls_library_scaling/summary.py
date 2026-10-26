"""Summarize completed real-agent library scaling without hiding capture gaps."""
from __future__ import annotations

import argparse
import json
import sqlite3
import statistics
import tomllib
from pathlib import Path


class LibraryScalingSummary:
    METRICS = ('task_cpu_ms', 'daemon_cpu_ms', 'total_cpu_ms', 'wall_ms')
    BACKENDS = ('bare', 'tls-sync', 'bpf-copy')

    def __init__(self, directory):
        self.out = directory.resolve()

    @classmethod
    def normalize(cls, value, directory):
        if isinstance(value, dict):
            return {key: cls.normalize(item, directory) for key, item in value.items()}
        if isinstance(value, list):
            return [cls.normalize(item, directory) for item in value]
        return value.replace(str(directory), '@GROUP') if isinstance(value, str) else value

    def verify(self, report):
        expected = {(n, pattern, backend) for n in (10, 20, 30) for pattern in ('same', 'fresh') for backend in self.BACKENDS}
        groups = report['groups']
        if report['status'] != 'passed' or len(groups) != 18 or {(g['count'], g['pattern'], g['backend']) for g in groups} != expected:
            raise RuntimeError('all 18 successful groups are required')
        profiles, seen_inodes = {}, set()
        for group in groups:
            if group['status'] != 'passed':
                raise RuntimeError('failed group')
            samples = group['samples']
            if len(samples) != 4 or {(s['phase'], s['round']) for s in samples} != {('warmup', 1), ('measured', 1), ('measured', 2), ('measured', 3)}:
                raise RuntimeError('each group requires one warmup and three measurements')
            directory = self.out / group['name']
            for sample in samples:
                if sample['status'] != 'passed' or sample['business']['curl_https_requests'] != group['count']:
                    raise RuntimeError('business workload failed or count differs')
                if sample['outer_workload']['llm_requests'] != 2 or sample['outer_workload']['verified_tool_outputs'] != 1:
                    raise RuntimeError('outer agent workload differs')
                libs = sample['libraries']
                if len(libs) != (1 if group['pattern'] == 'same' else group['count']):
                    raise RuntimeError('incorrect library object count')
                for lib in libs:
                    key = lib['device'], lib['inode']
                    if key in seen_inodes:
                        raise RuntimeError('target library identity reused across runs')
                    seen_inodes.add(key)
                if group['backend'] == 'bare':
                    if sample['daemon_cpu_ms'] != 0:
                        raise RuntimeError('bare unexpectedly includes a daemon')
                else:
                    with sqlite3.connect(f"file:{directory / 'runtime-P/data/actrail.sqlite'}?mode=ro", uri=True) as db:
                        state = db.execute('SELECT lifecycle_state,health FROM traces WHERE trace_id=?',
                                           (sample['coverage']['trace_id'],)).fetchone()
                        if state not in (('completed', 'clean'), ('exited', 'clean')):
                            raise RuntimeError('observed trace is not finalized and clean')
                    if len(sample['coverage']['clients']) != group['count']:
                        raise RuntimeError('per-client coverage is missing')
            if group['backend'] != 'bare':
                profile = tomllib.loads((directory / 'runtime-P/actraild.conf').read_text())
                tls = profile['payload']['tls']
                if tls['capture_backend'] != group['backend'] or tls['max_segment_bytes'] != 65535 or tls['max_operation_bytes'] != 65535:
                    raise RuntimeError('TLS profile differs from matched backend/limits')
                normalized = self.normalize(profile, directory)
                normalized['payload']['tls']['capture_backend'] = '@BACKEND'
                profiles[group['count'], group['pattern'], group['backend']] = normalized
        for count in (10, 20, 30):
            for pattern in ('same', 'fresh'):
                if profiles[count, pattern, 'tls-sync'] != profiles[count, pattern, 'bpf-copy']:
                    raise RuntimeError('resolved profiles differ beyond backend and isolation')

    def run(self):
        report = json.loads((self.out / 'results.json').read_text())
        self.verify(report)
        rows, index = [], {}
        for group in report['groups']:
            samples = [sample for sample in group['samples'] if sample['phase'] == 'measured']
            row = dict(count=group['count'], pattern=group['pattern'], backend=group['backend'])
            for metric in self.METRICS:
                values = [sample[metric] for sample in samples]
                row[metric] = dict(mean=statistics.mean(values), minimum=min(values), maximum=max(values))
            row['actual_curl_requests'] = group['count'] * 3
            if group['backend'] != 'bare':
                row['observed'] = {key: sum(sample['coverage']['observed'].get(key, 0) for sample in samples)
                                   for key in ('requests', 'responses', 'calls', 'complete_pairs')}
                row['complete_pair_coverage_percent'] = row['observed']['complete_pairs'] / row['actual_curl_requests'] * 100
                row['discovery'] = {key: sum(lib[key] for sample in samples for lib in sample['discovery']['libraries'])
                                    for key in ('analysis_started', 'cache_hits', 'coalesced', 'attachment_ready')}
            rows.append(row)
            index[row['count'], row['pattern'], row['backend']] = row
        comparisons, slopes = [], []
        for pattern in ('same', 'fresh'):
            for count in (10, 20, 30):
                for before, after in (('bare', 'tls-sync'), ('bare', 'bpf-copy'), ('tls-sync', 'bpf-copy')):
                    values = {}
                    for metric in self.METRICS:
                        old, new = index[count, pattern, before][metric]['mean'], index[count, pattern, after][metric]['mean']
                        values[metric] = dict(before=old, after=new, delta=new - old,
                            percent_of_before=(new - old) / old * 100 if old else None)
                    comparisons.append(dict(pattern=pattern, count=count, before=before, after=after, metrics=values))
            for low, high in ((10, 20), (20, 30)):
                for backend, baseline in [('bare', None), ('tls-sync', None), ('bpf-copy', None),
                                          ('tls-sync', 'bare'), ('bpf-copy', 'bare'), ('bpf-copy', 'tls-sync')]:
                    values = {}
                    for metric in self.METRICS:
                        delta = index[high, pattern, backend][metric]['mean'] - index[low, pattern, backend][metric]['mean']
                        if baseline:
                            delta -= index[high, pattern, baseline][metric]['mean'] - index[low, pattern, baseline][metric]['mean']
                        values[metric] = delta / (high - low)
                    slopes.append(dict(pattern=pattern, backend=backend, subtract=baseline,
                                       low=low, high=high, ms_per_additional_curl=values))
        result = dict(status='passed', measured_runs=54, warmups=18, rows=rows,
                      comparisons=comparisons, adjacent_growth=slopes)
        (self.out / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
        self.markdown(result)

    def markdown(self, result):
        lines = ['# Real-agent TLS library scaling', '',
            'Each sample runs real xiaoo for two model requests and one bash tool invocation, which serially executes N curl HTTPS requests. All responses and tool execution must succeed. TPOT is 0; no delay is added before HTTPS.', '',
            'Each observed group has one resident daemon for a warmup and three measured runs. Every run uses fresh target library inodes: same shares one within that run, fresh gives each curl a distinct copy. Common executable/library caches may be warmed. All library copies preserve system OpenSSL bytes and are prepared outside measurement.', '',
            'Values are ms: task uses wait4 user+system including reaped descendants, daemon extends to trace finalization. MaaS/controller CPU, preparation, validation and daemon startup/shutdown are excluded. Percentages use the named before backend as denominator. First-discovery capture gaps remain in these results; unequal coverage does not establish equivalent-capability savings.', '',
            '| Pattern | N | Backend | Task mean [min,max] | Daemon mean [min,max] | Total mean [min,max] | Wall mean [min,max] | Captured curl req/resp/call/pairs versus actual |',
            '|---|---:|---|---:|---:|---:|---:|---|']
        for row in result['rows']:
            values = [f"{row[m]['mean']:.2f} [{row[m]['minimum']:.2f},{row[m]['maximum']:.2f}]" for m in self.METRICS]
            coverage = 'bare; actual ' + str(row['actual_curl_requests'])
            if 'observed' in row:
                coverage = '/'.join(str(row['observed'][k]) for k in ('requests', 'responses', 'calls', 'complete_pairs')) + f" / {row['actual_curl_requests']} ({row['complete_pair_coverage_percent']:.1f}% pairs)"
            lines.append(f"| {row['pattern']} | {row['count']} | {row['backend']} | " + ' | '.join(values) + f' | {coverage} |')
        lines += ['', '| Pattern | N | Before → after | Task Δ (%) | Daemon Δ (%) | Total Δ (%) |', '|---|---:|---|---:|---:|---:|']
        for row in result['comparisons']:
            values = []
            for metric in self.METRICS[:3]:
                v = row['metrics'][metric]
                percent = 'n/a' if v['percent_of_before'] is None else f"{v['percent_of_before']:+.2f}%"
                values.append(f"{v['delta']:+.2f} ({percent})")
            lines.append(f"| {row['pattern']} | {row['count']} | {row['before']} → {row['after']} | " + ' | '.join(values) + ' |')
        lines += ['', 'Adjacent growth is (high − low) / additional curl processes, in ms/curl. Subtracted quantities remove the corresponding baseline growth before division.', '',
                  '| Pattern | Quantity | N interval | Task | Daemon | Total | Wall |', '|---|---|---|---:|---:|---:|---:|']
        for row in result['adjacent_growth']:
            label = row['backend'] + (f" minus {row['subtract']}" if row['subtract'] else '')
            values = [f"{row['ms_per_additional_curl'][metric]:+.6f}" for metric in self.METRICS]
            lines.append(f"| {row['pattern']} | {label} | {row['low']}→{row['high']} | " + ' | '.join(values) + ' |')
        lines += ['', '| Pattern | N | Backend | Analysis started | Cache hit | Coalesced | Attachment ready (includes reuse) |', '|---|---:|---|---:|---:|---:|---:|']
        for row in result['rows']:
            if 'discovery' in row:
                d = row['discovery']
                lines.append(f"| {row['pattern']} | {row['count']} | {row['backend']} | {d['analysis_started']} | {d['cache_hits']} | {d['coalesced']} | {d['attachment_ready']} |")
        lines += ['', 'Raw reports retain all 72 runs, individual client process identities/coverage, library versions, commands, effective configurations and binary hashes. This is a short quantity experiment; wall duration is reported rather than altered to reach a target range. External daemons remain untouched.', '']
        (self.out / 'summary.md').write_text('\n'.join(lines))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    LibraryScalingSummary(parser.parse_args().out).run()
