from __future__ import annotations

import json
import statistics
from pathlib import Path


class Report:
    def __init__(self, source: Path, destination: Path):
        self._source = source
        self._destination = destination

    @staticmethod
    def _summary(measurement: dict, decision: str) -> dict:
        rounds = [item for item in measurement["rounds"] if item["decision"] == decision]
        latencies = sorted(value for item in rounds for value in item["latency_ns"])
        count = sum(item["count"] for item in rounds)
        seconds = sum(item["elapsed_seconds"] for item in rounds)
        return {
            "backend": measurement["backend"], "decision": decision, "count": count,
            "elapsed_seconds": seconds, "operations_per_second": count / seconds,
            "mean_latency_us": statistics.mean(latencies) / 1000,
            "median_latency_us": statistics.median(latencies) / 1000,
            "p95_latency_us": latencies[min(len(latencies) - 1, int(len(latencies) * 0.95))] / 1000,
            "worker_cpu_ms": 1000 * sum(item["worker_user_cpu_seconds"] + item["worker_system_cpu_seconds"] for item in rounds),
            "voluntary_context_switches": sum(item["worker_voluntary_context_switches"] for item in rounds),
            "involuntary_context_switches": sum(item["worker_involuntary_context_switches"] for item in rounds),
            "daemon_cpu_ms": 1000 * sum(item["daemon_cpu_seconds"] for item in rounds),
        }

    def run(self) -> None:
        data = json.loads(self._source.read_text())
        if {item["backend"] for item in data["measurements"]} != {"seccomp", "lsm"}:
            raise ValueError("performance report requires both measured backends")
        summaries = [self._summary(item, decision) for item in data["measurements"]
                     for decision in ("allow", "deny")]
        data["summaries"] = summaries
        self._source.write_text(json.dumps(data, indent=2) + "\n")
        expected_connections = data["warmup_per_decision"] + data["rounds"] * data["count_per_decision_per_round"]
        tick_rates = {item["clock_ticks_per_second"] for item in data["measurements"]}
        tick_description = ", ".join(f"CLK_TCK={rate}（{1000 / rate:g} ms）" for rate in sorted(tick_rates))
        source = data.get("policy_source", "static")
        mechanism = "WASM 动态发布后" if source == "wasm" else "静态"
        rule_description = "经 WASM 发布的 allow/deny" if source == "wasm" else "静态 allow/deny"
        lines = [
            f"# {mechanism}网络访问控制性能", "",
            f"环境：Linux `{data['kernel']}`，`{data['architecture']}`。测试使用真实 Xiaoo 的 Bash 工具调用，",
            "通过本地确定性 HTTP provider 下发 Python TCP 工作负载。", "",
            f"每个后端先分别预热 allow/deny 各 {data['warmup_per_decision']} 次，随后在同一工作进程中各测量",
            f"{data['rounds']} 轮，每轮 {data['count_per_decision_per_round']} 次。后端顺序为 "
            + " → ".join(data["backend_order"]) + "；轮间交替 allow/deny 的先后顺序。", "",
            "两种配置均由 `actrailctl init -f --patch` 刷新默认配置，启用相同 proc-lifecycle、net-transport、",
            "socket bpf-copy 观测，关闭 TLS/stdio 采集、文件及命令治理和网络审计。策略均为本地 IPv4 TCP",
            f"endpoint，采用{rule_description} 规则，默认 allow。LSM 未安装 seccomp 或 fanotify。", "",
            "延迟覆盖 Python 创建 socket、设置 timeout、connect 裁决和 close；吞吐使用整轮墙钟时间。",
            "表内延迟统计合并各轮逐次样本，CPU/context switches 汇总各轮测量窗口。", "",
            "| 后端 | 决策 | 次数 | 平均 μs | 中位 μs | P95 μs | 次/秒 | 工作进程 CPU ms | 主动/被动切换 | daemon CPU ms |",
            "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|",
        ]
        for item in summaries:
            lines.append(
                f"| {item['backend']} | {item['decision']} | {item['count']} | "
                f"{item['mean_latency_us']:.2f} | {item['median_latency_us']:.2f} | "
                f"{item['p95_latency_us']:.2f} | {item['operations_per_second']:.0f} | "
                f"{item['worker_cpu_ms']:.2f} | {item['voluntary_context_switches']}/{item['involuntary_context_switches']} | "
                f"{item['daemon_cpu_ms']:.0f} |"
            )
        lines.extend(["", "本次测量中：", ""])
        for decision in ("allow", "deny"):
            values = {item["backend"]: item for item in summaries if item["decision"] == decision}
            gain = values["lsm"]["operations_per_second"] / values["seccomp"]["operations_per_second"]
            reduction = 100 * (1 - values["lsm"]["mean_latency_us"] / values["seccomp"]["mean_latency_us"])
            lines.append(f"- {decision}：LSM 吞吐为 seccomp 的 {gain:.2f} 倍，平均延迟降低 {reduction:.1f}%。")
        if source == "wasm":
            lines.extend([
                "", "规则通过已安装的官方 `network-policy-dynamic` WIT component 加载和发布。",
                "两个后端的静态规则文件均为空，插件仅获得两条精确 endpoint 的 allow/deny grant。",
                "发布前验证空规则默认 allow；真实 Configuration Validate/Update 后，通过插件 dry-run",
                "核验 rule_id、decision、owner 和 source_revision，再启动真实 Xiaoo。", "",
                "| 后端 | 单次 Update 墙钟 ms |", "|---|---:|",
            ])
            for item in data["measurements"]:
                lines.append(f"| {item['backend']} | {item['policy_publication']['update_wall_ms']:.3f} |")
            lines.extend([
                "", "Update 时间包含 curl 进程、HTTP/Web、WASM、宿主校验与规则发布；每后端仅一次，",
                "不能解释为纯 WASM 执行耗时。访问计时排除了发布、agent 启动和预热。WASM 执行规则发布，",
                "已发布规则在 seccomp 用户态决策路径或 LSM 内核路径中执行；每次 connect 不再次调用 WASM。",
            ])
        lines.extend([
            "", f"每个后端真实 listener 均收到 {expected_connections} 个 allow 连接（含预热），deny listener 未收到连接。",
            "工作负载逐次验证 allow 成功、deny 返回 EPERM/EACCES，并检查真实 Xiaoo 及工具后代的 seccomp 状态。",
            "运行器停止自有 daemon、Web、provider 和 listener，默认清理临时工作目录；`--keep-work` 保留目录。", "",
            f"这些数据仅覆盖本机、{mechanism}的 IPv4 TCP connect 策略及 audit=false。它们包含 Python、",
            "本地 listener、观测和调度开销，不是单个 LSM hook 的独立成本；不能直接推算文件、exec、",
            "逐请求 WASM 回调、规则发布高并发或开启审计的性能。两后端顺序执行，各只有一个真实 agent 进程，短测量窗口的波动仍可能存在。", "",
            f"daemon CPU 来自 `/proc/PID/stat` 的 utime+stime，计数器精度为 {tick_description}。",
            "短轮次显示 0 ms 只表示计数器未跨 tick，不代表零成本；该窗口也未等待全部异步观测处理完成。",
            "context switches 仅统计工作进程自身，额外被动切换可能来自观测线程、listener 或其他调度竞争，",
            "不能仅凭切换次数把全部成本归因于控制机制。", "",
            "复现：", "", "```bash",
            f"python3 tests/v2/regression/lsm_access_control_xiaoo/performance/run.py run --policy-source {source} "
            f"--backends {' '.join(data['backend_order'])} --count {data['count_per_decision_per_round']} "
            f"--rounds {data['rounds']} --warmup {data['warmup_per_decision']}",
            f"python3 tests/v2/regression/lsm_access_control_xiaoo/performance/run.py report --input local/lsm_access_control/{self._source.name} "
            f"--output local/lsm_access_control/{self._destination.name}", "```", "",
            f"[原始数据及配置]({self._source.name}) · [测量脚本](../../tests/v2/regression/lsm_access_control_xiaoo/performance/run.py)", "",
        ])
        for item in data["measurements"]:
            log = item.get("xiaoo_log", f"benchmark-{item['backend']}.log")
            lines.append(f"[{item['backend']} Xiaoo 日志]({log})")
        self._destination.write_text("\n".join(lines) + "\n")
        print(self._destination.read_text())
