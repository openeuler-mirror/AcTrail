#!/usr/bin/env python3
"""Pack/import real captures and verify cluster UID navigation in a browser."""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import time
from urllib.request import urlopen

REPO = Path(__file__).resolve().parents[4]

class ClusterAcceptance:
    def __init__(self, args):
        self.args = args
        self.work = args.work.resolve()
        self.work.mkdir(parents=True, exist_ok=False)
        self.config = self.work / 'cluster.conf'
        self.binary = REPO / 'target/release/actrailcluster'
        self.root = self.work / 'center'
        self.web = None

    def command(self, *args):
        result = subprocess.run([str(arg) for arg in args], cwd=REPO,
                                text=True, capture_output=True, timeout=120)
        with (self.work / 'commands.log').open('a') as log:
            log.write(result.stdout + result.stderr)
        if result.returncode:
            raise RuntimeError(f'{args[0]} failed; inspect commands.log')
        return result.stdout

    def run(self):
        report = {'status': 'failed'}
        try:
            source = self.args.source.resolve()
            database = source / 'data/actrail.sqlite'
            if not (source / 'agent.log').is_file():
                raise RuntimeError('source must contain a real-agent capture and agent.log')
            patch = self.work / 'cluster.patch.toml'
            patch.write_text(f'''[storage.sqlite]
path = {json.dumps(str(database))}
[cluster]
enabled = true
cluster_id = "timeline-cluster"
node_id = "timeline-node"
node_name = "Timeline acceptance"
node_ip = "127.0.0.1"
[cluster.center]
enabled = true
root_dir = {json.dumps(str(self.root))}
listen_host = "127.0.0.1"
listen_port = 18481
''')
            self.command(REPO / 'target/release/actrailctl', '--config', self.config,
                         'init', '-f', '--patch', patch)
            bundles = []
            for trace in [1, 2, 3]:
                bundle = self.work / f'trace-{trace}.actrailbundle'
                self.command(self.binary, '--config', self.config, 'pack', '--trace-id', f'trace-{trace}',
                             '--output', bundle)
                bundles.append(bundle)
            for bundle in bundles[:2]:
                self.command(self.binary, '--config', self.config, 'import', bundle)
            with sqlite3.connect(self.root / 'index.sqlite') as db:
                rows = db.execute('SELECT trace_uid,local_trace_id FROM imported_traces ORDER BY trace_uid').fetchall()
            if len(rows) != 2:
                raise RuntimeError('real bundle imports did not create two index entries')
            selected = rows[0][0]
            (self.work / 'selected.json').write_text(json.dumps({'uid': selected, 'rows': rows}))
            with (self.work / 'web.log').open('w') as log:
                self.web = subprocess.Popen([str(REPO / 'target/release/actrailweb'), 'cluster',
                    '--config', str(self.config), '--addr', '127.0.0.1', '--port', str(self.args.port)],
                    stdout=log, stderr=subprocess.STDOUT)
                base = f'http://127.0.0.1:{self.args.port}'
                for _ in range(100):
                    if self.web.poll() is not None:
                        raise RuntimeError('cluster web exited; inspect web.log')
                    try:
                        with urlopen(base + '/health', timeout=1):
                            break
                    except OSError:
                        time.sleep(.1)
                else:
                    raise RuntimeError('cluster web startup timeout')
                # Import while the viewer is running; prior identities must remain valid.
                self.command(self.binary, '--config', self.config, 'import', bundles[2])
                self.command(self.binary, '--config', self.config, 'import', bundles[0])
                self.command('node', Path(__file__).with_name('cluster.mjs'), base, selected,
                             self.work, self.args.playwright, self.args.chromium)
            report.update(status='passed', trace_uid=selected)
        finally:
            if self.web:
                self.web.terminate()
                try:
                    self.web.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.web.kill()
                    self.web.wait()
            (self.work / 'result.json').write_text(json.dumps(report, indent=2))

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--port', type=int, default=18480)
    parser.add_argument('--playwright', default='/tmp/cliproxy-endpoint-browser/node_modules/playwright/index.mjs')
    parser.add_argument('--chromium', default='/home/yzh/.cache/ms-playwright/chromium-1248/chrome-linux-arm64/chrome')
    ClusterAcceptance(parser.parse_args()).run()
