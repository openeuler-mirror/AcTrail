# AcTrail Tests

本表记录 `tests/` 下主要测例的运行入口、测试目标和预期现象。默认先构建 release 产物：

```bash
cargo build --release
```

除特别说明外，E2E 测例需要在仓库根目录运行，并依赖真实 AcTrail release binaries。真实 agent/provider 测例还需要对应 CLI、凭据和网络环境；依赖缺失时 regression runner 会按测例策略 `SKIP` 或 fail-fast。

| 测例项 | 运行指令 | 测试目标 | 预期现象 |
| --- | --- | --- | --- |
| Virtual-container deployment | `sudo -E python3 tests/v2/regression/test_all.py --case virtual_container` | 按 V2 规范汇总 guest bundle、systemd/rootfs、workload 接口及 StratoVirt/Cloud Hypervisor Kata E2E。 | 外部虚拟化条件缺失为 SKIPPED；AcTrail、部署契约或采集断言失败为 FAILED；完整矩阵通过为 PASSED。 |
| Regression quick suite | `python3 tests/regression/test_all.py` | 按 quick suite 汇总运行现有 regression case。 | 输出每个 case 的 PASS/SKIP/FAIL 汇总，并在 `/tmp/actrail-regression-*` 下生成 Markdown/JSON 报告。 |
| Regression case list | `python3 tests/regression/test_all.py --list` | 列出 regression runner 发现的所有 case、suite 归属和标题。 | 只打印 case 清单，不启动 daemon 或真实 workload。 |
| `e2e-xiaoo` | `python3 tests/regression/test_all.py --case e2e-xiaoo` | 验证新版 `xiaoo --cli run` 可用，并捕获 xiaoO 默认 provider LLM exchange。 | xiaoO availability marker 出现；HTTPS 路径使用 `TlsUserSpace`，plain HTTP 路径使用 `Syscall/socket-syscall`；payload、LLM actions、OTEL spans 完整。 |
| Agent trace: xiaoO default route | `python3 tests/agent-trace/run_case.py xiaoo-rustls` | 真实 xiaoO 默认 provider plaintext capture，覆盖新版 `xiaoo --cli run`。 | 输出 `ACTRAIL_XIAOO_RUSTLS_OK`；HTTPS evidence source 为 `TlsUserSpace`，plain HTTP evidence source 为 `Syscall/socket-syscall`；`xiaoo_llm_request_spans` 和 `xiaoo_llm_response_spans` 均非零。 |
| Fanotify enforcement direct | `python3 tests/enforcement/fanotify/run_e2e.py` | 直接生成临时 fanotify operator config，验证文件 permission enforcement。 | allowed path 读取成功，denied path 被拒绝；OTEL 中有 allow/deny spans。 |
| Concurrent launch: shell | `python3 tests/process/concurrent-launch/run_e2e.py --workload shell` | 在一个 daemon 下并发启动多个本地 shell workload，验证 active trace limit、trace lifecycle、stdout payload marker。 | 所有 trace 进入 Active/Clean 并完成；超过 active limit 的 track-add 被拒绝。 |
| Concurrent launch: xiaoO | `python3 tests/process/concurrent-launch/run_e2e.py --workload xiaoo --concurrency 2 --xiaoo-bin /root/projects/xiaoO/target/release/xiaoo` | 并发真实 xiaoO CLI workload，验证新版 `--cli run` 在多 trace 下的 capture 和 completion。 | 每个 xiaoO 输出对应 `ACTRAIL_XIAOO_N` marker；trace 完成且 outbound payload 中能查到 marker。 |
| File scan recording | `python3 tests/process/file-scan-recording/run_e2e.py` | 运行重复 `rg` 文件扫描，验证 canonical path set/chunk 复用，避免路径集合重复膨胀。 | trace 完成；SQLite 中 file path set/chunk 结构满足复用断言；输出 `file scan recording e2e passed`。 |
| Performance benchmark | `python3 tests/performance/run_benchmark.py --case all --mode all --output local/performance-benchmark.md` | 测量 baseline、daemon idle、eBPF core、payload、seccomp agent 等模式下的任务耗时分布。 | 生成 Markdown 报告，包含 median/p95、overhead、KS/Mann-Whitney/Hodges-Lehmann 统计和 raw timings。 |
| Performance single case | `python3 tests/performance/run_benchmark.py --case agent --mode baseline,observed-ebpf-payload --repetitions 30 --output local/performance-agent.md` | 对单个 workload/模式组合做可控重复测试。 | 报告只包含指定 case/mode；任一 run fail/timeout 时 benchmark 无效并 fail-fast。 |
| LLM HTTP proxy support smoke | `python3 tests/support/llm-http-proxy/provider_proxy.py --mode local-stream` | 启动本地 OpenAI-compatible SSE provider shim，供仍在使用它的测例调用。 | 监听本地端口并返回 deterministic SSE；它是 support server，不单独证明 AcTrail capture。 |
