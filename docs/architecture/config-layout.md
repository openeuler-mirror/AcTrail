| 配置项 | 类型 | 默认值 | 必填 | 所属模块 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `control.socket_path` | `string` | `"/run/actrail/control.sock"` | 否 | control | Unix socket 路径；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L11) |
| `control.socket_mode_octal` | `string` | `"660"` | 否 | control | Unix socket 权限的八进制字符串；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L12) |
| `control.pending_connection_max` | `uint32` | `256` | 否 | control | 待处理控制连接数上限；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L13) |
| `control.active_trace_max` | `uint32` | `128` | 否 | control | 同时活动的 trace 数量上限；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L14) |
| `control.pid_file` | `string` | `"/run/actrail/actraild.pid"` | 否 | control | daemon PID 文件路径；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L15) |
| `control.log_path` | `string` | `"/var/log/actrail/actraild.log"` | 否 | control | daemon 日志路径；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L16) |
| `control.diagnostic_log_level` | `string` | `"info"` | 否 | control | 诊断日志等级；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L17) |
| `control.host_id` | `string` | `未设置` | 否 | control | 覆盖 OTel host.id；省略或为空时读取宿主 DMI product_uuid；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L20) |
| `control.workload_diagnostics.enabled` | `bool` | `false` | 否 | control | 定期输出工作负载诊断；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L45) |
| `control.workload_diagnostics.interval_ms` | `uint64` | `1000` | 否 | control | 执行间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L46) |
| `control.finalization.traces_per_cycle` | `uint32` | `1` | 否 | control | 每轮处理的 trace 终结任务数上限；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L73) |
| `control.finalization.poll_interval_ms` | `uint64` | `100` | 否 | control | 轮询间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L74) |
| `control.finalization.settle_delay_ms` | `uint64` | `250` | 否 | control | trace 终结前等待迟到事件的时间，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L75) |
| `control.finalization.shutdown_drain_timeout_ms` | `uint64` | `30000` | 否 | control | 关闭时排空剩余任务的等待上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L76) |
| `control.finalization.post_trace.max_in_flight_tasks` | `uint32` | `64` | 否 | control | 同时执行的 trace 后处理任务数上限；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L95) |
| `control.finalization.post_trace.broker_queue_capacity` | `uint32` | `256` | 否 | control | 后处理请求代理队列容量；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L96) |
| `control.finalization.post_trace.requests_per_cycle` | `uint32` | `128` | 否 | control | 每轮处理的后处理请求数上限；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L97) |
| `control.finalization.post_trace.broker_reply_timeout_ms` | `uint64` | `5000` | 否 | control | 等待后处理请求代理回复的上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L98) |
| `control.finalization.post_trace.admission_timeout_ms` | `uint64` | `30000` | 否 | control | 后处理任务等待准入的时间上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L99) |
| `control.finalization.post_trace.execution_timeout_ms` | `uint64` | `60000` | 否 | control | 后处理任务执行时限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L100) |
| `control.finalization.post_trace.shutdown_drain_timeout_ms` | `uint64` | `30000` | 否 | control | 关闭时排空剩余任务的等待上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L101) |
| `storage.backend` | `string` | `"sqlite"` | 否 | storage | 主存储后端：sqlite 或 noop；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L202) |
| `storage.sqlite.path` | `string` | `"/var/lib/actrail/actrail.sqlite"` | 否 | storage | 文件路径；storage.backend=sqlite 时使用；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L278) |
| `storage.sqlite.busy_timeout_ms` | `uint64` | `5000` | 否 | storage | SQLite 忙等待上限，毫秒；storage.backend=sqlite 时使用；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L279) |
| `storage.sqlite.cold_field_compression_min_bytes` | `integer` | `64` | 否 | storage | 触发冷字段压缩的最小字节数；storage.backend=sqlite 时使用；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L280) |
| `storage.sqlite.cold_field_zstd_level` | `int32` | `3` | 否 | storage | 冷字段 Zstandard 压缩等级；storage.backend=sqlite 时使用；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L281) |
| `storage.sqlite.event_payload_dictionary_cache_bytes` | `integer` | `33554432` | 否 | storage | 事件 payload 字典缓存字节上限；storage.backend=sqlite 时使用；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L282) |
| `storage.sqlite.event_path_dictionary_cache_bytes` | `integer` | `8388608` | 否 | storage | 事件路径字典缓存字节上限；storage.backend=sqlite 时使用；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L283) |
| `storage.sqlite.event_record_layout` | `string` | `"rows"` | 否 | storage | 事件记录布局：rows 或 blocks；storage.backend=sqlite 时使用；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L284) |
| `storage.sqlite.event_record_block_max_events` | `integer` | `256` | 否 | storage | 一个事件块最多容纳的事件数；storage.backend=sqlite 时使用；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L285) |
| `storage.sqlite.event_record_block_max_uncompressed_bytes` | `integer` | `1048576` | 否 | storage | 单个事件块未压缩字节上限；storage.backend=sqlite 时使用；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L286) |
| `storage.sqlite.event_record_block_zstd_level` | `int32` | `3` | 否 | storage | 事件块 Zstandard 压缩等级；storage.backend=sqlite 时使用；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L287) |
| `storage.retention.enabled` | `bool` | `true` | 否 | storage | 启用过期 trace 清理；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L311) |
| `storage.retention.max_trace_age` | `string` | `"7d"` | 否 | storage | trace 保留时长；使用带 ms、s、m、h 或 d 单位的字符串；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L312) |
| `storage.retention.sweep_interval` | `string` | `"1m"` | 否 | storage | 过期 trace 清理间隔；使用带时间单位的字符串；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L313) |
| `storage.retention.min_terminal_age` | `string` | `"30s"` | 否 | storage | 终态 trace 可清理前的最小存留时长；使用带时间单位的字符串；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L314) |
| `storage.retention.max_traces_per_sweep` | `uint32` | `10` | 否 | storage | 每轮最多清理的 trace 数量；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L315) |
| `storage.retention.protected_tags` | `string[]` | `["retain", "pinned"]` | 否 | storage | 清理时保护的 trace 标签；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L316) |
| `storage.retention.checkpoint_after_sweep` | `bool` | `true` | 否 | storage | 清理后执行 SQLite checkpoint；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L317) |
| `web.listen_addr` | `string` | `"127.0.0.1:18080"` | 否 | web | 监听 IP 与端口；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L368) |
| `web.request_read_timeout_ms` | `string` | `"1000"` | 否 | web | 读取 Web 请求的时限，毫秒字符串；disabled 表示不设置时限；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L369) |
| `web.alerts.default_limit` | `uint32` | `30` | 否 | web | 告警列表的默认返回条数；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L386) |
| `web.alerts.max_limit` | `uint32` | `300` | 否 | web | 告警列表允许的最大返回条数；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L387) |
| `cluster.enabled` | `bool` | `false` | 否 | cluster | 启用集群节点配置；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L6) |
| `cluster.cluster_id` | `string` | `""` | cluster.enabled=true 时必填 | cluster | 集群标识；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L7) |
| `cluster.node_id` | `string` | `""` | cluster.enabled=true 时必填 | cluster | 节点标识；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L8) |
| `cluster.node_name` | `string` | `""` | cluster.enabled=true 时必填 | cluster | 节点展示名称；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L9) |
| `cluster.node_ip` | `string` | `""` | cluster.enabled=true 时必填 | cluster | 节点 IP；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L10) |
| `cluster.report.enabled` | `bool` | `false` | 否 | cluster | 启用节点向中心上报 trace；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L65) |
| `cluster.report.center_host` | `string` | `""` | cluster.enabled 与 cluster.report.enabled 均为 true 时必填 | cluster | 集群中心主机；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L66) |
| `cluster.report.center_port` | `uint16` | `0` | cluster.enabled 与 cluster.report.enabled 均为 true 时必填 | cluster | 集群中心端口；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L67) |
| `cluster.report.scheme` | `string` | `"http"` | 否 | cluster | 集群上报协议，当前支持 http；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L68) |
| `cluster.report.interval_secs` | `uint64` | `43200` | 否 | cluster | 执行间隔，秒；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L69) |
| `cluster.report.terminal_only` | `bool` | `true` | 否 | cluster | 仅上报终态 trace；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L70) |
| `cluster.report.spool_dir` | `string` | `"/var/lib/actrail/cluster-spool"` | 否 | cluster | 待上报 bundle 的本地暂存目录；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L71) |
| `cluster.report.state_path` | `string` | `"/var/lib/actrail/cluster-report-state.sqlite"` | 否 | cluster | 集群上报状态数据库路径；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L72) |
| `cluster.report.batch_max_traces` | `uint32` | `20` | 否 | cluster | 每批上报的 trace 数量上限；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L73) |
| `cluster.report.bundle_retention_days` | `uint32` | `7` | 否 | cluster | 已生成 bundle 的保留天数；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L74) |
| `cluster.report.upload_timeout_secs` | `uint64` | `60` | 否 | cluster | 上传超时，秒；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L75) |
| `cluster.report.retry_backoff_secs` | `uint64` | `30` | 否 | cluster | 上传重试的初始退避时间，秒；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L76) |
| `cluster.report.max_retry_backoff_secs` | `uint64` | `600` | 否 | cluster | 上传重试的最大退避时间，秒；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L77) |
| `cluster.report.auth_token_file` | `string` | `""` | 否 | cluster | 认证令牌文件；空字符串表示未配置；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L78) |
| `cluster.center.enabled` | `bool` | `false` | 否 | cluster | 启动集群中心接收服务；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L189) |
| `cluster.center.listen_host` | `string` | `""` | cluster.center.enabled=true 时必填 | cluster | 监听主机地址；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L190) |
| `cluster.center.listen_port` | `uint16` | `0` | cluster.center.enabled=true 时必填 | cluster | 监听端口；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L191) |
| `cluster.center.root_dir` | `string` | `"/var/lib/actrail-cluster"` | 否 | cluster | 集群中心存储根目录；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L192) |
| `cluster.center.auth_token_file` | `string` | `""` | 否 | cluster | 认证令牌文件；空字符串表示未配置；[声明](../../crates/core/config/src/daemon/operator/document/cluster.rs#L193) |
| `export.snapshot.graph_schema_version` | `string` | `"manual-v1"` | 否 | export | 导出图模式版本；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L446) |
| `export.snapshot.allow_active_trace_snapshot` | `bool` | `true` | 否 | export | 允许对活动 trace 生成快照；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L447) |
| `export.snapshot.directory` | `string` | `"/var/lib/actrail/export"` | 否 | export | 导出快照的输出目录；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L448) |
| `export.snapshot.payload_bytes_enabled` | `bool` | `true` | 否 | export | 快照包含 payload 字节；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L449) |
| `export.snapshot.payload_text_enabled` | `bool` | `true` | 否 | export | 快照包含 payload 文本；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L450) |
| `plugins.discovery.directory` | `string` | `"~/.actrail/plugins"` | 否 | plugins | 插件包发现目录；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L103) |
| `plugins.discovery.max_packages` | `uint32` | `128` | 否 | plugins | 发现的插件包数量上限；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L104) |
| `plugins.discovery.manifest_max_bytes` | `uint64` | `262144` | 否 | plugins | 插件 manifest 文件字节上限；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L105) |
| `plugins.alerts.queue_capacity` | `uint32` | `1024` | 否 | plugins | 队列容量；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L55) |
| `plugins.alerts.writes_per_cycle` | `uint32` | `256` | 否 | plugins | 每轮持久化插件告警的数量上限；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L56) |
| `plugins.alerts.drain_timeout_ms` | `uint64` | `30000` | 否 | plugins | 排空队列等待上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L57) |
| `plugins.startup.enabled` | `bool` | `false` | 否 | plugins | 启动时加载配置中的插件实例；启用时至少有一个启用的 load 元素；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L150) |
| `plugins.startup.failure_policy` | `string` | `"fail-fast"` | 否 | plugins | 插件启动失败策略：fail-fast 或 continue；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L151) |
| `plugins.startup.load` | `object[]` | `[]` | `plugins.startup.enabled=true` 时须含至少一个启用元素 | plugins | 启动时加载的插件实例列表；元素字段见下列完整路径；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L152) |
| `plugins.startup.load[].instance` | `string` | `""` | 提供列表元素时必填且非空 | plugins | 插件实例标识；启用实例间须唯一；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L213) |
| `plugins.startup.load[].enabled` | `bool` | `true` | 否 | plugins | 启动时加载该插件实例；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L214) |
| `plugins.startup.load[].failure_policy` | `string` | `""` | 否 | plugins | 插件启动失败策略：fail-fast 或 continue；空字符串继承 plugins.startup.failure_policy；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L215) |
| `plugins.startup.load[].manifest` | `string` | `""` | 提供列表元素时必填且非空 | plugins | 插件 manifest 文件路径；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L216) |
| `plugins.startup.load[].plugin_config` | `string` | `""` | 否 | plugins | 插件配置文件路径；空字符串表示未指定；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L217) |
| `plugins.startup.load[].host_grants` | `string[]` | `[]` | 否 | plugins | 授予插件的宿主能力列表；[声明](../../crates/core/config/src/daemon/operator/document/plugin/config.rs#L218) |
| `alert_forwarding.proxy_executable` | `string` | `"/usr/local/bin/actraild-alert-proxy"` | 否 | alert_forwarding | 告警代理可执行文件的绝对路径；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L13) |
| `alert_forwarding.proxy_config_path` | `string` | `"/etc/actrail/actraild-alert-proxy.toml"` | 否 | alert_forwarding | 告警代理配置文件的绝对路径；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L14) |
| `alert_forwarding.plugin_config_path` | `string` | `"/etc/actrail/plugins/alert-forwarding/alert-forwarding.config.json"` | 否 | alert_forwarding | 告警插件配置文件的绝对路径；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L15) |
| `alert_forwarding.socket_path` | `string` | `"/run/actrail/alert-proxy.sock"` | 否 | alert_forwarding | Unix socket 路径；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L16) |
| `alert_forwarding.queue_capacity` | `uint32` | `1024` | 否 | alert_forwarding | 队列容量；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L17) |
| `alert_forwarding.read_timeout_ms` | `uint64` | `1000` | 否 | alert_forwarding | 连接读取超时，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L18) |
| `alert_forwarding.write_timeout_ms` | `uint64` | `1000` | 否 | alert_forwarding | 连接写入超时，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L19) |
| `alert_forwarding.heartbeat_interval_ms` | `uint64` | `5000` | 否 | alert_forwarding | 心跳发送间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L20) |
| `alert_forwarding.heartbeat_ack_timeout_ms` | `uint64` | `2000` | 否 | alert_forwarding | 等待心跳应答的时限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L21) |
| `alert_forwarding.startup_timeout_ms` | `uint64` | `5000` | 否 | alert_forwarding | 等待启动就绪的时限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L22) |
| `alert_forwarding.startup_poll_interval_ms` | `uint64` | `20` | 否 | alert_forwarding | 启动状态轮询间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L23) |
| `alert_forwarding.max_frame_bytes` | `uint32` | `262144` | 否 | alert_forwarding | 单个传输帧字节上限；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L24) |
| `alert_forwarding.max_trace_id_bytes` | `uint32` | `128` | 否 | alert_forwarding | trace 标识字节上限；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L25) |
| `alert_forwarding.max_category_bytes` | `uint32` | `128` | 否 | alert_forwarding | 告警类别字节上限；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L26) |
| `alert_forwarding.max_description_bytes` | `uint32` | `4096` | 否 | alert_forwarding | 告警说明字节上限；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L27) |
| `alert_forwarding.max_extras_bytes` | `uint32` | `131072` | 否 | alert_forwarding | 告警附加字段字节上限；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L28) |
| `alert_forwarding.link_thread_stack_bytes` | `integer` | `524288` | 否 | alert_forwarding | 告警连接线程栈字节数；[声明](../../crates/core/config/src/daemon/operator/document/alert_forwarding/mod.rs#L29) |
| `hand_observation.enabled` | `bool` | `false` | 否 | hand_observation | 启用手侧 gateway 观测入口；[声明](../../crates/core/config/src/daemon/operator/document/hand_observation/mod.rs#L15) |
| `hand_observation.listen_addr` | `string` | `"127.0.0.1:9472"` | 否 | hand_observation | 监听 IP 与端口；[声明](../../crates/core/config/src/daemon/operator/document/hand_observation/mod.rs#L16) |
| `hand_observation.max_gateway_connections` | `uint32` | `64` | 否 | hand_observation | 同时连接的 gateway 数量上限；[声明](../../crates/core/config/src/daemon/operator/document/hand_observation/mod.rs#L17) |
| `hand_observation.accept_poll_interval_ms` | `uint64` | `20` | 否 | hand_observation | 接收新连接的轮询间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/hand_observation/mod.rs#L18) |
| `hand_observation.connection_poll_interval_ms` | `uint64` | `250` | 否 | hand_observation | 连接数据轮询间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/hand_observation/mod.rs#L19) |
| `hand_observation.connection_idle_timeout_ms` | `uint64` | `30000` | 否 | hand_observation | 连接空闲超时，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/hand_observation/mod.rs#L20) |
| `hand_observation.write_timeout_ms` | `uint64` | `5000` | 否 | hand_observation | 连接写入超时，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/hand_observation/mod.rs#L21) |
| `hand_observation.read_buffer_bytes` | `integer` | `65536` | 否 | hand_observation | 连接读取缓冲区字节数；[声明](../../crates/core/config/src/daemon/operator/document/hand_observation/mod.rs#L22) |
| `hand_observation.connection_thread_stack_bytes` | `integer` | `524288` | 否 | hand_observation | 连接线程栈字节数；[声明](../../crates/core/config/src/daemon/operator/document/hand_observation/mod.rs#L23) |
| `sandbox_evidence.path` | `string` | `"/var/lib/actrail/sandbox-evidence.sqlite"` | 否 | sandbox_evidence | 文件路径；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L12) |
| `sandbox_evidence.schema_version` | `uint32` | `3` | 否 | sandbox_evidence | 数据库模式版本；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L13) |
| `sandbox_evidence.create_parent_directory` | `bool` | `true` | 否 | sandbox_evidence | 自动创建数据库父目录；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L14) |
| `sandbox_evidence.busy_timeout_ms` | `uint64` | `5000` | 否 | sandbox_evidence | SQLite 忙等待上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L15) |
| `sandbox_evidence.writer_queue_capacity` | `uint32` | `1024` | 否 | sandbox_evidence | 后台数据库写入队列容量；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L16) |
| `sandbox_evidence.batch_max_observations` | `uint32` | `1024` | 否 | sandbox_evidence | 每批 observation 数量上限；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L17) |
| `sandbox_evidence.transaction_max_batches` | `uint32` | `32` | 否 | sandbox_evidence | 单个数据库事务合并的批次数上限；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L18) |
| `sandbox_evidence.flush_interval_ms` | `uint64` | `250` | 否 | sandbox_evidence | 缓冲数据刷新间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L19) |
| `sandbox_evidence.retention_max_observations` | `uint64` | `1000000` | 否 | sandbox_evidence | 允许保留的 observation 数量上限；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L20) |
| `sandbox_evidence.capacity_max_bytes` | `uint64` | `1073741824` | 否 | sandbox_evidence | 数据库容量字节上限；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L21) |
| `sandbox_evidence.synchronous` | `string` | `"normal"` | 否 | sandbox_evidence | SQLite synchronous 模式：normal 或 full；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L22) |
| `sandbox_evidence.wal_autocheckpoint_pages` | `uint32` | `1000` | 否 | sandbox_evidence | SQLite WAL 自动 checkpoint 页数；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L23) |
| `sandbox_evidence.shutdown_drain_timeout_ms` | `uint64` | `10000` | 否 | sandbox_evidence | 关闭时排空剩余任务的等待上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L24) |
| `sandbox_evidence.writer_thread_stack_bytes` | `integer` | `524288` | 否 | sandbox_evidence | 数据库写入线程栈字节数；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L25) |
| `sandbox_evidence.read_limit_max` | `uint32` | `10000` | 否 | sandbox_evidence | 查询返回记录数量上限；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_evidence/mod.rs#L26) |
| `sandbox_alerts.enabled` | `bool` | `false` | 否 | sandbox_alerts | 启用手侧插件告警存储；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L12) |
| `sandbox_alerts.path` | `string` | `"/var/lib/actrail/sandbox-alerts.sqlite"` | 否 | sandbox_alerts | 文件路径；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L13) |
| `sandbox_alerts.schema_version` | `uint32` | `2` | 否 | sandbox_alerts | 数据库模式版本；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L14) |
| `sandbox_alerts.create_parent_directory` | `bool` | `true` | 否 | sandbox_alerts | 自动创建数据库父目录；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L15) |
| `sandbox_alerts.busy_timeout_ms` | `uint64` | `5000` | 否 | sandbox_alerts | SQLite 忙等待上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L16) |
| `sandbox_alerts.writer_queue_capacity` | `uint32` | `1024` | 否 | sandbox_alerts | 后台数据库写入队列容量；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L17) |
| `sandbox_alerts.transaction_max_alerts` | `uint32` | `256` | 否 | sandbox_alerts | 单个数据库事务写入的告警数上限；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L18) |
| `sandbox_alerts.flush_interval_ms` | `uint64` | `250` | 否 | sandbox_alerts | 缓冲数据刷新间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L19) |
| `sandbox_alerts.retention_max_alerts` | `uint64` | `1000000` | 否 | sandbox_alerts | 允许保留的告警数量上限；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L20) |
| `sandbox_alerts.capacity_max_bytes` | `uint64` | `1073741824` | 否 | sandbox_alerts | 数据库容量字节上限；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L21) |
| `sandbox_alerts.synchronous` | `string` | `"normal"` | 否 | sandbox_alerts | SQLite synchronous 模式：normal 或 full；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L22) |
| `sandbox_alerts.wal_autocheckpoint_pages` | `uint32` | `1000` | 否 | sandbox_alerts | SQLite WAL 自动 checkpoint 页数；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L23) |
| `sandbox_alerts.shutdown_drain_timeout_ms` | `uint64` | `10000` | 否 | sandbox_alerts | 关闭时排空剩余任务的等待上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L24) |
| `sandbox_alerts.writer_thread_stack_bytes` | `integer` | `524288` | 否 | sandbox_alerts | 数据库写入线程栈字节数；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L25) |
| `sandbox_alerts.read_limit_max` | `uint32` | `10000` | 否 | sandbox_alerts | 查询返回记录数量上限；[声明](../../crates/core/config/src/daemon/operator/document/sandbox_alerts/mod.rs#L26) |
| `capture.profile_name` | `string` | `"default-full-monitor"` | 否 | capture | 采集 profile 名称；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L480) |
| `capture.agent_descendant_observation_depth` | `int32` | `-1` | 否 | capture | Agent 后代观测深度；-1 表示不限制，其他值须非负；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L481) |
| `capture.capabilities` | `string[]` | `["proc-lifecycle", "proc-exec-context", "fs-access-basic", "fs-mmap", "net-transport", "stdio-chunk", "tls-plaintext-payload", "socket-plaintext-payload", "net-application-plaintext-http", "net-application-http2-frames", "resource-metrics", "enforcement-file-permission-lsm"]` | 否 | capture | 必须具备的采集与治理能力列表；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L482) |
| `capture.opportunistic_capabilities` | `string[]` | `[]` | 否 | capture | 可按可用性启用的能力列表；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L483) |
| `capture.disabled_capabilities` | `string[]` | `[]` | 否 | capture | 显式禁用的能力列表；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L484) |
| `ebpf.enabled` | `string / bool` | `"true"` | 否 | ebpf | eBPF 启用策略：true、false 或 "auto"；布尔值与对应字符串均可；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L561) |
| `ebpf.memlock_rlimit` | `string` | `"inherit"` | 否 | ebpf | 锁定内存限制；inherit 表示继承当前限制；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L562) |
| `ebpf.tracked_process_max_entries` | `uint32` | `8192` | 否 | ebpf | eBPF 跟踪进程表容量；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L563) |
| `ebpf.pending_operation_max_entries` | `uint32` | `8192` | 否 | ebpf | 待完成操作表容量；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L564) |
| `ebpf.fd_per_process_max_entries` | `uint32` | `64` | 否 | ebpf | 每进程跟踪的文件描述符数量上限；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L565) |
| `ebpf.suppressed_fd_max_entries` | `uint32` | `8192` | 否 | ebpf | 被抑制的文件描述符表容量；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L566) |
| `ebpf.suppressed_fd_index_slots_per_process` | `uint32` | `64` | 否 | ebpf | 每进程用于索引被抑制文件描述符的槽数；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L567) |
| `ebpf.event_ring_buffer_max_bytes` | `uint32` | `33554432` | 否 | ebpf | eBPF 事件 ring buffer 字节上限；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L568) |
| `ebpf.diagnostics_summary_interval_ms` | `uint32` | `10000` | 否 | ebpf | 诊断摘要输出间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L569) |
| `ebpf.file_path_capture_enabled` | `bool` | `true` | 否 | ebpf | 启用文件路径采集；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L570) |
| `ebpf.file_path_max_bytes` | `uint32` | `255` | 否 | ebpf | 采集的文件路径字节上限；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L571) |
| `ebpf.net_send_recv_aggregation` | `bool` | `true` | 否 | ebpf | 聚合网络发送与接收计数；[声明](../../crates/core/config/src/daemon/operator/document/base/document.rs#L572) |
| `ebpf.ipc_lineage.enabled` | `bool` | `true` | 否 | ebpf | 采集和关联进程间通信连接来源；[声明](../../crates/core/config/src/daemon/operator/document/base/ipc_lineage.rs#L6) |
| `ebpf.ipc_lineage.max_processes_per_trace` | `uint32` | `8192` | 否 | ebpf | 每个 trace 记录的 IPC 进程数上限；[声明](../../crates/core/config/src/daemon/operator/document/base/ipc_lineage.rs#L7) |
| `ebpf.ipc_lineage.max_candidate_fds_per_trace` | `uint32` | `65536` | 否 | ebpf | 每个 trace 记录的候选 IPC 文件描述符数上限；[声明](../../crates/core/config/src/daemon/operator/document/base/ipc_lineage.rs#L8) |
| `ebpf.ipc_lineage.max_stdio_bundles_per_trace` | `uint32` | `8192` | 否 | ebpf | 每个 trace 保留的完整 stdio 连接组合数上限；[声明](../../crates/core/config/src/daemon/operator/document/base/ipc_lineage.rs#L9) |
| `payload.tls.enabled` | `bool` | `true` | 否 | payload | 采集 TLS 明文；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L117) |
| `payload.tls.capture_backend` | `string` | `"bpf-copy"` | 否 | payload | 明文采集后端；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L118) |
| `payload.tls.source` | `string` | `"auto"` | 否 | payload | TLS 目标来源选择；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L119) |
| `payload.tls.resolver` | `string` | `"auto"` | 否 | payload | TLS 符号和探针解析器选择；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L120) |
| `payload.tls.library` | `string` | `"auto"` | 否 | payload | TLS 库类型选择；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L121) |
| `payload.tls.library_path` | `string` | `"auto"` | 否 | payload | TLS 库路径；auto 表示自动发现；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L122) |
| `payload.tls.binary_path` | `string` | `"disabled"` | 否 | payload | TLS 可执行文件路径；disabled 表示未指定；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L123) |
| `payload.tls.pattern_path` | `string` | `"disabled"` | 否 | payload | TLS 机器码模式文件路径；disabled 表示未指定；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L124) |
| `payload.tls.max_segment_bytes` | `uint32` | `4095` | 否 | payload | 单个采集片段的最大字节数；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L125) |
| `payload.tls.max_operation_bytes` | `uint32` | `16777216` | 否 | payload | 单次操作采集的最大字节数；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L126) |
| `payload.tls.ring_buffer_bytes` | `uint32` | `8388608` | 否 | payload | 明文事件 ring buffer 容量，字节；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L127) |
| `payload.tls.pending_operation_max_entries` | `uint32` | `8192` | 否 | payload | 待完成操作表容量；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L128) |
| `payload.tls.seccomp_syscalls` | `string[]` | `["write", "writev", "sendto", "sendmsg"]` | 否 | payload | 使用 seccomp 通知处理的系统调用列表；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L129) |
| `payload.tls.diagnostics_enabled` | `bool` | `true` | 否 | payload | 输出明文采集诊断；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L130) |
| `payload.tls.retention_max_bytes_per_trace` | `uint64` | `104857600` | 否 | payload | 每个 trace 的明文保留字节上限；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L131) |
| `payload.tls.redaction_policy` | `string` | `"disabled"` | 否 | payload | 明文脱敏策略；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L132) |
| `payload.tls.sync_runtime_library_path` | `string` | `"auto"` | 否 | payload | TLS sync 注入库路径；auto 表示自动选择；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L133) |
| `payload.tls.sync_event_socket_path` | `string` | `"/run/actrail/tls-sync.sock"` | 否 | payload | TLS sync 事件 Unix socket 路径；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L134) |
| `payload.tls.sync_max_frame_bytes` | `uint32` | `33554432` | 否 | payload | TLS sync 单帧字节上限，包含帧头；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L135) |
| `payload.tls.sync_socket_mode_octal` | `string` | `"660"` | 否 | payload | TLS sync socket 权限的八进制字符串；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L136) |
| `payload.tls.sync_match_limit` | `uint32` | `8` | 否 | payload | TLS sync 探针匹配数量上限；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L137) |
| `payload.tls.binary_analysis_cache_capacity` | `uint32` | `256` | 否 | payload | 二进制分析结果缓存条目上限；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L138) |
| `payload.tls.dynamic_discovery_capacity` | `uint32` | `128` | 否 | payload | 动态 TLS 探针发现任务与结果的总容量；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L139) |
| `payload.tls.direct_startup_discovery_enabled` | `bool` | `true` | 否 | payload | 在启动时发现直接捕获所需的 TLS 探针；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L140) |
| `payload.tls.direct_dynamic_discovery_enabled` | `bool` | `true` | 否 | payload | 在运行时发现新出现的直接捕获 TLS 探针；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L141) |
| `payload.tls.dynamic_exec_plan_timeout_ms` | `uint64` | `30000` | 否 | payload | 动态执行探针计划查询时限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L142) |
| `payload.tls.sync_flow_control_enabled` | `bool` | `true` | 否 | payload | 启用 TLS sync 流量预算控制；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L143) |
| `payload.tls.sync_flow_sniff_bytes` | `uint32` | `65536` | 否 | payload | TLS sync 流识别阶段的字节预算；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L144) |
| `payload.tls.sync_flow_max_header_bytes` | `uint32` | `16384` | 否 | payload | TLS sync HTTP 头字节预算；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L145) |
| `payload.tls.sync_flow_large_transfer_bytes` | `uint64` | `1048576` | 否 | payload | TLS sync 判断 HTTP/1 大数据传输的字节阈值；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L146) |
| `payload.tls.sync_flow_unknown_stream_bytes` | `uint64` | `1048576` | 否 | payload | TLS sync 未识别流的字节预算；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L147) |
| `payload.tls.sync_flow_h2_data_probe_bytes` | `uint64` | `65536` | 否 | payload | TLS sync HTTP/2 DATA 探测字节预算；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L148) |
| `payload.tls.java_agent_enabled` | `bool` | `false` | 否 | payload | 启用 Java Agent 明文采集；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L149) |
| `payload.stdio.enabled` | `bool` | `true` | 否 | payload | 采集标准输入输出明文；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L339) |
| `payload.stdio.capture_stdin` | `bool` | `true` | 否 | payload | 采集标准输入；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L340) |
| `payload.stdio.capture_stdout` | `bool` | `true` | 否 | payload | 采集标准输出；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L341) |
| `payload.stdio.capture_stderr` | `bool` | `true` | 否 | payload | 采集标准错误输出；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L342) |
| `payload.stdio.stdin_storage_mode` | `string` | `"full"` | 否 | payload | 标准输入存储模式；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L343) |
| `payload.stdio.stdout_storage_mode` | `string` | `"drop"` | 否 | payload | 标准输出存储模式；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L344) |
| `payload.stdio.stderr_storage_mode` | `string` | `"metadata-only"` | 否 | payload | 标准错误输出存储模式；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L345) |
| `payload.stdio.max_segment_bytes` | `uint32` | `4095` | 否 | payload | 单个采集片段的最大字节数；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L346) |
| `payload.stdio.ring_buffer_bytes` | `uint32` | `4194304` | 否 | payload | 明文事件 ring buffer 容量，字节；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L347) |
| `payload.stdio.pending_operation_max_entries` | `uint32` | `8192` | 否 | payload | 待完成操作表容量；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L348) |
| `payload.stdio.stream_state_max_entries` | `uint32` | `8192` | 否 | payload | 活动流状态条目上限；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L349) |
| `payload.stdio.retention_max_bytes_per_trace` | `uint64` | `104857600` | 否 | payload | 每个 trace 的明文保留字节上限；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L350) |
| `payload.stdio.redaction_policy` | `string` | `"disabled"` | 否 | payload | 明文脱敏策略；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L351) |
| `payload.socket.enabled` | `bool` | `true` | 否 | payload | 采集 socket 明文；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L445) |
| `payload.socket.capture_backend` | `string` | `"bpf-copy"` | 否 | payload | 明文采集后端；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L446) |
| `payload.socket.max_segment_bytes` | `uint32` | `4095` | 否 | payload | 单个采集片段的最大字节数；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L447) |
| `payload.socket.max_operation_bytes` | `uint32` | `4194304` | 否 | payload | 单次操作采集的最大字节数；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L448) |
| `payload.socket.ring_buffer_bytes` | `uint32` | `8388608` | 否 | payload | 明文事件 ring buffer 容量，字节；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L449) |
| `payload.socket.pending_operation_max_entries` | `uint32` | `8192` | 否 | payload | 待完成操作表容量；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L450) |
| `payload.socket.stream_state_max_entries` | `uint32` | `8192` | 否 | payload | 活动流状态条目上限；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L451) |
| `payload.socket.retention_max_bytes_per_trace` | `uint64` | `104857600` | 否 | payload | 每个 trace 的明文保留字节上限；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L452) |
| `payload.socket.redaction_policy` | `string` | `"disabled"` | 否 | payload | 明文脱敏策略；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L453) |
| `payload.socket.http_sniff_max_bytes` | `uint64` | `65536` | 否 | payload | HTTP 协议识别字节预算；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L454) |
| `payload.socket.seccomp_syscalls` | `string[]` | `["write", "writev", "sendto", "sendmsg"]` | 否 | payload | 使用 seccomp 通知处理的系统调用列表；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L455) |
| `payload.mcp.enabled` | `bool` | `false` | 否 | payload | 识别和解析 MCP 消息；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L51) |
| `payload.mcp.parse_buffer_max_bytes` | `uint64` | `4194304` | 否 | payload | MCP 解析缓冲区字节上限；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L52) |
| `payload.mcp.stdio_candidate_max_bytes` | `uint64` | `65536` | 否 | payload | 单个 MCP stdio 候选数据的字节上限；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L53) |
| `payload.mcp.pending_stdio_candidate_max_entries` | `uint32` | `1024` | 否 | payload | 待确认 MCP stdio 候选条目数上限；[声明](../../crates/core/config/src/daemon/operator/document/payload.rs#L54) |
| `seccomp_notify.enabled` | `bool` | `false` | 否 | seccomp_notify | 启用 seccomp 用户态通知处理；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L6) |
| `seccomp_notify.reserved_listener_fd` | `uint32` | `253` | 否 | seccomp_notify | seccomp listener 保留的文件描述符编号；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L7) |
| `process_seccomp.enabled` | `bool` | `false` | 否 | process_seccomp | 通过 seccomp 采集进程执行参数；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L31) |
| `process_seccomp.syscalls` | `string[]` | `["execve", "execveat"]` | 否 | process_seccomp | 观测或控制的系统调用列表；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L32) |
| `process_seccomp.max_args` | `uint32` | `128` | 否 | process_seccomp | 采集的参数个数上限；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L33) |
| `process_seccomp.max_arg_bytes` | `uint32` | `4095` | 否 | process_seccomp | 单个参数的字节上限；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L34) |
| `process_seccomp.max_total_arg_bytes` | `uint32` | `4095` | 否 | process_seccomp | 全部参数的总字节上限；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L35) |
| `process_seccomp.pending_max_entries` | `uint32` | `8192` | 否 | process_seccomp | 待完成执行操作的数量上限；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L37) |
| `agent_invocation.enabled` | `bool` | `true` | 否 | agent_invocation | 启用 Agent 调用识别；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L81) |
| `agent_invocation.commands` | `string[]` | `[]` | 否 | agent_invocation | 用于识别 Agent 调用的命令列表；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L82) |
| `agent_invocation.tool_names` | `string[]` | `["Agent", "Task", "task", "spawn_agent"]` | 否 | agent_invocation | 用于识别 Agent 调用的工具名称列表；[声明](../../crates/core/config/src/daemon/operator/document/process.rs#L83) |
| `semantic_retention.projection_enabled` | `bool` | `true` | 否 | semantic_retention | 启用语义投影；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L17) |
| `semantic_retention.content_owner` | `string` | `"highest_consumed"` | 否 | semantic_retention | 内容保留层级的归属策略；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L18) |
| `semantic_retention.l0_llm_call.enabled` | `bool` | `true` | 否 | semantic_retention | 保留 LLM 语义观测；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L6) |
| `semantic_retention.l0_llm_call.request_content` | `string` | `"canonical_blocks"` | 否 | semantic_retention | 请求正文保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L7) |
| `semantic_retention.l0_llm_call.request_body_export` | `string` | `"none"` | 否 | semantic_retention | 请求正文导出模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L8) |
| `semantic_retention.l0_llm_call.request_body_export_max_bytes` | `uint64` | `131072` | 否 | semantic_retention | 单个请求正文的导出字节上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L9) |
| `semantic_retention.l0_llm_call.response_content` | `string` | `"assembled_provider"` | 否 | semantic_retention | 响应正文保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L10) |
| `semantic_retention.l0_llm_call.tool_calls` | `string` | `"assembled_json"` | 否 | semantic_retention | 工具调用内容保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L11) |
| `semantic_retention.l0_llm_call.tool_results_enabled` | `bool` | `true` | 否 | semantic_retention | 启用工具结果语义投影；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L12) |
| `semantic_retention.l0_llm_call.tool_result_content_export` | `string` | `"none"` | 否 | semantic_retention | 工具结果正文导出模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L13) |
| `semantic_retention.l0_llm_call.tool_result_content_export_max_bytes` | `uint64` | `131072` | 否 | semantic_retention | 单个工具结果的导出字节上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L14) |
| `semantic_retention.l0_llm_call.usage` | `string` | `"summary"` | 否 | semantic_retention | 模型 token 用量保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L15) |
| `semantic_retention.l0_llm_call.retain_assembled_payload` | `bool` | `false` | 否 | semantic_retention | 保留组装后的原始 payload；[声明](../../crates/core/config/src/daemon/operator/document/semantic/llm.rs#L16) |
| `semantic_retention.l0_llm_call.websocket.max_lanes_per_connection` | `uint32` | `32` | 否 | semantic_retention | 单条 WebSocket 连接中的 LLM 请求通道数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/websocket.rs#L67) |
| `semantic_retention.l0_llm_call.websocket.max_pending_requests_per_connection` | `uint32` | `32` | 否 | semantic_retention | 单条 WebSocket 连接中等待关联的请求数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/websocket.rs#L68) |
| `semantic_retention.l0_llm_call.assembly.max_buffer_bytes` | `uint64` | `8388608` | 否 | semantic_retention | 解析或组装缓冲区字节上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L273) |
| `semantic_retention.l0_llm_call.assembly.max_segment_ranges` | `uint32` | `8192` | 否 | semantic_retention | 组装时保留的原始片段范围数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L274) |
| `semantic_retention.l0_llm_call.stream_classifier.soft_sniff_max_bytes` | `uint64` | `16384` | 否 | semantic_retention | LLM 内容分类的软识别字节预算；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L236) |
| `semantic_retention.l0_llm_call.projection_state.max_action_versions_per_trace` | `uint32` | `4096` | 否 | semantic_retention | 每个 trace 保存的 action 版本状态数上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L179) |
| `semantic_retention.l0_llm_call.projection_state.max_pending_trajectory_actions_per_trace` | `uint32` | `1024` | 否 | semantic_retention | 每个 trace 等待轨迹关联的 action 数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L180) |
| `semantic_retention.l0_llm_call.projection_state.max_tool_entries_per_trace` | `uint32` | `4096` | 否 | semantic_retention | 每个 trace 的工具调用状态条目上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L181) |
| `semantic_retention.l0_llm_call.trajectory.enabled` | `bool` | `true` | 否 | semantic_retention | 关联 LLM 请求轨迹；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L320) |
| `semantic_retention.l0_llm_call.trajectory.max_active_trajectories_per_scope` | `uint32` | `128` | 否 | semantic_retention | 每个作用域的活动 LLM 轨迹数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L321) |
| `semantic_retention.l0_llm_call.trajectory.max_candidate_nodes_per_trajectory` | `uint32` | `256` | 否 | semantic_retention | 每条 LLM 轨迹的候选节点数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L322) |
| `semantic_retention.l0_llm_call.trajectory.max_prefix_nodes_per_scope` | `uint32` | `65536` | 否 | semantic_retention | 每个作用域的前缀索引节点数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L323) |
| `semantic_retention.l0_llm_call.trajectory.max_history_atoms_per_request` | `uint32` | `4096` | 否 | semantic_retention | 单个请求的历史内容单元数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L324) |
| `semantic_retention.l0_llm_call.trajectory.max_blocks_per_atom` | `uint32` | `64` | 否 | semantic_retention | 单个历史内容单元的块数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L325) |
| `semantic_retention.l0_llm_call.trajectory.max_structural_bytes_per_atom` | `uint32` | `4096` | 否 | semantic_retention | 单个历史内容单元的结构字节预算；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L326) |
| `semantic_retention.l0_llm_call.trajectory.idle_ttl` | `string` | `"30m"` | 否 | semantic_retention | LLM 轨迹空闲存留时间；使用带时间单位的字符串；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L327) |
| `semantic_retention.l0_mcp_call.request_content` | `string` | `"canonical_json"` | 否 | semantic_retention | 请求正文保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L148) |
| `semantic_retention.l0_mcp_call.response_content` | `string` | `"canonical_json"` | 否 | semantic_retention | 响应正文保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L149) |
| `semantic_retention.l1_sse.enabled` | `bool` | `true` | 否 | semantic_retention | 保留 SSE 层观测；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L409) |
| `semantic_retention.l1_sse.stream_summary` | `bool` | `true` | 否 | semantic_retention | 保留 SSE 流摘要；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L410) |
| `semantic_retention.l1_sse.event_content` | `string` | `"none"` | 否 | semantic_retention | SSE 事件内容保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L411) |
| `semantic_retention.l2_http.enabled` | `bool` | `true` | 否 | semantic_retention | 保留 HTTP 层观测；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L440) |
| `semantic_retention.l2_http.max_active_directions` | `uint32` | `4096` | 否 | semantic_retention | 同时活动的 HTTP 消息方向数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L441) |
| `semantic_retention.l2_http.message_summary` | `bool` | `true` | 否 | semantic_retention | 保留协议消息摘要；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L442) |
| `semantic_retention.l2_http.headers` | `string` | `"metadata"` | 否 | semantic_retention | HTTP 头保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L443) |
| `semantic_retention.l2_http.body_content` | `string` | `"text"` | 否 | semantic_retention | 正文保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L444) |
| `semantic_retention.l2_http.exchange.max_message_facts_per_trace` | `uint32` | `4096` | 否 | semantic_retention | 每个 trace 同时等待关联的 HTTP 消息事实数上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L489) |
| `semantic_retention.l2_http.exchange.max_pending_requests_per_stream` | `uint32` | `256` | 否 | semantic_retention | 每条流中等待关联的 HTTP 请求数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L490) |
| `semantic_retention.l2_http.exchange.max_pending_responses_per_stream` | `uint32` | `32` | 否 | semantic_retention | 每条流中等待关联的 HTTP 响应数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L491) |
| `semantic_retention.l2_http.exchange.response_lateness` | `string` | `"1s"` | 否 | semantic_retention | HTTP 响应迟到容忍时间；使用带时间单位的字符串；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L492) |
| `semantic_retention.l2_http.http1.max_buffer_bytes` | `uint64` | `8388608` | 否 | semantic_retention | 解析或组装缓冲区字节上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/http.rs#L5) |
| `semantic_retention.l2_http.http1.max_evidence_ranges` | `uint32` | `8192` | 否 | semantic_retention | 协议消息保存的证据范围数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/http.rs#L6) |
| `semantic_retention.l2_http.http2.max_active_streams_per_trace` | `uint32` | `4096` | 否 | semantic_retention | 每个 trace 中活动 HTTP/2 流的数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/http.rs#L36) |
| `semantic_retention.l2_http.http2.max_evidence_ranges` | `uint32` | `4096` | 否 | semantic_retention | 协议消息保存的证据范围数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/http.rs#L37) |
| `semantic_retention.l2_http.http2.max_connection_buffer_bytes` | `uint64` | `4194304` | 否 | semantic_retention | HTTP/2 framing 缓冲区上限，完整帧包含 9 字节头；[声明](../../crates/core/config/src/daemon/operator/document/semantic/http.rs#L38) |
| `semantic_retention.l2_http.http2.max_header_block_bytes` | `uint32` | `65536` | 否 | semantic_retention | HPACK 编码头块拼接字节上限 |
| `semantic_retention.l2_http.http2.max_header_list_bytes` | `uint32` | `65536` | 否 | semantic_retention | 解码头列表上限，按 name + value + 32 累计 |
| `semantic_retention.l2_http.http2.max_hpack_table_bytes` | `uint32` | `65536` | 否 | semantic_retention | 每个连接方向的 HPACK 动态表硬上限 |
| `semantic_retention.l2_websocket.enabled` | `bool` | `true` | 否 | semantic_retention | 保留 WebSocket 层观测；[声明](../../crates/core/config/src/daemon/operator/document/semantic/websocket.rs#L6) |
| `semantic_retention.l2_websocket.message_summary` | `bool` | `true` | 否 | semantic_retention | 保留协议消息摘要；[声明](../../crates/core/config/src/daemon/operator/document/semantic/websocket.rs#L7) |
| `semantic_retention.l2_websocket.max_active_connections` | `uint32` | `4096` | 否 | semantic_retention | 活动 WebSocket 连接数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/websocket.rs#L8) |
| `semantic_retention.l2_websocket.max_frame_buffer_bytes` | `uint64` | `8388608` | 否 | semantic_retention | WebSocket 帧缓冲区字节上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/websocket.rs#L9) |
| `semantic_retention.l2_websocket.max_message_bytes` | `uint64` | `16777216` | 否 | semantic_retention | WebSocket 原始消息字节上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/websocket.rs#L10) |
| `semantic_retention.l2_websocket.max_decoded_message_bytes` | `uint64` | `33554432` | 否 | semantic_retention | WebSocket 解码消息字节上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/websocket.rs#L11) |
| `semantic_retention.l2_websocket.max_evidence_ranges` | `uint32` | `4096` | 否 | semantic_retention | 协议消息保存的证据范围数量上限；[声明](../../crates/core/config/src/daemon/operator/document/semantic/websocket.rs#L12) |
| `semantic_retention.l2_websocket.compact_threshold_bytes` | `uint64` | `65536` | 否 | semantic_retention | WebSocket 缓冲区压缩整理的字节阈值；[声明](../../crates/core/config/src/daemon/operator/document/semantic/websocket.rs#L13) |
| `semantic_retention.l3_http2_frame.enabled` | `bool` | `true` | 否 | semantic_retention | 保留 HTTP/2 帧观测；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L536) |
| `semantic_retention.l3_http2_frame.frame_summary` | `bool` | `true` | 否 | semantic_retention | 保留 HTTP/2 帧摘要；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L537) |
| `semantic_retention.l3_http2_frame.data_content` | `string` | `"none"` | 否 | semantic_retention | HTTP/2 DATA 内容保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L538) |
| `semantic_retention.l4_payload.enabled` | `bool` | `false` | 否 | semantic_retention | 保留原始 payload 层观测；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L567) |
| `semantic_retention.l4_payload.stats` | `bool` | `true` | 否 | semantic_retention | 保留 payload 统计信息；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L568) |
| `semantic_retention.l4_payload.body_content` | `string` | `"none"` | 否 | semantic_retention | 正文保留模式；[声明](../../crates/core/config/src/daemon/operator/document/semantic.rs#L569) |
| `idle_detection.enabled` | `bool` | `false` | 否 | idle_detection | 启用执行空闲检测；[声明](../../crates/core/config/src/daemon/operator/document/idle_detection.rs#L9) |
| `idle_detection.threshold_secs` | `string` | `"30s"` | 否 | idle_detection | 判为空闲的持续时间；使用带时间单位的字符串；[声明](../../crates/core/config/src/daemon/operator/document/idle_detection.rs#L10) |
| `idle_detection.poll_interval_secs` | `string` | `"2s"` | 否 | idle_detection | 空闲检查间隔；使用带时间单位的字符串；[声明](../../crates/core/config/src/daemon/operator/document/idle_detection.rs#L11) |
| `file_observation.enabled` | `bool` | `true` | 否 | file_observation | 启用文件操作观测；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L114) |
| `file_observation.collection.writable_open` | `bool` | `true` | 否 | file_observation | 采集以可写方式打开文件的操作；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L47) |
| `file_observation.collection.path_mutations` | `bool` | `true` | 否 | file_observation | 采集文件路径变更操作；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L48) |
| `file_observation.collection.fd_mutations` | `bool` | `false` | 否 | file_observation | 采集文件描述符变更操作；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L49) |
| `file_observation.collection.read.observed` | `bool` | `false` | 否 | file_observation | 记录该类 I/O 是否出现；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L85) |
| `file_observation.collection.read.counts` | `bool` | `false` | 否 | file_observation | 记录该类 I/O 操作次数；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L86) |
| `file_observation.collection.read.bytes` | `bool` | `false` | 否 | file_observation | 记录该类 I/O 字节数；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L87) |
| `file_observation.collection.read.errors` | `bool` | `false` | 否 | file_observation | 记录该类 I/O 错误；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L88) |
| `file_observation.collection.write.observed` | `bool` | `false` | 否 | file_observation | 记录该类 I/O 是否出现；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L85) |
| `file_observation.collection.write.counts` | `bool` | `false` | 否 | file_observation | 记录该类 I/O 操作次数；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L86) |
| `file_observation.collection.write.bytes` | `bool` | `false` | 否 | file_observation | 记录该类 I/O 字节数；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L87) |
| `file_observation.collection.write.errors` | `bool` | `false` | 否 | file_observation | 记录该类 I/O 错误；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L88) |
| `file_observation.summary.flush_interval_ms` | `uint32` | `100` | 否 | file_observation | 缓冲数据刷新间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L6) |
| `file_observation.summary.max_entries` | `uint32` | `16384` | 否 | file_observation | 文件 I/O 聚合条目数量上限；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L7) |
| `file_observation.summary.object_max_entries` | `uint32` | `16384` | 否 | file_observation | 文件对象状态数量上限；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L8) |
| `file_observation.metadata_retention` | `string` | `"compact"` | 否 | file_observation | 文件元数据保留模式；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L117) |
| `file_observation.tty.enabled` | `bool` | `false` | 否 | file_observation | 启用 TTY 文件读写观测；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L229) |
| `file_observation.tty.paths` | `string[]` | `["/dev/tty", "/dev/pts/*", "/dev/ptmx"]` | 否 | file_observation | TTY 路径匹配列表；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L230) |
| `file_observation.tty.operations` | `string[]` | `["read", "readv", "write", "writev"]` | 否 | file_observation | TTY 操作列表，支持 read、readv、write、writev；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L231) |
| `file_observation.tty.raw_event_retention` | `string` | `"summary"` | 否 | file_observation | 原始文件事件保留模式；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L232) |
| `file_observation.bulk_read.enabled` | `bool` | `true` | 否 | file_observation | 聚合批量文件读取；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L282) |
| `file_observation.bulk_read.mode` | `string` | `"path_set"` | 否 | file_observation | 批量文件读取的聚合模式；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L283) |
| `file_observation.bulk_read.raw_event_retention` | `string` | `"errors_only"` | 否 | file_observation | 原始文件事件保留模式；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L284) |
| `file_observation.bulk_read.max_paths_per_set` | `uint32` | `4096` | 否 | file_observation | 单个路径集合的路径数量上限；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L285) |
| `file_observation.bulk_read.path_set_chunk_max_paths` | `uint32` | `256` | 否 | file_observation | 单个路径集合分块的路径数量上限；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L286) |
| `file_observation.enumerate.enabled` | `bool` | `false` | 否 | file_observation | 聚合文件路径枚举；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L325) |
| `file_observation.enumerate.raw_event_retention` | `string` | `"errors_only"` | 否 | file_observation | 原始文件事件保留模式；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L326) |
| `file_observation.enumerate.min_unique_paths` | `uint32` | `2` | 否 | file_observation | 形成枚举聚合所需的最小不同路径数；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L327) |
| `file_observation.enumerate.max_paths_per_set` | `uint32` | `4096` | 否 | file_observation | 单个路径集合的路径数量上限；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L328) |
| `file_observation.enumerate.path_set_chunk_max_paths` | `uint32` | `256` | 否 | file_observation | 单个路径集合分块的路径数量上限；[声明](../../crates/core/config/src/daemon/operator/document/file.rs#L329) |
| `application.enabled` | `bool` | `true` | 否 | application | 启用应用协议观测；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L6) |
| `application.http1_enabled` | `bool` | `true` | 否 | application | 启用 HTTP/1 协议观测；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L7) |
| `application.http2_enabled` | `bool` | `true` | 否 | application | 启用 HTTP/2 协议观测；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L8) |
| `application.http.capture_host` | `bool` | `true` | 否 | application | 记录 HTTP Host；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L76) |
| `application.http.sse_enabled` | `bool` | `false` | 否 | application | 启用 SSE 观测；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L77) |
| `application.http.sse_data_policy` | `string` | `"disabled"` | 否 | application | SSE data 保留策略；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L78) |
| `application.http.sse_max_buffer_bytes` | `uint64` | `4194304` | 否 | application | SSE 解析缓冲区字节上限；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L79) |
| `application.http.sse_max_data_bytes` | `uint64` | `65536` | 否 | application | 单个 SSE data 内容字节上限；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L80) |
| `application.http2.emit_data_preview` | `bool` | `true` | 否 | application | 输出 HTTP/2 DATA 预览；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L98) |
| `application.http2.max_data_preview_bytes` | `uint64` | `65536` | 否 | application | HTTP/2 DATA 预览字节上限；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L99) |
| `provider.rules_enabled` | `bool` | `false` | 否 | provider | 启用模型服务方规则文件；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L114) |
| `provider.rules_path` | `string` | `"/etc/actrail/provider-rules.conf"` | 否 | provider | 规则文件路径；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L115) |
| `provider.unknown_provider_label` | `string` | `"unknown"` | 否 | provider | 未识别模型服务方的标签；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L116) |
| `access_control.file.open.enabled` | `bool` | `true` | 否 | access_control | 操作控制开关；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L60) |
| `access_control.file.open.backend` | `string` | `"lsm"` | 否 | access_control | 操作控制后端；fanotify / lsm；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L60) |
| `access_control.file.mkdir.enabled` | `bool` | `false` | 否 | access_control | 操作控制开关；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L61) |
| `access_control.file.mkdir.backend` | `string` | `"lsm"` | 否 | access_control | 操作控制后端；seccomp / lsm；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L61) |
| `access_control.file.rmdir.enabled` | `bool` | `false` | 否 | access_control | 操作控制开关；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L62) |
| `access_control.file.rmdir.backend` | `string` | `"lsm"` | 否 | access_control | 操作控制后端；seccomp / lsm；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L62) |
| `access_control.command.exec.enabled` | `bool` | `true` | 否 | access_control | 操作控制开关；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L103) |
| `access_control.command.exec.backend` | `string` | `"lsm"` | 否 | access_control | 操作控制后端；seccomp / lsm；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L103) |
| `access_control.network.connect.enabled` | `bool` | `true` | 否 | access_control | 操作控制开关；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L109) |
| `access_control.network.connect.backend` | `string` | `"lsm"` | 否 | access_control | 操作控制后端；seccomp / lsm；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L109) |
| `access_control.backends.lsm.policy_max_entries` | `uint32` | `4096` | 否 | access_control | 每份 LSM 完整策略的条目数上限，包含默认项；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L140) |
| `access_control.backends.lsm.path_max_bytes` | `uint32` | `252` | 否 | access_control | 路径缓冲区字节上限；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L142) |
| `access_control.backends.lsm.audit_buffer_bytes` | `uint32` | `1048576` | 否 | access_control | LSM 审计 ring buffer 字节数；须为不小于 65536 的二次幂；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L144) |
| `access_control.backends.lsm.audit_batch_max` | `uint32` | `1024` | 否 | access_control | 每轮处理的 LSM 审计事件数量上限；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L146) |
| `access_control.backends.lsm.audit_default_allow` | `bool` | `false` | 否 | access_control | 审计默认策略允许的操作；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L148) |
| `access_control.backends.lsm.alert_window_ms` | `uint32` | `60000` | 否 | access_control | 边界告警聚合窗口，毫秒；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L150) |
| `access_control.backends.lsm.alert_max_groups` | `uint32` | `256` | 否 | access_control | 每个窗口接纳的边界告警组数量上限；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L152) |
| `access_control.backends.lsm.deny_on_error` | `bool` | `true` | 否 | access_control | 无法解析资源时拒绝访问；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L154) |
| `access_control.backends.lsm.deny_signal` | `string` | `"none"` | 否 | access_control | 拒绝访问后发送的信号：none、stop 或 kill；[声明](../../crates/core/config/src/daemon/access_control/config.rs#L156) |
| `enforcement.enabled` | `bool` | `true` | 否 | enforcement | 启用文件访问治理；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L141) |
| `enforcement.scope` | `string` | `"trace"` | 否 | enforcement | 文件治理作用域；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L142) |
| `enforcement.rules_path` | `string` | `"/etc/actrail/enforcement-rules.conf"` | 否 | enforcement | 规则文件路径；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L143) |
| `enforcement.builtin_rules` | `object[]` | `[]` | 否 | enforcement | 配置项列表；元素字段见下列完整路径；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L144) |
| `enforcement.builtin_rules[].rule_id` | `string` | `""` | 提供列表元素时必填且非空 | enforcement | 内置文件治理规则标识；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L197) |
| `enforcement.builtin_rules[].path` | `string` | `""` | 提供列表元素时必填且非空 | enforcement | 文件路径；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L198) |
| `enforcement.default_decision` | `string` | `"allow"` | 否 | enforcement | 未匹配规则时的默认裁决；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L145) |
| `enforcement.mark_strategy` | `string` | `"parent-directories"` | 否 | enforcement | 文件治理标记策略；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L146) |
| `enforcement.audit_enabled` | `bool` | `true` | 否 | enforcement | 启用治理审计；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L147) |
| `enforcement.event_buffer_bytes` | `uint32` | `65536` | 否 | enforcement | 文件治理事件缓冲区字节数；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L148) |
| `enforcement.seccomp_path_max_bytes` | `uint32` | `4096` | 否 | enforcement | seccomp 文件路径采集字节上限；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L149) |
| `command_control.enabled` | `bool` | `false` | 否 | command_control | 启用命令执行治理；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L6) |
| `command_control.rules_path` | `string` | `"/etc/actrail/command-control.rules"` | 否 | command_control | 规则文件路径；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L7) |
| `command_control.default_decision` | `string` | `"allow"` | 否 | command_control | 未匹配规则时的默认裁决；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L8) |
| `command_control.failure_decision` | `string` | `"deny"` | 否 | command_control | 治理执行失败时的裁决；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L9) |
| `command_control.audit_enabled` | `bool` | `true` | 否 | command_control | 启用治理审计；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L10) |
| `command_control.audit_default_allow` | `bool` | `false` | 否 | command_control | 审计默认策略允许的操作；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L11) |
| `command_control.path_max_bytes` | `uint32` | `4096` | 否 | command_control | 路径缓冲区字节上限；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L12) |
| `command_control.argv_max_count` | `uint32` | `128` | 否 | command_control | 命令参数数量上限；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L13) |
| `command_control.argv_max_arg_bytes` | `uint32` | `8192` | 否 | command_control | 单个命令参数字节上限；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L14) |
| `command_control.argv_max_total_bytes` | `uint32` | `65536` | 否 | command_control | 命令参数总字节上限；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L15) |
| `command_control.pending_decision_max` | `uint32` | `64` | 否 | command_control | 等待治理裁决的操作数量上限；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L16) |
| `command_control.reusable_cache_max_entries` | `uint32` | `4096` | 否 | command_control | 可复用裁决缓存的条目数量上限；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L17) |
| `command_control.gray.timeout_ms` | `uint64` | `5000` | 否 | command_control | 外部灰名单裁决等待上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L24) |
| `command_control.gray.concurrency_limit` | `uint32` | `8` | 否 | command_control | 同时执行的灰名单裁决数量上限；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L25) |
| `command_control.gray.fallback` | `string` | `"deny"` | 否 | command_control | 灰名单裁决失败或超时后的裁决；[声明](../../crates/core/config/src/daemon/operator/document/command.rs#L26) |
| `network_control.enabled` | `bool` | `false` | 否 | network_control | 启用网络连接治理；[声明](../../crates/core/config/src/daemon/operator/document/network.rs#L6) |
| `network_control.rules_path` | `string` | `"/etc/actrail/network-control.rules"` | 否 | network_control | 规则文件路径；[声明](../../crates/core/config/src/daemon/operator/document/network.rs#L7) |
| `network_control.syscalls` | `string[]` | `["connect"]` | 否 | network_control | 观测或控制的系统调用列表；[声明](../../crates/core/config/src/daemon/operator/document/network.rs#L8) |
| `network_control.default_decision` | `string` | `"allow"` | 否 | network_control | 未匹配规则时的默认裁决；[声明](../../crates/core/config/src/daemon/operator/document/network.rs#L9) |
| `network_control.failure_decision` | `string` | `"deny"` | 否 | network_control | 治理执行失败时的裁决；[声明](../../crates/core/config/src/daemon/operator/document/network.rs#L10) |
| `network_control.audit_enabled` | `bool` | `true` | 否 | network_control | 启用治理审计；[声明](../../crates/core/config/src/daemon/operator/document/network.rs#L11) |
| `network_control.audit_default_allow` | `bool` | `false` | 否 | network_control | 审计默认策略允许的操作；[声明](../../crates/core/config/src/daemon/operator/document/network.rs#L12) |
| `network_control.pending_decision_max` | `uint32` | `64` | 否 | network_control | 等待治理裁决的操作数量上限；[声明](../../crates/core/config/src/daemon/operator/document/network.rs#L13) |
| `network_control.reusable_cache_max_entries` | `uint32` | `4096` | 否 | network_control | 可复用裁决缓存的条目数量上限；[声明](../../crates/core/config/src/daemon/operator/document/network.rs#L14) |
| `supervision.startup_wait_ms` | `uint64` | `30000` | 否 | supervision | CLI 等待 daemon 启动的时间上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L235) |
| `supervision.shutdown_wait_ms` | `uint64` | `150100` | 否 | supervision | CLI 等待 daemon 关闭的时间上限，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L236) |
| `supervision.poll_interval_ms` | `uint64` | `100` | 否 | supervision | 轮询间隔，毫秒；[声明](../../crates/core/config/src/daemon/operator/document/app.rs#L237) |
| `resource_profiling.enabled` | `bool` | `true` | 否 | resource_profiling | 总开关；关闭时不启动任何资源采集任务；[声明](../../crates/core/resource_profiling/src/config/resource_profiling_config.rs) |
| `resource_profiling.collection.host.enabled` | `bool` | `true` | 否 | resource_profiling | 启用宿主采集，不依赖 workload；[声明](../../crates/core/resource_profiling/src/collection/host/config/host_profiling_config.rs) |
| `resource_profiling.collection.host.mode` | `string` | `"linux"` | 否 | resource_profiling | `linux`：真实宿主采集；`noop`：空采样后端；[声明](../../crates/core/resource_profiling/src/collection/host/config/host_profiling_config.rs) |
| `resource_profiling.collection.host.interval_ms` | `uint64` | `1000` | 否 | resource_profiling | 宿主采样周期，毫秒，须大于 0；[声明](../../crates/core/resource_profiling/src/collection/host/config/host_profiling_config.rs) |
| `resource_profiling.collection.workload.enabled` | `bool` | `true` | 否 | resource_profiling | 启用工作负载采集，与 host 可同时开启；[声明](../../crates/core/resource_profiling/src/collection/workload/config/workload_profiling_config.rs) |
| `resource_profiling.collection.workload.interval_ms` | `uint64` | `1000` | 否 | resource_profiling | 工作负载采样周期，毫秒，须大于 0；[声明](../../crates/core/resource_profiling/src/collection/workload/config/workload_profiling_config.rs) |
| `resource_profiling.collection.workload.mode` | `string` | `"auto"` | 否 | resource_profiling | `procfs`：使用进程采集；`cgroup`：要求有效 cgroup 绑定；`auto`：优先适用的 cgroup，允许降级时选 procfs；配置错误不降级；`noop`：空采样后端，不解析 procfs/cgroup 子表；[声明](../../crates/core/resource_profiling/src/collection/workload/config/workload_profiling_config.rs) |
| `resource_profiling.collection.workload.procfs.include_children` | `bool` | `true` | 否 | resource_profiling | 包含工作负载后代进程；false 时只采根进程；仅影响 procfs 分支；[声明](../../crates/core/resource_profiling/src/collection/workload/procfs/config/procfs_workload_profiling_config.rs) |
| `resource_profiling.collection.workload.cgroup.mode` | `string` | `"auto"` | 否 | resource_profiling | `auto`：按目标绑定与指标能力优先选 v2，其次 v1；`v1/v2`：固定版本；无可用候选时交外层 workload.mode 处理；配置错误不降级；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/config/cgroup_profiling_config.rs) |
| `resource_profiling.collection.workload.cgroup.v1.binding_attempts` | `uint32` | `2` | 否 | resource_profiling | 绑定时成员路径变化的总尝试次数，须大于 0；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v1/config/cgroup_v1_config.rs) |
| `resource_profiling.collection.workload.cgroup.v1.allow_hierarchy_root` | `bool` | `false` | 否 | resource_profiling | 允许绑定可见 controller 层级根；其计数包含该根范围内全部工作负载；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v1/config/cgroup_v1_config.rs) |
| `resource_profiling.collection.workload.cgroup.v1.managed.enabled` | `bool` | `true` | 否 | resource_profiling | 启用 v1 自建范围；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v1/managed/config/managed_v1_scope_config.rs) |
| `resource_profiling.collection.workload.cgroup.v1.managed.root_name` | `string` | `"actrail"` | 否 | resource_profiling | 每个 v1 层级挂载点下的根目录名，启动时自动创建；只能是单个 `[A-Za-z0-9_-]` 目录名；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v1/managed/config/managed_v1_scope_config.rs) |
| `resource_profiling.collection.workload.cgroup.v1.managed.recovery_scan_limit` | `uint32` | `1024` | 否 | resource_profiling | 每个层级的启动恢复扫描数量上限，须大于 0；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v1/managed/config/managed_v1_scope_config.rs) |
| `resource_profiling.collection.workload.cgroup.v1.managed.finalization_timeout_ms` | `uint64` | `30000` | 否 | resource_profiling | managed 最终采样等待上限，毫秒，须大于 0；超时输出 partial；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v1/managed/config/managed_v1_scope_config.rs) |
| `resource_profiling.collection.workload.cgroup.v2.managed.root` | `string` | `"/sys/fs/cgroup/actrail"` | 否 | resource_profiling | 自建范围根路径；须为绝对路径且不能为 `/`；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v2/managed/config/managed_v2_scope_config.rs) |
| `resource_profiling.collection.workload.cgroup.v2.managed.recovery_scan_limit` | `uint32` | `1024` | 否 | resource_profiling | 启动恢复扫描数量上限，须大于 0；达到上限报告恢复未完成；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v2/managed/config/managed_v2_scope_config.rs) |
| `resource_profiling.collection.workload.cgroup.v2.managed.finalization_timeout_ms` | `uint64` | `30000` | 否 | resource_profiling | managed 最终采样等待上限，毫秒，须大于 0；超时输出 partial；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v2/managed/config/managed_v2_scope_config.rs) |
| `resource_profiling.collection.workload.cgroup.v2.external.policy` | `string` | `"disabled"` | 否 | resource_profiling | `disabled`：关闭 v2 外部接入；`prefer`：绑定不可用时交 Factory 选择下一候选；`require`：核验失败则拒绝目标接入；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v2/binding/config/binding_config.rs) |
| `resource_profiling.collection.workload.cgroup.v2.external.failure_threshold` | `uint32` | `3` | 否 | resource_profiling | policy 启用时校验，须大于 0；连续可重试失败达到阈值则绑定失效，成功重置计数；身份不匹配立即失效；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v2/binding/config/binding_config.rs) |
| `resource_profiling.collection.workload.cgroup.v2.external.binding_attempts` | `uint32` | `2` | 否 | resource_profiling | policy 启用时须大于 0；绑定期间成员路径变化时重新核验的总尝试次数；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/implementations/v2/binding/config/binding_config.rs) |
| `resource_profiling.collection.workload.cgroup.v2.process_rss.enabled` | `bool` | `false` | 否 | resource_profiling | 额外采集 v2 绑定范围及后代范围内的进程 RSS；[声明](../../crates/core/resource_profiling/src/collection/workload/cgroup/common/sampler/process_rss/config/cgroup_process_rss_config.rs) |
| `resource_profiling.storage.backend` | `string` | `"database"` | 否 | resource_profiling | `remote`：第三方外发；`database`：本地数据库；`hybrid`：多后端 best-effort 分发；[声明](../../crates/core/resource_profiling/src/storage/config/resource_storage_config.rs) |
| `resource_profiling.storage.queue_capacity_batches` | `uint32` | `64` | 否 | resource_profiling | remote/database 的后台队列容量，须大于 0；满时丢弃本批；Hybrid 使用自身队列配置；[声明](../../crates/core/resource_profiling/src/storage/config/resource_storage_config.rs) |
| `resource_profiling.storage.remote.protocol` | `string` | 无 | 是（选择 remote 后端） | resource_profiling | 外发协议：`prometheus_remote_write`、`influxdb_line_protocol`；[声明](../../crates/core/resource_profiling/src/storage/remote/config/remote_resource_storage_config.rs) |
| `resource_profiling.storage.remote.endpoint` | `string` | 无 | 是（选择 remote 后端） | resource_profiling | 完整 HTTP(S) 写入地址，包含接收端要求的数据库等查询参数；[声明](../../crates/core/resource_profiling/src/storage/remote/config/remote_resource_storage_config.rs) |
| `resource_profiling.storage.remote.timeout_ms` | `uint64` | `1000` | 否 | resource_profiling | 单次远端请求超时，毫秒，须大于 0；[声明](../../crates/core/resource_profiling/src/storage/remote/config/remote_resource_storage_config.rs) |
| `resource_profiling.storage.database.backend` | `string` | `"sqlite"` | 否 | resource_profiling | 由 DatabaseStorageFactory 选择本地数据库实现；[声明](../../crates/core/resource_profiling/src/storage/database/config/database_storage_config.rs) |
| `resource_profiling.storage.database.sqlite.path` | `string` | `"./actrail-resources.sqlite"` | 否 | resource_profiling | 资源数据库文件，可显式设为主 SQLite 同一路径；相对路径以 operator 配置文件目录为基准；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_storage_sqlite_config.rs) |
| `resource_profiling.storage.database.sqlite.write_timeout_ms` | `uint64` | `1000` | 否 | resource_profiling | SQLite 单批写入与锁等待上限，毫秒，须大于 0；Hybrid 可施加更短超时；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_storage_sqlite_config.rs) |
| `resource_profiling.storage.database.sqlite.retention.enabled` | `bool` | `true` | 否 | resource_profiling | 启用过期资源样本清理；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_retention_config.rs) |
| `resource_profiling.storage.database.sqlite.retention.max_age_seconds` | `uint64` | `604800` | 否 | resource_profiling | 样本保留期限，秒，须大于 0；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_retention_config.rs) |
| `resource_profiling.storage.database.sqlite.retention.sweep_interval_seconds` | `uint64` | `60` | 否 | resource_profiling | 清理周期，秒，须大于 0；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_retention_config.rs) |
| `resource_profiling.storage.database.sqlite.retention.max_samples_per_sweep` | `uint32` | `4096` | 否 | resource_profiling | 每批删除的样本数，须大于 0；每轮按批循环删到没有过期样本或用满 50 ms，没删完时下次写入立即继续；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_retention_config.rs) |
| `resource_profiling.storage.hybrid.queue_capacity_batches` | `uint32` | `64` | 否 | resource_profiling | 每个子后端独立队列的批次数量上限，须大于 0；满时丢弃该目标本批请求；[声明](../../crates/core/resource_profiling/src/storage/hybrid/config/hybrid_resource_storage_config.rs) |
| `resource_profiling.storage.hybrid.write_timeout_ms` | `uint64` | `1000` | 否 | resource_profiling | 每个子后端单批写入的等待上限，毫秒，须大于 0；超时局部记录；[声明](../../crates/core/resource_profiling/src/storage/hybrid/config/hybrid_resource_storage_config.rs) |
| `resource_profiling.storage.hybrid.backends` | `object[]` | 无 | 选择 hybrid 后端时必填且非空 | resource_profiling | 至少配置一个子后端；每项指定唯一名称、后端类型和对应子配置；[声明](../../crates/core/resource_profiling/src/storage/hybrid/config/hybrid_resource_storage_config.rs) |
| `resource_profiling.storage.hybrid.backends[].name` | `string` | 无 | 是（选择 hybrid 后端） | resource_profiling | 本 Hybrid 内唯一的非空名称，用于标识外发目标与局部错误；[声明](../../crates/core/resource_profiling/src/storage/hybrid/config/hybrid_child_config.rs) |
| `resource_profiling.storage.hybrid.backends[].backend` | `string` | 无 | 是（选择 hybrid 后端） | resource_profiling | `remote` 或 `database`；Hybrid Factory 委托对应 Factory 构造；[声明](../../crates/core/resource_profiling/src/storage/hybrid/config/hybrid_child_config.rs) |
| `resource_profiling.alerts.cpu_alert_percent_millis` | `string` | `"disabled"` | 否 | resource_profiling | 正整数字符串为 CPU 告警阈值；100000 表示一个逻辑 CPU 的 100%；[声明](../../crates/core/resource_profiling/src/alerts/resource_alert_config.rs) |
| `resource_profiling.alerts.memory_alert_rss_kb` | `string` | `"disabled"` | 否 | resource_profiling | 正整数字符串为 RSS 阈值；缺失指标标记 unknown；[声明](../../crates/core/resource_profiling/src/alerts/resource_alert_config.rs) |
| `resource_profiling.alerts.memory_alert_current_bytes` | `string` | `"disabled"` | 否 | resource_profiling | 正整数字符串为 cgroup charged memory 阈值；阈值不控制采集开关；[声明](../../crates/core/resource_profiling/src/alerts/resource_alert_config.rs) |
| `resource_profiling.storage.remote.headers.<name>` | `string` | 无（默认无自定义请求头） | 否 | resource_profiling | remote 后端的自定义 HTTP 请求头；[声明](../../crates/core/resource_profiling/src/storage/remote/config/remote_resource_storage_config.rs) |
| `resource_profiling.storage.hybrid.backends[].remote.protocol` | `string` | 无 | 是（该 hybrid 子后端选择 remote） | resource_profiling | 该 hybrid 子后端选择 remote 时使用；外发协议：`prometheus_remote_write`、`influxdb_line_protocol`；[声明](../../crates/core/resource_profiling/src/storage/remote/config/remote_resource_storage_config.rs) |
| `resource_profiling.storage.hybrid.backends[].remote.endpoint` | `string` | 无 | 是（该 hybrid 子后端选择 remote） | resource_profiling | 该 hybrid 子后端选择 remote 时使用；完整 HTTP(S) 写入地址，包含接收端要求的数据库等查询参数；[声明](../../crates/core/resource_profiling/src/storage/remote/config/remote_resource_storage_config.rs) |
| `resource_profiling.storage.hybrid.backends[].remote.timeout_ms` | `uint64` | `1000` | 否 | resource_profiling | 该 hybrid 子后端选择 remote 时使用；单次远端请求超时，毫秒，须大于 0；[声明](../../crates/core/resource_profiling/src/storage/remote/config/remote_resource_storage_config.rs) |
| `resource_profiling.storage.hybrid.backends[].database.backend` | `string` | `"sqlite"` | 否 | resource_profiling | 该 hybrid 子后端选择 database 时使用；由 DatabaseStorageFactory 选择本地数据库实现；[声明](../../crates/core/resource_profiling/src/storage/database/config/database_storage_config.rs) |
| `resource_profiling.storage.hybrid.backends[].database.sqlite.path` | `string` | `"./actrail-resources.sqlite"` | 否 | resource_profiling | 该 hybrid 子后端选择 database 时使用；资源数据库文件，可显式设为主 SQLite 同一路径；相对路径以 operator 配置文件目录为基准；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_storage_sqlite_config.rs) |
| `resource_profiling.storage.hybrid.backends[].database.sqlite.write_timeout_ms` | `uint64` | `1000` | 否 | resource_profiling | 该 hybrid 子后端选择 database 时使用；SQLite 单批写入与锁等待上限，毫秒，须大于 0；Hybrid 可施加更短超时；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_storage_sqlite_config.rs) |
| `resource_profiling.storage.hybrid.backends[].database.sqlite.retention.enabled` | `bool` | `true` | 否 | resource_profiling | 该 hybrid 子后端选择 database 时使用；启用过期资源样本清理；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_retention_config.rs) |
| `resource_profiling.storage.hybrid.backends[].database.sqlite.retention.max_age_seconds` | `uint64` | `604800` | 否 | resource_profiling | 该 hybrid 子后端选择 database 时使用；样本保留期限，秒，须大于 0；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_retention_config.rs) |
| `resource_profiling.storage.hybrid.backends[].database.sqlite.retention.sweep_interval_seconds` | `uint64` | `60` | 否 | resource_profiling | 该 hybrid 子后端选择 database 时使用；清理周期，秒，须大于 0；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_retention_config.rs) |
| `resource_profiling.storage.hybrid.backends[].database.sqlite.retention.max_samples_per_sweep` | `uint32` | `4096` | 否 | resource_profiling | 该 hybrid 子后端选择 database 时使用；每批删除的样本数，须大于 0；每轮按批循环删到没有过期样本或用满 50 ms，没删完时下次写入立即继续；[声明](../../crates/core/resource_profiling/src/storage/database/sqlite/config/resource_retention_config.rs) |
| `resource_profiling.storage.hybrid.backends[].remote.headers.<name>` | `string` | 无（默认无自定义请求头） | 否 | resource_profiling | 该 hybrid 子后端选择 remote 时使用；remote 后端的自定义 HTTP 请求头；[声明](../../crates/core/resource_profiling/src/storage/remote/config/remote_resource_storage_config.rs) |
| `ACTRAIL_WEB_TIMELINE_DEFAULT_LIMIT` | `integer`（环境变量） | `250` | 否 | web | 时间线默认每页数量；不得超过最大每页数量。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_TIMELINE_MAX_LIMIT` | `integer`（环境变量） | `1000` | 否 | web | 时间线每页数量上限；不得超过候选预算。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_TIMELINE_CANDIDATES` | `integer`（环境变量） | `2000` | 否 | web | 时间线查询的候选扫描数量上限。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_TIMELINE_LINKS` | `integer`（环境变量） | `2000` | 否 | web | 时间线查询的关系数量预算。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_IDLE_DEFAULT_LIMIT` | `integer`（环境变量） | `100` | 否 | web | 显式 idle 查询的默认每页数量；不得超过最大每页数量。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_IDLE_MAX_LIMIT` | `integer`（环境变量） | `500` | 否 | web | 显式 idle 查询的每页数量上限；不得超过候选预算。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_IDLE_CANDIDATES` | `integer`（环境变量） | `1000` | 否 | web | 显式 idle 查询的候选数量预算。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_CONTENT_MAX_BYTES` | `integer`（环境变量） | `16777216` | 否 | web | 正文返回字节数上限。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_CONTENT_NODE_MAX_ITEMS` | `integer`（环境变量） | `250` | 否 | web | 正文节点每页条目数上限。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_CONTENT_NODE_PREVIEW_BYTES` | `integer`（环境变量） | `65536` | 否 | web | 正文叶子节点与普通 HTTP 正文的默认页字节数。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_CONTENT_NODE_BLOCK_BYTES` | `integer`（环境变量） | `8388608` | 否 | web | 单次正文查询读取 block 和解码普通 HTTP 正文的字节预算。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_TRACES_DEFAULT_LIMIT` | `integer`（环境变量） | `50` | 否 | web | 本地与 Cluster trace 列表的默认每页数量；不得超过最大每页数量。启动时读取，须为小于 i64::MAX 的正整数。 |
| `ACTRAIL_WEB_TRACES_MAX_LIMIT` | `integer`（环境变量） | `250` | 否 | web | 本地与 Cluster trace 列表的每页数量上限。启动时读取，须为小于 i64::MAX 的正整数。 |
