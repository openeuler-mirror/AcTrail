#!/usr/bin/env python3
"""Real Xiaoo resource output through SQLite, Prometheus and InfluxDB."""
from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
import secrets
import re
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request

SPEC = importlib.util.spec_from_file_location("resource_acceptance", Path(__file__).with_name("e2e.py"))
BASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BASE)


class ResourceStorageAcceptance(BASE.ResourceProfilingAcceptance):
    def request(self, url, data=None, headers=None):
        request = urllib.request.Request(url, data=data, headers=headers or {})
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.read().decode()

    @staticmethod
    def port():
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]

    def wait(self, check, label):
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            try:
                if check():
                    return
            except (OSError, ValueError):
                pass
            time.sleep(0.2)
        raise TimeoutError(label)

    def start_receivers(self, work):
        self.prom = f"http://127.0.0.1:{self.port()}"
        self.influx = f"http://127.0.0.1:{self.port()}"
        self.token = secrets.token_hex(24)
        config = work / "prometheus.yml"
        config.write_text("scrape_configs: []\n")
        commands = [
            [self.args.prometheus.resolve(), "--config.file", config,
             "--storage.tsdb.path", work / "prometheus-data",
             "--web.listen-address", self.prom.removeprefix("http://"),
             "--web.enable-remote-write-receiver"],
            [self.args.influxd.resolve(), "--http-bind-address", self.influx.removeprefix("http://"),
             "--bolt-path", work / "influx.bolt", "--engine-path", work / "influx-engine",
             "--reporting-disabled"],
        ]
        for name, command in zip(("prometheus", "influxdb"), commands):
            log = (self.out / f"{name}.log").open("w")
            self.logs.append(log)
            self.receivers.append(subprocess.Popen([str(x) for x in command], stdout=log, stderr=log))
        self.wait(lambda: self.request(self.prom + "/-/ready"), "Prometheus readiness")
        self.wait(lambda: self.request(self.influx + "/health"), "InfluxDB readiness")
        self.request(self.influx + "/api/v2/setup", json.dumps({
            "username": "resource-e2e", "password": secrets.token_hex(16),
            "org": "actrail", "bucket": "resources", "token": self.token,
            "retentionPeriodSeconds": 0,
        }).encode(), {"Content-Type": "application/json"})

    def write_paths(self, work):
        patch = super().write_paths(work)
        stanza = f'''
[resource_profiling.storage]
backend = "hybrid"
[resource_profiling.storage.hybrid]
queue_capacity_batches = 4
write_timeout_ms = 200
[[resource_profiling.storage.hybrid.backends]]
name = "local"
backend = "database"
[resource_profiling.storage.hybrid.backends.database.sqlite]
path = "./actrail-resources.sqlite"
[[resource_profiling.storage.hybrid.backends]]
name = "prometheus"
backend = "remote"
[resource_profiling.storage.hybrid.backends.remote]
protocol = "prometheus_remote_write"
endpoint = "{self.prom}/api/v1/write"
[[resource_profiling.storage.hybrid.backends]]
name = "influxdb"
backend = "remote"
[resource_profiling.storage.hybrid.backends.remote]
protocol = "influxdb_line_protocol"
endpoint = "{self.influx}/api/v2/write?org=actrail&bucket=resources&precision=ns"
[resource_profiling.storage.hybrid.backends.remote.headers]
Authorization = "Token {self.token}"
[[resource_profiling.storage.hybrid.backends]]
name = "unavailable"
backend = "remote"
[resource_profiling.storage.hybrid.backends.remote]
protocol = "prometheus_remote_write"
endpoint = "http://127.0.0.1:{self.unavailable_port}/api/v1/write"
timeout_ms = 100
'''
        if self.single_remote:
            stanza = f'''
[resource_profiling.storage]
backend = "remote"
[resource_profiling.storage.remote]
protocol = "prometheus_remote_write"
endpoint = "{self.prom}/api/v1/write"
[resource_profiling.collection.workload]
mode = "procfs"
'''
        with patch.open("a") as output:
            output.write(stanza)
        return patch

    def verify_receivers(self):
        query = urllib.parse.urlencode({"query": 'actrail_resource_rss_kb{kind="workload"}'})
        response = json.loads(self.request(self.prom + "/api/v1/query?" + query))
        if response["status"] != "success" or not response["data"]["result"]:
            raise AssertionError("Prometheus has no real workload RSS series")
        (self.out / "prometheus-query.json").write_text(json.dumps(response, indent=2))
        flux = 'from(bucket:"resources") |> range(start:-1h) |> filter(fn:(r)=>r._measurement=="actrail_resource_rss_kb") |> limit(n:3)'
        csv = self.request(self.influx + "/api/v2/query?org=actrail", json.dumps({"query": flux}).encode(), {
            "Authorization": "Token " + self.token, "Content-Type": "application/json", "Accept": "application/csv",
        })
        if "actrail_resource_rss_kb" not in csv or "workload" not in csv:
            raise AssertionError("InfluxDB has no real workload RSS points")
        (self.out / "influxdb-query.csv").write_text(csv)

    def verify_web(self, work):
        port = self.port()
        with sqlite3.connect(work / "actrail-resources.sqlite") as connection:
            trace = connection.execute("SELECT trace_id FROM resource_samples WHERE kind='workload' LIMIT 1").fetchone()[0]
        with (self.out / "web.log").open("w") as log:
            process = subprocess.Popen([str(self.bins / "actrailweb"), "--config", str(work / "operator.toml"),
                                        "--addr", "127.0.0.1", "--port", str(port)], stdout=log, stderr=log)
            try:
                url = f"http://127.0.0.1:{port}/api/traces/{trace}/resources?limit=3"
                self.wait(lambda: self.request(url), "resource Web API readiness")
                response = json.loads(self.request(url))
                if len(response["resources"]) != 3 or any(sample["trace_id"] != trace for sample in response["resources"]):
                    raise AssertionError("Web resource API returned incorrect trace samples")
                (self.out / "web-resources.json").write_text(json.dumps(response, indent=2))
            finally:
                process.terminate()
                process.wait(timeout=10)
        self.report["web"] = {"trace_id": trace, "sample_count": 3}
        offline = work / "offline.toml"
        config = (work / "operator.toml").read_text()
        disabled, count = re.subn(r"(\[resource_profiling\]\n)enabled = true", r"\1enabled = false", config)
        if count != 1:
            raise AssertionError("resource root switch missing from refreshed defaults")
        offline.write_text(disabled)
        samples = json.loads(self.run_command([
            self.bins / "actrailviewer", "--config", offline, "--output-format", "json",
            "workload-resources", "--trace-id", f"trace-{trace}", "--limit", "3",
        ]))
        if len(samples) != 3 or samples != response["resources"]:
            raise AssertionError("Viewer query depends on collection enablement or disagrees with Web")
        conflict = subprocess.run([
            str(self.bins / "actrailviewer"), "--storage-path", str(work / "actrail-resources.sqlite"),
            "--storage-config", str(offline), "workload-resources",
        ], capture_output=True, text=True)
        if conflict.returncode == 0 or "cannot be combined" not in conflict.stderr:
            raise AssertionError("Viewer accepted conflicting resource source options")
        self.report["viewer"] = {"limit": 3, "disabled_collection_query": True, "conflicting_source_rejected": True}

    def remote_only(self, work):
        self.single_remote = True
        config = work / "operator.toml"
        self.init(config, "--force", "--patch", self.write_paths(work))
        marker = "REMOTE_RESOURCE_" + secrets.token_hex(8)
        self.write_workload(work, marker)
        with (self.out / "remote-daemon.log").open("w") as log:
            daemon = subprocess.Popen([str(self.bins / "actraild"), "--config", str(config), "run"],
                                      cwd=work, env=self.environment, stdout=log, stderr=log, start_new_session=True)
            try:
                self.wait(lambda: (work / "control.sock").exists(), "remote-only daemon readiness")
                started = time.time()
                output = self.run_command([
                    self.bins / "actrailctl", "--config", config, "launch", "--", self.agent,
                    "--cli", "run", "--tools", "bash", "--max-turns", "4", "--prompt",
                    "Use bash once to run `python3 workload.py`; wait and reply with its completion marker.",
                ], cwd=work, timeout=self.args.launch_timeout)
                if marker not in output or not (work / "workload-result.json").exists():
                    raise AssertionError("remote-only real agent workload incomplete")
                traces = re.findall(r"trace trace-(\d+) entered Active", output)
                if len(traces) != 1:
                    raise AssertionError("remote-only workload trace missing")
                query = urllib.parse.urlencode({"query": f'timestamp(actrail_resource_rss_kb{{kind="workload",trace_id="{traces[0]}"}})'})
                def received_new_sample():
                    response = json.loads(self.request(self.prom + "/api/v1/query?" + query))
                    return any(float(series["value"][1]) >= started for series in response["data"]["result"])
                self.wait(received_new_sample, "remote-only fresh Prometheus sample")
            finally:
                daemon.terminate()
                try:
                    daemon.wait(timeout=240)
                except subprocess.TimeoutExpired:
                    self.terminate(daemon)
        if daemon.returncode != 0:
            raise AssertionError("remote-only daemon failed")
        if (work / "actrail-resources.sqlite").exists():
            raise AssertionError("remote-only backend opened a local resource database")
        with sqlite3.connect(work / "actrail.sqlite") as connection:
            if connection.execute("SELECT count(*) FROM events WHERE kind_code=6").fetchone()[0]:
                raise AssertionError("resource samples leaked into general event storage")
        self.report["modes"]["remote-only"] = {"real_agent_completed": True, "local_resource_database": False}

    def run(self):
        self.receivers, self.logs = [], []
        self.single_remote = False
        self.unavailable_port = self.port()
        receiver_directory = tempfile.TemporaryDirectory(prefix="arp-receivers-")
        try:
            self.start_receivers(Path(receiver_directory.name))
            with tempfile.TemporaryDirectory(prefix="arp-hybrid-") as case:
                self.run_case("procfs-tree", True, Path(case))
                self.verify_web(Path(case))
            self.verify_receivers()
            if "resource output failed" not in (self.out / "procfs-tree-daemon.log").read_text():
                raise AssertionError("unavailable backend failure was not exercised")
            with tempfile.TemporaryDirectory(prefix="arp-remote-") as case:
                self.remote_only(Path(case))
            self.verify_receivers()
            self.report["status"] = "passed"
        except BaseException as error:
            self.report.update(status="failed", error=str(error))
            raise
        finally:
            for process in self.receivers:
                process.terminate()
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)
            for log in self.logs:
                log.close()
            receiver_directory.cleanup()
            (self.out / "report.json").write_text(json.dumps(self.report, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, default=BASE.ROOT / "target/release")
    parser.add_argument("--xiaoo", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--launch-timeout", type=int, default=240)
    parser.add_argument("--prometheus", type=Path, required=True)
    parser.add_argument("--influxd", type=Path, required=True)
    ResourceStorageAcceptance(parser.parse_args()).run()
