# 用 checkpoint 保存和恢复文件

> 本文说明如何为 launch 的 Agent 启用 checkpoint，并用 Btrfs backend 保存和恢复受保护目录。

checkpoint 在 Agent 开始执行前保存一次受保护目录。满足下文 TLS-sync 要求后，Agent 每次发出 LLM 请求，daemon 都在 Agent 收到响应前再保存一次；也可以随时手动保存。Trace 运行期间，目录可以恢复到任意一个未失效的 checkpoint，恢复在下一次 LLM 请求时执行。Trace 结束后 daemon 释放它的快照，不能再恢复。恢复只回退文件，不回退 Agent 的对话和进程内存。设计见 [Checkpoint](../architecture/components/checkpoint.md)。

## 准备 Btrfs

Btrfs backend 用只读快照保存目录，恢复时把快照内容写回原目录，原目录的子卷保持不变。快照覆盖整个受保护目录，Btrfs scope 不支持排除路径。

连接期间 backend 持有受保护子卷的目录句柄，每次保存和恢复先确认原路径仍对应这个子卷。检测到根目录或其上级目录被移动、删除或替换时，该目录的保存和恢复会报错；后续文件操作也通过句柄定位，不会跟随被替换的根路径。

保存和恢复使用 `snapshot_root/locks/<子卷UUID>.lock` 串行协调，不对受保护目录本身加锁。操作同一子卷的 backend 实例须共用 `snapshot_root`。锁文件由 daemon 管理，disconnect 后保留，避免其他实例仍持有或等待旧锁时产生两把锁。

| 要求 | 说明 |
| --- | --- |
| 受保护目录 | 必须是 Btrfs 子卷的根，并位于 backend 配置的 `allowed_workspace_roots` 之下 |
| 排除路径 | 不支持。scope 不能写 `exclude`，也不能嵌套其他 scope |
| 嵌套子卷 | connect 时受保护目录内不能有嵌套子卷，因为快照不包含它们。Agent 运行中新建的子卷不受保护：保存不包含其内容，恢复也不改动它 |
| 嵌套挂载 | 恢复保留挂载目录中的内容，包括跨设备挂载、同设备 bind mount 和用户 FUSE 挂载；先按挂载表跳过挂载点，再读取其他条目的元数据 |
| 快照目录 | `snapshot_root` 与受保护目录在同一个 Btrfs 文件系统上，且与 `allowed_workspace_roots` 互不包含；其中 `snapshots` 目录在 backend 初始化时设为 0700，已有目录也会收紧权限 |
| 路径视图 | Agent 与 backend 位于同一个 mount namespace；容器中的 Agent 暂不支持 |

以下示例假设 `/data` 是已挂载的 Btrfs 文件系统，项目子卷放在 `/data/workspaces` 下，快照放在 `/data/.actrail`：

```bash
sudo mkdir -p /data/workspaces /data/.actrail
sudo btrfs subvolume create /data/workspaces/project
```

## 配置 daemon

backend 是 daemon 进程内加载的动态库，用 `cargo build --release -p actrail_ckpt_fs_btrfs` 构建，得到 `target/release/libactrail_ckpt_fs_btrfs.so`；AgentNS backend 的包名是 `actrail_ckpt_fs_agentns`。在 operator 配置中启用 checkpoint 并列出动态库，`name` 是 launch 配置引用的名字，`library` 和 `config` 都是绝对路径，`config` 是该 backend 自己的配置文件。`catalog_path` 是记录库文件，省略时为 `/var/lib/actrail/checkpoint.db`。

```toml
[checkpoint]
enabled = true
catalog_path = "/var/lib/actrail/checkpoint.db"

[checkpoint.runtime]
busy_timeout_ms = 5000
stop_timeout_ms = 5000
stop_poll_interval_ms = 10
shutdown_drain_timeout_ms = 10000
idle_poll_interval_ms = 10

[[checkpoint.backends]]
name = "project-btrfs"
library = "/usr/lib/actrail/checkpoint/libactrail_ckpt_fs_btrfs.so"
config = "/etc/actrail/checkpoint/btrfs.json"
```

列表决定一个 Trace 由哪些 backend 组成，保存和恢复时按列表顺序调用。文件系统类 backend（Btrfs、AgentNS）保护 launch 配置点名的目录，这次 launch 没有点名它时不参与；上下文类 backend 没有 scope，每个 Trace 都参与。类型由 backend 自己声明，不用配置。

`[checkpoint.runtime]` 各项可省略，上例为默认值。字段职责见 [daemon 配置参考](../reference/configuration/daemon.md#checkpointruntime)。Web 的状态查询超时和自动刷新间隔由 `[web.checkpoints]` 配置。

Btrfs backend 的配置为 JSON：

```json
{
  "btrfs_path": "/usr/bin/btrfs",
  "findmnt_path": "/usr/bin/findmnt",
  "copy_path": "/usr/bin/cp",
  "copy_batch_max_bytes": 65536,
  "snapshot_root": "/data/.actrail",
  "allowed_workspace_roots": ["/data/workspaces"]
}
```

`copy_batch_max_bytes` 是每次 `cp` 调用的源路径参数预算，默认 `65536` 字节，须大于零，按路径字节、结束符和指针大小估算。单个路径超过预算时仍单独复制。

LLM 请求处的保存和恢复依赖 TLS-sync runtime 在交付响应前等待 daemon，需要：

```toml
[payload.tls]
enabled = true
capture_backend = "tls-sync"
```

当前启用 checkpoint 不检查 TLS backend，也不会自动切换后端。`capture_backend` 默认是 `bpf-copy`，保持这个默认值仍能启动 daemon 和 checkpoint launch，但没有 checkpoint 的 LLM 边界：自动保存不执行，运行中 Trace 的 restore 会一直处于 `waiting_boundary`，直到用户执行 `force` 或 Trace 结束。动作视图识别到 LLM 请求也不能替代 TLS-sync 的响应等待。

初始保存、手动保存和 `force` 不依赖 LLM 边界。

LLM 请求由 daemon 的 LLM 投影识别。`semantic_retention.projection_enabled` 或 `semantic_retention.l0_llm_call.enabled` 关闭时，daemon 拒绝启用 checkpoint；两者默认开启。

静态链接 TLS 的可执行文件不注入 runtime，只有初始保存和手动保存，运行中的恢复要用 `force` 执行。

daemon 启动时打开记录库并加载全部动态库，加载失败时启动失败；修改配置后重启 daemon。每个 Trace 用各 backend 的一个新实例。backend 在 daemon 进程内运行，它崩溃会使 daemon 退出。

AgentNS backend 依赖 RNS（当前的 AgentNS）：`rns.ko` 由 `rnsctl start` 运行，配置的 `library_path` 指向同一安装的 `librns.so`，例如 `/usr/lib/rns/current/librns.so`。模块、BPF、SDK 和 backend 须配套更新，支持 `rns_capabilities`、进程树捕获和保留暂停进程恢复；缺少能力时 backend 初始化失败。

backend 创建会话时选择进程树模式：Agent 的线程和后代跟随当前保存窗口，Unix socket 连接不把 daemon 等外部服务纳入会话。恢复时由 coordinator 暂停全部受控进程，RNS 检查整个线程组均已暂停并排空在途 I/O，恢复文件后保留这些进程，成功后 coordinator 继续运行它们。

它只接受根为 `/` 的一个 scope；与 Btrfs scope 一起使用时，Btrfs 保护的目录自动加入它的排除路径。恢复没有完成时，RNS 保留这次请求并阻止该区域的其他恢复，下一次恢复使用相同请求和恢复策略先把它做完；disconnect 不再继续回滚，直接连同区域一起丢弃。

AgentNS 已受理的恢复在完成前中断时，区域保持冻结。此时保存会因 `EAGAIN` 失败，受管写入也可能返回 `EAGAIN`。checkpoint 的 `resume` 只继续进程并放行响应等待，不能解除 RNS 区域冻结；应再次恢复，把未完成的请求做完，或执行 `disconnect` 放弃该区域及其保护。

## 启动受保护的 Agent

launch 配置文件列出本次保护的目录，`backend_instance` 对应 `[[checkpoint.backends]]` 的 `name`：

```toml
[[scopes]]
id = "project"
root = "/data/workspaces/project"
backend_instance = "project-btrfs"
```

```bash
sudo actrailctl launch --name agent --checkpoint /path/to/checkpoint.toml -- python agent.py
```

`root` 必须是绝对路径，actrailctl 会把它解析为实际目录。Btrfs scope 不支持 `exclude`；其他 backend 的 `exclude` 可以写相对 `root` 的路径。初始保存完成后 Agent 才开始执行，完成前不接受手动保存。backend 无法落实某个 scope 时，launch 以 `connect_failed` 失败，Agent 不会运行。backend 以 root 身份保存和恢复，所以宿主机 root 以外的用户只能把自己拥有的目录作为 `root`，否则 launch 以 `scope_not_owned` 失败；AgentNS 的 `/` 属于 root，只有 root 能用。

## 保存、恢复与删除

| 命令 | 作用 |
| --- | --- |
| `actrailctl checkpoint save <trace>` | 立即保存运行中 Trace 的全部 scope |
| `actrailctl checkpoint list <trace> [--last <N> \| --since <秒> --until <秒>]` | 按创建顺序列出 checkpoint 及其是否可恢复，已删除的也列出；`--last` 只列最新的 N 个，`--since`、`--until` 按创建时间筛选，单位与输出中的 `created_unix_secs` 相同 |
| `actrailctl checkpoint restore <trace> <checkpoint>` | 把运行中的 Trace 恢复到指定 checkpoint，等到下一次 LLM 请求时执行 |
| `actrailctl checkpoint force <operation>` | 让等待 LLM 请求的恢复立即执行 |
| `actrailctl checkpoint delete <trace> <checkpoint>` | 删除 checkpoint 并释放其快照 |
| `actrailctl checkpoint disconnect <trace>` | 在 Trace 结束前放弃它的全部 checkpoint：删除剩下的 checkpoint，backend 释放为它保存的全部内容；之后不能再保存或恢复。释放失败时再次执行即重试 |
| `actrailctl checkpoint operations <trace>` | 列出保存和恢复操作 |
| `actrailctl checkpoint operation <operation>` | 查看一次操作的进展和结果 |
| `actrailctl checkpoint resume <trace>` | 继续恢复失败后保持暂停的进程 |

编号可以写 `7`，也可以写输出中的 `checkpoint-7`、`operation-7`、`trace-7`。checkpoint 编号在每个 Trace 内从 0 开始，`checkpoint-0` 是 launch 时的初始保存；编号删除后不再复用，所以恢复和删除都要同时指定 Trace；operation 编号在所有 Trace 间唯一。保存和恢复命令立即返回操作编号，用 `operation` 查看结果：

```text
$ sudo actrailctl checkpoint restore trace-1 checkpoint-0
checkpoint operation-3 accepted
$ sudo actrailctl checkpoint operation operation-3
operation-3 trace-1 restore trigger=manual phase=finished outcome=succeeded target=checkpoint-0
```

恢复先显示 `phase=waiting_boundary`。Agent 发出下一次 LLM 请求时，daemon 暂停 Agent 的进程并恢复文件，Agent 在恢复结束后才收到这次响应；不想等时用 `force` 立即执行。同一恢复 operation 可重复 `force`：已经触发或结束时直接接受，不重复恢复，结果仍以原 operation 为准。这次 LLM 请求也可能来自 Agent 运行的工具，此时恢复发生在工具执行中途，效果与 `force` 相同。等待期间 Trace 结束时直接执行。恢复成功后 daemon 继续它暂停的进程；恢复失败时进程和这次响应都保持暂停，确认文件状态后用 `resume` 继续，或再次恢复。

恢复或 disconnect 的结果写入记录库失败时，daemon 在内存中保留结果及失效、释放信息。排除磁盘空间或权限等写入问题后，再执行保存、恢复、删除、disconnect 或 `resume` 会先补记结果，不重复已经完成的 backend 操作。`resume` 即使补记仍失败，也会尝试继续已暂停的进程。daemon 在补记前退出时，下一次启动仍按中断操作收尾。

Trace 结束时 daemon 自动 disconnect：已排队的工作完成后，剩下的 checkpoint 标为已删除，backend 释放全部快照。之后 `save`、`restore` 返回 `trace_ended`，历史仍可查询。需要在 Trace 结束后保留文件状态时，自行复制受保护目录。

也可以在 Trace 运行中执行 `disconnect` 提前放弃：它在已排队的工作完成后执行，恢复失败后保持的暂停随之解除；之后不再保存，LLM 请求也不再等待。有未完成的恢复时不能 disconnect。

释放失败时，`checkpoint list` 显示 `release=failed` 和 `release_error`。排除原因后再次执行 `disconnect` 重试；运行中的 Trace 结束时也会自动重试一次。

## LLM 请求处的保存

daemon 按请求内容判断是否为 LLM 请求，规则与 ActionTree 识别 LLM 调用相同，装有 llm-codec 插件时也用它解码。Trace 中任一进程的 LLM 请求都算，包括 Agent 运行的工具和以独立进程运行的子 Agent。其他 HTTPS 请求立即放行，放行通常先于响应到达。每个 LLM 请求都保存一次，不判断此前有没有文件修改，保存的内容由 backend 决定。一轮对话中的每次 LLM 请求，包括子 Agent 和标题生成等辅助请求，都会产生一个 checkpoint；不需要的 checkpoint 可以删除以释放空间。

保存期间只有等待响应的 TLS 读取停住，后台工具继续运行；保存时间过长时 LLM 请求可能超时。保存失败也会放行响应，失败记录在 operation 中。

TLS-sync 在已判定协议的地址上观察到出站片段以完整 HTTP/2 preface 开头时（旧连接是 HTTP/2 时须在帧边界），先结束旧连接，再处理新连接；运行时同时清除该地址的流控状态，避免复用地址的新连接沿用旧 stream 编号和正文丢弃状态。尚未跟踪 TLS 对象释放，因此旧 HTTP/2 地址被新 HTTP/1 连接复用时仍不能重新识别。进程的上报连接断开时，daemon 关闭它上报过的全部 stream；存活进程中正常结束的 TLS 连接仍占用路由，直到地址被新 HTTP/2 preface 重置或 Trace 结束。地址持续不复用时仍可能达到路由方向数上限，导致新连接的 LLM 边界漏报；手动保存和 `force` 不依赖 LLM 请求识别。

## 在 Web 中查看

actrailweb 读取同一份 operator 配置。Trace 视图的执行分组中有 Checkpoint 标签，手动保存按钮为“创建 checkpoint”。界面中的 checkpoint 编号与 actrailctl 相同。从上到下展示恢复后的 checkpoint 路径，例如 `0 → 1 → 2 → 3` 恢复到 `1` 后，这一列默认只保留 `0 → 1`，并在1上标明“回滚至此”和恢复时间。点击标记展开这次恢复收起的 `2 → 3`，可逐个打开旧记录详情，也可单独查看恢复结果；再保存 `4`，主路径接成 `0 → 1 → 4`，1上的展开内容仍是 `2 → 3`。展开状态在自动刷新时保留。成功恢复不增加重复节点，等待中或失败的恢复不会截短路径，也不生成成功标记。同一目标重复恢复时，保留所有收起过记录的恢复标记，以及最新一次恢复标记；未收起记录的旧标记移除，完整操作记录仍保留。可从 ActionTree 的 `Cn` 圆胶囊打开旧 checkpoint 的详情。

运行中的 Trace、执行中的操作和待释放快照会自动刷新；快照释放失败时可以重试释放。在 LLM 请求处执行的操作可以跳到动作视图中这次请求的 LLM 调用，动作视图也在这些 LLM 调用上标出保存或恢复，点击回到对应记录。观测数据被留存策略清理后，历史仍可查看，只是没有对应的调用。

自动刷新读取失败后继续定时重试。Trace 已 disconnect 时，“创建 checkpoint”按钮禁用。

暂停提示读取 daemon 当前是否仍保持该 Trace 的进程暂停或响应等待。点击“解除暂停”后，命令受理只表示已排队，daemon 确认解除后提示才消失；解除失败时继续显示。在 CLI 中解除或刷新页面也以这个状态为准。无法连接 daemon 或查询超时时显示暂停状态未知，历史仍可查看。这个查询只反映当前 daemon 持有的暂停，异常退出遗留的进程暂停仍需手动处理。

## 记录的生命周期

backend 的快照只活到 Trace 结束或 disconnect，也不超过 daemon 这一次运行。checkpoint 历史保存在记录库中，不随 daemon 重启或观测数据的留存清理而消失，Trace 结束后、daemon 重启后都可以查询。

backend 实例只存在于 daemon 进程内。daemon 退出后，上一次运行中的 Trace 不能再保存、恢复或删除 checkpoint。daemon 正常停止时，运行中的 Trace 在已排队的工作完成后 disconnect，释放快照；等待预算由 `checkpoint.runtime.shutdown_drain_timeout_ms` 配置，默认 10 秒，来不及完成的工作随进程结束。daemon 启动时收尾：执行中的操作记为 `unknown`，排队和等待 LLM 请求的操作记为失败，这些 Trace 记为已 disconnect，它们的 checkpoint 标为已删除，释放状态为失败。异常退出时 backend 留下的快照没有 daemon 负责清理，Btrfs 的在 `snapshot_root/snapshots` 下，AgentNS 的在 `area_root` 下，需要手动删除。因恢复失败而保持暂停的进程也需要手动继续。

初始保存失败、Trace 结束或 daemon 正常停止时，会记录该 Trace 的结束时间，Web 随之显示已结束。启动收尾也会为旧记录补上缺失的结束时间；无法确定实际结束时刻时，使用本次收尾时间。运行中手动 `disconnect` 只结束 checkpoint 保护，不把仍在运行的 Trace 标为已结束。
