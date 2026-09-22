"""Real xiaoo bash-tool HTTPS requests through a newly mapped OpenSSL library."""
from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import threading
import time
import tomllib
from pathlib import Path

from scripts.bench.overall.runtime.config_patch import ConfigPatch
from scripts.bench.payload.agent import AgentWorkload
from scripts.bench.payload.benchmark import ROOT
from scripts.bench.payload.measurement import CommandMeasurement, DaemonCpu
from scripts.bench.payload.nested_acceptance import OuterAgent
from scripts.bench.payload.profile_acceptance import ProfileAcceptance
from scripts.bench.payload.runtime import CollectionRuntime
from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput


class LibraryResponseServer(AgentWorkload):
    def _write_scenario(self, directory):
        super()._write_scenario(directory)
        name = self._fixture['name']
        sequence = directory / f'{name}.seq.json'
        document = json.loads(sequence.read_text())
        for index, generator in enumerate(document['generators']):
            generator['response']['blocks'] = [dict(type='message', text=f'LIBRARY_HTTPS_RESPONSE_{index + 1}')]
        sequence.write_text(json.dumps(document))
        metadata = directory / f'{name}.meta.json'
        document = json.loads(metadata.read_text())
        document.update(tool_rounds=0, message_rounds=self.turns, tools=[])
        metadata.write_text(json.dumps(document))


class LibraryObserver:
    def __init__(self, executable, library):
        self.executable, self.library = executable.resolve(), library.resolve()
        self.rows = {}
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)

    def run(self):
        while not self.stop_event.is_set():
            for entry in Path('/proc').iterdir():
                if not entry.name.isdigit():
                    continue
                try:
                    if (entry / 'exe').resolve(strict=True) != self.executable:
                        continue
                    ticks = (entry / 'stat').read_text().rsplit(')', 1)[1].split()[19]
                    maps = (entry / 'maps').read_text().splitlines()
                    now_mono, now_wall = time.monotonic_ns(), time.time_ns()
                    key = (int(entry.name), ticks)
                    row = self.rows.setdefault(key, dict(pid=key[0], start_ticks=ticks, library_maps=[], runtime_maps=[],
                        first_observed_monotonic_ns=now_mono, first_observed_time_ns=now_wall,
                        library_mapping_observations=[]))
                    row.update(last_observed_monotonic_ns=now_mono, last_observed_time_ns=now_wall)
                    library_maps = [line for line in maps if str(self.library) in line]
                    row['library_maps'] = sorted(set(row['library_maps'] + library_maps))
                    if library_maps:
                        row.setdefault('first_library_monotonic_ns', now_mono)
                        row.setdefault('first_library_time_ns', now_wall)
                        row.update(last_library_monotonic_ns=now_mono, last_library_time_ns=now_wall)
                        row['library_mapping_observations'].append(dict(monotonic_ns=now_mono, time_ns=now_wall,
                                                                       maps=library_maps))
                    row['runtime_maps'] = sorted(set(row['runtime_maps'] + [line for line in maps if 'libactrail_tls_payload_probe_sync' in line]))
                except (OSError, ProcessLookupError):
                    continue
            self.stop_event.wait(0.01)

    def finish(self):
        self.stop_event.set()
        self.thread.join(timeout=5)
        return list(self.rows.values())


class LibraryAcceptance:
    def __init__(self, args, files):
        self.args, self.files = args, files
        self.out = args.out.resolve() / ('files-on' if files else 'files-off')
        self.out.mkdir()
        self.bins = args.bin_dir.resolve()
        self.work = self.out / 'runtime-P'
        self.work.mkdir()
        self.config = self.work / 'actraild.conf'
        patch = self.work / 'actraild.patch.toml'
        ActrailRuntime.write_isolated_operator_config_patch(patch, self.work)
        profile = ConfigPatch(Path(__file__).parents[1] / 'configs/P.toml')
        overlay = tomllib.loads((Path(__file__).parents[1] / 'configs/tls-bpf-copy.toml').read_text())
        profile.values['payload']['tls'].update(overlay['payload']['tls'])
        if not files:
            profile.values['capture']['capabilities'] = [c for c in profile.values['capture']['capabilities']
                                                        if c not in ('fs-access-basic', 'fs-mmap')]
        profile.apply_isolation(patch)
        self.runtime = ActrailRuntime(ROOT, self.bins, 60, TestOutput(), self.config, patch)
        self.collection = CollectionRuntime(self.work, self.bins, patch,
            dict(agent_turns=5, drain_timeout_seconds=30, poll_seconds=0.01))
        self.report = dict(status='running', scope='functional only', file_capture=files)

    def viewer(self, kind, trace):
        result = subprocess.run([str(self.bins / 'actrailviewer'), '--config', str(self.config),
            '--output-format', 'json', kind, '--trace-id', str(trace)],
            capture_output=True, text=True, check=True, timeout=30)
        (self.out / f'{kind}-P-{trace}.json').write_text(result.stdout)
        return json.loads(result.stdout)

    def run(self):
        observer = server = outer = None
        try:
            library_dir = self.out / 'library'
            library_dir.mkdir()
            library = library_dir / 'libssl.so.3'
            shutil.copy2(self.args.libssl.resolve(), library)
            self.report['library'] = dict(path=str(library), source=str(self.args.libssl.resolve()),
                                          inode=library.stat().st_ino, device=library.stat().st_dev)
            self.report['hashes'] = {}
            for path in [self.bins / name for name in ('actraild', 'actrailctl', 'actrailviewer')] + [self.args.agent_bin.resolve(), self.args.curl.resolve(), library]:
                with path.open('rb') as source:
                    self.report['hashes'][str(path)] = hashlib.file_digest(source, 'sha256').hexdigest()
            self.runtime.prepare()
            pid = int((self.work / 'run/actraild.pid').read_text())
            if (Path('/proc') / str(pid) / 'exe').resolve() != self.bins / 'actraild':
                raise RuntimeError('unexpected actual daemon binary')
            actual = tomllib.loads(self.config.read_text())
            if actual['payload']['tls']['capture_backend'] != 'bpf-copy' or actual['seccomp_notify']['enabled']:
                raise RuntimeError('unexpected TLS configuration')
            (self.out / 'configs').mkdir()
            shutil.copy2(self.config, self.out / 'configs/P.resolved.toml')
            self.collection.cpu = DaemonCpu(pid)
            log_path = self.work / 'log/actraild.log'
            self.collection.log = log_path.open()
            # The server returns genuine SSE responses. TPOT permits observing
            # each short curl mapping without adding a target-process sleep.
            server = LibraryResponseServer(ROOT, self.out / 'maas-client', self.args.agent_bin,
                turns=2, input_bytes=128, tpot_ms=30, timeout_seconds=60)
            server.start()
            server.reset()
            server_args = server.command(self.out)
            url = server_args[server_args.index('--api-base') + 1]
            request = self.out / 'request.json'
            request.write_text(json.dumps(dict(model='deepseek-v4-flash', stream=True,
                messages=[dict(role='user', content='Return the configured response.')])) + '\n')
            for index in (1, 2):
                command = [str(self.args.curl.resolve()), '--http1.1', '--silent', '--show-error', '--fail-with-body',
                    '--cacert', server.env['SSL_CERT_FILE'], '-H', 'Content-Type: application/json',
                    '--data-binary', '@' + str(request), url]
                (self.out / f'client-{index}-argv.json').write_text(json.dumps(command, indent=2))
                (self.out / f'client-{index}.sh').write_text('#!/bin/bash\nset -euo pipefail\n'
                    + 'echo $$ > ' + shlex.quote(str(self.out / f'client-{index}.pid')) + '\n'
                    + 'export LD_LIBRARY_PATH=' + shlex.quote(str(library_dir)) + '\n'
                    + 'exec ' + shlex.join(command) + ' > ' + shlex.quote(str(self.out / f'client-{index}.sse'))
                    + ' 2> ' + shlex.quote(str(self.out / f'client-{index}.stderr')) + '\n')
            outer = OuterAgent(ROOT, self.out / 'maas-outer', self.args.agent_bin,
                turns=3, input_bytes=128, tpot_ms=0, timeout_seconds=60,
                tool_script=self.out / 'client-{index}.sh')
            outer.start()
            outer_dir = self.out / 'outer'
            outer.prepare(outer_dir)
            outer.reset()
            observer = LibraryObserver(self.args.curl, library)
            observer.thread.start()
            previous = self.collection.mark()
            command = self.collection.launch(outer.command(outer_dir))
            self.report['outer_command'] = command
            CommandMeasurement(60).run(command, outer_dir, dict(os.environ, **outer.env, ACTRAIL_LAUNCH_TIMING='1'))
            self.report['outer_workload'] = outer.validate(outer_dir, outer_dir / 'stdout.log')
            self.report['finalization'] = self.collection.drain(previous)
            records = [json.loads(line) for line in (self.out / 'maas-client/maas.log').read_text().splitlines() if line.startswith('{')]
            requests = [record for record in records if record.get('event') == 'local_maas_request']
            if len(requests) != 2 or any(record['status'] != 200 for record in requests):
                raise RuntimeError('curl server did not complete exactly two real requests')
            self.report['client_requests'] = requests
            traces = self.collection.query('SELECT trace_id,lifecycle_state,health FROM traces WHERE trace_id>?', (previous,))
            if len(traces) != 1 or traces[0][2] != 'clean':
                raise RuntimeError(f'unexpected trace state: {traces}')
            trace = traces[0][0]
            graph = self.viewer('actions', trace)
            events = self.viewer('events', trace)['events']
            counts = collections.Counter(action['kind'] for action in graph['actions'])
            self.report['action_counts'] = dict(counts)
            self.report['file_event_count'] = sum(event['kind'] == 'File' for event in events)
            if not self.files and (self.report['file_event_count'] or any(kind.startswith(('file.', 'fs.')) for kind in counts)):
                raise RuntimeError('disabled file capture produced file events/actions')
            maps = observer.finish()
            self.report['observed_curl_maps'] = maps
            self.report['clients'] = []
            by_id = {action['action_id']: action for action in graph['actions']}
            root_process = self.collection.query('SELECT root_process_id FROM traces WHERE trace_id=?', (trace,))[0][0]
            outer_counts = collections.Counter(a['kind'] for a in graph['actions'] if a['process']['process_id'] == root_process)
            if any(outer_counts[kind] != 3 for kind in ('llm.request', 'llm.response', 'llm.call')):
                raise RuntimeError('outer real agent has incomplete LLM coverage')
            for action in graph['actions']:
                if action['kind'] in ('llm.request', 'llm.response'):
                    if (action['status'] != 'success' or action['completeness'] != 'complete'
                        or not action['start_time_unix_nanos'] or not action['end_time_unix_nanos']):
                        raise RuntimeError('captured LLM evidence is incomplete')
                if action['kind'] == 'llm.response' and action['attributes'].get('llm.response.done') != 'true':
                    raise RuntimeError('response lacks protocol completion')
            for link in graph['links']:
                if link['role'] in ('llm.call.request', 'llm.call.response', 'command.contains_llm_call'):
                    if not link['valid'] or by_id[link['parent_action_id']]['process'] != by_id[link['child_action_id']]['process']:
                        raise RuntimeError('invalid or cross-process LLM relationship')
            for index in (1, 2):
                if '[DONE]' not in (self.out / f'client-{index}.sse').read_text():
                    raise RuntimeError('curl did not receive complete SSE response')
                client_pid = int((self.out / f'client-{index}.pid').read_text())
                identities = self.collection.query('SELECT process_id,host_start_ticks,host_start_boottime_ns FROM processes WHERE host_pid=?', (client_pid,))
                if len(identities) != 1:
                    raise RuntimeError('curl process identity is not unique')
                process_id, ticks, boot_ns = identities[0]
                ticks = ticks or boot_ns * os.sysconf('SC_CLK_TCK') // 1_000_000_000
                observed = [row for row in maps if row['pid'] == client_pid and row['start_ticks'] == str(ticks)]
                if len(observed) != 1 or not observed[0]['library_maps'] or observed[0]['runtime_maps']:
                    raise RuntimeError('curl did not demonstrably map the fixture library without a runtime')
                own = [a for a in graph['actions'] if a['process']['process_id'] == process_id]
                own_counts = collections.Counter(a['kind'] for a in own)
                complete = all(own_counts[kind] == 1 for kind in ('llm.request', 'llm.response', 'llm.call'))
                self.report['clients'].append(dict(index=index, pid=client_pid, process_id=process_id,
                    maps=observed, action_counts=dict(own_counts), full_coverage=complete))
                if index == 2 and not complete:
                    raise RuntimeError('repeated mapped library request lacks complete coverage')
                for action in own:
                    if action['kind'] in ('llm.request', 'llm.response') and (action['status'] != 'success' or action['completeness'] != 'complete'):
                        raise RuntimeError('captured curl action lacks successful complete evidence')
                links = [link for link in graph['links'] if link['role'] == 'command.contains_llm_call'
                         and by_id[link['child_action_id']]['process']['process_id'] == process_id]
                if complete and (len(links) != 1 or not links[0]['valid']
                    or by_id[links[0]['parent_action_id']]['process']['process_id'] != process_id):
                    raise RuntimeError('curl command attribution mismatch')
            inode = library.stat().st_ino
            self.report['library_diagnostics'] = [line for line in log_path.read_text().splitlines() if re.search(rf'\binode={inode}\b', line)]
            if all(counts[kind] == 5 for kind in ('llm.request', 'llm.response', 'llm.call')):
                self.report['profile'] = ProfileAcceptance(self.out).trace(dict(mode='P', collection=dict(traces=traces)), 5)
            self.report['status'] = 'passed' if self.report['clients'][0]['full_coverage'] else 'first_discovery_coverage_gap'
        except BaseException as error:
            self.report.update(status='failed', error=f'{type(error).__name__}: {error}')
            raise
        finally:
            if observer and observer.thread.ident:
                observer.finish()
            if outer:
                outer.stop()
            if server:
                server.stop()
            if self.collection.log:
                self.collection.log.close()
            try:
                result = self.runtime.stop()
                if result is not None and result.returncode:
                    raise RuntimeError('own isolated daemon failed to stop')
            finally:
                (self.out / 'acceptance.json').write_text(json.dumps(self.report, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--bin-dir', type=Path, default=ROOT / 'target/release')
    parser.add_argument('--agent-bin', type=Path, default=Path('/home/yzh/.cargo/bin/xiaoo'))
    parser.add_argument('--curl', type=Path, default=Path('/usr/bin/curl'))
    parser.add_argument('--libssl', type=Path, default=Path('/usr/lib64/libssl.so.3'))
    args = parser.parse_args()
    args.out.resolve().mkdir(parents=True, exist_ok=False)
    for files in (True, False):
        LibraryAcceptance(args, files).run()
