#!/usr/bin/env python3
"""Real OpenCode hang detection with model, tool and user-wait exclusions."""
from __future__ import annotations
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import signal
import shutil
import socket
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.request

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO))
from scripts.bench.payload.agent import AgentWorkload


class Acceptance:
    def __init__(self, args):
        self.args = args
        self.root = args.output.resolve()
        self.root.mkdir(parents=True, exist_ok=False)
        self.binary = args.bin_dir.resolve()
        self.config = self.root / 'operator.conf'
        self.db = self.root / 'data/actrail.sqlite'
        self.processes = []
        self.logs = []
        self.evidence = {'status': 'running', 'binary_directory': str(self.binary), 'turns': [],
                         'permission_poll_interval_ms': args.permission_poll_interval_ms}
        self.maas = None
        self.trace = None
        self.gates = self.root / 'gates'
        self.gates.mkdir()
        self.save_gate(None)
        self.project = self.root / 'project'
        self.pool = concurrent.futures.ThreadPoolExecutor(max_workers=2)

    def save(self, name, value):
        (self.root / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')

    def command(self, *args):
        result = subprocess.run([str(a) for a in args], capture_output=True, text=True, timeout=60)
        if result.returncode:
            raise RuntimeError(f'{args[0]} exit={result.returncode}: {result.stderr[-3000:]}')
        return result.stdout

    def launch(self, name, argv, env=None, cwd=None):
        log = (self.root / f'{name}.log').open('wb')
        self.logs.append(log)
        process = subprocess.Popen([str(a) for a in argv], stdout=log, stderr=subprocess.STDOUT,
                                   env=env, cwd=cwd or REPO, start_new_session=True)
        self.processes.append(process)
        return process

    def request(self, method, path, payload=None, base=None, timeout=30):
        data = None if payload is None else json.dumps(payload).encode()
        req = urllib.request.Request((base or self.base) + path, data=data, method=method,
                                     headers={'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=timeout) as response:
            raw = response.read()
            return json.loads(raw) if raw else None

    def wait(self, predicate, label, timeout=30):
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            try:
                value = predicate()
                if value:
                    return value
            except (OSError, urllib.error.URLError, sqlite3.Error) as error:
                last = str(error)
            time.sleep(.1)
        raise RuntimeError(f'timeout {label}; {last}')

    def alerts(self, session=None, kind=None):
        with sqlite3.connect(self.db.as_uri() + '?mode=ro', uri=True) as db:
            db.row_factory = sqlite3.Row
            rows = [dict(row) for row in db.execute(
                'SELECT a.alert_id,a.trace_id,a.created_at,a.payload_json,d.definition_key '
                'FROM alerts a JOIN alert_definitions d USING(alert_definition_id) ORDER BY a.alert_id')]
        for row in rows:
            row['payload'] = json.loads(row.pop('payload_json'))
        return [r for r in rows
                if (session is None or r['payload'].get('session_id') == session)
                and (kind is None or r['payload'].get('kind') == kind)]

    def save_gate(self, session, skip=0):
        (self.gates / 'gate.json').write_text(json.dumps({'session': session, 'skip': skip,
                                                       'token': str(time.time_ns())}))
        (self.gates / 'release').unlink(missing_ok=True)

    def detected(self, session=None):
        return self.alerts(session, 'hang_detected')

    def assert_no_new_alert(self, count, seconds=3.3):
        time.sleep(seconds)
        assert len(self.detected()) == count, 'normal waiting produced a hang alert'

    def prepare(self):
        for directory in ['run', 'data', 'log', 'export', 'plugins']:
            (self.root / directory).mkdir()
        patch = self.root / 'operator.patch.toml'
        patch.write_text(f'''[control]
socket_path = "{self.root / 'run/control.sock'}"
pid_file = "{self.root / 'run/daemon.pid'}"
log_path = "{self.root / 'log/daemon.log'}"
[storage.sqlite]
path = "{self.db}"
[storage.retention]
enabled = false
[export.snapshot]
directory = "{self.root / 'export'}"
[plugins.discovery]
directory = "{self.root / 'plugins'}"
[plugins.startup]
enabled = false
load = []
[capture]
profile_name = "user-lifecycle-real-agent"
capabilities = ["proc-lifecycle"]
opportunistic_capabilities = []
disabled_capabilities = []
[ebpf]
enabled = "false"
[payload.tls]
enabled = false
sync_event_socket_path = "{self.root / 'run/tls.sock'}"
[payload.socket]
enabled = false
[payload.stdio]
enabled = false
[seccomp_notify]
enabled = false
[process_seccomp]
enabled = false
[resource_metrics]
enabled = false
[enforcement]
enabled = false
[command_control]
enabled = false
[network_control]
enabled = false
[idle_detection]
enabled = true
threshold_secs = "2s"
poll_interval_secs = "1s"
''')
        integration_config = self.root / 'agent-host.toml'
        adapter_directory = self.root / 'agent-host/opencode'
        adapter_source = REPO / 'crates/adapters/agent_host/assets/opencode'
        for subdirectory in ('plugins', 'lib'):
            target = adapter_directory / subdirectory
            target.mkdir(parents=True)
            for source in (adapter_source / subdirectory).glob('*.js'):
                shutil.copyfile(source, target / source.name)
        integration_config.write_text(f'[opencode]\nenabled = true\nplugin_dir = "{adapter_directory}"\n')
        self.command(self.binary / 'actrailctl', '--config', self.config, 'init', '--force', '--patch', patch)
        self.launch('daemon', [self.binary / 'actraild', '--config', self.config, 'run'])
        self.wait(lambda: subprocess.run([str(self.binary/'actrailctl'), '--config', str(self.config), 'doctor'],
                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0, 'daemon')
        self.maas = AgentWorkload(REPO, self.root/'maas', self.args.opencode, turns=2,
                                 kind='opencode', https=False, timeout_seconds=30, input_bytes=128,
                                 setup_timeout_seconds=120)
        self.maas._fixture['tool_command'] = 'sleep 4; cat input.txt | tee result-{index}.txt'
        self.maas.start()
        # Delay the first real provider SSE frame beyond the detector threshold.
        command = self.maas._process.args + ['--ttft-milliseconds', '5000']
        self.maas._process.terminate()
        self.maas._process.wait(timeout=10)
        self.maas._process = self.launch('maas-delayed', command)
        self.wait(lambda: self.maas._request('GET', '/healthz') is None, 'delayed MaaS')
        self.maas.prepare(self.project)
        cfg_path = self.project/'opencode.json'
        cfg = json.loads(cfg_path.read_text())
        cfg['permission'] = {'*': 'allow', 'bash': 'ask'}
        cfg.setdefault('experimental', {})['continue_loop_on_deny'] = True
        gate_plugin = Path(__file__).with_name('hang-gate-plugin.js').resolve()
        cfg['plugin'] = [gate_plugin.as_uri()]
        cfg_path.write_text(json.dumps(cfg, indent=2))
        port = AgentWorkload._free_port()
        self.base = f'http://127.0.0.1:{port}'
        self.agent = self.launch('opencode', [self.binary/'actrailctl', '--config', self.config,
                    'launch', '--agent-host-config', integration_config, '--name', 'user-lifecycle-real-opencode', '--host-ebpf', 'disabled',
                    '--seccomp-notify', 'disabled', '--', self.args.opencode,
                    'serve', '--hostname', '127.0.0.1', '--port', str(port)],
                    env=dict(os.environ, **self.maas.env,
                             ACTRAIL_ACCEPTANCE_GATE_DIR=str(self.gates),
                             ACTRAIL_OPENCODE_PERMISSION_POLL_INTERVAL_MS=str(self.args.permission_poll_interval_ms)),
                    cwd=self.project)
        self.wait(lambda: self.request('GET', '/global/health'), 'opencode server', timeout=90)
        self.api = self.request('GET', '/doc')
        self.save('opencode-api.json', self.api)
        self.trace = self.wait(self.trace_id, 'trace registration')
        web_port = AgentWorkload._free_port()
        self.web = f'http://127.0.0.1:{web_port}'
        self.web_process = self.launch('web', [self.binary/'actrailweb', '--config', self.config, '--addr', '127.0.0.1', '--port', str(web_port)])
        self.wait(lambda: self.request('GET', f'/api/traces/{self.trace}/action-tree', base=self.web), 'web')

    def trace_id(self):
        with sqlite3.connect(self.db.as_uri()+'?mode=ro', uri=True) as db:
            rows = db.execute('SELECT trace_id FROM traces').fetchall()
        if len(rows) != 1:
            return None
        return rows[0][0]

    def permissions(self):
        paths = self.api.get('paths', {})
        permissions = {}
        for path in ['/permission', '/api/permission/request']:
            if path not in paths:
                continue
            result = self.request('GET', path)
            rows = result if isinstance(result, list) else result.get('data', result.get('requests', []))
            for row in rows:
                permissions[(row.get('sessionID'), row['id'])] = {**row, '_endpoint': path}
        return list(permissions.values())

    def approve(self, pending, reply="once"):
        request_id = pending['id']
        paths = self.api.get('paths', {})
        if pending['_endpoint'].startswith('/api/'):
            template = '/api/session/{sessionID}/permission/{requestID}/reply'
            if template not in paths:
                raise RuntimeError('OpenCode V2 permission reply endpoint is absent')
            path = template.replace('{sessionID}', pending['sessionID']).replace('{requestID}', request_id)
            return self.request('POST', path, {'reply': reply})
        for path in paths:
            if (path.startswith(pending['_endpoint']+'/') and path.endswith('/reply')
                    and 'post' in paths[path]):
                import re
                resolved = re.sub(r'\{[^}]+\}', request_id, path)
                return self.request('POST', resolved, {'reply': reply})
        raise RuntimeError('OpenCode API has no permission reply endpoint')

    def snapshot(self, name):
        graph = self.request('GET', f'/api/traces/{self.trace}/action-tree', base=self.web)
        waterfall = self.request('GET', f'/api/traces/{self.trace}/waterfall', base=self.web)
        assert graph['idle_intervals'] == waterfall['idle_intervals'], 'Web projections disagree'
        self.save(name+'.json', {'alerts': self.alerts(), 'graph': graph, 'waterfall': waterfall})
        return graph

    def start_turn(self, session, label, gated=False, gate_skip=0):
        for kind in ('gate', 'model'):
            (self.gates / f'{session}.{kind}').unlink(missing_ok=True)
        if gated:
            self.save_gate(session, gate_skip)
        payload = {'model': {'providerID': 'bench', 'modelID': 'deepseek-v4-flash'},
                   'parts': [{'type': 'text', 'text': f'Run the requested bash command. {label}.'}]}
        return self.pool.submit(self.request, 'POST', f'/session/{session}/message', payload, timeout=120)

    def finish_turn(self, session, future, label, reject=False, model_wait=True, after_error_gate=False):
        count = len(self.detected())
        self.wait(lambda: (self.gates / f'{session}.model').exists(), 'real model request boundary')
        self.assert_no_new_alert(count)
        assert not future.done(), 'model request completed before waiting exclusion check'
        if model_wait:
            assert not any(p.get('sessionID') == session for p in self.permissions()), \
                'provider delay ended before model-wait exclusion was verified'
        pending = self.wait(lambda: next((p for p in self.permissions()
                           if p.get('sessionID') == session), None), 'actual tool permission', 45)
        self.save(label+'-permission.json', pending)
        self.assert_no_new_alert(count)
        assert not future.done(), 'actual tool ran without user permission'
        self.approve(pending, 'reject' if reject else 'once')
        if after_error_gate:
            self.wait(lambda: (self.gates/f'{session}.gate').exists(), 'internal pause after tool failure')
            messages = self.request('GET', f'/session/{session}/message')
            assert any(part.get('type') == 'tool' and part.get('state', {}).get('status') == 'error'
                       for message in messages for part in message.get('parts', []))
            self.wait(lambda: len(self.detected()) == count + 1, 'hang after failed tool counter cleanup', 12)
            self.snapshot(label+'-hang-open')
            (self.gates/'release').touch()
            count += 1
        if not reject:
            self.wait(lambda: any(part.get('type') == 'tool' and part.get('state', {}).get('status') == 'running'
                      for message in self.request('GET', f'/session/{session}/message')
                      for part in message.get('parts', [])), 'real running bash tool')
            self.assert_no_new_alert(count)
        response = future.result(timeout=90)
        self.save(label+'-response.json', response)
        messages = self.request('GET', f'/session/{session}/message')
        self.save(label+'-messages.json', messages)
        tool_states = [part['state']['status'] for message in messages
                       for part in message.get('parts', []) if part.get('type') == 'tool']
        assert ('error' if reject else 'completed') in tool_states
        if not reject:
            assert (self.project/'result-1.txt').read_bytes() == (self.project/'input.txt').read_bytes()
        self.assert_no_new_alert(count)
        self.evidence['turns'].append({'session_id': session, 'label': label, 'tool_states': tool_states})

    def run(self):
        try:
            self.prepare()
            a = self.request('POST', '/session', {'title':'hang A'})['id']
            b = self.request('POST', '/session', {'title':'hang B'})['id']
            self.evidence['session_ids'] = [a, b]

            self.maas.reset()
            self.finish_turn(a, self.start_turn(a, 'normal-waits'), 'normal-waits')
            assert not self.detected(), 'normal model/tool/user/completed states must not alert'

            self.maas.reset()
            short = self.start_turn(a, 'short-pause', gated=True)
            self.wait(lambda: (self.gates/f'{a}.gate').exists(), 'short internal pause')
            time.sleep(.2)
            (self.gates/'release').touch()
            self.finish_turn(a, short, 'short-pause')
            assert not self.detected(), 'short pause produced a hang alert'

            self.maas.reset()
            rejected = self.start_turn(a, 'tool-rejection', gated=True, gate_skip=1)
            self.finish_turn(a, rejected, 'tool-rejection', reject=True, after_error_gate=True)
            self.wait(lambda: self.alerts(a, 'hang_recovered'), 'recovery after failed-tool pause')
            assert len(self.detected(a)) == 1 and len(self.alerts(a, 'hang_recovered')) == 1

            # Session A waits for real user approval while B stalls before dispatch.
            self.maas.reset()
            active = self.start_turn(a, 'parallel-normal')
            self.wait(lambda: any(p.get('sessionID') == a for p in self.permissions()), 'session A approval')
            stalled = self.start_turn(b, 'long-pause', gated=True)
            self.wait(lambda: (self.gates/f'{b}.gate').exists(), 'long internal pause')
            detected = self.wait(lambda: self.detected(b), 'hang alert', 12)
            assert len(detected) == 1 and len(self.detected(a)) == 1, 'session state leaked across execution units'
            self.assert_no_new_alert(2)
            graph = self.snapshot('hang-open')
            assert any(row['session_id'] == b and row['kind'] == 'hang'
                       and row['end_time_unix_nanos'] is None for row in graph['idle_intervals'])
            self.finish_turn(a, active, 'parallel-normal', model_wait=False)
            self.maas.reset()
            (self.gates/'release').touch()
            self.finish_turn(b, stalled, 'long-pause')
            self.wait(lambda: self.alerts(b, 'hang_recovered'), 'hang recovery')
            assert len(self.detected(b)) == 1 and len(self.alerts(b, 'hang_recovered')) == 1
            graph = self.snapshot('hang-recovered')
            assert len(graph['idle_intervals']) == 2
            assert all(row['end_time_unix_nanos'] is not None for row in graph['idle_intervals'])
            messages = self.request('GET', f'/session/{b}/message')
            user_ids = {m['info']['id'] for m in messages if m['info']['role'] == 'user'}
            assert self.detected(b)[0]['payload']['task_id'] in user_ids

            self.request('DELETE', f'/session/{b}')
            self.assert_no_new_alert(2)
            self.agent.terminate()
            self.agent.wait(timeout=30)
            self.wait(self.trace_terminal, 'trace finalization', timeout=45)
            history = self.snapshot('trace-closed-history')
            self.web_process.terminate()
            self.web_process.wait(timeout=10)
            web_port = AgentWorkload._free_port()
            self.web = f'http://127.0.0.1:{web_port}'
            self.web_process = self.launch('web-history', [self.binary/'actrailweb', '--config', self.config,
                '--addr', '127.0.0.1', '--port', str(web_port)])
            self.wait(lambda: self.request('GET', f'/api/traces/{self.trace}/action-tree', base=self.web), 'fresh historical Web')
            reread = self.snapshot('fresh-web-history')
            assert reread['idle_intervals'] == history['idle_intervals'], 'history differs after Web restart'
            self.evidence['alerts'] = self.alerts()
            self.evidence['status'] = 'passed'
        except BaseException as error:
            self.evidence['status'] = 'failed'
            self.evidence['error'] = repr(error)
            raise
        finally:
            self.evidence['trace_id'] = self.trace
            self.save('result.json', self.evidence)
            for process in reversed(self.processes):
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                    try: process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL); process.wait(timeout=5)
            if self.maas is not None: self.maas.stop()
            for log in self.logs: log.close()
            self.pool.shutdown(wait=False, cancel_futures=True)

    def trace_terminal(self):
        with sqlite3.connect(self.db.as_uri()+'?mode=ro', uri=True) as db:
            return db.execute('SELECT completed_at IS NOT NULL OR exited_at IS NOT NULL OR failed_at IS NOT NULL '
                              'FROM traces WHERE trace_id=?',(self.trace,)).fetchone()[0]


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--bin-dir', type=Path, default=REPO/'target/release')
    parser.add_argument('--opencode', type=Path, default=Path('/usr/local/bin/opencode'))
    parser.add_argument('--permission-poll-interval-ms', type=int, default=5000)
    args=parser.parse_args()
    Acceptance(args).run()

if __name__=='__main__': main()
