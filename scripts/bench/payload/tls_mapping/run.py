"""Real-agent mprotect, replacement and unlinked OpenSSL mapping acceptance."""
from __future__ import annotations

import argparse
import collections
import json
import os
import re
import shlex
import shutil
import socket
import subprocess
import threading
import time
from pathlib import Path
from urllib.parse import urlsplit

from scripts.bench.overall.runtime.config_patch import ConfigPatch
from scripts.bench.payload.benchmark import ROOT
from scripts.bench.payload.measurement import CommandMeasurement, DaemonCpu
from scripts.bench.payload.nested_acceptance import OuterAgent
from scripts.bench.payload.profile_acceptance import ProfileAcceptance
from scripts.bench.payload.tls_library.run import LibraryAcceptance, LibraryResponseServer


class MappingCoordinator:
    def __init__(self, library, source, log):
        self.library, self.source, self.log = library, source, log
        self.rows, self.error = [], None
        self.stop_event = threading.Event()
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(1)
        self.listener.settimeout(90)
        self.port = self.listener.getsockname()[1]
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.replace()

    def replace(self):
        staging = self.library.with_suffix('.replacement')
        shutil.copy2(self.source, staging)
        os.replace(staging, self.library)

    @staticmethod
    def snapshot(pid, inode):
        process = Path('/proc') / str(pid)
        stat = (process / 'stat').read_text().rsplit(')', 1)[1].split()
        maps = (process / 'maps').read_text().splitlines()
        return dict(pid=pid, start_ticks=stat[19], monotonic_ns=time.monotonic_ns(),
                    library_maps=[line for line in maps if line.split(None, 5)[4] == str(inode)],
                    runtime_maps=[line for line in maps if 'libactrail_tls_payload_probe_sync' in line])

    def ready_lines(self, inode):
        return [line for line in self.log.read_text().splitlines()
                if 'TLS direct object attachment ready' in line and 'SharedLibrary' in line
                and re.search(rf'\binode={inode}\b', line)]

    def run(self):
        try:
            for index, mode in enumerate(('mprotect', 'replacement', 'unlink'), 1):
                with self.listener.accept()[0] as connection:
                    connection.settimeout(40)
                    with connection.makefile('rwb', buffering=0) as stream:
                        readonly = json.loads(stream.readline())
                        if readonly['phase'] != 'readonly' or readonly['mode'] != mode:
                            raise RuntimeError('unexpected mapping phase')
                        pid, inode = readonly['pid'], readonly['inode']
                        if inode != self.library.stat().st_ino:
                            raise RuntimeError('client did not map current library inode')
                        if inode in {row['inode'] for row in self.rows}:
                            raise RuntimeError('library replacement reused an earlier inode')
                        before = self.snapshot(pid, inode)
                        if not before['library_maps'] or any('x' in line.split()[1] for line in before['library_maps']):
                            raise RuntimeError('initial library mapping is not demonstrably non-executable')
                        if before['runtime_maps'] or self.ready_lines(inode):
                            raise RuntimeError('fixture library was already attached or TLS runtime was injected')
                        row = dict(index=index, mode=mode, pid=pid, inode=inode, readonly=readonly,
                                   before_mprotect=before)
                        self.rows.append(row)
                        stream.write(b'X')
                        executable = json.loads(stream.readline())
                        if executable != dict(phase='executable', pid=pid, inode=inode):
                            raise RuntimeError('mprotect completion missing')
                        deadline = time.monotonic() + 30
                        while not (ready := self.ready_lines(inode)):
                            if self.stop_event.wait(0.01) or time.monotonic() >= deadline:
                                raise RuntimeError('SharedLibrary attachment did not become ready')
                        row['after_mprotect'] = self.snapshot(pid, inode)
                        if row['after_mprotect']['start_ticks'] != before['start_ticks']:
                            raise RuntimeError('client process identity changed')
                        if not any('x' in line.split()[1] for line in row['after_mprotect']['library_maps']):
                            raise RuntimeError('executable mapping missing after successful mprotect')
                        if mode == 'unlink' and (self.library.exists() or
                            not all('(deleted)' in line for line in row['after_mprotect']['library_maps'])):
                            raise RuntimeError('unlinked mapping was not demonstrated')
                        row['attachment_ready'] = ready
                        row['dlopen_permitted_monotonic_ns'] = time.monotonic_ns()
                        stream.write(b'D')
                        if json.loads(stream.readline()) != dict(phase='done', pid=pid):
                            raise RuntimeError('client did not finish its HTTPS exchange')
                        if index < 3:
                            self.replace()
                        stream.write(b'A')
        except BaseException as error:
            self.error = error
        finally:
            self.listener.close()

    def stop(self):
        self.stop_event.set()
        self.listener.close()
        if self.thread.ident:
            self.thread.join(timeout=45)


class MappingAcceptance(LibraryAcceptance):
    def __init__(self, args):
        super().__init__(args, False)
        patch = self.work / 'actraild.patch.toml'
        values = ConfigPatch(patch).values
        values.setdefault('control', {})['diagnostic_log_level'] = 'debug'
        patch.write_text('\n'.join(f'{json.dumps(key)} = {ConfigPatch._render(value)}'
                                   for key, value in values.items()) + '\n')
        self.report.update(scope='functional mapping boundaries; explicit fixture coordination',
                           todos=['pin followed by exit before attachment is not deterministically exercised'])

    def run(self):
        server = outer = coordinator = None
        try:
            executable = self.out / 'client'
            subprocess.run(['cc', '-O2', '-Wall', '-Wextra', str(Path(__file__).with_name('client.c')),
                            '-o', str(executable), '-ldl'], check=True, timeout=30)
            linkage = subprocess.run(['ldd', str(executable)], capture_output=True, text=True, check=True)
            (self.out / 'client.ldd').write_text(linkage.stdout)
            if 'libssl' in linkage.stdout or 'libcrypto' in linkage.stdout:
                raise RuntimeError('client unexpectedly links OpenSSL at startup')
            self.runtime.prepare()
            daemon_pid = int((self.work / 'run/actraild.pid').read_text())
            if Path(f'/proc/{daemon_pid}/exe').resolve() != self.bins / 'actraild':
                raise RuntimeError('unexpected daemon binary')
            self.collection.cpu = DaemonCpu(daemon_pid)
            log = self.work / 'log/actraild.log'
            self.collection.log = log.open()
            (self.out / 'configs').mkdir()
            shutil.copy2(self.config, self.out / 'configs/P.resolved.toml')
            library = self.out / 'libssl.so.3'
            coordinator = MappingCoordinator(library, self.args.libssl.resolve(), log)
            coordinator.thread.start()
            server = LibraryResponseServer(ROOT, self.out / 'maas-client', self.args.agent_bin,
                turns=3, input_bytes=128, tpot_ms=3, timeout_seconds=90)
            server.start()
            server.reset()
            arguments = server.command(self.out)
            url = urlsplit(arguments[arguments.index('--api-base') + 1])
            for index, mode in enumerate(('mprotect', 'replacement', 'unlink'), 1):
                command = [str(executable), str(library), mode, str(coordinator.port), str(url.port), url.path]
                (self.out / f'client-{index}.sh').write_text('#!/bin/bash\nset -euo pipefail\nexec '
                    + shlex.join(command) + ' > ' + shlex.quote(str(self.out / f'client-{index}.http'))
                    + ' 2> ' + shlex.quote(str(self.out / f'client-{index}.stderr')) + '\n')
            outer = OuterAgent(ROOT, self.out / 'maas-outer', self.args.agent_bin,
                turns=4, input_bytes=128, tpot_ms=0, timeout_seconds=120,
                tool_script=self.out / 'client-{index}.sh')
            outer.start()
            directory = self.out / 'outer'
            outer.prepare(directory)
            outer.reset()
            previous = self.collection.mark()
            command = self.collection.launch(outer.command(directory))
            self.report['outer_command'] = command
            CommandMeasurement(150).run(command, directory, dict(os.environ, **outer.env))
            self.report['outer_workload'] = outer.validate(directory, directory / 'stdout.log')
            coordinator.thread.join(timeout=5)
            if coordinator.error or coordinator.thread.is_alive() or len(coordinator.rows) != 3:
                raise RuntimeError(f'mapping coordinator incomplete: {coordinator.error}')
            self.report['finalization'] = self.collection.drain(previous)
            records = [json.loads(line) for line in (self.out / 'maas-client/maas.log').read_text().splitlines()
                       if line.startswith('{')]
            requests = [record for record in records if record.get('event') == 'local_maas_request']
            if len(requests) != 3 or any(record['status'] != 200 for record in requests):
                raise RuntimeError('server did not complete three genuine HTTPS requests')
            self.report['client_requests'] = requests
            traces = self.collection.query('SELECT trace_id,lifecycle_state,health FROM traces WHERE trace_id>?', (previous,))
            if len(traces) != 1 or traces[0][2] != 'clean':
                raise RuntimeError(f'unclean trace: {traces}')
            trace = traces[0][0]
            self.report['profile'] = ProfileAcceptance(self.out).trace(dict(mode='P', collection=dict(traces=traces)), 7)
            graph = self.viewer('actions', trace)
            events = self.viewer('events', trace)['events']
            if any(event['kind'] == 'File' for event in events):
                raise RuntimeError('disabled file capture produced events')
            by_id = {action['action_id']: action for action in graph['actions']}
            for row in coordinator.rows:
                response = (self.out / f"client-{row['index']}.http").read_text()
                if '[DONE]' not in response or not response.startswith('HTTP/1.1 200'):
                    raise RuntimeError('client lacks complete successful HTTP/SSE response')
                identities = self.collection.query('SELECT process_id,host_start_ticks,host_start_boottime_ns '
                    'FROM processes WHERE host_pid=?', (row['pid'],))
                if len(identities) != 1:
                    raise RuntimeError('client process identity is ambiguous')
                process_id, ticks, boot = identities[0]
                ticks = ticks or boot * os.sysconf('SC_CLK_TCK') // 1_000_000_000
                if str(ticks) != row['before_mprotect']['start_ticks']:
                    raise RuntimeError('stored client generation mismatch')
                own = [action for action in graph['actions'] if action['process']['process_id'] == process_id]
                counts = collections.Counter(action['kind'] for action in own)
                if any(counts[kind] != 1 for kind in ('llm.request', 'llm.response', 'llm.call')):
                    raise RuntimeError('mapping client lacks one complete LLM exchange')
                links = [link for link in graph['links'] if link['role'] == 'command.contains_llm_call'
                         and by_id[link['child_action_id']]['process']['process_id'] == process_id]
                if len(links) != 1 or not links[0]['valid'] or by_id[links[0]['parent_action_id']]['process']['process_id'] != process_id:
                    raise RuntimeError('mapping client command attribution mismatch')
                row.update(process_id=process_id, action_counts=dict(counts))
            self.report['status'] = 'passed'
        except BaseException as error:
            self.report.update(status='failed', error=f'{type(error).__name__}: {error}')
            raise
        finally:
            if coordinator:
                coordinator.stop()
                self.report['cases'] = coordinator.rows
                if coordinator.error:
                    self.report['coordination_error'] = str(coordinator.error)
            if outer:
                outer.stop()
            if server:
                server.stop()
            if self.collection.log:
                self.collection.log.close()
            try:
                result = self.runtime.stop()
                if result is not None and result.returncode:
                    raise RuntimeError('isolated daemon failed to stop')
            finally:
                (self.out / 'acceptance.json').write_text(json.dumps(self.report, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--bin-dir', type=Path, default=ROOT / 'target/release')
    parser.add_argument('--agent-bin', type=Path, default=Path('/home/yzh/.cargo/bin/xiaoo'))
    parser.add_argument('--libssl', type=Path, default=Path('/usr/lib64/libssl.so.3'))
    args = parser.parse_args()
    args.out.resolve().mkdir(parents=True, exist_ok=False)
    MappingAcceptance(args).run()
