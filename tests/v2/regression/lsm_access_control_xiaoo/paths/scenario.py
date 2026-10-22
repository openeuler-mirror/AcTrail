from __future__ import annotations

import errno
import json
import shutil
from pathlib import Path

from tests.v2.common.core import TestResult, TestStatus

from ..dynamic import DynamicPolicies

from tests.v2.regression.lsm_access_control_xiaoo.constants import CASE_CONSTANTS


class PathBindingScenario:
    """Verify published path bindings through actual Xiaoo filesystem operations."""

    def __init__(self, case):
        self.case = case
        self.root = case._work / "path-bindings"
        self.source = "static"
        self.documents: dict[str, dict] = {}

    def run(self, context, results: dict, port: int) -> None:
        scenario = {}
        results[self.case._selection.name] = TestResult(
            TestStatus.COMPOSITE, "published file and command path bindings", scenario,
        )
        for source in ("static", "dynamic"):
            self.source = source
            self._prepare_paths()
            self._write_rules()
            self.case._write_patch()
            self._start(context, port)
            if source == "dynamic":
                self._publish_paths()
            self._create_future_files()
            self._verify("bound_b", "b", "b", "allow", context, scenario)

            before = self._configured_paths()
            self._retarget("c")
            stale = self.root / "stale-parent/file"
            stale.unlink()
            stale.parent.rmdir()
            (self.root / "retained-exec").rename(self.root / "retained-exec.saved")
            dynamic = self.case._dynamic
            dynamic._publish_network(dynamic.owner("network"), [{
                "rule_id": "lsm-path-network", "decision": "deny", "remote": dynamic._remote,
            }])
            if self._configured_paths() != before:
                raise AssertionError("network publication changed declared file or command policies")
            self._verify("retarget_c_network_deny", "b", "c", "deny", context, scenario)

            owner = dynamic.owner("network")
            unloaded = dynamic.api.unload(owner)
            if unloaded.get("plugin", {}).get("state") == "active":
                raise AssertionError(f"network policy owner remained active: {unloaded}")
            dynamic._extra_owners.remove(owner)
            if self._configured_paths() != before:
                raise AssertionError("network owner unload changed declared path policies")
            self._verify("network_owner_unloaded", "b", "c", "allow", context, scenario)

            if source == "static":
                self._stop()
                stale.parent.mkdir()
                stale.write_text("", encoding="utf-8")
                (self.root / "retained-exec.saved").rename(self.root / "retained-exec")
                self._start(context, port)
            else:
                self._publish_paths()
            self._verify("republished_c", "c", "c", "allow", context, scenario)
            self._stop()

    def _prepare_paths(self) -> None:
        if self.root.exists():
            shutil.rmtree(self.root)
        self.root.mkdir()
        (self.root / "empty-root").mkdir()
        for target in ("b", "c"):
            (self.root / f"file-{target}").write_text("", encoding="utf-8")
            shutil.copy2(self.case._executable, self.root / f"exec-{target}")
            (self.root / f"dir-{target}").mkdir()
        self._retarget("b")
        (self.root / "future-alias").symlink_to(self.root / "future-target")
        stale = self.root / "stale-parent/file"
        stale.parent.mkdir()
        stale.write_text("", encoding="utf-8")
        shutil.copy2(self.case._executable, self.root / "retained-exec")
        self.documents = {
            "file": {"rules": [
                {"rule_id": "lsm-path-file", "decision": "deny", "operation": "open",
                 "path": str(self.root / "file-a"), "priority": CASE_CONSTANTS.specific_rule_priority},
                {"rule_id": "lsm-path-future-file", "decision": "deny", "operation": "open",
                 "path": str(self.root / "future-file"), "priority": CASE_CONSTANTS.specific_rule_priority},
                {"rule_id": "lsm-path-mkdir", "decision": "deny", "operation": "mkdir",
                 "path": str(self.root / "future-directory"), "priority": CASE_CONSTANTS.specific_rule_priority},
                {"rule_id": "lsm-path-dangling-file", "decision": "deny", "operation": "open",
                 "path": str(self.root / "future-alias"), "priority": CASE_CONSTANTS.specific_rule_priority},
                {"rule_id": "lsm-path-alias-mkdir", "decision": "deny", "operation": "mkdir",
                 "path": str(self.root / "dir-a") + "/**", "priority": CASE_CONSTANTS.specific_rule_priority},
                {"rule_id": "lsm-path-alias-rmdir", "decision": "deny", "operation": "rmdir",
                 "path": str(self.root / "dir-a") + "/**", "priority": CASE_CONSTANTS.specific_rule_priority},
            ]},
            "command": {"rules": [
                {"rule_id": "lsm-path-command", "decision": "deny",
                 "executable": str(self.root / "exec-a"), "priority": CASE_CONSTANTS.specific_rule_priority},
            ]},
        }
        if self.case._selection.name == "lsm":
            self.documents["file"]["rules"].append({
                "rule_id": "lsm-path-tree", "decision": "deny", "operation": "open",
                "path": str(self.root / "future-tree") + "/**", "priority": CASE_CONSTANTS.specific_rule_priority,
            })

    def _retarget(self, target: str) -> None:
        for domain in ("file", "exec", "dir"):
            alias = self.root / f"{domain}-a"
            alias.unlink(missing_ok=True)
            alias.symlink_to(self.root / f"{domain}-{target}")

    def _write_rules(self) -> None:
        file_lines = [f"lsm-path-retained-file deny open {self.root / 'stale-parent/file'}\n"]
        command_lines = [
            f"lsm-root allow exec {self.case._xiaoo} priority 0\n",
            f"lsm-path-retained-command deny exec {self.root / 'retained-exec'} priority 0\n",
        ]
        if self.source == "static":
            file_lines.extend(
                f"{rule['rule_id']} deny {rule['operation']} {rule['path']}\n"
                for rule in self.documents["file"]["rules"]
            )
            command_lines.extend(
                f"{rule['rule_id']} deny exec {rule['executable']} priority {CASE_CONSTANTS.specific_rule_priority}\n"
                for rule in self.documents["command"]["rules"]
            )
        (self.case._work / "file.rules").write_text("".join(file_lines), encoding="utf-8")
        (self.case._work / "command.rules").write_text("".join(command_lines), encoding="utf-8")
        (self.case._work / "network.rules").write_text("", encoding="utf-8")

    def _start(self, context, port: int) -> None:
        case = self.case
        case._dynamic = DynamicPolicies(case._config, context.output, port, case._patches())
        case._runtime = case._dynamic.runtime
        case._dynamic.prepare()

    def _stop(self) -> None:
        result = self.case._dynamic.cleanup()
        if result.has_failure():
            raise AssertionError(f"path scenario service cleanup failed: {result}")
        self.case._dynamic = None
        self.case._runtime = None

    def _publish_paths(self) -> None:
        dynamic = self.case._dynamic
        for domain, candidate in self.documents.items():
            owner = dynamic.owner(domain)
            query = ["rule", "dry-run", str(self.root / ("file-b" if domain == "file" else "exec-b"))]
            if domain == "file":
                query.extend(["--operation", "open"])
            before = dynamic._command_fields(domain, query)
            validation = dynamic.api.validate_config(owner, candidate)
            if validation.get("valid") is not True:
                raise AssertionError(f"{domain} path publication rejected: {validation}")
            if dynamic._command_fields(domain, query) != before:
                raise AssertionError(f"{domain} validation changed effective rules or revision")
            published = dynamic.api.update_config(owner, candidate)
            if published.get("config") != candidate:
                raise AssertionError(f"{domain} declaration changed during publication: {published}")
            after = dynamic._command_fields(domain, query)
            revision = "revision" if domain == "file" else "source_revision"
            if int(after[revision]) <= int(before[revision]):
                raise AssertionError(f"{domain} explicit publication did not advance revision")

    def _configured_paths(self) -> dict:
        dynamic = self.case._dynamic
        return {domain: {
            "config": dynamic.api.config(dynamic.owner(domain)).get("config"),
            "rules": dynamic._command_output(dynamic.owner(domain), ["rule", "list"]),
            "match": dynamic._command_fields(domain, [
                "rule", "dry-run", str(self.root / ("file-b" if domain == "file" else "exec-b")),
                *(["--operation", "open"] if domain == "file" else []),
            ]),
        } for domain in ("file", "command")}

    def _create_future_files(self) -> None:
        (self.root / "future-file").write_text("", encoding="utf-8")
        (self.root / "future-target").write_text("", encoding="utf-8")
        if self.case._selection.name == "lsm":
            tree = self.root / "future-tree/child"
            tree.parent.mkdir()
            tree.write_text("", encoding="utf-8")

    def _verify(self, phase: str, bound: str, alias: str, network: str, context, results: dict) -> None:
        name = f"{self.source}_{phase}"
        context.report_progress(name, f"real Xiaoo verifies path binding to {bound}, alias to {alias}")
        probes = []
        expected = {}
        for operation, domain in (("open", "file"), ("exec", "exec")):
            for target in ("a", "b", "c"):
                label = f"{domain}-{target}"
                probes.append({"name": label, "operation": operation, "path": str(self.root / label)})
                expected[label] = (alias if target == "a" else target) != bound
        for target in ("b", "c"):
            label = f"exec-fd-{target}"
            probes.append({"name": label, "operation": "exec-fd", "path": str(self.root / f"exec-{target}")})
            expected[label] = target != bound
        probes.extend([
            {"name": "future-file", "operation": "open", "path": str(self.root / "future-file")},
            {"name": "future-directory", "operation": "mkdir", "path": str(self.root / "future-directory")},
            {"name": "future-alias", "operation": "open", "path": str(self.root / "future-alias")},
            {"name": "future-target", "operation": "open", "path": str(self.root / "future-target")},
        ])
        expected.update({"future-file": False, "future-directory": False, "future-alias": False, "future-target": False})
        directories = self._directory_probes(probes, expected, bound, alias)
        files = [self.root / f"file-{target}" for target in ("b", "c")] + [
            self.root / "future-file", self.root / "future-target",
        ]
        if self.case._selection.name == "lsm":
            files.append(self.root / "future-tree/child")
            probes.append({"name": "tree-child", "operation": "open", "path": str(files[-1])})
            expected["tree-child"] = False
        for path in files:
            path.write_text("", encoding="utf-8")
        (self.case._work / "path-probes.json").write_text(json.dumps(probes), encoding="utf-8")
        trace_id = self.case._run_xiaoo(name)
        self.case._require_workload("allow", network_decision=network)
        observed = json.loads(self.case._results.read_text(encoding="utf-8"))["path_probes"]
        for label, allowed in expected.items():
            result = observed[label]
            if result.get("allowed") != allowed or (
                not allowed and result.get("errno") not in (errno.EPERM, errno.EACCES)
            ):
                raise AssertionError(f"{name}: unexpected {label} operation result: {result}")
        for target in ("b", "c"):
            content = (self.root / f"file-{target}").read_text(encoding="utf-8")
            if content != ("" if target == bound else "ACTRAIL_PATH_WRITE"):
                raise AssertionError(f"{name}: file-{target} has unexpected content: {content!r}")
        if any(path.read_text(encoding="utf-8") for path in files[2:]):
            raise AssertionError(f"{name}: future file protection was bypassed")
        if (self.root / "future-directory").exists():
            raise AssertionError(f"{name}: denied future directory was created")
        for label, path, operation in directories:
            should_exist = expected[label] if operation == "mkdir" else not expected[label]
            if path.exists() != should_exist:
                raise AssertionError(f"{name}: unexpected directory effect for {label}: {path}")
        required = {rule["rule_id"]: "deny" for document in self.documents.values() for rule in document["rules"]}
        if network == "deny":
            required["lsm-path-network"] = "deny"
        self.case._require_evidence(trace_id, "allow", required)
        self._require_bound_paths(trace_id, bound)
        results[name] = TestResult(TestStatus.PASSED, f"trace-{trace_id}: path operations, filesystem effects and audits verified")

    def _directory_probes(self, probes: list, expected: dict, bound: str, alias: str) -> list:
        for target in ("b", "c"):
            for child in (self.root / f"dir-{target}").iterdir():
                shutil.rmtree(child)
            if self.case._selection.name == "fanotify_seccomp":
                (self.root / f"dir-{target}/sub").mkdir()
        directories = []
        operations = ["mkdir", "rmdir"]
        if self.case._selection.name == "fanotify_seccomp":
            operations.extend(f"{operation}-outside-{mode}"
                              for operation in ("mkdir", "rmdir") for mode in ("cwd", "dirfd"))
            operations.extend(("mkdir-magic-dirfd", "rmdir-magic-dirfd"))
        for operation in operations:
            for target in (("a", "b", "c") if operation in ("mkdir", "rmdir") else ("b", "c")):
                label = f"{operation}-{target}"
                parent = self.root / f"dir-{target}"
                if "-magic-" in operation:
                    parent /= "sub"
                path = parent / label
                action = operation.split("-", 1)[0]
                if action == "rmdir":
                    path.mkdir()
                probes.append({"name": label, "operation": operation, "path": str(path),
                               "root": str(self.root / "empty-root")})
                expected[label] = (alias if target == "a" else target) != bound
                directories.append((label, path, action))
        return directories

    def _require_bound_paths(self, trace_id: int, bound: str) -> None:
        runtime = self.case._runtime
        output = runtime.run_checked(runtime.viewer_command(
            "events", "--trace-id", str(trace_id), "--output-format", "json",
        ), echo=False)
        events = json.loads(output.stdout)["events"]
        for rule_id, target in (
            ("lsm-path-file", f"file-{bound}"), ("lsm-path-command", f"exec-{bound}"),
            ("lsm-path-dangling-file", "future-target"),
        ):
            matches = [event.get("payload", {}) for event in events
                       if event.get("payload", {}).get("rule_id") == rule_id]
            if not matches or any(payload.get("path") != str(self.root / target) for payload in matches):
                raise AssertionError(f"{rule_id} audits did not retain the published target: {matches}")
        for operation in ("mkdir", "rmdir"):
            rule_id = f"lsm-path-alias-{operation}"
            matches = [event.get("payload", {}) for event in events
                       if event.get("payload", {}).get("rule_id") == rule_id]
            if not matches or any(not Path(payload.get("path") or "").is_relative_to(self.root / f"dir-{bound}")
                                  for payload in matches):
                raise AssertionError(f"{rule_id} audits did not resolve the parent alias: {matches}")
