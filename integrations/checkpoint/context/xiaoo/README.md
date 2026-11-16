# Xiaoo 上下文 checkpoint 集成

该集成通过 Xiaoo 本地 TUI 的 Unix socket 操作上下文快照，编译产物为 `libactrail_ckpt_cxt_xiaoo.so`。调用方使用 `contracts/checkpoint` 中的统一 ABI 对象。

## 构建与配置

在 AcTrail 仓库根目录构建：

```bash
cargo build --release -p actrail_ckpt_cxt_xiaoo
```

产物位于 Cargo 输出目录，默认路径是 `target/release/libactrail_ckpt_cxt_xiaoo.so`。

在 Xiaoo 配置中启用 checkpoint socket：

```toml
[checkpoint]
enabled = true

[checkpoint.unix_socket]
endpoint = "/tmp/xiaoo-checkpoint.sock"
```

在 daemon 的 `[[checkpoint.backends]]` 中，用 `config` 指定集成的运行配置 JSON。使用全部默认值时可写为 `{}`：

```json
{
  "request_timeout_ms": 5000,
  "max_response_bytes": 65536,
  "restore_mode": "promise_mode"
}
```

为每个 Xiaoo 任务准备 Trace 私有 JSON，其 `endpoint` 与该任务的 Xiaoo socket 相同。在 launch checkpoint 配置的 `[[backends]]` 中，用 `config` 指定这个文件的绝对路径：

```json
{"endpoint": "/tmp/xiaoo-checkpoint.sock"}
```

集成声明 `BackendKind::Context`，由 launch checkpoint 配置的 `backends` 选择，不配置文件系统 scope。

daemon 启动时读取运行配置并保留原文。多个任务共用 daemon 中已加载的动态库和运行配置，各自提供 Trace 配置。

创建入口接收 `BackendConfig.runtime_json` 和 `BackendConfig.trace_config_path`。集成读取 Trace 文件，分别解析两份配置并创建实例。配置有误时，本次实例创建失败。

| 配置项 | 类型 | 默认值 | 必填 | 所属模块 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `endpoint` | string | 无 | 是 | Xiaoo checkpoint Trace 配置 | Xiaoo checkpoint socket 的绝对路径 |
| `request_timeout_ms` | uint64 | 5000 | 否 | Xiaoo checkpoint 运行配置 | 一次 RPC 的连接、写入和读取共用的超时，单位为毫秒，必须大于 0 |
| `max_response_bytes` | uint | 65536 | 否 | Xiaoo checkpoint 运行配置 | 单条响应含换行符的最大字节数，必须大于 0 |
| `restore_mode` | string | `promise_mode` | 否 | Xiaoo checkpoint 运行配置 | `promise_mode` 接受已排队或已保存的快照，`ensure_safe_mode` 要求快照已经保存 |

## 操作与生命周期

| contract 操作 | Xiaoo 请求 | 行为 |
| --- | --- | --- |
| `connect(context)` | `init` | 指定 `current`，获取 session，并绑定调用方的 `trace_id` |
| `snapshot()` | `save` | 返回封装 session ID 与快照 ID 的 `SnapshotId` |
| `restore(snapshot_id)` | `restore` | 向绑定 session 提交恢复请求 |
| `delete(snapshot_id)` | `delete_snapshot` | 删除指定快照 |
| `disconnect()` | `release` | 结束该 session 的 checkpoint 对接 |

### 实例绑定与快照标识

每个后端实例绑定一个 trace 和一个 Xiaoo session。`connect` 接收完整的 `SnapshotContext`，以 `trace_id` 绑定 session。不同任务使用各自的 socket 和后端实例。

集成在 `connect` 时发送 `{"op":"init","config":{"save_mode":"current"}}`，为本次对接选择 `current` 捕获方式。它在保存请求到达时选择最近一个完整上下文边界。

Xiaoo 需要支持 `init.config.save_mode`。每次连接时，请求中的 `current` 覆盖 Xiaoo 启动配置里的快照模式。

`disconnect` 成功后清除本地绑定。

上下文及快照数据由 Xiaoo 存储。集成封装 session ID 与快照 ID，形成不透明的 `SnapshotId`。调用方原样保存并传回这个标识，内容由集成解释。

### 组合调用时机

通过 daemon launch 组合文件系统 backend 时，启动阶段连接文件系统并记录 Trace。首个 LLM 请求边界调用本集成的 `connect`，随后放行 Agent，该边界跳过保存。Agent 发布完整上下文后，后续边界和手动保存创建包含文件与上下文的 checkpoint。

手动保存会先连接尚未连接的上下文 backend。Xiaoo 尚无完整上下文时，保存返回失败。

组合恢复先在进程暂停期间恢复文件，再继续进程并调用本集成的 `restore`。此时 daemon 保持 LLM 响应等待，让 Xiaoo 的 socket 服务处理请求。上下文请求成功后放行响应，失败时 daemon 再次暂停进程。

接入配置见[Xiaoo 快照配置](../../../../docs/operations/checkpoint.md#xiaoo)。

### 请求结果

成功结果表示 Xiaoo 已受理请求。排队操作由 Xiaoo 在循环边界处理。

`restore` 成功返回空的失效快照列表，失败通过 `RestoreFailure.error` 返回错误。其他操作的错误通过 `BackendError` 返回。

每次请求建立一个 socket 连接。超时或连接中断时，客户端返回错误，请求不会自动重发。

## 真实 Agent 验证

先构建动态库和 ABI 驱动程序：

```bash
cargo build --release \
  -p actrail_ckpt_cxt_xiaoo \
  --lib \
  --example xiaoo_checkpoint_e2e
```

测试需要 `tmux`、Python 3、支持 checkpoint socket 的 Xiaoo，以及可用的 DeepSeek 凭证。默认从 `~/.config/xiaoo` 读取凭证文件位置，可通过 `--secrets-dir` 指定其他目录。

```bash
python3 tests/integration/checkpoint/context/xiaoo/run_e2e.py \
  --xiaoo /absolute/path/to/xiaoo
```

脚本创建独立的最小配置，使用当前 Xiaoo 的默认值，并启用 DeepSeek 和 checkpoint socket。启动配置保留默认的快照模式，通过集成初始化选择 `current`。

验证时启动真实 TUI，通过动态库执行五个接口，用实际对话和导出的消息检查恢复结果。

测试结束后清理临时配置、凭证副本、TUI 会话和本次消息导出文件。

通过 daemon、TLS-sync 和 Btrfs 验证组合保存与恢复，运行[组合端到端脚本](../../../../tests/e2e/regression/checkpoint_composite/run_e2e.py)：

```bash
XIAOO_BINARY=/absolute/path/to/xiaoo \
  python3.11 tests/e2e/regression/checkpoint_composite/run_e2e.py
```
