"""Run fixed-work fork/exec scaling with matched TLS profiles and one release."""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import tomllib
from pathlib import Path

from scripts.bench.overall.runtime.config_patch import ConfigPatch
from scripts.bench.payload.benchmark import ROOT
from scripts.bench.payload.prepare_direct_configs import DirectBenchmarkConfig
from scripts.bench.payload.tls_scaling.summary import ScalingSummary


class ScalingRunner:
    def __init__(self, args):
        self.args = args
        self.out, self.bins = args.out.resolve(), args.bin_dir.resolve()
        self.manifest = dict(status='running', jobs=[], scope='native process startup; no business TLS traffic')

    @staticmethod
    def digest(path):
        with path.open('rb') as stream:
            return hashlib.file_digest(stream, 'sha256').hexdigest()

    def binary_hashes(self):
        files = [self.bins / name for name in ('actraild', 'actrailctl', 'actrailviewer')]
        files += sorted(self.bins.glob('*.so'))
        return {str(path): self.digest(path) for path in files}

    def save(self):
        (self.out / 'manifest.json').write_text(json.dumps(self.manifest, indent=2) + '\n')

    def prepare(self):
        self.out.mkdir(parents=True, exist_ok=False)
        source = Path(__file__).parents[1] / 'configs'
        sync, direct = self.out / 'configs-sync', self.out / 'configs-direct'
        DirectBenchmarkConfig(source, sync).write()
        DirectBenchmarkConfig(source, direct).write()
        sync_profile = ConfigPatch(sync / 'P.toml')
        sync_profile.values['payload']['tls']['capture_backend'] = 'tls-sync'
        (sync / 'P.toml').write_text('\n'.join(f'{key} = {ConfigPatch._render(value)}'
                                             for key, value in sync_profile.values.items()) + '\n')
        before = tomllib.loads((sync / 'P.toml').read_text())
        after = tomllib.loads((direct / 'P.toml').read_text())
        overlay = tomllib.loads((source / 'tls-bpf-copy.toml').read_text())
        expected = tomllib.loads((sync / 'P.toml').read_text())
        expected['payload']['tls']['capture_backend'] = 'bpf-copy'
        if before['payload']['tls']['capture_backend'] != 'tls-sync' or after != expected:
            raise RuntimeError('matched P profiles differ outside capture_backend')
        self.manifest.update(binary_hashes=self.binary_hashes(), tls_overlay=overlay,
            source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            provenance='same release files checked before and after all jobs; hashes do not establish source equivalence',
            driver_source_hash=self.digest(Path(__file__).parents[1] / 'workload.c'))

    def run(self):
        self.prepare()
        try:
            for count, backends in ((1000, ('sync', 'direct')), (2000, ('direct', 'sync')), (3000, ('sync', 'direct'))):
                for backend in backends:
                    directory = self.out / f'{backend[0]}{count}'
                    command = [sys.executable, str(Path(__file__).parents[1]), '--skip-build',
                        '--bin-dir', str(self.bins), '--config-dir', str(self.out / f'configs-{backend}'),
                        '--modes', *(['0', 'P'] if backend == 'sync' else ['P']),
                        '--workloads', 'fork', 'exec', '--fork-operations', str(count), '--exec-operations', str(count),
                        '--task-iterations', '1', '--warmups', '1', '--rounds', '3',
                        '--timeout-seconds', str(self.args.timeout_seconds), '--keep-runtime', '--out', str(directory)]
                    job = dict(backend=backend, count=count, directory=directory.name, command=command, status='running')
                    self.manifest['jobs'].append(job)
                    self.save()
                    print(f'[{backend} N={count}] {directory}', flush=True)
                    with (self.out / f'{directory.name}.log').open('w') as log:
                        subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                       check=True, timeout=self.args.job_timeout_seconds)
                    job.update(status='passed', driver_sha256=self.digest(directory / 'workload'))
                    if self.binary_hashes() != self.manifest['binary_hashes']:
                        raise RuntimeError('release files changed during the experiment')
                    self.save()
            if len({job['driver_sha256'] for job in self.manifest['jobs']}) != 1:
                raise RuntimeError('driver binaries differ between jobs')
            self.manifest['status'] = 'passed'
            self.save()
            ScalingSummary(self.out).run()
        except BaseException as error:
            self.manifest.update(status='failed', error=f'{type(error).__name__}: {error}')
            self.save()
            raise


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--bin-dir', type=Path, default=ROOT / 'target/release')
    parser.add_argument('--timeout-seconds', type=float, default=90)
    parser.add_argument('--job-timeout-seconds', type=float, default=1800)
    args = parser.parse_args()
    if args.timeout_seconds <= 0 or args.job_timeout_seconds <= 0:
        parser.error('timeouts must be positive')
    ScalingRunner(args).run()
