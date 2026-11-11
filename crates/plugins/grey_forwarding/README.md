# Grey 上下文转发插件

该 WASM 插件把待决动作交给所属运行时的 proxy 子进程。proxy 按 daemon 提供的逻辑进程 ID 和 trace 查询 SQLite 中近期的访问裁决与执行记录，再以 HTTP PUT 将增强请求发送给外部裁决服务。

## 构建

```sh
cargo build --release \
  -p daemon \
  -p grey-proxy
cargo build --release \
  --target wasm32-wasip2 \
  --target-dir target \
  --manifest-path crates/plugins/grey_forwarding/Cargo.toml
```

| 加载项 | 要求 |
|---|---|
| Proxy 程序 | 将 `actraild-grey-proxy` 放在 `actraild` 同一目录 |
| 插件产物 | `target/wasm32-wasip2/release/actrail_grey_forwarding.wasm` |
| 插件类型 | WIT component 控制插件 |
| HTTP 授权 | 授予 `http.request` |
| 命令上下文 | 声明 `command-execution.current-context-query` |
| `host.http.allowed_urls` | 在 manifest 中填写外部裁决服务地址 |
| HTTP 运行限制 | 在 manifest 中设置超时、请求大小和响应大小上限 |

## 插件配置

| 字段 | 必填 | 含义与范围 |
|---|---|---|
| `grey_proxy.endpoint` | 是 | 外部安全裁决服务地址 |
| `grey_proxy.window_ms` | 是 | 历史查询时间窗口，单位毫秒，必须大于 `0` |
| `grey_proxy.max_actions` | 是 | 历史记录数量上限，必须大于 `0` |

```json
{
  "grey_proxy": {
    "endpoint": "http://127.0.0.1:9000/analyze",
    "window_ms": 60000,
    "max_actions": 64
  }
}
```

| 配置内容 | 来源与生效方式 |
|---|---|
| 外部服务地址与历史范围 | 通过普通插件配置入口更新，重新加载实例后生效 |
| 数据库位置与 HTTP 限制 | 由运行时传入 |
| Proxy 本地连接地址 | 由运行时注入 |

## 裁决与历史

外部服务实现者请阅读 [Grey 外部安全裁决服务协议](../../../docs/reference/protocols/grey-decision-service.md)，其中包含完整字段定义、请求响应示例和故障处理约定。

外部服务收到 `decision_id`、`operation`、`target` 和 `history`。当前待决动作为命令执行时，还提供完整 `argv`。进程和 trace 标识用于 proxy 本地查询。

`history.actions` 包含时间窗口内最新的至多 `max_actions` 条访问裁决和进程执行记录，按时间倒序排列。proxy 先通过索引选取有限记录 ID，再批量读取这些记录的详细属性。原时间线简略查询保持独立。

| 历史记录 | 字段与表示 |
|---|---|
| 访问裁决 | `time`、`operation`、`target`、`decision`，网络裁决可附带 `protocol` |
| 进程执行 | `time`、`operation: "exec"`、`command`。命令文本由采集参数用空格连接形成 |
| 执行参数截断 | 附带 `parameters_truncated` |
| 执行参数缺失 | 提供执行目标和 `parameters_unavailable` |
| 执行失败或结果未知 | 附带 `result`，区分执行成功与未成功的尝试 |

### 历史采集配置

| 配置位置 | 字段 | 作用 |
|---|---|---|
| 所用 capture profile | `capabilities` | 加入 `proc-exec-context`，启用执行参数采集 |
| Daemon 配置 | `process_seccomp.max_args` | 执行参数数量上限 |
| Daemon 配置 | `process_seccomp.max_arg_bytes` | 单个执行参数的字节上限 |
| Daemon 配置 | `process_seccomp.max_total_arg_bytes` | 执行参数总字节上限 |

敏感访问规则需启用对应领域的审计记录。

### 查询与裁决结果

历史查询使用数据库已有记录。

| 条件 | 结果 |
|---|---|
| 数量或字节上限导致截断 | `history.truncated` 标记截断 |
| 身份缺失或读取失败 | `history.error` 说明原因 |
| 外部服务放行 | 以成功 HTTP 状态返回 `{"verdict":"allow"}`，可附带 `reason` 字符串 |
| 外部服务拒绝 | 以成功 HTTP 状态返回 `{"verdict":"deny"}`，可附带 `reason` 字符串 |
| 传输失败或无效裁决 | 接入现有 grey 故障处理 |

## 运行与退出

| 项目 | 行为 |
|---|---|
| 进程归属 | 每个插件实例拥有一个 proxy |
| 请求处理 | 按接收顺序处理 |
| 插件等待 | 受现有 grey 和 HTTP 限制约束 |
| 外部调用 | 受 manifest HTTP 超时约束 |
| 卸载插件 | 运行时终止并回收 proxy |
| daemon 异常退出 | Proxy 检测所属管道关闭后退出 |
