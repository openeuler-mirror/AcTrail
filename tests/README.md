# AcTrail Tests

测试按验证范围组织：

| 目录 | 职责 |
| --- | --- |
| `unit/` | 模块内部行为验证。Rust 测试通过原模块的 `#[cfg(test)]` 和 `#[path]` 接入，保留模块作用域。前端测试使用 Node 测试入口。 |
| `integration/` | 插件与宿主、存储后端、checkpoint 和 CLI 的集成验证。Rust 测试目标由所属 crate 的 Cargo.toml 声明。 |
| `e2e/` | 完整产品的端到端验证，公共代码位于 `e2e/common/`，回归入口位于 `e2e/regression/`。 |

测试素材跟随所属场景保存。`unit/` 镜像原代码路径并省略 Rust 或前端的 `src/` 层，主代码中的测试模块声明指向这些文件。

Rust 测试编译检查使用 `cargo check --release --tests -p <package>`。前端测试可以通过 `node --test tests/unit/crates/apps/web/frontend/api.test.mjs` 等具体文件路径运行。

本表记录 `tests/` 下主要测例的运行入口、测试目标和预期现象。默认先构建 release 产物：

```bash
cargo build --release
```

除特别说明外，E2E 测例需要在仓库根目录运行，并依赖真实 AcTrail release binaries。真实 agent/provider 测例还需要对应 CLI、凭据和网络环境；运行条件与结果判定见 [E2E 测试约定](e2e/README.md)。

| 测例项 | 运行指令 | 测试目标 | 预期现象 |
| --- | --- | --- | --- |
| Virtual-container deployment | `sudo -E python3 tests/e2e/regression/test_all.py --case virtual_container` | 按 E2E 规范汇总 guest bundle、systemd/rootfs、workload 接口及 StratoVirt/Cloud Hypervisor Kata E2E。 | 外部虚拟化条件缺失为 SKIPPED；AcTrail、部署契约或采集断言失败为 FAILED；完整矩阵通过为 PASSED。 |
| E2E 默认回归 | `sudo -E python3.11 tests/e2e/regression/test_all.py` | 运行默认回归集合。 | 汇总各测例的 PASSED、FAILED 或 SKIPPED 结果。 |
| E2E 测例清单 | `python3.11 tests/e2e/regression/test_all.py --list` | 查看可选测例。 | 输出测例清单。 |
| xiaoO LLM 观测 | `sudo -E python3.11 tests/e2e/regression/test_all.py --case probe_xiaoo_llm` | 运行真实 xiaoO 并验证 LLM 请求响应及关联证据。 | trace 正常结束，响应包含指定 marker，动作关联符合断言。 |
| Checkpoint Btrfs | `python3.11 tests/e2e/regression/test_all.py --case checkpoint_btrfs` | 在已配置 Rust 工具链的 root 终端运行，公共入口自动构建并定位 Btrfs backend。在回环 Btrfs 上验证 launch 初始保存、手动保存、删除、Trace 结束时释放快照、连接失败、每次 LLM 请求均在响应前保存（含文件未修改）、`force`、Web 的边界匹配和命令、运行中 Trace 在下一次 LLM 请求时恢复，以及 daemon 重启后历史仍可查询。 | 各步骤为 PASSED；缺少工具或 Btrfs 支持时为 SKIPPED，后端构建产物缺失时为 FAILED。 |
| LLM HTTP proxy support smoke | `python3 tests/e2e/common/llm-http-proxy/provider_proxy.py --mode local-stream` | 启动本地 OpenAI-compatible SSE provider shim，供仍在使用它的测例调用。 | 监听本地端口并返回 deterministic SSE；它是 support server，不单独证明 AcTrail capture。 |
