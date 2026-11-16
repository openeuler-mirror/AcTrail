# 接入快照与回滚

按本页操作，可以为一个 Agent 任务启用文件快照、上下文快照，或同时启用两者。保存后得到一个 checkpoint（恢复点），运行中的任务可以回滚到这个恢复点。

| 名称 | 含义 |
| --- | --- |
| Trace | 通过 `actrailctl launch` 启动的一个任务 |
| `actraild` | AcTrail 后台服务 |
| backend | 实际执行快照和回滚的后端 |

## 目录

1. [通用准备](#1-通用准备)
2. [文件快照](#2-文件快照)
   - [Btrfs](#btrfs)：保护指定子卷
   - [AgentNS](#agentns)：追踪任务进程树的文件修改
   - [不启用文件快照](#不启用文件快照)
3. [上下文快照](#3-上下文快照)
   - [Xiaoo](#xiaoo)：回滚对话上下文
   - [不启用上下文快照](#不启用上下文快照)
4. [启动后台服务](#4-启动后台服务)
5. [启动任务](#5-启动任务)
6. [保存、回滚和释放](#6-保存回滚和释放)
7. [新增并行任务](#7-新增并行任务)
8. [按现象排查](#8-按现象排查)

## 1. 通用准备

### 确认环境

准备以下环境：

- Linux 宿主机和 `sudo` 权限。
- 完整安装的 AcTrail。
- 已能以任务账户正常运行的 Agent。使用模型服务时，先完成一次真实对话。

尚未安装 AcTrail 时，在仓库根目录以普通用户执行下列命令。脚本会按需调用 `sudo`：

```bash
bash scripts/install-release.sh
```

确认程序和配套运行库存在：

```bash
command -v actraild actrailctl
ls "$(dirname "$(readlink -f "$(command -v actrailctl)")")/libactrail_tls_payload_probe_sync.so"
```

第二条命令应列出库文件。

本文的 `cargo build` 命令在同一版本的 AcTrail 仓库根目录执行，需要可用的 Rust 构建环境。构建产物使用默认的 `target/release` 目录。

### 配置文件

```text
/etc/actrail/checkpoint/
+-- setup.patch.toml        通用设置 + 选中的后端注册信息
+-- actraild.conf           第4节由工具生成，后台服务读取它
+-- btrfs.json              仅Btrfs：工具、存储位置、允许保护的范围
+-- agentns.json            仅AgentNS：运行库、存储位置、全局排除
+-- xiaoo-runtime.json      仅Xiaoo：超时、响应上限、恢复模式
+-- task-a.toml             任务A：选择后端、指定保护目录
+-- xiaoo-a.json            仅Xiaoo：任务A的socket地址

Xiaoo原有配置所在目录/
+-- actrail-task-a.toml     仅Xiaoo：保留模型配置，启用同一个socket
```

| 配置类别 | 管什么 | 何时生效 |
| --- | --- | --- |
| 后台服务配置和后端运行 JSON | 加载哪些后端，以及工具路径、运行库、存储范围等 | 启动后台服务时加载。修改后需要重启 |
| `task-a.toml` 和任务私有 JSON | 本次任务使用哪些后端、保护哪些目录、连接哪个上下文服务 | 每次 launch 时加载。新增任务不需要重启后台服务 |

后端名称是自定义的引用名。本页的 `files` 用于文件快照，`context` 用于上下文快照。任务配置中的名称必须与后台服务注册的名称一致。

### 创建公共配置

下面创建一套新配置和一个新任务 `task-a`。若这些路径已有正在使用的配置，请换一个配置目录，并同步替换后续命令中的路径。

```bash
sudo mkdir -p /etc/actrail/checkpoint /usr/local/lib/actrail/checkpoint
sudo tee /etc/actrail/checkpoint/setup.patch.toml >/dev/null <<'TOML'
[checkpoint]
enabled = true
catalog_path = "/var/lib/actrail/checkpoint.db"

[payload.tls]
enabled = true
capture_backend = "tls-sync"

[semantic_retention]
projection_enabled = true

[semantic_retention.l0_llm_call]
enabled = true
TOML

sudo tee /etc/actrail/checkpoint/task-a.toml >/dev/null <<'TOML'
# 本次任务使用的后端和保护范围。
TOML
```

`tls-sync` 用于在 Agent 请求模型时自动保存，并在模型请求处执行回滚。

后端配置追加到这两份文件中，同一后端只注册一次。

## 2. 文件快照

### Btrfs

**适用条件：** `/data` 已挂载为 Btrfs，任务目录是独立子卷。后台服务和任务运行在同一宿主机的挂载环境中。

检查文件系统和工具：

```bash
findmnt -n -o FSTYPE -T /data
command -v btrfs findmnt cp
```

第一条应输出 `btrfs`。否则先准备 Btrfs 挂载点，或使用其他文件后端。

安装后端：

```bash
cargo build --release -p actrail_ckpt_fs_btrfs
sudo install -m 0755 target/release/libactrail_ckpt_fs_btrfs.so /usr/local/lib/actrail/checkpoint/
```

创建任务子卷和快照目录。本例的 `task-a` 是新目录：

```bash
sudo mkdir -p /data/workspaces /data/.actrail
sudo btrfs subvolume create /data/workspaces/task-a
sudo chown "$(id -u):$(id -g)" /data/workspaces/task-a
```

写入运行配置。快照目录与任务子卷必须位于同一 Btrfs 文件系统。`snapshot_root` 与 `allowed_workspace_roots` 中的目录不能互相包含：

```bash
sudo tee /etc/actrail/checkpoint/btrfs.json >/dev/null <<JSON
{
  "btrfs_path": "$(command -v btrfs)",
  "findmnt_path": "$(command -v findmnt)",
  "copy_path": "$(command -v cp)",
  "snapshot_root": "/data/.actrail",
  "allowed_workspace_roots": ["/data/workspaces"]
}
JSON
```

注册后端，并为任务指定保护目录：

```bash
sudo tee -a /etc/actrail/checkpoint/setup.patch.toml >/dev/null <<'TOML'

[[checkpoint.backends]]
name = "files"
library = "/usr/local/lib/actrail/checkpoint/libactrail_ckpt_fs_btrfs.so"
config = "/etc/actrail/checkpoint/btrfs.json"
TOML

sudo tee -a /etc/actrail/checkpoint/task-a.toml >/dev/null <<'TOML'

[[backends]]
name = "files"

[[scopes]]
id = "project"
root = "/data/workspaces/task-a"
backend_instance = "files"
TOML
```

| 项目 | 保护范围与要求 |
| --- | --- |
| `/data/workspaces/task-a` | 保护整个子卷 |
| 排除和嵌套范围 | 不支持 `exclude` 或嵌套 scope |
| 嵌套子卷 | 连接时目录内不能存在。运行中新建的嵌套子卷不受保护 |
| 嵌套挂载 | 挂载中的内容不受回滚保护 |

### AgentNS

| 环境项 | 要求 |
| --- | --- |
| RNS 组件 | 已安装相互配套的内核模块、`rnsctl` 和 `librns.so` |
| RNS 能力 | 支持进程树捕获和保留暂停进程的恢复 |
| 任务账户 | 宿主机 root。AgentNS 的 scope 根为 `/` |

启动 RNS，并检查运行库。本例安装位置是 `/usr/lib/rns/current/librns.so`，实际位置不同时修改下面的变量：

```bash
sudo rnsctl start
RNS_LIBRARY=/usr/lib/rns/current/librns.so
test -r "$RNS_LIBRARY"
```

安装后端并准备任务目录：

```bash
cargo build --release -p actrail_ckpt_fs_agentns
sudo install -m 0755 target/release/libactrail_ckpt_fs_agentns.so /usr/local/lib/actrail/checkpoint/
sudo mkdir -p /data/workspaces/task-a
```

写入运行配置。`area_root` 是后端保存回滚记录的位置：

```bash
sudo tee /etc/actrail/checkpoint/agentns.json >/dev/null <<JSON
{
  "library_path": "$RNS_LIBRARY",
  "area_root": "/var/lib/actrail/agentns",
  "extra_excluded_paths": []
}
JSON

sudo tee -a /etc/actrail/checkpoint/setup.patch.toml >/dev/null <<'TOML'

[[checkpoint.backends]]
name = "files"
library = "/usr/local/lib/actrail/checkpoint/libactrail_ckpt_fs_agentns.so"
config = "/etc/actrail/checkpoint/agentns.json"
TOML

sudo tee -a /etc/actrail/checkpoint/task-a.toml >/dev/null <<'TOML'

[[backends]]
name = "files"

[[scopes]]
id = "agent-files"
root = "/"
exclude = ["/tmp"]
backend_instance = "files"
TOML
```

```text
本任务进程及其后代的文件修改 -> 记录，排除 /tmp
其他进程的文件修改           -> 不计入本任务
```

第5节启动任务时，在 `actrailctl` 前加 `sudo`。

### 不启用文件快照

仅使用上下文快照时，准备一个普通工作目录：

```bash
sudo mkdir -p /data/workspaces/task-a
sudo chown "$(id -u):$(id -g)" /data/workspaces/task-a
```

## 3. 上下文快照

### Xiaoo

**适用条件：** Xiaoo 已支持 checkpoint socket，并且已用准备采用的配置和运行账户完成过真实对话。若选择 AgentNS，先确认 Xiaoo 在 root 身份下也能使用这份配置运行。

安装后端，写入公共运行配置并注册：

```bash
cargo build --release -p actrail_ckpt_cxt_xiaoo
sudo install -m 0755 target/release/libactrail_ckpt_cxt_xiaoo.so /usr/local/lib/actrail/checkpoint/

sudo tee /etc/actrail/checkpoint/xiaoo-runtime.json >/dev/null <<'JSON'
{
  "request_timeout_ms": 5000,
  "max_response_bytes": 65536,
  "restore_mode": "promise_mode"
}
JSON

sudo tee -a /etc/actrail/checkpoint/setup.patch.toml >/dev/null <<'TOML'

[[checkpoint.backends]]
name = "context"
library = "/usr/local/lib/actrail/checkpoint/libactrail_ckpt_cxt_xiaoo.so"
config = "/etc/actrail/checkpoint/xiaoo-runtime.json"
TOML
```

为任务指定 socket。这个地址只属于 `task-a`：

```bash
sudo tee /etc/actrail/checkpoint/xiaoo-a.json >/dev/null <<'JSON'
{"endpoint": "/tmp/actrail-task-a.sock"}
JSON

sudo tee -a /etc/actrail/checkpoint/task-a.toml >/dev/null <<'TOML'

[[backends]]
name = "context"
config = "/etc/actrail/checkpoint/xiaoo-a.json"
TOML
```

在原有 Xiaoo 配置的同一目录创建副本，以保留模型设置和凭据位置。若原来通过 `--config` 使用其他文件，先把 `XIAOO_CONFIG` 设置为那个文件的绝对路径：

```bash
xiaoo_source="$(realpath "${XIAOO_CONFIG:-$HOME/.config/xiaoo/config.toml}")"
xiaoo_task_a="$(dirname "$xiaoo_source")/actrail-task-a.toml"
cp -- "$xiaoo_source" "$xiaoo_task_a"
printf '%s\n' "$xiaoo_task_a"
```

用文本编辑器打开输出的文件，添加或修改以下两节。已有同名节时修改原字段，避免重复添加：

```toml
[checkpoint]
enabled = true

[checkpoint.unix_socket]
endpoint = "/tmp/actrail-task-a.sock"
```

两处 `endpoint` 必须完全相同：

```text
任务A的 AcTrail 配置 -> xiaoo-a.json -> /tmp/actrail-task-a.sock
任务A的 Xiaoo 配置 -----------------> /tmp/actrail-task-a.sock
```

第5节通用启动命令的 `--` 后填写：

```bash
"$(command -v xiaoo)" --config "$xiaoo_task_a"
```

在准备配置的同一终端继续操作。换了终端时，用刚输出的完整路径替换 `$xiaoo_task_a`。

**启动检查：** 应生成 `/tmp/actrail-task-a.sock`。

### 不启用上下文快照

文件快照可以独立使用，任务配置只保留文件后端。

| 用途 | 第5节 `--` 后的命令 |
| --- | --- |
| 运行 Agent | 平时的启动命令及参数 |
| 验证文件快照 | `bash`，在这个受保护的 shell 内修改文件 |

## 4. 启动后台服务

一台主机使用同一个后台服务承载这些任务。切换已有服务前，先完成以下操作：

```text
结束受保护任务 -> 按原配置停止旧服务 -> 启动新配置
                 停止时释放已有快照
```

生成配置并启动：

```text
默认配置 + setup.patch.toml --init--> actraild.conf --start--> 后台服务
```

```bash
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf init \
  --patch /etc/actrail/checkpoint/setup.patch.toml

sudo actraild --config /etc/actrail/checkpoint/actraild.conf start
sudo actraild --config /etc/actrail/checkpoint/actraild.conf status
```

服务默认以 root 创建两个通信 socket。为当前任务账户的主组设置访问权限：

```bash
sudo chgrp "$(id -gn)" /run/actrail/control.sock /run/actrail/tls-sync.sock
```

daemon 重启会重新创建 socket，需要重新执行 `chgrp`。以上路径对应本页生成配置的默认值。

检查服务：

```bash
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf doctor
```

**成功标志：** `status` 显示服务正在运行，`doctor` 能连接服务。

| 生成目标 | `init` 的行为 |
| --- | --- |
| 不存在 | 创建完整配置 |
| 已存在 | 拒绝覆盖 |
| 已备份并使用 `--force` | 按默认配置和 patch 重建整个文件，原文件中未写入 patch 的修改不会保留 |

## 5. 启动任务

进入任务工作目录。工作目录决定 Agent 从哪里运行，`task-a.toml` 中的 `scopes` 决定保护范围：

```bash
cd /data/workspaces/task-a
```

将最后的占位部分替换为第3节选定的 Agent 命令及参数：

```text
actrailctl --config /etc/actrail/checkpoint/actraild.conf launch \
  --name task-a \
  --checkpoint /etc/actrail/checkpoint/task-a.toml \
  -- <Agent启动命令及参数>
```

使用 AgentNS 时，在 `actrailctl` 前加 `sudo`，由宿主机 root 启动任务。

另开一个终端查询任务：

```bash
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf list-traces --name task-a
```

输出首列如 `trace-7` 就是任务编号。根据名称、PID 和运行状态找到本次任务，在查询终端记录实际编号：

```bash
TRACE_ID=trace-7
```

**成功标志：** Agent 正常运行，能够查询到对应 Trace。

保存带上下文的恢复点前，先让 Agent 完成一轮模型对话。

## 6. 保存、回滚和释放

下面命令在查询终端执行，`TRACE_ID` 使用上一步查到的实际值。

快照只在任务运行期间有效，验收期间保持 Agent 运行。

验收目标如下，只检查已启用的快照能力：

| 阶段 | 文件内容 | Agent 记住的口令 |
| --- | --- | --- |
| 保存时 | `ALPHA` | `ALPHA` |
| 修改后 | `BETA` | `BETA` |
| 回滚后 | `ALPHA` | `ALPHA` |

### 保存恢复点

文件快照：让被 launch 启动的 Agent 创建 `/data/workspaces/task-a/checkpoint-check.txt`，写入 `ALPHA`。若启动的是 `bash`，在那个 shell 中执行：

```bash
printf 'ALPHA\n' > /data/workspaces/task-a/checkpoint-check.txt
```

上下文快照：告诉 Agent“记住口令 ALPHA”，等待回复完成。

```bash
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf checkpoint save "$TRACE_ID"
```

命令返回如 `checkpoint operation-12 accepted`。这表示已受理，继续用返回的实际编号查询：

```bash
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf checkpoint operation operation-12
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf checkpoint list "$TRACE_ID"
```

**成功标志：** 操作显示 `phase=finished outcome=succeeded`，并显示 `created=checkpoint-N`。

记录返回的恢复点编号，例如：

```bash
CHECKPOINT_ID=checkpoint-3
```

### 回滚并检查结果

让同一个 Agent 完成上表的“修改后”状态，等待操作结束。

```bash
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf checkpoint restore "$TRACE_ID" "$CHECKPOINT_ID"
```

记录返回的 operation 编号。任务正常运行时，恢复过程如下：

```text
restore -> waiting_boundary
                 |
                 v
        新的模型请求或 force
                 |
                 v
              执行恢复
                 |
                 v
         查询 operation 结果
```

在 Agent 中发送一条新消息，然后在查询终端查看恢复操作。失败时按[第8节](#8-按现象排查)处理。

仅使用 shell 等不会请求模型的程序时，用恢复命令返回的实际编号立即执行：

```bash
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf checkpoint force operation-15
```

**成功标志：** 恢复操作显示 `phase=finished outcome=succeeded`。

文件快照：检查文件回到 `ALPHA`：

```bash
cat /data/workspaces/task-a/checkpoint-check.txt
```

上下文快照：等本轮回复完成后，再问 Agent“现在口令是什么”，确认它回答 `ALPHA`。

### 删除和结束保护

删除单个恢复点：

```bash
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf checkpoint delete "$TRACE_ID" "$CHECKPOINT_ID"
```

让 Agent 继续运行，但释放该任务全部快照并结束快照保护：

```bash
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf checkpoint disconnect "$TRACE_ID"
sudo actrailctl --config /etc/actrail/checkpoint/actraild.conf checkpoint list "$TRACE_ID"
```

```text
disconnect -> 查询 list -> release=released -> 退出 Agent
```

任务退出也会自动触发释放。Xiaoo 的 socket 会随进程退出而关闭，提前 disconnect 可以确认释放结果。

所有任务结束后，需要停服务时执行：

```bash
sudo actraild --config /etc/actrail/checkpoint/actraild.conf stop
```

## 7. 新增并行任务

后台服务保持运行。复制任务配置，然后按所选后端修改对应部分：

```bash
sudo cp /etc/actrail/checkpoint/task-a.toml /etc/actrail/checkpoint/task-b.toml
```

运行配置由两个任务共用。任务配置按下表准备：

| 配置项 | 任务A | 任务B |
| --- | --- | --- |
| 启动参数 `--name` | `task-a` | `task-b` |
| 启动参数 `--checkpoint` | `/etc/actrail/checkpoint/task-a.toml` | `/etc/actrail/checkpoint/task-b.toml` |
| 工作目录 | `/data/workspaces/task-a` | 新建 `/data/workspaces/task-b` |
| Btrfs：`scopes[].root` | `/data/workspaces/task-a` | `/data/workspaces/task-b`，创建独立子卷并设置目录所有者 |
| AgentNS：`scopes[].root` | `/` | `/`，按任务调整 `exclude` |
| Xiaoo：`backends[].config` | `/etc/actrail/checkpoint/xiaoo-a.json` | 复制为 `/etc/actrail/checkpoint/xiaoo-b.json` |
| Xiaoo：启动配置 | `actrail-task-a.toml` | 在原目录复制为 `actrail-task-b.toml` |
| Xiaoo：两处 `endpoint` | `/tmp/actrail-task-a.sock` | 都改为 `/tmp/actrail-task-b.sock` |

Btrfs 的新子卷须在运行配置允许的范围内。

两个任务如果共用同一子卷，文件回滚会影响共享目录。

按第5节在任务B目录中启动，使用表中的任务B配置。

查询并记录两个任务各自的 Trace 编号。保存、回滚和释放始终指定对应编号。

**成功标志：** 同一个服务下能查到两个 Trace。回滚任务A后，任务B的独立文件和上下文保持原状态。

| 修改内容 | 是否需要重启后台服务 |
| --- | --- |
| 新任务名称、保护目录、任务私有配置 | 不需要，下次 launch 生效 |
| 已注册后端的工具路径、运行库、存储范围或请求参数 | 需要，先结束正在保护的任务 |
| 新增一种后端动态库及其注册配置 | 需要，先结束正在保护的任务 |

## 8. 按现象排查

| 现象 | 处理方法 |
| --- | --- |
| 服务启动失败 | 检查注册的 `.so` 和运行 JSON 路径是否存在。修改 `setup.patch.toml` 后要重新生成服务配置，服务读取的是 `actraild.conf` |
| launch 提示后端未加载或未选择 | 对照服务的 `checkpoint.backends[].name`、任务的 `backends[].name` 和 scope 的 `backend_instance` |
| Btrfs 连接失败 | 检查保护目录是子卷根、位于允许范围内、与快照目录同一文件系统。目录内不要有嵌套子卷 |
| `scope_not_owned` | 普通用户只能保护自己拥有的目录。核对目录所有者。AgentNS 使用宿主机 root 启动 |
| Xiaoo 连接失败 | 确认 Xiaoo 已运行，两处 endpoint 相同，socket 已生成 |
| 首次上下文保存失败 | 先完成一轮真实模型对话再重试。上下文尚未就绪时无法保存 |
| 恢复一直 `waiting_boundary` | 让 Agent 发出新模型请求。检查服务使用 `tls-sync`。静态程序或 shell 可用 `force` |
| 恢复失败后 Agent 停住 | 查看 `checkpoint operation` 的错误。修正后重试恢复，或用 `checkpoint resume "$TRACE_ID"` 继续进程 |
| AgentNS 恢复中断后写入返回 `EAGAIN` | 再次恢复以完成操作，或 disconnect 放弃保护。`resume` 只能继续进程，不能解除 RNS 区域冻结 |
| `release=failed` | 查看 `release_error`，处理原因后再次 disconnect。Xiaoo 已退出时，其 socket 消失会导致上下文释放请求失败 |
| 任务退出或服务重启后无法恢复 | 快照只属于本次运行，历史记录可查询。服务异常退出时，Btrfs 的 `snapshot_root/snapshots` 和 AgentNS 的 `area_root` 可能留有需要人工清理的数据 |

表中的 checkpoint 命令均需加上同一份 `--config /etc/actrail/checkpoint/actraild.conf`。

在 Web 的 Trace 视图中打开 Checkpoint 标签，可以查看恢复点、操作结果和暂停状态。

需要查询字段时看[配置表](../architecture/config-layout.md)。开发新后端看[Backend 接口](../reference/checkpoint-backend.md)，了解内部实现看[架构与时序](../architecture/components/checkpoint.md)。
