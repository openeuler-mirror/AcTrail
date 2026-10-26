# AcTrail Tests

本表记录 `tests/` 下主要测例的运行入口、测试目标和预期现象。默认先构建 release 产物：

```bash
cargo build --release
```

除特别说明外，E2E 测例需要在仓库根目录运行，并依赖真实 AcTrail release binaries。真实 agent/provider 测例还需要对应 CLI、凭据和网络环境；运行条件与结果判定见 [V2 测试约定](v2/README.md)。

| 测例项 | 运行指令 | 测试目标 | 预期现象 |
| --- | --- | --- | --- |
| Virtual-container deployment | `sudo -E python3 tests/v2/regression/test_all.py --case virtual_container` | 按 V2 规范汇总 guest bundle、systemd/rootfs、workload 接口及 StratoVirt/Cloud Hypervisor Kata E2E。 | 外部虚拟化条件缺失为 SKIPPED；AcTrail、部署契约或采集断言失败为 FAILED；完整矩阵通过为 PASSED。 |
| V2 默认回归 | `sudo -E python3.11 tests/v2/regression/test_all.py` | 运行默认回归集合。 | 汇总各测例的 PASSED、FAILED 或 SKIPPED 结果。 |
| V2 测例清单 | `python3.11 tests/v2/regression/test_all.py --list` | 查看可选测例。 | 输出测例清单。 |
| xiaoO LLM 观测 | `sudo -E python3.11 tests/v2/regression/test_all.py --case probe_xiaoo_llm` | 运行真实 xiaoO 并验证 LLM 请求响应及关联证据。 | trace 正常结束，响应包含指定 marker，动作关联符合断言。 |
| Fanotify enforcement direct | `python3 tests/enforcement/fanotify/run_e2e.py` | 直接生成临时 fanotify operator config，验证文件 permission enforcement。 | allowed path 读取成功，denied path 被拒绝；OTEL 中有 allow/deny spans。 |
| LLM HTTP proxy support smoke | `python3 tests/v2/common/llm-http-proxy/provider_proxy.py --mode local-stream` | 启动本地 OpenAI-compatible SSE provider shim，供仍在使用它的测例调用。 | 监听本地端口并返回 deterministic SSE；它是 support server，不单独证明 AcTrail capture。 |
