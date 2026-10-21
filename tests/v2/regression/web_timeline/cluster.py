#!/usr/bin/env python3
"""Pack/import real captures and verify cluster UID navigation in a browser."""
from constants import CASE_CONSTANTS as C
import argparse
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import time
from urllib.request import urlopen

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))
from tests.v2.common.patch_builder import PatchBuilder

class ClusterAcceptance:
    def __init__(self, args):
        self.args = args
        self.work = args.work.resolve()
        self.work.mkdir(parents=True, exist_ok=False)
        self.config = self.work / 'cluster.conf'
        self.binary = REPO / C.bin_dir / 'actrailcluster'
        self.root = self.work / 'center'
        self.web = None

    def command(self, *args):
        result = subprocess.run([str(arg) for arg in args], cwd=REPO,
                                text=True, capture_output=True, timeout=C.cluster_command_timeout_seconds)
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
            (
                PatchBuilder().with_values({
                    "storage": {"sqlite": {"path": str(database)}},
                    "cluster": {"center": {"root_dir": str(self.root)}},
                }).build().save(patch)
            )
            self.command(REPO / C.bin_dir / 'actrailctl', '--config', self.config,
                         'init', '-f', '--patch', patch,
                         '--patch', Path(__file__).with_name('cluster') / 'patch_config.toml')
            bundles = []
            for trace in C.cluster_trace_ids:
                bundle = self.work / f'trace-{trace}.actrailbundle'
                self.command(self.binary, '--config', self.config, 'pack', '--trace-id', f'trace-{trace}',
                             '--output', bundle)
                bundles.append(bundle)
            for bundle in bundles[:C.cluster_initial_imports]:
                self.command(self.binary, '--config', self.config, 'import', bundle)
            with sqlite3.connect(self.root / 'index.sqlite') as db:
                rows = db.execute('SELECT trace_uid,local_trace_id FROM imported_traces ORDER BY trace_uid').fetchall()
            if len(rows) != C.cluster_initial_imports:
                raise RuntimeError('real bundle imports did not create two index entries')
            selected = rows[0][0]
            (self.work / 'selected.json').write_text(json.dumps({'uid': selected, 'rows': rows}))
            with (self.work / 'web.log').open('w') as log:
                self.web = subprocess.Popen([str(REPO / C.bin_dir / 'actrailweb'), 'cluster',
                    '--config', str(self.config), '--addr', C.host, '--port', str(self.args.port)],
                    stdout=log, stderr=subprocess.STDOUT)
                base = f'http://{C.host}:{self.args.port}'
                for _ in range(C.web_ready_attempts):
                    if self.web.poll() is not None:
                        raise RuntimeError('cluster web exited; inspect web.log')
                    try:
                        with urlopen(base + '/health', timeout=C.health_timeout_seconds):
                            break
                    except OSError:
                        time.sleep(C.health_poll_seconds)
                else:
                    raise RuntimeError('cluster web startup timeout')
                # Import while the viewer is running; prior identities must remain valid.
                for bundle in bundles[C.cluster_initial_imports:]:
                    self.command(self.binary, '--config', self.config, 'import', bundle)
                self.command(self.binary, '--config', self.config, 'import', bundles[0])
                self.command('node', Path(__file__).with_name('cluster.mjs'), base, selected,
                             self.work, self.args.playwright, self.args.chromium)
            report.update(status='passed', trace_uid=selected)
        finally:
            if self.web:
                self.web.terminate()
                try:
                    self.web.wait(timeout=C.web_stop_timeout_seconds)
                except subprocess.TimeoutExpired:
                    self.web.kill()
                    self.web.wait()
            (self.work / 'result.json').write_text(json.dumps(report, indent=C.json_indent))

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--port', type=int, default=C.cluster_port)
    parser.add_argument('--playwright', default=C.playwright_path)
    parser.add_argument('--chromium', default=C.chromium_path)
    ClusterAcceptance(parser.parse_args()).run()
