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

集成在 `connect` 时发送 `{"op":"init","config":{"save_mode":"current"}}`，为本次对接选择 `current` 捕获方式。它在保存请求到达时选择最近一个完整上下文边界。

启动本地 Xiaoo TUI 后，将同一个 socket 路径传给集成的创建入口。`BackendConfig.json` 的内容如下：

```json
{
  "endpoint": "/tmp/xiaoo-checkpoint.sock",
  "request_timeout_ms": 5000,
  "max_response_bytes": 65536,
  "restore_mode": "promise_mode"
}
```

| 配置项 | 类型 | 默认值 | 必填 | 所属模块 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `endpoint` | string | 无 | 是 | Xiaoo checkpoint 集成 | Xiaoo checkpoint socket 的绝对路径 |
| `request_timeout_ms` | uint64 | 5000 | 否 | Xiaoo checkpoint 集成 | 一次 RPC 的连接、写入和读取共用的超时，单位为毫秒，必须大于 0 |
| `max_response_bytes` | uint | 65536 | 否 | Xiaoo checkpoint 集成 | 单条响应含换行符的最大字节数，必须大于 0 |
| `restore_mode` | string | `promise_mode` | 否 | Xiaoo checkpoint 集成 | `promise_mode` 接受已排队或已保存的快照，`ensure_safe_mode` 要求快照已经保存 |

Xiaoo 需要支持 `init.config.save_mode`。初始化请求中的 `current` 优先于 Xiaoo 启动配置，每次重新连接都会显式指定。

## 操作与生命周期

| contract 操作 | Xiaoo 请求 | 行为 |
| --- | --- | --- |
| `connect(context)` | `init` | 指定 `current`，获取 session，并绑定调用方的 `trace_id` |
| `snapshot()` | `save` | 返回封装 session ID 与快照 ID 的 `SnapshotId` |
| `restore(snapshot_id)` | `restore` | 向绑定 session 提交恢复请求 |
| `delete(snapshot_id)` | `delete_snapshot` | 删除指定快照 |
| `disconnect()` | `release` | 结束该 session 的 checkpoint 对接 |

每个后端实例绑定一个 trace 和一个 Xiaoo session。不同 Agent 使用各自的 socket 和后端实例。快照 ID 仅由该集成解释，调用方应原样保存并传回。上下文及快照存储由 Xiaoo 管理。

成功结果沿用 Xiaoo 的受理语义。排队操作由 Xiaoo 在循环边界处理。每次请求建立一个 socket 连接。超时或连接中断通过 `BackendError` 返回，客户端不会自动重发。`disconnect` 成功后清除本地绑定，调用方可再次 `connect`。

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
python3 integrations/checkpoint/context/xiaoo/tests/run_e2e.py \
  --xiaoo /absolute/path/to/xiaoo
```

脚本创建独立的最小配置，使用当前 Xiaoo 的默认值，并启用 DeepSeek 和 checkpoint socket。启动配置保留默认的快照模式，通过集成初始化选择 `current`。它启动真实 TUI，通过动态库执行五个接口，用实际对话和导出的消息验证恢复结果。测试结束后清理临时配置、凭证副本、TUI 会话和本次消息导出文件。
