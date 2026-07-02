# 手动测试手册：host actraild + 受限容器 actrailctl

本文是 [README.md](README.md) 中降级链路的逐命令、可复制粘贴的手动测试手册。假设宿主机为类 Ubuntu（glibc 2.39）且已安装 Docker，工作负载容器使用 openEuler 24.03 镜像（glibc 2.38）。每一步都列出命令与预期输出，便于边执行边核对。

> English version: [manual-test-walkthrough.md](manual-test-walkthrough.md)

测试目标：在工作负载容器**不**带 `--security-opt seccomp=unconfined`、**无** `CAP_BPF` 的前提下，验证 AcTrail 仍能 (1) 以 `[ebpf] enabled = "auto"` 启动 host daemon；(2) 在容器内报告 `seccomp_notify=unavailable` 并推荐 `auto`；(3) 将 `actrailctl launch` 降级为 tls-sync-only，并在 host 侧采集到 TLS 明文 + `llm.*` 语义动作。

## 每条命令在哪里执行

下文每个代码块都标注了执行位置：

- **`[host]`** — 在宿主机普通 shell 中执行（运行 Docker 和 `actraild` 的那台机器）。这些命令使用宿主机路径（`target/release/...`、`/etc/actrail`、`/var/lib/actrail`、`/run/actrail`）和宿主机工具（`cargo`、`docker`、`sqlite3`、`curl`）。
- **`[host → container]`** — 形如 `docker exec ... bash -lc '...'` 的命令：在宿主机输入，但脚本体在容器内执行。
- 纯 **`[container]`** — 仅当你用 `docker exec -it actrail-restricted bash` 进容器后手动执行命令时才算。

变量（`$TRACE_ID`、`$TRACE_NUM` 等）在较早步骤中设置、后续复用；请在同一个 shell 中设置并执行依赖它的命令，或重新 `export`。

## 两台机器、一个容器 —— 各自跑什么

```text
┌──────────────────────────── HOST（宿主机）────────────────────┐
│  actraild, actrailviewer, actrailweb （release 二进制）        │
│  /etc/actrail/actraild.conf   /var/lib/actrail/actrail.sqlite │
│  /run/actrail/{control.sock, tls-sync.sock}  ◄── 挂载进容器   │
│  Docker daemon                                                │
│                                                               │
│  你执行：cargo build、actraild start/stop/status、           │
│          actrailctl doctor/list-traces、actrailviewer、       │
│          actrailweb、docker run/exec、sqlite3、curl           │
└───────────────────────────────────────────────────────────────┘
                          ▲ 挂载 /run/actrail (RW)、/etc/actrail (RO) ▲
┌──────────────────────── CONTAINER（actrail-restricted）──────┐
│  openEuler 24.03，glibc 2.38，默认 seccomp，无 CAP_BPF        │
│  actrailctl + libactrail_tls_payload_probe_sync.so（Step 3   │
│    在容器内编译）                                              │
│  opencode agent + 配置（Step 4 安装）                         │
│  /AcTrail —— 宿主机源码树只读挂载（仅用于编译）              │
│                                                               │
│  你执行（经 docker exec）：actrailctl probe/launch、         │
│          dnf install、rustup、cargo build、opencode            │
└───────────────────────────────────────────────────────────────┘
```

## 容器部署与启动（总览）

容器用 `docker run -d` 创建一次，靠 `tail -f /dev/null` 常驻，之后 Step 3-6 都用 `docker exec` 进去操作。关键的 `docker run` 选项及其原因：

| `docker run` 选项 | 值 | 为什么 |
| --- | --- | --- |
| `--name` | `actrail-restricted` | 固定名字，后续所有 `docker exec` 都指向它 |
| `--user` | `0:0` | 容器内以 root 运行；launch 需要 fork 子进程并设置 `LD_PRELOAD` |
| `-v /run/actrail:/run/actrail` | RW 绑定挂载 | 容器内的 `actrailctl` 通过此挂载连接 host daemon 的 control + tls-sync socket。**没它就采不到数据** |
| `-v /etc/actrail:/etc/actrail:ro` | RO 绑定挂载 | 容器读取与 host daemon 相同的 operator 配置，使 socket 路径与 TLS 设置一致 |
| `-v "$(pwd)":/AcTrail:ro` | RO 绑定挂载 | 仅 Step 3（在容器内编译 `actrailctl` + `.so`）需要。源码树只读挂载，cargo 通过 `CARGO_TARGET_DIR` 写到 `/tmp`。Step 3 之后若想要更精简的运行时容器，可去掉此挂载 |
| `openeuler/openeuler:24.03-lts-sp3` | 镜像 | glibc 2.38，与 agent 匹配；刻意与宿主机 glibc(2.39) 不同以演示容器内编译 |
| `tail -f /dev/null` | 入口 | 保持容器常驻，使 `docker exec` 跨步骤可用；真正的 agent 后续由 `actrailctl launch` 启动 |

**刻意不添加的选项**（这正是"受限"测试的意义）：

- **不加 `--security-opt seccomp=unconfined`** → Docker 应用默认 seccomp profile，拦截 `pidfd_getfd`。正是这一约束迫使 launch 时的 seccomp 路径降级为 tls-sync-only。
- **不加 `--cap-add`（无 `CAP_BPF`/`CAP_PERFMON`）**、**不挂 `/sys/kernel`** → 容器自身无法做 eBPF。（eBPF 采集本就由 host daemon 完成，所以这只在你指望容器帮忙时才有影响。）

**启动**容器：

```bash
# [host]
docker run -d --name actrail-restricted \
  --user 0:0 \
  -v /run/actrail:/run/actrail \
  -v /etc/actrail:/etc/actrail:ro \
  -v "$(pwd)":/AcTrail:ro \
  openeuler/openeuler:24.03-lts-sp3 \
  tail -f /dev/null
```

**进入**容器交互式（可选，步骤间 poking）：

```bash
# [host → container]
docker exec -it actrail-restricted bash
```

进入后即处于容器内；`exit` 返回宿主机。下文脚本化步骤使用非交互的 `docker exec ... bash -lc '...'`，每条都是可整段粘贴的命令。

**停止/删除**容器（Step 9）：

```bash
# [host]
docker rm -f actrail-restricted
```

> **顺序很重要**：先创建 host socket（Step 1 启动 `actraild`，会创建 `/run/actrail/*.sock`），**再**创建容器，这样绑定挂载才能看到 socket。若先建容器、后起 daemon，即便挂载存在，容器内也看不到 socket 文件，`actrailctl probe` 会报 `control_socket=unavailable`。解决办法是先启动 `actraild`。

---

## Step 0 —— 宿主机预检

```bash
# [host]
# 0.1 release 产物（确保是当前源码最新编译的）
ls -la target/release/actraild target/release/actrailctl \
         target/release/actrailviewer target/release/actrailweb \
         target/release/libactrail_tls_payload_probe_sync.so

# 0.2 内核能跑 eBPF（本机将在 auto 下保持 eBPF 启用）
uname -a                       # 预期 aarch64/x86_64，内核 >= 5.10
test -f /sys/kernel/btf/vmlinux && echo "BTF ok"
test -d /sys/kernel/tracing && echo "tracefs ok"

# 0.3 Docker 可用
docker --version
docker ps --filter name=actrail-opencode-openeuler --format '{{.Names}} {{.Status}}'
# ^ 这个 unconfined 容器是后续 actrailctl/.so/opencode 二进制的来源。
#   若你没有它，请改用其他方式在 Step 2 中准备这些二进制。

# 0.4 你是 root（eBPF + socket 目录需要）
id                             # 预期 uid=0(root)

# 0.5 重新编译（仅当 0.1 的时间戳早于你最近一次 `git pull` 时才需要）
cargo build --release -p daemon -p ctl -p view -p web -p tls_payload_probe_sync
```

预期：所有二进制存在；BTF + tracefs 存在；Docker 可响应；`actrail-opencode-openeuler` 处于 Up；`id` 显示 root。

若宿主机无法运行 eBPF（无 BTF / 非 root / 无 tracefs），daemon 会在 Step 1 自动降级，`doctor` 的 collectors 里不会出现 `ebpf` —— 其余测试仍可进行，因为 TLS 明文来自 tls-sync 而非 eBPF。

---

## Step 1 —— 安装宿主机配置并启动 actraild

```bash
# [host]
# 1.1 创建运行时目录
install -d /etc/actrail /var/lib/actrail /run/actrail /var/log/actrail

# 1.2 安装受限示例配置（hierarchical TOML：ebpf enabled="auto"、process_seccomp enabled=false、tls-sync 开启）
install -m 0644 docs/examples/container-agent-restricted/operator.conf /etc/actrail/actraild.conf

# 1.3 复核关键字段（hierarchical TOML —— key 在 [section] 下）
grep -nE '^\[control\]|^\[ebpf\]|^\[process_seccomp\]|^\[payload.tls\]|^\[storage.sqlite\]|^socket_path|^enabled|^binary_path|^capture_backend|^sync_event_socket_path|^path' /etc/actrail/actraild.conf
# 预期 [ebpf] 下：          enabled = "auto"
# 预期 [process_seccomp] 下：enabled = false
# 预期 [payload.tls] 下：   enabled = true、capture_backend = "tls-sync"、binary_path = "disabled"、
#                          sync_event_socket_path = "/run/actrail/tls-sync.sock"
# 预期 [control] 下：       socket_path = "/run/actrail/control.sock"
# 预期 [storage.sqlite] 下：path = "/var/lib/actrail/actrail.sqlite"

# 1.4 启动 daemon
./target/release/actraild --config /etc/actrail/actraild.conf start
```

预期：
```
actraild started pid=<PID> socket=/run/actrail/control.sock
```

```bash
# [host]
# 1.5 状态 + socket
./target/release/actraild --config /etc/actrail/actraild.conf status
ls -la /run/actrail/      # 应包含 control.sock 和 tls-sync.sock（均为 srw-rw----）
```

```bash
# [host]
# 1.6 doctor —— 确认 collectors + storage
./target/release/actrailctl --config /etc/actrail/actraild.conf doctor
```

预期（宿主机能跑 eBPF）：
```
collectors=ebpf,tls-sync,application-protocol-analyzer plugins= storage_ready=true
```
若宿主机无法跑 eBPF，daemon 日志（`/var/log/actrail/actraild.log`）会含 `actraild ebpf auto-degraded: <reason>; continuing without host eBPF collection`，`doctor` 显示 `collectors=tls-sync,application-protocol-analyzer`（无 `ebpf`）。daemon **没有**拒绝启动 —— 这就是 auto 降级行为。

<details>
<summary><b>替代方案：根据探测结果生成 host 配置（而非用示例文件）</b></summary>

若你想根据本机实际探测结果生成一份配置，而非直接安装示例 `operator.conf`，可在 host 上运行 `probe --suggest-config`。它在配置文件尚不存在时也能工作（首次部署）：

```bash
# [host]
# 1.7（替代 1.2）根据 host 探测结果生成裁剪后的配置。
#     输出到 stdout；重定向到文件再安装。先看头部摘要——
#     它反映了本机 seccomp/eBPF/tls-sync 的可用性。
./target/release/actrailctl probe --suggest-config > /tmp/suggested.conf
head -12 /tmp/suggested.conf          # 检查探测摘要 + 关键字段
install -m 0644 /tmp/suggested.conf /etc/actrail/actraild.conf
```

在**受限容器内**运行（Step 5）时，同一个 flag 反映容器的探测结果：`seccomp_notify=unavailable` 会使生成的配置设 `[process_seccomp] enabled = false` 并从 `[capture] capabilities` 中去掉 `proc-exec-context`，从而 host daemon 无需 seccomp 即可启动。`--suggest-config` 本身从不写文件 —— 由你重定向落盘。

</details>

---

## Step 2 —— 创建受限工作负载容器

创建一个**不带** `--security-opt seccomp=unconfined`（Docker 默认 seccomp）、**不**覆盖 `/sys/kernel`、只挂载 `/run/actrail`（RW）与 `/etc/actrail`（RO）的容器。同时把 AcTrail 源码树只读挂载进去，以便容器内用自己的 glibc 编译 `actrailctl` + `.so`。

```bash
# [host]
# 2.1 没有镜像就拉取
docker image inspect openeuler/openeuler:24.03-lts-sp3 >/dev/null 2>&1 \
  || docker pull openeuler/openeuler:24.03-lts-sp3

# 2.2 创建受限容器
docker run -d --name actrail-restricted \
  --user 0:0 \
  -v /run/actrail:/run/actrail \
  -v /etc/actrail:/etc/actrail:ro \
  -v "$(pwd)":/AcTrail:ro \
  openeuler/openeuler:24.03-lts-sp3 \
  tail -f /dev/null

docker ps --filter name=actrail-restricted --format '{{.Names}} {{.Status}}'
# -> actrail-restricted Up ...
```

验证容器确实处于受限状态：

```bash
# [host]
# 2.3 Docker 默认 seccomp（非 unconfined）—— inspect 是宿主机侧的 Docker 查询
docker inspect actrail-restricted --format '{{json .HostConfig.SecurityOpt}}'
# -> null          （含义：默认 profile，非 seccomp=unconfined）

# [host → container]
#     grep 在容器内执行；docker exec 是宿主机侧的启动器。
docker exec actrail-restricted grep -i seccomp /proc/self/status
# -> Seccomp: 2     （filter 模式，已生效）
# -> Seccomp_filters: 1

# [host → container]
# 2.4 从宿主机挂载的 socket —— 这些 test 命令在容器内执行
docker exec actrail-restricted test -S /run/actrail/control.sock && echo "control.sock ok"
docker exec actrail-restricted test -S /run/actrail/tls-sync.sock && echo "tls-sync.sock ok"
docker exec actrail-restricted test -r /etc/actrail/actraild.conf && echo "conf readable"
```

预期：`SecurityOpt` 为 `null`；`Seccomp: 2`；两个 socket 均 `ok`；conf `readable`。

---

## Step 3 —— 在容器内编译 actrailctl + TLS-sync .so

宿主机编译的 `actrailctl` 依赖宿主机 glibc（如 2.39），在 openEuler 容器（glibc 2.38）中会以 `GLIBC_2.39 not found` 失败。改为在容器内编译。只需 Rust + gcc/openssl/zlib devel（无需 clang/llvm —— eBPF C 编译在 daemon 侧，宿主机已编译好）。

```bash
# [host → container]
# 3.1 安装编译依赖（一次性，约 1-2 分钟）
docker exec actrail-restricted bash -lc '
  dnf install -y --setopt=install_weak_deps=False \
    gcc make pkgconf-pkg-config openssl-devel zlib-devel perl
  gcc --version | head -1
'
```

预期：末行类似 `gcc (GCC) 12.3.1 (...)`（版本可能不同）。

```bash
# [host → container]
# 3.2 通过 rustup 安装 Rust（用国内镜像加速，一次性，约 1-2 分钟）
docker exec actrail-restricted bash -lc '
  export RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup
  export RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup
  curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh
  sh /tmp/rustup.sh -y --default-toolchain stable --profile minimal
  rm -f /tmp/rustup.sh
  /root/.cargo/bin/rustc --version
'
```

预期：打印 rustc 版本（如 `rustc 1.96.0 ...`）。

```bash
# [host → container]
# 3.3 配置 cargo 用镜像，然后编译 ctl + tls_payload_probe_sync
#     CARGO_TARGET_DIR 放在 /tmp 下，因为 /AcTrail 是只读挂载。
docker exec actrail-restricted bash -lc '
  set -e
  export RUSTUP_HOME=/root/.rustup CARGO_HOME=/root/.cargo
  export PATH=/root/.cargo/bin:$PATH
  export CARGO_TARGET_DIR=/tmp/actrail-target
  mkdir -p /root/.cargo
  cat > /root/.cargo/config.toml <<EOF
[source.crates-io]
replace-with = "tuna"
[source.tuna]
registry = "sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/"
[net]
git-fetch-with-cli = true
EOF
  cd /AcTrail
  cargo build --release -p ctl -p tls_payload_probe_sync
  install -m 0755 /tmp/actrail-target/release/actrailctl /usr/local/bin/actrailctl
  install -m 0755 /tmp/actrail-target/release/libactrail_tls_payload_probe_sync.so /usr/local/bin/libactrail_tls_payload_probe_sync.so
  rm -rf /tmp/actrail-target
'
```

预期：`Finished \`release\` profile [optimized] target(s) in ...`。首次约 5-10 分钟（编译所有依赖）。

```bash
# [host → container]
# 3.4 验证编译产物可运行且含 probe + seccomp-notify
docker exec actrail-restricted /usr/local/bin/actrailctl --help | grep -iE "probe|launch"
docker exec actrail-restricted /usr/local/bin/actrailctl launch --help | grep -i seccomp
# -> --seccomp-notify <SECCOMP_NOTIFY>  [possible values: auto, required, disabled]

# 3.5 确认 glibc 兼容（无缺失库）
docker exec actrail-restricted ldd /usr/local/bin/actrailctl | grep -i "not found" || echo "no missing libs"
docker exec actrail-restricted ldd /usr/local/bin/libactrail_tls_payload_probe_sync.so | grep -i "not found" || echo "so: no missing libs"
```

预期：`probe`、`launch` 子命令存在；`--seccomp-notify` 默认 `auto`；`ldd` 报告无缺失库。

---

## Step 4 —— 在容器内安装 agent（opencode）

若你的受限容器镜像未自带 agent，需安装。这里从已有的 unconfined 容器 `actrail-opencode-openeuler` 经宿主机中转拷贝 `opencode` + 其配置。

```bash
# [host]
# 4.1 从 unconfined 容器导出 opencode + 配置到宿主机临时目录。
#     `mkdir`、`docker cp`、`tar` 在宿主机执行；`docker exec ... bash -lc`
#     的脚本体在 unconfined *来源*容器内执行。
mkdir -p /tmp/actrail-agent-pkg
docker exec actrail-opencode-openeuler bash -lc '
  cd /usr/local/bin && tar -cf /tmp/opencode.tar opencode
  cd /root/.config/opencode && tar -rf /tmp/opencode.tar opencode.json
'
docker cp actrail-opencode-openeuler:/tmp/opencode.tar /tmp/actrail-agent-pkg/opencode.tar
tar -C /tmp/actrail-agent-pkg -xf /tmp/actrail-agent-pkg/opencode.tar

# 4.2 装进受限容器（宿主机侧 docker cp；chmod/mkdir 经 docker exec 在受限容器内执行）
docker cp /tmp/actrail-agent-pkg/opencode actrail-restricted:/usr/local/bin/opencode
docker exec actrail-restricted chmod 0755 /usr/local/bin/opencode
docker exec actrail-restricted mkdir -p /root/.config/opencode
docker cp /tmp/actrail-agent-pkg/opencode.json actrail-restricted:/root/.config/opencode/opencode.json

# 4.3 验证（opencode --version 在受限容器内执行）
docker exec actrail-restricted /usr/local/bin/opencode --version
# -> 1.15.13  （或你的版本）
docker exec actrail-restricted test -x /usr/local/bin/opencode && echo "opencode ok"
```

> 若你的 agent 不是 opencode，请在 4.1-4.2 中替换二进制路径与配置，并将后续步骤中的 `opencode run "..."` 换成你的 agent 的非交互式命令。

---

## Step 5 —— 运行 actrailctl probe（验证 seccomp 不可用 + 推荐 auto 降级）

```bash
# [host → container]
# 5.1 人类可读 —— actrailctl 在容器内执行
docker exec actrail-restricted \
  actrailctl --config /etc/actrail/actraild.conf probe
```

预期：
```
unix_socket=ok connected /run/actrail/control.sock
unix_socket=ok connected /run/actrail/tls-sync.sock
no_new_privs=ok enabled=0
seccomp_notify=unavailable pidfd_getfd seccomp listener: Operation not permitted (os error 1)
tls_sync_runtime_library=ok found /usr/local/bin/libactrail_tls_payload_probe_sync.so
collectors=ebpf,tls-sync,application-protocol-analyzer plugins= storage_ready=true
launch_seccomp_notify=disabled
launch_note=seccomp-notify unavailable; use --seccomp-notify auto (default) to select a non-notify deployment
```

关键行：`seccomp_notify=unavailable`（默认 seccomp 拦截 `pidfd_getfd`）、`launch_seccomp_notify=disabled`、`launch_note` 推荐 `--seccomp-notify auto`。

```bash
# [host → container]（probe --json 在容器内执行；输出重定向到宿主机文件）
# 5.2 JSON（用于断言）
docker exec actrail-restricted \
  actrailctl --config /etc/actrail/actraild.conf probe --json > /tmp/probe.json

# [host]
cat /tmp/probe.json | python3 -m json.tool | head -30

# 5.3 断言（python3 在宿主机执行，读取宿主机的 /tmp/probe.json）
python3 - <<'PY'
import json
d = json.load(open('/tmp/probe.json'))
socks = [x for x in d['statuses'] if x['name'] == 'unix_socket']
assert len(socks) == 2 and all(x['available'] for x in socks), "sockets not ok"
sec = {x['name']: x for x in d['statuses']}['seccomp_notify']
assert not sec['available'], "seccomp_notify should be unavailable"
assert 'pidfd_getfd' in sec['detail'] and 'Operation not permitted' in sec['detail']
assert {x['name']: x for x in d['statuses']}['tls_sync_runtime_library']['available']
assert d['launch_seccomp_notify'] is False
assert 'non-notify deployment' in d['launch_note']
print("probe assertions ALL PASSED")
PY
```

预期：`probe assertions ALL PASSED`。

```bash
# [host → container]
# 5.4 仅本地检查（跳过 daemon doctor）
docker exec actrail-restricted \
  actrailctl --config /etc/actrail/actraild.conf probe --skip-daemon
```

预期：本地状态检查相同，但无 `collectors=...` 行。此时权限选择仅为本地预览；launch 仍会向 daemon 请求最终 profile。

---

## Step 6 —— 以 --seccomp-notify auto 启动（降级为 tls-sync-only）

```bash
# [host → container]
# 6.1 默认 auto 模式 —— 应降级并成功。
#     actrailctl launch 在容器内执行；agent（opencode）是它的子进程。
docker exec actrail-restricted bash -lc '
  export PATH=/usr/local/bin:$PATH
  export LD_LIBRARY_PATH=/usr/local/bin:$LD_LIBRARY_PATH
  actrailctl --config /etc/actrail/actraild.conf launch -- \
    /usr/local/bin/opencode run "用一句话回答：AcTrail 是什么"
'
```

预期（权限输出后进入 active trace）：
```
deployment_permissions_degraded=true
deployment_permission_reasons=seccomp_notify_unavailable: ...pidfd_getfd...
trace trace-<N> entered Active
```
随后 agent 运行（调用模型、打印响应）并以退出码 0 结束。

> 注意 `<N>` —— 这是你在 Step 7 要用的 trace id。若从空库开始，它是 `trace-1`；本机已有旧 trace，可能是 `trace-4` 或更大。用 `list-traces`（Step 7.1）找最新的。

对照组（同一容器）对比三种模式：

```bash
# [host → container]
# 6.2 required —— 必须直接失败（不偷偷降级）
docker exec actrail-restricted bash -lc '
  export PATH=/usr/local/bin:$PATH
  actrailctl --config /etc/actrail/actraild.conf launch --seccomp-notify required -- \
    /usr/local/bin/opencode run "hi"
'
# [host]
echo "required exit=$?"
# -> 非零退出，末行: "pidfd_getfd seccomp listener: Operation not permitted (os error 1)"
```

```bash
# [host → container]
# 6.3 disabled —— 必须成功（本受限环境中选择结果与 auto 相同）
docker exec actrail-restricted bash -lc '
  export PATH=/usr/local/bin:$PATH
  actrailctl --config /etc/actrail/actraild.conf launch --seccomp-notify disabled -- \
    /usr/local/bin/opencode run "说一个字: 好"
'
# [host]
echo "disabled exit=$?"
# -> 退出 0，权限输出: deployment_permissions_degraded=false
```

---

## Step 7 —— 验证宿主机侧采集（tls-sync 仍交付明文 + 语义）

```bash
# [host]
# 7.1 列出 trace；挑 6.1 那条最新的 Completed
./target/release/actrailctl --config /etc/actrail/actraild.conf list-traces
```

预期：列表末尾是你刚启动的 trace，如 `trace-4 pid-<PID> pid=<PID> Exited/Clean`。

```bash
# [host]
# 7.2 将 TRACE_ID 设为 6.1 的 trace（auto 降级、完整提问的那次）。
#     这些变量供 7.3、7.4 与 Step 8 使用 —— 请在同一 shell 中保持。
TRACE_ID=trace-4        # <-- 替换为你的最新 Exited trace id
TRACE_NUM=${TRACE_ID#trace-}
echo "TRACE_ID=$TRACE_ID TRACE_NUM=$TRACE_NUM"
```

```bash
# [host]
# 7.3 摘要 —— actrailviewer 读取宿主机 sqlite
./target/release/actrailviewer --config /etc/actrail/actraild.conf summary --trace-id "$TRACE_ID"
```

预期：`state=Exited health=Clean`，`processes`、`events`、`network_events` 非零。

```bash
# [host]
# 7.4 TLS 明文 payload_segments + semantic_actions（仅计数，不打印敏感正文）。
#     sqlite3 直接打开宿主机 /var/lib/actrail/actrail.sqlite。
sqlite3 /var/lib/actrail/actrail.sqlite "
select library, symbol, direction, count(*) as cnt, sum(captured_size) as bytes
  from payload_segments where trace_id = $TRACE_NUM
  group by library, symbol, direction order by library, symbol, direction;
select '---semantic---';
select kind, count(*) as cnt from semantic_actions where trace_id = $TRACE_NUM
  group by kind order by kind;
"
```

预期：`payload_segments` 含 `boringssl SSL_read`（inbound）与 `boringssl SSL_write`（outbound）行（opencode 用 BoringSSL）。`semantic_actions` 含 `llm.request`、`llm.response`、`llm.call`，以及 `command.invocation`、`process.exec`、`http.message`、`sse.stream`。这是核心结论：**容器无 seccomp/eBPF 权限，host 仍经 tls-sync 采到 TLS 明文 + LLM 语义。**

---

## Step 8 —— 验证 web UI 的 action graph

```bash
# [host]
# 8.1 启动 web UI，绑定所有网卡、端口 9999（后台）
./target/release/actrailweb --config /etc/actrail/actraild.conf --addr 0.0.0.0 --port 9999 &
echo "web pid=$!"
sleep 2
```

预期：`actrailweb listening on http://0.0.0.0:9999 storage=/var/lib/actrail/actrail.sqlite`。

```bash
# [host]
# 8.2 浏览器打开
#     先找宿主机 IP：
ip -4 addr show | grep -oP 'inet \K[0-9.]+' | grep -v '^127\.'
#     然后打开：http://<其中一个IP>:9999/
#     选择 Step 7.2 的 trace，查看 action tree、payloads、commands。
```

```bash
# [host]
# 8.3 宿主机侧 API 检查（用 --noproxy 绕过 shell 代理）
curl --noproxy '*' -fsS "http://127.0.0.1:9999/api/traces/$TRACE_NUM/action-tree" \
  | jq -r '[(.roots|length),(.actions|length),(.links|length)]|@tsv'
curl --noproxy '*' -fsS "http://127.0.0.1:9999/api/traces/$TRACE_NUM/commands" | jq -r 'length'
```

预期：action tree 三个数（roots ≥ 1、actions ≥ 1、links ≥ 0）与一个非零 commands 数。

> 若 `curl` 返回 `502` 或 `000`，说明你的 shell 设置了 `http_proxy`/`https_proxy`。用 `--noproxy '*'`（如上）或 `NO_PROXY=localhost,127.0.0.1 curl ...`。浏览器一般不走这些 shell 代理，UI 可正常打开。

```bash
# [host]
# 8.4 用完后停止 web UI（找到并杀掉）
pkill -f 'actrailweb.*9999'
```

---

## Step 9 —— 清理

```bash
# [host]
# 9.1 停止宿主机 daemon
./target/release/actraild --config /etc/actrail/actraild.conf stop

# 9.2 删除受限容器（宿主机侧 Docker 命令）
docker rm -f actrail-restricted

# 9.3 删除 agent 包临时目录
rm -rf /tmp/actrail-agent-pkg /tmp/probe.json

# 9.4 （可选）清空 SQLite 库，下次从干净状态开始
# rm -f /var/lib/actrail/actrail.sqlite
```

`/etc/actrail/actraild.conf` 与运行时目录保留供下次使用。

---

## 故障排查

| 现象 | 原因 | 处理 |
| --- | --- | --- |
| `invalid ebpf.enabled`（非 true/false/auto） | `target/release/actraild` 早于 `[ebpf] enabled = "auto"` 支持 | 重新编译：`cargo build --release -p daemon -p ctl -p view -p web -p tls_payload_probe_sync`（Step 0.5） |
| `missing config key payload_stdio_capture_stdin` | 用了过时的示例配置（如旧版 `container-agent-minimal/operator.conf`） | 改用 `docs/examples/container-agent-restricted/operator.conf`（与当前 daemon 匹配） |
| 容器内运行 `actrailctl` 报 `GLIBC_2.39 not found` | 宿主机编译的产物依赖更新的 glibc | 在容器内编译 `actrailctl` + `.so`（Step 3） |
| `tls-sync auto plan requires payload.tls.binary_path=disabled` | 在 tls-sync 后端下把 `[payload.tls] binary_path` 设成了路径 | tls-sync 下必须为 `disabled`。受限示例配置已设好，勿改 |
| 容器内 `seccomp_notify=ok` | 容器启动时带了 `--security-opt seccomp=unconfined` | 重建容器去掉该 flag（Step 2.2）。本测试特意需要默认 profile |
| probe 报 `control_socket=unavailable` | 先建容器、后起 daemon，socket 没挂进去 | 先 Step 1 起 daemon，再 Step 2 建容器 |
| `payload_segments` 无 TLS 行 | 没经 `actrailctl launch` 启动；`.so` 缺失；socket 没挂载；或 agent 的 TLS 库不是 BoringSSL/rustls | 逐项检查 Step 2.4、3.4、6.1；确认 agent 用受支持的 TLS 库 |
| web 的 `curl` 返回 `502` | shell 的 `http_proxy`/`https_proxy` 拦截了 localhost | 用 `curl --noproxy '*'` 或 `NO_PROXY=localhost,127.0.0.1` |
