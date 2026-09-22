# Agent 执行停滞检测

`idle_detector` 检测仍有任务需要推进、却持续没有进入模型请求、工具调用或用户交互等待的执行单元。默认阈值为 30 秒，每 2 秒巡检一次。模型尚未返回、工具仍在运行、等待用户批准或输入，以及已经完成的任务均不计入停滞。

这里的告警表示观测到的执行停滞，不能单凭告警确定进程死锁。接入必须提供可靠的任务和工作边界；缺少这些信号的 Agent 无法通过此功能判断停滞。

## 模块与状态

Agent 接入模块按 `(trace_id, session_id, task_id)` 保存唯一执行状态，包括模型、工具及用户交互的在途计数。任务开始建立状态，任务完成移除状态，会话关闭和 trace 结束清理所属执行单元。计数异常使该执行单元退出检测，避免用不完整状态发出误报；后续任务重新建立状态。

检测功能关闭时，状态更新立即返回，不建立执行状态 map。启用后，收到一个生命周期事件只更新对应执行单元。三个计数均为零时开始计时；进入任一种等待会取消该次计时，等待结束后从新的时刻重新计时。计时采用 daemon 接收事件时的单调时钟，延迟到达的旧时间戳不会直接触发告警。

`idle_detector` 持有巡检定时器及已经告警的区间。到达巡检时间才检查 map，连续停滞超过阈值时发出一次告警；后续巡检观察到恢复或任务结束时记录恢复。短于阈值的停顿不生成区间。轮询会带来约一个巡检周期的时间误差，daemon 调度或投递拥塞还可能增加延迟。

生命周期信号仅更新内存状态。告警和恢复记录使用现有告警存储，Web 从这两类记录还原历史区间。daemon 负责装配通用模块及授权告警投递，OpenCode 专用处理位于 Agent 接入模块。

## 配置

在 daemon operator 配置中设置：

```toml
[idle_detection]
enabled = false
threshold_secs = "30s"
poll_interval_secs = "2s"
```

| 配置项 | 含义 |
| --- | --- |
| `enabled` | 开启状态消费、巡检和告警，默认关闭 |
| `threshold_secs` | 无等待但未推进的持续时间阈值，单位秒 |
| `poll_interval_secs` | 巡检周期，单位秒 |

时长使用带单位的字符串（例如 `"30s"`），必须为正值；非法配置启动时报错。修改配置后重启 daemon。实时 map 不从历史告警重建，重启后需要新的任务开始事件；持久化告警仍可在 Web 查询。

Agent 接入使用独立配置文件：

```toml
[opencode]
enabled = true
plugin_dir = "/usr/local/lib/actrail/opencode"
```

```bash
actrailctl --config /path/to/operator.conf launch \
  --agent-host-config /path/to/agent-host.toml \
  --name opencode-session -- opencode
```

部署与原生事件接入见 [Agent Host 接入](../../deploy/agent-host/README.md)。接入插件的启用与检测功能的启用分别配置。

## 告警与 Web

producer 为 `actrail.agent-idle`，定义键 `hang_detected` 表示检测到停滞，`hang_recovered` 表示后续巡检观察到恢复。两者通过 `episode_key` 配对，载荷包含 session、task、起点、观测时间和阈值；恢复记录包含终点。时间载荷使用 Unix 纳秒十进制字符串。

Waterfall 和 action-tree API 返回 `idle_intervals`：

```bash
curl -fsS "http://127.0.0.1:18092/api/traces/<TRACE_ID>/waterfall" \
  | jq '.idle_intervals'
```

| 字段 | 含义 |
| --- | --- |
| `id` | 起始告警生成的区间标识 |
| `kind` | `hang` |
| `session_id` / `task_id` | 所属执行单元 |
| `start_time_unix_nanos` | 本次连续停滞的起点 |
| `end_time_unix_nanos` | 恢复被巡检观察到的时间；未结束为 `null` |

页面标记为 **Agent stalled**。区间起点早于告警产生时间，二者相差阈值及巡检延迟。trace 终止时，Web 将开放区间截断到 trace 终态。Web 重启后仍从告警存储查询这些区间。

## 真实场景验收

`tests/process/idle-detection/e2e.py` 使用刷新后的默认配置及真实 OpenCode。测试通过独立测试插件暂停 Agent 推进，覆盖长停滞告警与恢复、短暂停顿、模型及工具等待、用户权限等待、工具异常、并发会话隔离及 Web 重启后的历史读取。测试接入资产必须复制到隔离目录，避免 OpenCode 安装依赖污染源码目录。
