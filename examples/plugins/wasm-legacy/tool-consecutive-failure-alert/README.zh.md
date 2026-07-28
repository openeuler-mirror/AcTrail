# WIT Component 同一工具连续失败告警插件

类别：WIT component 观测消费者。

这个示例实现按 trace 维护每个工具的连续失败状态，当同一工具连续失败次数超过配置阈值时生成告警。插件订阅 `semantic-action` 事件族，过滤 `CommandInvocation` 类型的语义动作，按 `(trace_id, 工具名)` 二元组独立维护计数器。

核心特性：

- 按 `(trace_id, tool_name)` 独立计数，不同 trace、不同工具互不影响
- 同一工具成功调用后计数器立即归零
- 告警触发后不重置计数器（持续告警直到成功或 trace 结束）
- 冷却时间控制，避免告警风暴
- TTL 状态回收，防止内存泄漏
- 支持工具过滤（monitored\_tools / ignored\_tools）

文件：

- `plugin.toml`：插件 manifest。
- `config.toml`：插件自己的 TOML 配置。
- `config.schema.json`：`schema_ref` 指向的 JSON Schema。
- `tool_consecutive_failure_alert.wasm`：已编译的 component artifact。
- `src/lib.rs`：Rust 源码。

### 工作原理

```
eBPF 采集 process.exec
        ↓
daemon 生成 CommandInvocation 语义动作
        ↓
工具名传播：LlmResponse.tool_calls_json → CommandInvocation.command.tool.name
        ↓
observation pipeline 批量发送给插件
        ↓
插件 consume batch，按 (trace_id, tool_name) 累计失败计数
        ↓
连续失败 >= 阈值 → alert_write::submit() → daemon AlertIngress
        ↓
TraceAlertToken 校验 → payload JSON Schema 校验 → SQLite 存储
```

插件只处理 `CommandInvocation` 类型的语义动作。成功/失败判定依据：

| 条件                   | 结果 |
| ---------------------- | ---- |
| `status = "success"`   | 成功 |
| `exit_code = "0"` 或空 | 成功 |
| 其他                   | 失败 |

#### 重新编译：

```bash
rustup target add wasm32-wasip2
cd examples/plugins/wasm-legacy/tool-consecutive-failure-alert
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/actrail_tool_consecutive_failure_alert.wasm .
```

### 插件侧：运行时参数

编辑 `config.toml`：

```toml
[alert]
consecutive_failure_threshold = 3   # 连续失败触发阈值（>=1）
cooldown_seconds = 60               # 同一 (trace, tool) 的冷却时间（秒）
tool_name_format = "full"           # 工具名格式：bare 或 full
desensitization = "summary_only"    # 脱敏策略

[alert.filter]
monitored_tools = []                # 监控的工具列表（空 = 全部）
ignored_tools = []                  # 忽略的工具列表

[alert.behavior]
state_ttl_seconds = 300             # 状态自动回收时间（秒）
policy_denied_counts_as_failure = true  # 策略拒绝是否计入失败
```

### 使用

~~~
#启动daemon
./target/release/actraild start    
#加载插件
./target/release/actraild plugin load   --manifest examples/plugins/wasm-legacy/tool-consecutive-failure-alert/plugin.toml   --plugin-config examples/plugins/wasm-legacy/tool-consecutive-failure-alert/config.toml   --instance tool-alert.test   --grant alert-write
#运行agent 
./target/release/actrailctl launch --name opencode-test -- opencode run "依次执行 ll /home/aa.txt;ll /home/bb.txt;ll /etc/aaa，告诉我结果"
~~~

### 查看告警结果

```
sqlite3 /var/lib/actrail/actrail.sqlite "SELECT a.*, d.title, d.severity_code
FROM alerts a
JOIN alert_definitions d ON a.alert_definition_id = d.alert_definition_id
ORDER BY a.created_at DESC
LIMIT 10;"
```

相关 ABI 说明见 [WIT Component 观测消费者示例](../../wit-component/observation-read-config/README.zh.md) 和 [观测消费者 ABI](../../../../docs/plugins/abi/observation-consumer.zh.md)。
