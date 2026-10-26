# 校验记录

本文件记录各项校验的触发条件、检查内容、失败行为和用户许可状态。每个编号对应一项许可决定。

- `[?]`：校验已存在，明确许可来源待确认。
- `[x]`：用户明确拒绝。
- `[√]`：用户明确允许。
- `[ ]`：尚未实现，待用户确认。

用户决定记录在对应编号下，写明适用范围和来源。

## 配置与资源

采集配置（profile）中的 Required 表示必需能力，Opportunistic 表示尽力提供的能力，Disabled 表示禁用。下文“请求能力”包括 Required 和 Opportunistic，“必需能力”只包括 Required。

访问控制的“有效操作”指文件、命令、网络治理总开关与各操作开关合并后的结果。合并入口见 [operator 文档](../../../crates/core/config/src/daemon/operator/document.rs#L502)，规则见 [访问控制配置](../../../crates/core/config/src/daemon/access_control/config.rs#L177)。

### 1. 采集能力列表非空

状态：`[?]`。

- 触发：调用 `OperatorDocument::validate`，且前面的资源配置解析和资源能力一致性检查已通过。
- 检查：`capture.capabilities` 必须非空。此项检查的是必需能力列表本身。`capture.opportunistic_capabilities` 或 `capture.disabled_capabilities` 非空不能满足该条件。
- 失败：返回 `ConfigError("capture.capabilities must not be empty")`。当前配置读取或序列化失败。
- 源码：[document.rs:199](../../../crates/core/config/src/daemon/operator/document.rs#L199)。读取和序列化都会调用 `validate`，调用点见 [framework.rs:54](../../../crates/core/config/src/framework.rs#L54) 和 [framework.rs:61](../../../crates/core/config/src/framework.rs#L61)。

### 2. 资源指标能力的采集来源

状态：`[?]`。

- 触发：调用 `OperatorDocument::validate`，资源子配置已成功解析，且 `capture.capabilities` 中含精确字符串 `resource-metrics`。
- 检查：解析后的 `resource_profiling.enabled` 与 `resource_profiling.collection.workload.enabled` 必须同时为 true。仅启用 Host 采集不能满足该条件。本检查不查询 opportunistic 或 disabled 能力列表。
- 失败：返回说明资源指标能力与工作负载采集关闭相冲突的 `ConfigError`。当前配置读取或序列化失败。
- 源码：[document.rs:184](../../../crates/core/config/src/daemon/operator/document.rs#L184)，判断位于 188–197 行。`workload_enabled` 的定义见 [resource_profiling_config.rs:80](../../../crates/core/resource_profiling/src/config/resource_profiling_config.rs#L80)。

### 3. seccomp 通知服务依赖

状态：`[?]`。

- 触发与检查：下列任一条件成立时，要求 `seccomp_notify.enabled=true`：
  1. `payload.tls.enabled=true`，且 TLS 后端为 `seccomp-user-read` 或 `bpf-copy-seccomp-fallback`。
  2. `payload.socket.enabled=true`，且 socket 后端为 `bpf-copy-seccomp-fallback`。
  3. `process_seccomp.enabled=true`。
  4. `enforcement.enabled=true`，且有效文件 mkdir 或 rmdir 操作启用 seccomp 后端。
  5. profile 请求 `enforcement-command-execution-seccomp`。
  6. profile 请求 `enforcement-network-connect-seccomp`。
- 失败：`validate_seccomp_config` 返回指出对应使用方缺少通知服务的 `Err(String)`。operator 配置构造失败。
- 源码：[operator.rs:472](../../../crates/core/config/src/daemon/operator.rs#L472)、[operator.rs:505](../../../crates/core/config/src/daemon/operator.rs#L505)、[operator.rs:525](../../../crates/core/config/src/daemon/operator.rs#L525)。后端范围见 [payload.rs:120](../../../crates/core/config/src/daemon/payload.rs#L120) 和 [payload.rs:154](../../../crates/core/config/src/daemon/payload.rs#L154)。文件 seccomp 操作集合见 [access_control/config.rs:88](../../../crates/core/config/src/daemon/access_control/config.rs#L88)。

### 4. 治理操作的进程身份来源

状态：`[?]`。

- 触发与检查：
  1. profile 请求 `enforcement-command-execution-seccomp` 时，必须请求 `proc-lifecycle`，其模式可以为 Required 或 Opportunistic。
  2. profile 请求 `enforcement-file-permission-fanotify` 时，必须请求 `proc-lifecycle`，其模式可以为 Required 或 Opportunistic。
  3. 任一有效文件 open、mkdir、rmdir、命令 exec 或网络 connect 操作使用 LSM，或者有效文件 mkdir、rmdir 操作使用 seccomp 时，`proc-lifecycle` 必须以 Required 模式声明。
- 失败：对应函数返回说明缺失进程生命周期能力或其 Required 声明的 `Err(String)`。operator 配置构造失败。
- 源码：[operator.rs:511](../../../crates/core/config/src/daemon/operator.rs#L511)、[operator.rs:634](../../../crates/core/config/src/daemon/operator.rs#L634)、[access_control/config.rs:278](../../../crates/core/config/src/daemon/access_control/config.rs#L278)。LSM 有效操作集合见 [access_control/config.rs:187](../../../crates/core/config/src/daemon/access_control/config.rs#L187) 和 [access_control/config.rs:85](../../../crates/core/config/src/daemon/access_control/config.rs#L85)。

### 5. 应用协议开关依赖

状态：`[?]`。

- 触发：operator 构造配置时调用 `validate_application_protocol_config`。
- 检查：`application.http1_enabled=true` 要求 `application.enabled=true`。`application.http2_enabled=true` 要求 `application.enabled=true`。`application.http.sse_enabled=true` 要求 `application.http1_enabled=true`。`application.http.sse_data_policy=preview` 要求 `application.http.sse_enabled=true`。
- 失败：返回指出首个不成立开关依赖的 `Err(String)`。operator 配置构造失败。
- 源码：[operator.rs:539](../../../crates/core/config/src/daemon/operator.rs#L539)。

### 6. HTTP 观测的解析器与明文来源

状态：`[?]`。

- 触发：profile 请求 `net-application-plaintext-http` 或 `net-application-http2-frames`。
- 检查：HTTP/1 能力要求 `application.enabled=true` 且 `application.http1_enabled=true`。HTTP/2 能力要求 `application.enabled=true` 且 `application.http2_enabled=true`。两类能力均要求同一 profile 具备以下至少一种来源：`payload.tls.enabled=true` 且请求 `tls-plaintext-payload`，或 `payload.socket.enabled=true` 且请求 `socket-plaintext-payload`。来源能力为 Required 或 Opportunistic 均满足本检查。
- 失败：返回指出应用协议开关缺失或明文来源缺失的 `Err(String)`。operator 配置构造失败。
- 源码：[operator.rs:563](../../../crates/core/config/src/daemon/operator.rs#L563)。

### 7. MCP 标准输入投影的连接身份

状态：`[?]`。

- 触发：`semantic_retention.projection_enabled`、`payload.mcp.enabled`、`payload.stdio.enabled` 和 `payload.stdio.capture_stdin` 同时为 true，且 profile 请求 `stdio-chunk`。
- 检查：上述组合要求 `ebpf.ipc_lineage.enabled=true`，以提供 stdio bundle 身份。如果 IPC lineage 已启用，函数直接返回成功。上述条件未全部成立时不要求启用 IPC lineage。
- 失败：返回 `MCP stdio projection requires ebpf.ipc_lineage.enabled=true for stdio bundle identity` 的 `Err(String)`。operator 配置构造失败。
- 源码：[operator.rs:600](../../../crates/core/config/src/daemon/operator.rs#L600)。

### 8. LSM 策略容量与路径长度

状态：`[?]`。

- 触发：至少一个有效文件、命令或网络操作启用 LSM 后端。
- 检查：`access_control.backends.lsm.policy_max_entries >= 5`。`access_control.backends.lsm.path_max_bytes` 必须位于闭区间 `[2, 252]`。
- 失败：`AccessControlConfig::validate` 返回说明策略条目下限或路径范围不成立的 `Err(String)`。operator 配置构造失败。
- 源码：[access_control/config.rs:192](../../../crates/core/config/src/daemon/access_control/config.rs#L192)。

### 9. LSM 审计缓冲区与批次

状态：`[?]`。

- 触发：至少一个有效文件、命令或网络操作启用 LSM 后端。
- 检查：`access_control.backends.lsm.audit_batch_max > 0`。`access_control.backends.lsm.audit_buffer_bytes` 必须是二次幂且不小于 65536。
- 失败：`AccessControlConfig::validate` 返回说明审计批次为零或缓冲区范围不成立的 `Err(String)`。operator 配置构造失败。
- 源码：[access_control/config.rs:200](../../../crates/core/config/src/daemon/access_control/config.rs#L200)、[access_control/config.rs:208](../../../crates/core/config/src/daemon/access_control/config.rs#L208)。

### 10. LSM 告警窗口与容量

状态：`[?]`。

- 触发：至少一个有效文件、命令或网络操作启用 LSM 后端。
- 检查：`access_control.backends.lsm.alert_window_ms > 0`，且 `access_control.backends.lsm.alert_max_groups > 0`。
- 失败：`AccessControlConfig::validate` 返回说明告警窗口和组数必须为正的 `Err(String)`。operator 配置构造失败。
- 源码：[access_control/config.rs:192](../../../crates/core/config/src/daemon/access_control/config.rs#L192)。

### 11. 治理能力与操作后端匹配

状态：`[?]`。

- 触发：operator 构造治理配置并调用组合校验和 `AccessControlConfig::validate`。
- 检查：profile 请求下表任一能力时，对应有效操作必须启用并使用匹配后端。“反向要求 Required”为“是”的行，在操作已选中时，还必须将对应能力以 Required 模式声明。完整映射如下：

| 能力 | 必须选中的有效操作及后端 | 反向要求 Required |
| --- | --- | --- |
| `enforcement-file-permission-fanotify` | 文件 open 使用 fanotify | 否 |
| `enforcement-file-mkdir-seccomp` | 文件 mkdir 使用 seccomp | 是 |
| `enforcement-file-rmdir-seccomp` | 文件 rmdir 使用 seccomp | 是 |
| `enforcement-file-permission-lsm` | 文件 open、mkdir、rmdir 中至少一个使用 LSM | 是 |
| `enforcement-command-execution-seccomp` | 命令 exec 使用 seccomp | 否 |
| `enforcement-command-execution-lsm` | 命令 exec 使用 LSM | 是 |
| `enforcement-network-connect-seccomp` | 网络 connect 使用 seccomp | 否 |
| `enforcement-network-connect-lsm` | 网络 connect 使用 LSM | 是 |

组合函数还显式检查三个总开关：请求 `enforcement-command-execution-seccomp` 要求 `command_control.enabled=true`。请求 `enforcement-network-connect-seccomp` 要求 `network_control.enabled=true`。请求 `enforcement-file-permission-fanotify` 要求 `enforcement.enabled=true`。这些检查与有效操作路由检查共同保证能力声明有对应的执行方。

- 失败：返回指出治理总开关未开启、操作后端未选中或缺少 Required 能力声明的 `Err(String)`。operator 配置构造失败。
- 源码：[access_control/config.rs:215](../../../crates/core/config/src/daemon/access_control/config.rs#L215)。总开关检查见 [operator.rs:495](../../../crates/core/config/src/daemon/operator.rs#L495)、[operator.rs:518](../../../crates/core/config/src/daemon/operator.rs#L518)、[operator.rs:627](../../../crates/core/config/src/daemon/operator.rs#L627)。

### 12. 资源样本清理参数

状态：`[?]`。

- 触发：解析被选中的资源 SQLite 存储配置，且该配置的 `retention.enabled=true`。适用于 `resource_profiling.storage.database.sqlite.retention` 和 `resource_profiling.storage.hybrid.backends[].database.sqlite.retention`。
- 检查：`max_age_seconds > 0`、`sweep_interval_seconds > 0`、`max_samples_per_sweep > 0`。`retention.enabled=false` 时，本方法允许这三个字段为零。
- 失败：返回 `resource SQLite retention values must be positive` 的 `Err(String)`。该资源 SQLite 配置解析失败，错误沿当前配置构造调用链返回。
- 源码：[resource_retention_config.rs:21](../../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_retention_config.rs#L21)，调用点见 [resource_storage_sqlite_config.rs:31](../../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_storage_sqlite_config.rs#L31)。

## 插件

### 13. 插件身份与 API 版本

状态：`[?]`。

- 触发：清单进入 `validate_loadable`。
- 检查：`general.id` 去除首尾空白后非空，`general.api_version` 必须为 `actrail.plugin.v2`。
- 失败：返回清单错误。daemon 加载入口将错误标为 `plugin_manifest`，停止本次加载。
- 源码：[身份与版本检查](../../../crates/core/plugin_system/src/manifest/validation.rs#L13)、[插件加载入口](../../../crates/apps/daemon/src/services/attach/plugins.rs#L87)。

### 14. 插件运行时声明

状态：`[?]`。

- 触发：清单检查运行时选择。
- 检查：选择 Wasm 时存在 `runtime.wasm`，且其 `artifact_path` 去除首尾空白后非空。选择 native-dylib 时存在对应运行时段。未使用运行时段策略为 deny 时，清单不得包含未选中的 wasm、builtin 或 native-dylib 段。选择 builtin 时没有必填运行时段检查。
- 失败：返回清单错误。停止本次加载。策略为 warn 时返回未使用段警告并允许后续检查继续。
- 源码：[validation.rs:140](../../../crates/core/plugin_system/src/manifest/validation.rs#L140)、[validation.rs:513](../../../crates/core/plugin_system/src/manifest/validation.rs#L513)、[validation.rs:528](../../../crates/core/plugin_system/src/manifest/validation.rs#L528)。

### 15. 插件角色的运行条件

状态：`[?]`。

- 触发：清单检查角色段。
- 检查：observation-consumer 不得声明 sandbox-observation-consumer 或 control-decider 段。显式提供的事件族列表非空且无重复。声明 post-trace 时，delivery 为 trace-consistent、运行时为 Wasm WIT component，并订阅 semantic-action 和 trace-lifecycle。sandbox-observation-consumer 必须有对应角色段、使用 builtin，且不得声明 observation-consumer 或 control-decider。observation_kinds 非空且无重复。alert-consumer 使用 builtin、host capabilities 为空，不得声明 observation-consumer、sandbox-observation-consumer 或 control-decider。control-decider 不得声明 observation-consumer 或 sandbox-observation-consumer。llm-codec 使用 Wasm，不得声明 observation-consumer、sandbox-observation-consumer 或 control-decider。
- 失败：返回首个角色条件错误。停止本次加载。
- 源码：[validation.rs:179](../../../crates/core/plugin_system/src/manifest/validation.rs#L179)。

### 16. 插件宿主能力去重

状态：`[?]`。

- 触发：清单检查 host capabilities。
- 检查：`host.capabilities` 各项唯一。
- 失败：返回清单错误。停止本次加载。
- 源码：[validation.rs:327](../../../crates/core/plugin_system/src/manifest/validation.rs#L327)。

### 17. 插件运行时配置管理条件

状态：`[?]`。

- 触发：`plugin_config.runtime_managed=true`。
- 检查：角色为 control-decider。运行时为 Wasm WIT component。`schema_ref` 已提供。配置 format 为 json。control-decider concurrency_limit 为 1。此入口只判断 schema_ref 是否提供，空字符串由配置管理器处理。
- 失败：返回清单错误。停止本次加载。
- 源码：[validation.rs:32](../../../crates/core/plugin_system/src/manifest/validation.rs#L32)。

### 18. 插件告警输出条件

状态：`[?]`。

- 触发：清单包含至少一个 `outputs.alerts` 定义。
- 检查：具有 alert-write capability。角色为 observation-consumer。运行时为 Wasm WIT component。每项告警的定义 key、kind、title、payload_schema_id、payload_schema_ref 去除首尾空白后非空。
- 失败：返回清单错误。停止本次加载。
- 源码：[validation.rs:335](../../../crates/core/plugin_system/src/manifest/validation.rs#L335)。

### 19. 插件资源预算

状态：`[?]`。

- 触发：清单检查资源限额，相关可选字段已填写。
- 检查：已填写的下列预算字段必须大于零。花括号列出同一路径前缀下的全部受检字段。

| 字段范围 |
| --- |
| `runtime.wasm.resources.{fuel_per_call,memory_max_bytes}` |
| `role.control-decider.resources.concurrency_limit` |
| `role.observation-consumer.resources.queue_capacity` |
| `role.sandbox-observation-consumer.resources.queue_capacity` |
| `hostcall_limits.env.{name_max_bytes,value_max_bytes}` |
| `hostcall_limits.payload.{segment_max_count,ref_max_bytes,read_max_bytes}` |
| `hostcall_limits.context.{ref_max_bytes,query_max_bytes,read_max_bytes}` |
| `hostcall_limits.file_policy.{context_ref_max_bytes,query_max_bytes,read_max_bytes}` |
| `hostcall_limits.command_policy.read_max_bytes` |
| `hostcall_limits.command_context.{context_ref_max_bytes,query_max_bytes,read_max_bytes}` |
| `hostcall_limits.network_policy.{context_ref_max_bytes,query_max_bytes,read_max_bytes}` |
| `hostcall_limits.plugin_config.read_max_bytes` |
| `hostcall_limits.plugin_command.{argv_max_count,arg_max_bytes,output_max_bytes,timeout_ms}` |
| `hostcall_limits.trace_analysis.{action_page_max_count,action_total_max_count}` |
| `hostcall_limits.trace_activity.{page_max_count,total_max_count}` |
| `hostcall_limits.trace_file_state.{query_max_count,timeout_ms}` |
| `hostcall_limits.alert.payload_max_bytes` |

- 失败：返回首个零值字段错误。停止本次加载。未填写的可选字段通过本项检查。
- 源码：[validation.rs:364](../../../crates/core/plugin_system/src/manifest/validation.rs#L364)、[validation.rs:551](../../../crates/core/plugin_system/src/manifest/validation.rs#L551)、[validation.rs:558](../../../crates/core/plugin_system/src/manifest/validation.rs#L558)。

### 20. 插件实例的配置编辑条件

状态：`[?]`。

- 触发：daemon 调用 `validate`、`validate_runtime` 或准备配置更新。
- 检查：指定实例已登记。实例同时具有 JSON Schema 和配置文件路径。
- 失败：实例缺失返回 `plugin_not_found`。缺少编辑条件返回 `plugin_config_read_only`。本次校验或准备更新停止。
- 源码：[plugin_config.rs:134](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L134)、[plugin_config.rs:149](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L149)、[plugin_config.rs:171](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L171)、[plugin_config.rs:197](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L197)、[plugin_config.rs:226](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L226)、[plugin_config.rs:370](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L370)。

### 21. 插件配置格式与 Schema

状态：`[?]`。

- 触发：初始加载。运行时配置提交到管理器。校验候选配置。准备更新。
- 检查：初始加载声明 schema_ref 时，引用字符串去除首尾空白后非空，文件可读，内容可解析为 JSON 并编译为 JSON Schema。提供初始配置内容时，按声明的 json 或 toml 解析，并在存在 Schema 时检查配置值。候选配置必须为 JSON 并符合 Schema。运行时返回的 current_json 必须为 JSON。Schema 实例错误最多收集 8 项。
- 失败：加载阶段的读取、格式或 Schema 编译错误返回 `plugin_config` 并停止本次加载。Schema 不匹配返回 `plugin_config_validation`。显式校验时，候选 JSON 解析或 Schema 实例错误形成 `valid=false` 及 errors。当前运行时 JSON 错误直接返回 `plugin_config`。准备更新时错误列表转成 `plugin_config_validation`。`commit_runtime_config` 失败时不执行 `loaded.config = config`。
- 源码：[plugin_config.rs:37](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L37)、[plugin_config.rs:112](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L112)、[plugin_config.rs:134](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L134)、[plugin_config.rs:149](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L149)、[plugin_config.rs:197](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L197)、[plugin_config.rs:235](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L235)、[plugin_config.rs:282](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L282)、[plugin_config.rs:328](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L328)、[plugin_config.rs:410](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L410)。

### 22. 插件配置只读字段保护

状态：`[?]`。

- 触发：候选 JSON 解析成功后执行 `validation_errors`。
- 检查：按 Schema 的 properties 递归遍历。遇到 `readOnly=true` 时比较当前值与候选值。值不同记入只读错误。缺失属性按 JSON null 比较。遇到 readOnly 节点后停止深入该节点。此实现按 properties 递归，不遍历数组 items。
- 失败：只读错误加入 errors。显式校验返回 `valid=false`。准备更新返回 `plugin_config_validation`。
- 源码：[plugin_config.rs:434](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L434)、[plugin_config.rs:443](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L443)。[plugin_config.rs:134](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L134)、[plugin_config.rs:197](../../../crates/apps/daemon/src/services/attach/plugin_config.rs#L197)。

### 23. 插件业务配置校验

状态：`[?]`。

- 触发：runtime-managed 配置先通过 daemon Schema 与只读校验，然后调用插件 runtime-config.validate。
- 检查：将候选配置交给插件导出的 validate。成功载荷必须为字符串列表，每个字符串表示一项配置问题。业务规则由所加载插件实现。此宿主入口不定义规则内容。
- 失败：非空问题列表使 daemon 回复 `valid=false`。插件返回错误、调用失败或载荷不符合字符串列表形状时，返回 `PluginRuntimeError`，由 daemon 转为 ControlError。该校验调用不执行 submit。
- 源码：[runtime_config.rs:72](../../../crates/core/plugin_wasm_runtime/src/component_control/runtime_config.rs#L72)、[runtime_config.rs:173](../../../crates/core/plugin_wasm_runtime/src/component_control/runtime_config.rs#L173)、[runtime_config.rs:207](../../../crates/core/plugin_wasm_runtime/src/component_control/runtime_config.rs#L207)。[plugin_configuration.rs:64](../../../crates/apps/daemon/src/services/attach/plugin_configuration.rs#L64)。

### 24. 插件配置接口的数据体积

状态：`[?]`。

- 触发：runtime-config 的 get、validate 或 submit 调用。
- 检查：get 返回配置的字符串字节数不超过 `plugin_config_read_max_bytes`。validate/submit 的候选配置字节数不超过同一预算。validate 返回所有错误字符串的字节数之和不超过 `plugin_command_output_max_bytes`。
- 失败：返回 `plugin_config` 类型的 PluginRuntimeError。输入超限在调用插件前失败。输出超限在插件返回后失败，本次结果不交给上层。
- 源码：[runtime_config.rs:52](../../../crates/core/plugin_wasm_runtime/src/component_control/runtime_config.rs#L52)、[runtime_config.rs:72](../../../crates/core/plugin_wasm_runtime/src/component_control/runtime_config.rs#L72)、[runtime_config.rs:100](../../../crates/core/plugin_wasm_runtime/src/component_control/runtime_config.rs#L100)、[runtime_config.rs:115](../../../crates/core/plugin_wasm_runtime/src/component_control/runtime_config.rs#L115)。

### 25. 插件管理命令的输入规模

状态：`[?]`。

- 触发：`handle_command` 准备将请求传入 Wasm 前。
- 检查：argv 数量不超过 `plugin_command_argv_max_count`。每个参数字符串的字节数不超过 `plugin_command_arg_max_bytes`。
- 失败：返回 `plugin_command` 类型的 PluginRuntimeError。本次请求不传入插件。
- 源码：[value.rs:100](../../../crates/core/plugin_wasm_runtime/src/component_control/value.rs#L100)。[component_control.rs:318](../../../crates/core/plugin_wasm_runtime/src/component_control.rs#L318)。

## 隔离通信、Web 查询与平台能力

### 26. 上游连接轮询与超时

状态：`[?]`。

- 触发：`UpstreamTcpServer::start` 在绑定监听地址前调用配置校验。
- 检查：`accept_poll_interval`、`connection_poll_interval`、`connection_idle_timeout`、`write_timeout` 均非零。`connection_poll_interval` 不超过 `connection_idle_timeout`。
- 失败：返回 `ServerStartError::config`，本次上游服务启动失败，尚未绑定监听 socket。
- 源码：[config.rs:28](../../../crates/adapters/sandbox_link/upstream/src/config.rs#L28)。[server.rs:23](../../../crates/adapters/sandbox_link/upstream/src/server.rs#L23)。

### 27. 上游连接资源容量

状态：`[?]`。

- 触发：上游服务绑定监听地址前调用配置校验。
- 检查：`max_connections > 0`。`read_buffer_bytes` 在 1024 至 `MAX_FRAME_BYTES`（272 × 1024 字节）之间，包含端点。`connection_thread_stack_bytes >= 64 × 1024` 字节。
- 失败：返回配置错误，本次服务启动失败。
- 源码：[config.rs:24](../../../crates/adapters/sandbox_link/upstream/src/config.rs#L24)。[frame.rs:5](../../../crates/contracts/sandbox_link/upstream/src/frame.rs#L5)。

### 28. 隔离通信协议标识与帧类型

状态：`[?]`。

- 触发：VSOCK 或上游增量解码器收到完整的 8 字节帧头。`Frame::new` 也调用帧头解码核对构造结果。
- 检查：VSOCK magic 为 `0xac71`、版本为 `1`、帧码属于 `SbHello=1`、`SbWelcome=2`、`Heartbeat=3`、`ObservationBatch=16`。上游 magic 为 `0xac72`、版本为 `1`、帧码属于 `GatewayHello=32`、`GatewayWelcome=33`、`Heartbeat=34`、`ForwardedSbFrame=48`。
- 失败：返回 `WireError`，不生成该帧。上游连接工作线程结束当前连接并增加连接失败计数。网关 Guest 连接工作线程结束当前会话。Guest 或网关握手读取返回 `io::ErrorKind::InvalidData`，本次握手失败。
- 源码：[frame.rs:25](../../../crates/contracts/sandbox_link/vsock/src/frame.rs#L25)。[frame.rs:24](../../../crates/contracts/sandbox_link/upstream/src/frame.rs#L24)。[stream.rs:23](../../../crates/contracts/sandbox_link/upstream/src/stream.rs#L23)。[connection.rs:59](../../../crates/adapters/sandbox_link/upstream/src/connection.rs#L59)。[runtime.rs:239](../../../crates/core/vsock_gateway_runtime/src/runtime.rs#L239)。[protocol.rs:67](../../../crates/core/sandbox_agent_runtime/src/session/protocol.rs#L67)。[upstream.rs:335](../../../crates/core/vsock_gateway_runtime/src/upstream.rs#L335)。

### 29. 隔离通信帧长度

状态：`[?]`。

- 触发：已通过协议标识和帧码检查后解析载荷长度。
- 检查：8 字节帧头长度与 `u32` 载荷长度转换后的 `usize` 相加成功。完整 VSOCK 帧不超过 256 × 1024 字节。完整上游帧不超过 272 × 1024 字节。两个上限均包含帧头。
- 失败：返回长度溢出或超长 `WireError`。连接和握手的失败处理与协议标识校验相同。
- 源码：[frame.rs:67](../../../crates/contracts/sandbox_link/vsock/src/frame.rs#L67)。[frame.rs:67](../../../crates/contracts/sandbox_link/upstream/src/frame.rs#L67)。连接失败出口见第 28 项。

### 30. Web 查询预算

状态：`[?]`。

- 触发：本地或 Cluster Web 服务构造 `WebContext`，进入服务循环前加载环境变量。
- 检查：`ACTRAIL_WEB_TIMELINE_DEFAULT_LIMIT`、`ACTRAIL_WEB_TIMELINE_MAX_LIMIT`、`ACTRAIL_WEB_TIMELINE_CANDIDATES`、`ACTRAIL_WEB_TIMELINE_LINKS`、`ACTRAIL_WEB_IDLE_DEFAULT_LIMIT`、`ACTRAIL_WEB_IDLE_MAX_LIMIT`、`ACTRAIL_WEB_IDLE_CANDIDATES`、`ACTRAIL_WEB_TRACES_DEFAULT_LIMIT`、`ACTRAIL_WEB_TRACES_MAX_LIMIT`、`ACTRAIL_WEB_CONTENT_MAX_BYTES`、`ACTRAIL_WEB_CONTENT_NODE_MAX_ITEMS`、`ACTRAIL_WEB_CONTENT_NODE_PREVIEW_BYTES`、`ACTRAIL_WEB_CONTENT_NODE_BLOCK_BYTES` 在设置时必须是可读取的 Unicode 字符串，并可解析为 `usize`，满足 `0 < value < i64::MAX`。未设置时使用各自默认值。Timeline 和 idle 分别要求 `default_limit <= max_limit <= candidates`。trace 列表要求 `default_limit <= max_limit`。
- 失败：加载函数返回字符串错误，经 `?` 终止 Web 服务初始化。不会进入请求服务循环。Content 预算之间没有额外的大小关系校验。
- 源码：[timeline.rs:20](../../../crates/apps/web/src/http/timeline.rs#L20)。[idle.rs:16](../../../crates/apps/web/src/http/idle.rs#L16)。[trace_list.rs:30](../../../crates/apps/web/src/http/trace_list.rs#L30)。[content.rs:14](../../../crates/apps/web/src/http/query/content.rs#L14)。调用 [http.rs:85](../../../crates/apps/web/src/http.rs#L85)。

### 31. 查询参数解码与数值类型

状态：`[?]`。

- 触发：本登记范围内的时间线、idle、trace 列表、正文、token 统计、模型活动、模型行列表和时间归因行列表解析请求参数。
- 检查：被读取参数中的 `%` 转义必须后接两个十六进制字符。解码后的字节必须是 UTF-8。统计查询必需的 `from_ms`、`to_ms` 存在并可解析为 `u64`。模型行列表和时间归因行列表必需的 `offset`、`limit` 存在并可解析为 `usize`。正文 HTTP 查询必需的 `offset`、正文前缀查询必需的 `max_bytes`、正文节点查询必需的 `offset`、`limit` 存在并可解析为 `usize`。可选时间线/idle/trace 列表 `limit`、trace 列表 `page` 在提供时可解析为 `usize`。时间线的路径 trace ID、`process_id` 和本地 trace 列表的 `trace_id` 可解析为 `u64`。idle 的 trace ID 在解析后还满足 `1..=i64::MAX`。Cluster trace 列表的 `trace_id` 使用字符串，空值按未提供处理。
- 失败：参数解析返回错误，相关请求由路由返回 HTTP 400。这里的数值解析本身接受 `u64` 范围内的零值，idle trace ID 的额外范围检查按上面的独立条件执行。
- 源码：[query.rs:125](../../../crates/apps/web/src/http/query.rs#L125)。[query.rs:27](../../../crates/apps/web/src/http/query.rs#L27)。[timeline.rs:47](../../../crates/apps/web/src/http/timeline.rs#L47)（47–55、80–82、110）。[idle.rs:40](../../../crates/apps/web/src/http/idle.rs#L40)。[trace_list.rs:14](../../../crates/apps/web/src/http/trace_list.rs#L14)。[content.rs:38](../../../crates/apps/web/src/http/query/content.rs#L38)。HTTP 400 出口：[http.rs:377](../../../crates/apps/web/src/http.rs#L377)、[trace_routes.rs:20](../../../crates/apps/web/src/http/trace_routes.rs#L20)、[cluster_routes.rs:57](../../../crates/apps/web/src/http/cluster_routes.rs#L57)。

### 32. 分页与正文读取量

状态：`[?]`。

- 触发：解析上述接口中的分页和正文读取参数。
- 检查：时间线、idle、trace 列表的 `limit` 在 `1..=各自 max_limit` 内。正文节点 `limit` 在 `1..=max_node_items` 内。正文前缀 `max_bytes` 在 `1..=max_bytes` 预算内。HTTP 正文省略 `max_bytes` 时采用 `min(preview_bytes, max_bytes)`。模型行列表和时间归因行列表 `limit > 0`。Trace 列表 `page > 0`，`(page - 1) * limit` 不发生 `usize` 溢出，结果不超过 `i64::MAX`。时间线所选不同动作类型的数量不超过 `candidates`。
- 失败：返回参数错误，当前请求返回 HTTP 400。模型行列表与时间归因行列表在这些入口只检查 `limit` 为正，没有额外的最大页大小检查。
- 源码：[timeline.rs:56](../../../crates/apps/web/src/http/timeline.rs#L56)。[idle.rs:49](../../../crates/apps/web/src/http/idle.rs#L49)。[trace_list.rs:73](../../../crates/apps/web/src/http/trace_list.rs#L73)。[content.rs:42](../../../crates/apps/web/src/http/query/content.rs#L42)。[query.rs:60](../../../crates/apps/web/src/http/query.rs#L60)。HTTP 400 出口见第 31 项。

### 33. 查询时间范围

状态：`[?]`。

- 触发：解析 Timeline、idle、trace 列表的 `from`/`to`，以及四个统计接口的 `from_ms`/`to_ms`。Timeline 游标中的时间也执行纳秒范围检查。
- 检查：Timeline、idle、trace 列表中的时间是 `u64` Unix 纳秒，不超过 `i64::MAX`，与 `UNIX_EPOCH` 相加不得超出 `SystemTime` 表示范围。两个边界都存在时，Timeline 要求 `from < to`，idle 与 trace 列表要求 `from <= to`。Token 统计、模型活动、模型行列表、时间归因行列表要求 `from_ms < to_ms`。这些毫秒字段在当前解析入口只执行 `u64` 解析和前后关系检查。
- 失败：返回时间格式、范围或区间错误，当前请求返回 HTTP 400。
- 源码：[timeline.rs:83](../../../crates/apps/web/src/http/timeline.rs#L83)。[idle.rs:55](../../../crates/apps/web/src/http/idle.rs#L55)。[trace_list.rs:86](../../../crates/apps/web/src/http/trace_list.rs#L86)。[query.rs:27](../../../crates/apps/web/src/http/query.rs#L27)。HTTP 400 出口见第 31 项。

### 34. 查询游标格式

状态：`[?]`。

- 触发：请求提供 `cursor` 参数。
- 检查：Timeline 游标可按首个冒号分为时间与动作键，动作键可解析为正 `i64`，时间按第 33 项检查。idle 游标可解析为 `u64`，并满足 `1..=i64::MAX`。trace 列表只使用页码，出现 `cursor` 参数即拒绝，即使其值为空。
- 失败：当前请求返回 HTTP 400。
- 源码：[timeline.rs:95](../../../crates/apps/web/src/http/timeline.rs#L95)。[idle.rs:64](../../../crates/apps/web/src/http/idle.rs#L64)。[trace_list.rs:53](../../../crates/apps/web/src/http/trace_list.rs#L53)。HTTP 400 出口见第 31 项。

### 35. 时间线动作类型

状态：`[?]`。

- 触发：Timeline 请求提供 `kinds` 参数，按逗号拆分并忽略空项。
- 检查：每项必须属于 `process.exec`、`process.exit`、`agent.identity`、`agent.exit`、`file.modify`、`file.read`、`file.write`、`file.tty_io`、`file.bulk_read`、`fs.enumerate`、`http.message`、`websocket.message`、`llm.call`、`llm.request`、`llm.response`、`llm.tool_call`、`llm.tool_result`、`mcp.tool_call`、`mcp.request`、`mcp.response`、`mcp.stdin`、`mcp.stdout`、`sse.stream`、`sse.event`、`enforcement.decision`、`process.fork_attempt`、`agent.invocation`、`command.invocation`。
- 失败：返回 `unknown action kind`，当前请求返回 HTTP 400。不同种类数量预算另见第 32 项。
- 源码：[timeline.rs:62](../../../crates/apps/web/src/http/timeline.rs#L62)。[kind.rs:67](../../../crates/contracts/semantic_action/src/model/kind.rs#L67)。HTTP 400 出口 [trace_routes.rs:34](../../../crates/apps/web/src/http/trace_routes.rs#L34)。

### 36. 正文节点定位格式

状态：`[?]`。

- 触发：正文节点查询解析可选 `pointer`。
- 检查：省略或空字符串表示根节点。非空值必须以 `/` 开头。此入口没有完整检查 JSON Pointer 转义语法。
- 失败：返回 `invalid query parameter pointer`，当前请求返回 HTTP 400。
- 源码：[content.rs:66](../../../crates/apps/web/src/http/query/content.rs#L66)。HTTP 400 出口 [trace_routes.rs:218](../../../crates/apps/web/src/http/trace_routes.rs#L218)、[cluster_routes.rs:280](../../../crates/apps/web/src/http/cluster_routes.rs#L280)。

### 37. 模型活动统计粒度

状态：`[?]`。

- 触发：`parse_llm_activity_query` 收到可选 `rollup`。
- 检查：值必须是 `minute`、`hour`、`day`、`week` 或 `month`。省略时保留未指定状态。
- 失败：返回不支持的聚合粒度错误，当前请求返回 HTTP 400。
- 源码：[query.rs:44](../../../crates/apps/web/src/http/query.rs#L44)。[model.rs:262](../../../crates/apps/web/src/view/stats/model.rs#L262)。HTTP 400 出口 [http.rs:409](../../../crates/apps/web/src/http.rs#L409)。

### 38. 时间归因过滤条件

状态：`[?]`。

- 触发：`parse_time_attribution_rows_query` 解析可选 `dimension` 和 `key`。
- 检查：`dimension` 只能是 `category`、`model`、`tool` 或 `command`。`dimension` 与 `key` 必须同时提供或同时省略。已提供的空字符串 `key` 在此入口被视为存在，没有非空检查。
- 失败：返回维度无效或维度与键未成对的错误，当前请求返回 HTTP 400。
- 源码：[query.rs:93](../../../crates/apps/web/src/http/query.rs#L93)。[api.rs:17](../../../crates/apps/web/src/view/time_attribution/api.rs#L17)。HTTP 400 出口 [http.rs:427](../../../crates/apps/web/src/http.rs#L427)。

### 39. 宿主 eBPF 能力预检

状态：`[?]`。

- 触发：daemon 装配时逐一预检请求宿主 eBPF 观测的 capture profile。LSM 合并能力装载也复用此检查。
- 检查：平台探测没有 `reason_unavailable`。去掉禁用项和不在 collector descriptor 中的能力，并按能力合并、保留 Required 优先级后，有效请求不为空。其中 Required 能力受当前实现支持。依据请求生成的计划能够加载。加载后的 `attached_capabilities` 包含全部有效 Required 能力。无条件支持检查中的能力为 `ProcLifecycle`、`ProcExecContext`、`NetTransport`、`FsAccessBasic`、`FsMmap`、`IpcPipeFifo`、`IpcUnixSocket`。`TlsPlaintextPayload` 还要求 TLS 已启用且后端不是 sync。`SocketPlaintextPayload` 要求 socket 采集已启用。`StdioChunk` 要求 stdio 已启用且 stdin/stdout/stderr 至少一个采集开关启用。
- 失败：collector 返回 `CollectorError`。逐 profile 预检记录失败阶段和消息，将该 profile 标为 unavailable，继续检查其余 profile。本方法本身不返回启动错误。声明中未暴露的请求被过滤，不能描述成“拒绝任何未知 capability”。有效计划相同的 profile 复用同一预检报告。
- 源码：[runtime.rs:35](../../../crates/adapters/collectors/ebpf/src/collector/runtime.rs#L35)。[lib.rs:336](../../../crates/adapters/collectors/ebpf/src/lib.rs#L336)。[preflight.rs:99](../../../crates/apps/daemon/src/services/attach/preflight.rs#L99)。调用 [wiring.rs:284](../../../crates/apps/daemon/src/services/wiring.rs#L284)。

### 40. LSM 常驻采集能力预检

状态：`[?]`。

- 触发：daemon 装配时 `access_control_config.uses_lsm()` 为真，在逐 profile 预检后执行。
- 检查：要求 LSM 的每个 profile 存在预检报告且报告 available。汇集所有有可用报告的 profile 的非禁用能力后，并集非空。并集再次通过第 39 项预检并完成装载，以提供 daemon 生命周期内的 LSM task scope。非 LSM profile 的缺报告或 unavailable 被跳过。
- 失败：返回阶段为 `lsm_preflight` 的 `ControlError`。装配调用通过 `?` 返回错误，daemon 初始化失败。
- 源码：[preflight.rs:38](../../../crates/apps/daemon/src/services/attach/preflight.rs#L38)。[wiring.rs:284](../../../crates/apps/daemon/src/services/wiring.rs#L284)。

### 41. seccomp 通知 ABI 兼容性

状态：`[?]`。

- 触发：`SeccompNotifyService::register_listener` 已确认通知功能启用并取得传入的 listener FD，准备登记新监听器时。
- 检查：`seccomp(SECCOMP_GET_NOTIF_SIZES)` 调用成功。内核返回的 `seccomp_notif`、`seccomp_notif_resp`、`seccomp_data` 三个大小，分别与对应 libc 类型的 `size_of` 一致。
- 失败：返回阶段为 `seccomp_notif_sizes` 的 `ControlError`，本次 listener 不加入 `listeners`。调用方将该错误作为本次监听器注册操作的结果返回。校验发生在监听器注册时。
- 源码：[notify.rs:112](../../../crates/apps/daemon/src/services/seccomp_notify/notify.rs#L112)。[service.rs:29](../../../crates/apps/daemon/src/services/seccomp_notify/service.rs#L29)。调用 [service.rs:383](../../../crates/apps/daemon/src/services/attach/service.rs#L383)。

## 待补充记录

TODO：补录各项用户许可决定及其来源。

`OperatorDocument::validate` 调用 [ResourceProfilingConfig::parse](../../../crates/core/resource_profiling/src/config/resource_profiling_config.rs#L41) 解析资源配置。该调用转交资源采集、存储和告警模块处理其子配置。TODO：按具体校验目的登记此调用链中尚未列出的检查。
