"""Real-agent HTTPS library scaling with resident daemons and fresh target inodes."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import tomllib
from pathlib import Path

from scripts.bench.overall.runtime.config_patch import ConfigPatch
from scripts.bench.payload.benchmark import ROOT
from scripts.bench.payload.measurement import CommandMeasurement, DaemonCpu
from scripts.bench.payload.nested_acceptance import OuterAgent
from scripts.bench.payload.runtime import CollectionRuntime
from scripts.bench.payload.tls_library.run import LibraryResponseServer
from scripts.bench.payload.tls_library_scaling.evidence import SampleEvidence
from scripts.bench.payload.tls_library_scaling.summary import LibraryScalingSummary
from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput


class LibraryScaling:
    def __init__(self, args):
        self.args, self.out = args, args.out.resolve()
        self.bins = args.bin_dir.resolve()
        self.identities = set()
        self.report = dict(status='running', groups=[], settings=dict(sizes=[10, 20, 30], warmups=1,
            rounds=3, outer_turns=2, outer_input_bytes=128, client_tpot_ms=0,
            library_patterns=['same', 'fresh'], backends=['bare', 'tls-sync', 'bpf-copy']))

    @staticmethod
    def digest(path):
        with path.open('rb') as stream:
            return hashlib.file_digest(stream, 'sha256').hexdigest()

    def save(self):
        (self.out / 'results.json').write_text(json.dumps(self.report, indent=2) + '\n')

    def hashes(self):
        paths = [self.bins / name for name in ('actraild', 'actrailctl', 'actrailviewer')]
        paths += sorted(self.bins.glob('*.so'))
        paths += [self.args.agent_bin.resolve(), self.args.curl.resolve(), self.args.libssl.resolve()]
        return {str(path): self.digest(path) for path in paths}

    def run(self):
        self.out.mkdir(parents=True, exist_ok=False)
        self.report.update(hashes=self.hashes(), concurrent_daemons_at_start=CollectionRuntime.running_daemons(),
            source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            measurement=dict(task='wait4 user+system including reaped descendants; observed includes ctl',
                daemon='proc stat from launch through trace finalization, excluding startup/shutdown',
                excluded='library/script preparation, MaaS/controller CPU, validation, daemon startup/shutdown',
                coverage='first-discovery misses remain in CPU results and are reported separately'))
        try:
            for count, backends in ((10, ('bare', 'tls-sync', 'bpf-copy')),
                                    (20, ('bpf-copy', 'bare', 'tls-sync')),
                                    (30, ('tls-sync', 'bpf-copy', 'bare'))):
                for pattern in ('same', 'fresh'):
                    for backend in backends:
                        group = LibraryGroup(self, count, pattern, backend)
                        self.report['groups'].append(group.report)
                        group.run()
                        if self.hashes() != self.report['hashes']:
                            raise RuntimeError('release or workload binaries changed during experiment')
                        self.save()
            self.report['status'] = 'passed'
            self.report['concurrent_daemons_at_end'] = CollectionRuntime.running_daemons()
            self.save()
        except BaseException as error:
            self.report.update(status='failed', error=f'{type(error).__name__}: {error}')
            self.report['concurrent_daemons_at_end'] = CollectionRuntime.running_daemons()
            self.save()
            raise
        LibraryScalingSummary(self.out).run()


class LibraryGroup:
    def __init__(self, owner, count, pattern, backend):
        self.owner, self.args = owner, owner.args
        self.count, self.pattern, self.backend = count, pattern, backend
        tag = {'bare': 'b', 'tls-sync': 's', 'bpf-copy': 'd'}[backend]
        self.name = f'{pattern[0]}-{tag}{count}'
        self.out = owner.out / self.name
        self.out.mkdir()
        self.runtime = self.collection = None
        self.report = dict(name=self.name, count=count, pattern=pattern, backend=backend, status='running', samples=[])

    def prepare_runtime(self):
        if self.backend == 'bare':
            return
        work = self.out / 'runtime-P'
        work.mkdir()
        patch = work / 'actraild.patch.toml'
        ActrailRuntime.write_isolated_operator_config_patch(patch, work)
        profile = ConfigPatch(Path(__file__).parents[1] / 'configs/P.toml')
        overlay = tomllib.loads((Path(__file__).parents[1] / 'configs/tls-bpf-copy.toml').read_text())
        profile.values['payload']['tls'].update(overlay['payload']['tls'])
        profile.values['payload']['tls']['capture_backend'] = self.backend
        profile.apply_isolation(patch)
        self.runtime = ActrailRuntime(ROOT, self.owner.bins, 60, TestOutput(), work / 'actraild.conf', patch)
        self.runtime.prepare()
        pid = int((work / 'run/actraild.pid').read_text())
        if (Path('/proc') / str(pid) / 'exe').resolve() != self.owner.bins / 'actraild':
            raise RuntimeError('actual isolated daemon executable mismatch')
        self.collection = CollectionRuntime(work, self.owner.bins, patch,
            dict(agent_turns=self.count + 2, drain_timeout_seconds=30, poll_seconds=0.01))
        self.collection.cpu = DaemonCpu(pid)
        self.collection.log = (work / 'log/actraild.log').open()
        self.report['daemon_identity'] = dict(pid=pid, start_ticks=self.collection.cpu.start_time)
        actual = tomllib.loads((work / 'actraild.conf').read_text())
        if (actual['payload']['tls']['capture_backend'] != self.backend
            or actual['payload']['tls']['max_segment_bytes'] != 65535
            or actual['payload']['tls']['max_operation_bytes'] != 65535):
            raise RuntimeError('effective TLS configuration mismatch')

    def prepare_libraries(self, directory):
        libraries = []
        for index in range(1, (1 if self.pattern == 'same' else self.count) + 1):
            target = directory / 'libs' / str(index) / 'libssl.so.3'
            target.parent.mkdir(parents=True)
            shutil.copy2(self.args.libssl.resolve(), target)
            stat = target.stat()
            key = stat.st_dev, stat.st_ino
            if key in self.owner.identities:
                raise RuntimeError('library inode reused across prepared target instances')
            self.owner.identities.add(key)
            digest = self.owner.digest(target)
            if digest != self.owner.report['hashes'][str(self.args.libssl.resolve())]:
                raise RuntimeError('copied library contents differ from system OpenSSL')
            libraries.append(dict(path=str(target), device=stat.st_dev, inode=stat.st_ino,
                                  bytes=stat.st_size, sha256=digest))
        return libraries

    def sample(self, phase, round_):
        directory = self.out / f'{phase[0]}{round_}'
        directory.mkdir()
        sample = dict(phase=phase, round=round_, directory=directory.name, status='running')
        self.report['samples'].append(sample)
        sample['libraries'] = self.prepare_libraries(directory)
        server = LibraryResponseServer(ROOT, directory / 'maas-client', self.args.agent_bin,
            turns=self.count, input_bytes=128, tpot_ms=0, timeout_seconds=self.args.timeout_seconds)
        outer = None
        try:
            server.start()
            server.reset()
            server_command = server.command(directory)
            url = server_command[server_command.index('--api-base') + 1]
            request = directory / 'request.json'
            request.write_text(json.dumps(dict(model='deepseek-v4-flash', stream=True,
                messages=[dict(role='user', content='Return the configured response.')])) + '\n')
            loop = directory / 'clients.sh'
            lines = ['#!/bin/bash', 'set -euo pipefail']
            clients = []
            for index in range(1, self.count + 1):
                library = sample['libraries'][0 if self.pattern == 'same' else index - 1]
                script = directory / f'client-{index}.sh'
                command = [str(self.args.curl.resolve()), '--http1.1', '--silent', '--show-error', '--fail-with-body',
                    '--cacert', server.env['SSL_CERT_FILE'], '-H', 'Content-Type: application/json',
                    '--data-binary', '@' + str(request), url]
                script.write_text('#!/bin/bash\nset -euo pipefail\n'
                    + 'echo $$ > ' + shlex.quote(str(directory / f'client-{index}.pid')) + '\n'
                    + 'export LD_LIBRARY_PATH=' + shlex.quote(str(Path(library['path']).parent)) + '\n'
                    + 'exec ' + shlex.join(command) + ' > ' + shlex.quote(str(directory / f'client-{index}.sse'))
                    + ' 2> ' + shlex.quote(str(directory / f'client-{index}.stderr')) + '\n')
                lines.append('bash ' + shlex.quote(str(script)))
                clients.append(dict(index=index, command=command, library=library['path']))
            loop.write_text('\n'.join(lines) + '\n')
            sample['clients'] = clients
            outer = OuterAgent(ROOT, directory / 'maas-outer', self.args.agent_bin,
                turns=2, input_bytes=128, tpot_ms=0, timeout_seconds=self.args.timeout_seconds, tool_script=loop)
            outer.start()
            work = directory / 'outer'
            outer.prepare(work)
            outer.reset()
            command = outer.command(work)
            if self.collection:
                command = self.collection.launch(command)
            sample['command'] = command
            self.owner.save()
            previous = self.collection.mark() if self.collection else 0
            log_offset = (self.out / 'runtime-P/log/actraild.log').stat().st_size if self.collection else 0
            cpu_start = self.collection.cpu.read_ms() if self.collection else 0
            sample.update(CommandMeasurement(self.args.timeout_seconds).run(command, work, dict(os.environ, **outer.env)))
            cpu_exit = self.collection.cpu.read_ms() if self.collection else 0
            sample['daemon_run_cpu_ms'] = cpu_exit - cpu_start
            sample['drain_ms'] = 0
            if self.collection:
                sample['finalization'] = self.collection.drain(previous)
                sample['drain_ms'] = sample['finalization']['drain_ms']
            cpu_end = self.collection.cpu.read_ms() if self.collection else 0
            sample['daemon_drain_cpu_ms'] = cpu_end - cpu_exit
            sample['daemon_cpu_ms'] = cpu_end - cpu_start
            sample['total_cpu_ms'] = sample['task_cpu_ms'] + sample['daemon_cpu_ms']
            # Business, database, and diagnostics checks occur after CPU accounting.
            sample['outer_workload'] = outer.validate(work, work / 'stdout.log')
            sample['business'] = SampleEvidence(self.out, directory, self.owner.bins, self.count).business(clients)
            if self.collection:
                evidence = SampleEvidence(self.out, directory, self.owner.bins, self.count)
                sample['coverage'] = evidence.observed(self.collection, previous, clients)
                with (self.out / 'runtime-P/log/actraild.log').open() as stream:
                    stream.seek(log_offset)
                    log = stream.read()
                (directory / 'daemon-window.log').write_text(log)
                sample['discovery'] = evidence.discovery(log, sample['libraries'])
            sample['status'] = 'passed'
            print(f'  {self.name}/{phase}{round_}: task={sample["task_cpu_ms"]:.2f} daemon={sample["daemon_cpu_ms"]:.2f} wall={sample["wall_ms"]:.2f} ms', flush=True)
        except BaseException as error:
            sample.update(status='failed', error=f'{type(error).__name__}: {error}')
            raise
        finally:
            if outer:
                outer.stop()
            server.stop()
            self.owner.save()

    def run(self):
        print(f'[{self.name}] N={self.count} {self.pattern} {self.backend}', flush=True)
        try:
            self.prepare_runtime()
            self.sample('warmup', 1)
            for round_ in (1, 2, 3):
                self.sample('measured', round_)
            self.report['status'] = 'passed'
        finally:
            if self.collection:
                self.collection.log.close()
                # CollectionRuntime verifies PID/starttime and stops only its own daemon.
                self.collection.stop()
            elif self.runtime:
                self.runtime.stop()
            self.owner.save()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--bin-dir', type=Path, default=ROOT / 'target/release')
    parser.add_argument('--agent-bin', type=Path, default=Path('/home/yzh/.cargo/bin/xiaoo'))
    parser.add_argument('--curl', type=Path, default=Path('/usr/bin/curl'))
    parser.add_argument('--libssl', type=Path, default=Path('/usr/lib64/libssl.so.3'))
    parser.add_argument('--timeout-seconds', type=float, default=90)
    args = parser.parse_args()
    if args.timeout_seconds <= 0:
        parser.error('timeout must be positive')
    LibraryScaling(args).run()
