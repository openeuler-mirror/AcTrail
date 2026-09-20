"""Real xiaoo tool launches of unknown executable inodes; functional evidence only."""
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
import tomllib
from pathlib import Path

from scripts.bench.overall.runtime.config_patch import ConfigPatch
from scripts.bench.payload.agent import AgentWorkload
from scripts.bench.payload.benchmark import ROOT
from scripts.bench.payload.direct_acceptance import AgentObserver
from scripts.bench.payload.measurement import CommandMeasurement, DaemonCpu
from scripts.bench.payload.profile_acceptance import ProfileAcceptance
from scripts.bench.payload.runtime import CollectionRuntime
from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput


class OuterAgent(AgentWorkload):
    def __init__(self, *args, tool_script, **kwargs):
        super().__init__(*args, **kwargs)
        # The real agent executes this through its existing bash tool.
        self._fixture['tool_command'] = ('bash ' + shlex.quote(str(tool_script))
            + ' && cat input.txt | tee result-{index}.txt')


class NestedAcceptance:
    def __init__(self, args):
        self.args = args
        self.out = args.out.resolve()
        self.bins = args.bin_dir.resolve()
        self.source = args.agent_bin.resolve()
        self.report = dict(status='running', scope='functional only', samples=[], short_exit=[])
        self.work = self.out / 'runtime-P'
        self.work.mkdir(parents=True)
        patch = self.work / 'actraild.patch.toml'
        ActrailRuntime.write_isolated_operator_config_patch(patch, self.work)
        profile = ConfigPatch(Path(__file__).parent / 'configs/P.toml')
        overlay = tomllib.loads((Path(__file__).parent / 'configs/tls-bpf-copy.toml').read_text())
        profile.values['payload']['tls'].update(overlay['payload']['tls'])
        profile.apply_isolation(patch)
        self.runtime = ActrailRuntime(ROOT, self.bins, 60, TestOutput(), self.work / 'actraild.conf', patch)
        self.collection = CollectionRuntime(self.work, self.bins, patch,
            dict(agent_turns=6, drain_timeout_seconds=30, poll_seconds=0.01))
        self.log_path = self.work / 'log/actraild.log'

    def copy_agent(self, name):
        path = self.out / name
        staging = self.out / (name + '.replacement')
        shutil.copy2(self.source, staging)
        os.replace(staging, path)
        return path

    def diagnostics(self, inode):
        return [line for line in self.log_path.read_text().splitlines()
                if re.search(rf'\binode={inode}\b', line)]

    def nested(self, name, binary, require_ready, inner_turns=4):
        directory = self.out / name
        directory.mkdir()
        inode = binary.stat().st_ino
        before = self.diagnostics(inode)
        if require_ready and not any('attachment ready' in line for line in before):
            raise RuntimeError('reuse requires demonstrated prior attachment readiness')
        inner_dir, outer_dir = directory / 'inner', directory / 'outer'
        inner = AgentWorkload(ROOT, directory / 'maas-inner', binary,
            turns=inner_turns, input_bytes=1024, tpot_ms=0, timeout_seconds=60)
        outer = None
        observers = [AgentObserver(binary), AgentObserver(self.source)]
        sample = dict(name=name, inode=inode, executable=str(binary), require_ready=require_ready,
                      inner_turns=inner_turns,
                      diagnostics_before=before, status='running')
        self.report['samples'].append(sample)
        try:
            inner.start()
            inner.prepare(inner_dir)
            inner.reset()
            script = directory / 'launch-inner.sh'
            inner_argv = inner.command(inner_dir)
            script.write_text('#!/bin/bash\nset -euo pipefail\n'
                + 'cd ' + shlex.quote(str(inner_dir)) + '\n'
                + 'echo $$ > ' + shlex.quote(str(directory / 'inner.pid')) + '\n'
                + ''.join('export ' + key + '=' + shlex.quote(value) + '\n' for key, value in inner.env.items())
                + 'exec ' + shlex.join(inner_argv) + ' > stdout.log 2> stderr.log\n')
            outer = OuterAgent(ROOT, directory / 'maas-outer', self.source, tool_script=script,
                turns=2, input_bytes=128, tpot_ms=0, timeout_seconds=60)
            outer.start()
            outer.prepare(outer_dir)
            outer.reset()
            for observer in observers:
                observer.worker.start()
            previous = self.collection.mark()
            env = dict(os.environ, **outer.env, ACTRAIL_LAUNCH_TIMING='1')
            command = self.collection.launch(outer.command(outer_dir))
            sample['outer_launch_command'] = command
            sample['inner_command'] = inner_argv
            (directory / 'commands.json').write_text(json.dumps(dict(outer=command, inner=inner_argv), indent=2))
            CommandMeasurement(60).run(command, outer_dir, env)
            sample['outer_workload'] = outer.validate(outer_dir, outer_dir / 'stdout.log')
            sample['inner_workload'] = inner.validate(inner_dir, inner_dir / 'stdout.log')
            sample['finalization'] = self.collection.drain(previous)
            traces = self.collection.query('SELECT trace_id,lifecycle_state,health FROM traces WHERE trace_id>?', (previous,))
            if len(traces) != 1 or traces[0][2] != 'clean':
                raise RuntimeError(f'unexpected trace state: {traces}')
            trace = traces[0][0]
            sample.update(mode='P', collection=dict(traces=traces))
            output = subprocess.run([str(self.bins / 'actrailviewer'), '--config', str(self.work / "actraild.conf"),
                '--output-format', 'json', 'actions', '--trace-id', str(trace)],
                capture_output=True, text=True, check=True, timeout=30)
            graph = json.loads(output.stdout)
            (self.out / f'actions-P-{trace}.json').write_text(output.stdout)
            counts = collections.Counter(action['kind'] for action in graph['actions'])
            sample['action_counts'] = dict(counts)
            inner_pid = int((directory / 'inner.pid').read_text())
            inner_identity = self.collection.query('SELECT process_id,host_start_ticks,host_start_boottime_ns FROM processes WHERE host_pid=?', (inner_pid,))
            if len(inner_identity) != 1:
                raise RuntimeError('inner PID does not resolve uniquely in isolated database')
            stored_ticks = inner_identity[0][1]
            boot_ns = inner_identity[0][2]
            # Live kernel identities can have only boottime ns. procfs stat
            # converts that same start_boottime to USER_HZ clock ticks.
            ticks = stored_ticks or (boot_ns * os.sysconf('SC_CLK_TCK') // 1_000_000_000)
            sample['inner_identity'] = dict(pid=inner_pid, process_id=inner_identity[0][0],
                stored_start_ticks=stored_ticks, start_boottime_ns=boot_ns, proc_start_ticks=ticks)
            all_maps = [row for observer in observers for row in observer.finish()]
            root_identity = self.collection.query('SELECT p.host_pid,p.host_start_ticks FROM traces t JOIN processes p ON p.process_id=t.root_process_id WHERE t.trace_id=?', (trace,))[0]
            sample['observed_agent_maps'] = all_maps
            wanted = {(inner_pid, str(ticks)), (root_identity[0], str(root_identity[1]))}
            sample['agent_maps'] = [row for row in all_maps if (row['pid'], row['start_ticks']) in wanted]
            if {(r['pid'], r['start_ticks']) for r in sample['agent_maps']} != wanted or any(r['runtime_mappings'] for r in sample['agent_maps']):
                raise RuntimeError('outer/inner maps missing or TLS runtime was injected')
            by_id = {action['action_id']: action for action in graph['actions']}
            for action in graph['actions']:
                if action['kind'] in ('llm.request', 'llm.response'):
                    if (action['status'] != 'success' or action['completeness'] != 'complete'
                            or not action['start_time_unix_nanos'] or not action['end_time_unix_nanos']):
                        raise RuntimeError('captured request/response lacks complete successful timed evidence')
                if action['kind'] == 'llm.response' and action['attributes'].get('llm.response.done') != 'true':
                    raise RuntimeError('response lacks protocol completion')
            for link in graph['links']:
                if link['role'] in ('llm.call.request', 'llm.call.response', 'command.contains_llm_call'):
                    parent, child = by_id[link['parent_action_id']], by_id[link['child_action_id']]
                    if not link['valid'] or parent['process'] != child['process']:
                        raise RuntimeError('invalid or cross-process LLM/command relationship')
            expected = 2 + inner_turns
            complete = all(counts[kind] == expected for kind in ('llm.request', 'llm.response', 'llm.call'))
            sample['full_coverage'] = complete
            if complete:
                sample['profile'] = ProfileAcceptance(self.out).trace(sample, expected)
                inner_pairs = sum(action['kind'] == 'llm.call' and action['process']['process_id'] == inner_identity[0][0] for action in graph['actions'])
                command_links = sum(link['role'] == 'command.contains_llm_call' and link['valid'] for link in graph['links'])
                if inner_pairs != inner_turns or command_links != expected:
                    raise RuntimeError('inner LLM process or command association mismatch')
                sample['verified_inner_calls'] = inner_pairs
                sample['verified_command_llm_links'] = command_links
            elif require_ready:
                raise RuntimeError('already attached executable lost full LLM coverage')
            sample['diagnostics_after'] = self.diagnostics(inode)
            sample['status'] = 'passed' if complete else 'first_discovery_coverage_gap'
        finally:
            for observer in observers:
                if observer.worker.ident:
                    observer.finish()
            if outer:
                outer.stop()
            inner.stop()
            self.save()

    def short_exit(self, binary):
        directory = self.out / 'short-help'
        directory.mkdir()
        previous = self.collection.mark()
        command = self.collection.launch(['/bin/bash', '-c', 'exec "$@"', 'short-exec', str(binary), '--help'])
        CommandMeasurement(30).run(command, directory, dict(os.environ))
        finalization = self.collection.drain(previous)
        lines = self.diagnostics(binary.stat().st_ino)
        self.report['short_exit'].append(dict(command=command, inode=binary.stat().st_ino,
            finalization=finalization, diagnostics=lines,
            scope='A fast exit alone does not prove pinning or analysis continued after exit.'))
        self.save()

    def save(self):
        (self.out / 'acceptance.json').write_text(json.dumps(self.report, indent=2) + '\n')

    def run(self):
        try:
            self.runtime.prepare()
            config = self.work / 'actraild.conf'
            actual = tomllib.loads(config.read_text())
            if actual['payload']['tls']['capture_backend'] != 'bpf-copy' or actual['seccomp_notify']['enabled']:
                raise RuntimeError('unexpected effective TLS configuration')
            pid = int((self.work / 'run/actraild.pid').read_text())
            if (Path('/proc') / str(pid) / 'exe').resolve() != self.bins / 'actraild':
                raise RuntimeError('unexpected daemon executable')
            self.report['binary_hashes'] = {}
            for path in [self.bins / name for name in ('actraild', 'actrailctl', 'actrailviewer')] + [self.source]:
                with path.open('rb') as stream:
                    self.report['binary_hashes'][str(path)] = hashlib.file_digest(stream, 'sha256').hexdigest()
            (self.out / 'configs').mkdir()
            shutil.copy2(config, self.out / 'configs/P.resolved.toml')
            self.collection.cpu = DaemonCpu(pid)
            self.collection.log = self.log_path.open()
            if self.args.https_short_only:
                binary = self.copy_agent('https-short-xiaoo')
                self.nested('https-short', binary, False, inner_turns=2)
                self.nested('https-short-reuse', binary, True)
                self.report['status'] = 'passed'
                return
            if not self.args.short_only:
                binary = self.copy_agent('inner-xiaoo')
                original_inode = binary.stat().st_ino
                self.nested('first', binary, False)
                self.nested('reuse', binary, True)
                binary = self.copy_agent('inner-xiaoo')
                if binary.stat().st_ino == original_inode:
                    raise RuntimeError('same-path replacement did not change inode')
                self.nested('replacement', binary, False)
                self.nested('replacement-reuse', binary, True)
            short_binary = self.copy_agent('short-xiaoo')
            self.short_exit(short_binary)
            self.nested('after-short', short_binary, False)
            self.nested('after-short-reuse', short_binary, True)
            self.report['status'] = 'passed'
        except BaseException as error:
            self.report.update(status='failed', error=f'{type(error).__name__}: {error}')
            raise
        finally:
            if self.collection.log:
                self.collection.log.close()
            try:
                result = self.runtime.stop()
                if result is not None and result.returncode:
                    raise RuntimeError('own isolated daemon failed to stop')
            finally:
                self.save()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--bin-dir', type=Path, default=ROOT / 'target/release')
    parser.add_argument('--agent-bin', type=Path, default=Path('/home/yzh/.cargo/bin/xiaoo'))
    parser.add_argument('--short-only', action='store_true')
    parser.add_argument('--https-short-only', action='store_true')
    args = parser.parse_args()
    if args.short_only and args.https_short_only:
        parser.error('select one independent short-lifecycle case')
    args.out.resolve().mkdir(parents=True, exist_ok=False)
    NestedAcceptance(args).run()
