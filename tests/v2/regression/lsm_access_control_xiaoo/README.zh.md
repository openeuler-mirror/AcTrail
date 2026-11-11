# 真实 Xiaoo 按操作选择访问控制后端

使用真实 Xiaoo、本地 streaming provider 和实际文件、ELF、TCP 操作，验证独立 BPF LSM 访问控制。

每次启动 daemon 都通过 `actrailctl init -f` 刷新默认配置，并依次传入多个 `--patch`。公共构建器生成隔离路径和运行参数补丁，`configuration/patch_config.toml` 设置观测行为，各后端子目录的 `patch_config.toml` 设置能力与操作后端。

测例开启 socket `bpf-copy` 与 L4 payload 保留，关闭 process seccomp 观测。默认运行五组操作配置：

| 名称 | open | mkdir | rmdir | exec | connect |
| --- | --- | --- | --- | --- | --- |
| lsm | LSM | LSM | LSM | LSM | LSM |
| fanotify_seccomp | fanotify | seccomp | seccomp | seccomp | seccomp |
| fanotify_lsm | fanotify | LSM | LSM | LSM | LSM |
| mixed_a | fanotify | LSM | seccomp | LSM | seccomp |
| mixed_b | LSM | seccomp | LSM | seccomp | LSM |

每组分别验证静态 deny/allow、动态五操作 deny/allow 和 owner 卸载。只有选中 seccomp 操作时启用 seccomp notify；fanotify 句柄只随 open 选择创建。

Xiaoo 的 Bash 工具执行仓库中的 `workload.py`。它尝试写入预创建文件、执行专用 ELF、连接本地 TCP 服务，记录系统调用实际结果。拒绝阶段要求三类操作返回 EPERM/EACCES，文件和网络服务均没有成功操作效果；允许阶段要求三个操作成功。

IPv4 TCP 服务绑定 `127.0.0.2`，静态网络规则使用 `127.0.0.2:*` 验证全端口选择器，动态规则使用实际端口。provider 和管理 API 使用 `127.0.0.1`。

动态阶段复用仓库中 `file-policy-dynamic`、`command-policy-dynamic`、`network-policy-dynamic` 的实际 WASM 工件、manifest 和 schema，通过管理 API 调用插件配置，再由插件调用 policy host 发布规则。文件规则包括递归 open 范围、尚不存在的 mkdir 目标和现有空目录的 rmdir 目标。

文件 policy host 统一经过文件服务，LSM controller 保存文件编译快照及选中的命令、网络策略。每次 apply 完成授权和候选规则编译后，填充并冻结新的内核 LPM map，再原子切换生效快照。实际访问由 LSM hook 查询快照并裁决；WASM 在发布阶段运行。单次 host 更新为原子发布边界，插件卸载通过重新发布剩余 owner 规则撤销策略。

动态验证顺序（argv、标识符与容量负例仅在全 LSM 组运行）：

1. 发布三域 deny，真实 Xiaoo 验证 open、exec、connect、mkdir、rmdir 拒绝及实际文件系统、网络效果。
2. 分别验证和提交包含 argv 条件、rule_id 内嵌 NUL 的命令候选，要求拒绝；比较插件配置及策略 revision，并再次运行真实 Xiaoo 验证当前 deny。
3. 验证和提交超过 `policy_max_entries` 的完整候选，要求拒绝；比较配置及 revision，再次验证当前 deny。
4. fanotify/seccomp 组发布目标插件尚未加载的三域 gray 规则，验证五类真实操作拒绝；网络规则配置 `fallback=allow`，确认未加载目标仍拒绝。
5. 全 LSM 和 fanotify/seccomp 组分别发布同 owner 的全端口拒绝、精确端口允许，以及全端口允许、精确端口拒绝，真实 Xiaoo 验证精确端口优先，并查询另一端口确认全端口规则生效。
6. 两个 owner 先分别占用同 IP 的不同端口，再尝试加入全端口规则；同时验证全端口规则先发布时拒绝另一 owner 的精确规则。配置验证和实际提交均检查拒绝后的配置、规则列表和 revision 不变，再运行真实 Xiaoo 确认原规则继续执行。
7. 发布 allow，验证五类真实操作成功及对应后端审计。
8. 再次发布 deny 并卸载三个 owner，验证真实操作恢复默认 allow。

测试同时核验：

- viewer 中每项操作的审计 backend 与配置一致，并匹配对应规则。
- Xiaoo 首次 exec 的显式允许规则 `lsm-root` 有对应审计。
- socket bpf-copy 产生已捕获的 payload。
- 工作负载至真实 Xiaoo 的进程祖先及线程，其 Seccomp 状态与选中操作一致。
- daemon 的 fanotify 和 seccomp listener FD 与实际配置一致。
- 动态候选发布失败保留已有配置与规则 revision，owner 卸载撤销内核生效策略。

```bash
python3 tests/v2/regression/lsm_access_control_xiaoo/run_e2e.py --color never
```

此用例注册为可选回归，要求 root、支持所需 BPF LSM hook/helper 的内核、已启用 BPF LSM、BTF、eBPF 和真实 Xiaoo。环境预先继承的 seccomp 必须为 0；不满足条件时报错。runner 构建并安装 release 工件，默认清理服务和测试工作目录。

配置项：`LSM_ACCESS_CONTROL_XIAOO_E2E_BINARY` 指定 Xiaoo；`LSM_ACCESS_CONTROL_XIAOO_E2E_READY_TIMEOUT_SECONDS` 设置服务就绪、操作及审计超时（默认 15 秒）；`LSM_ACCESS_CONTROL_XIAOO_E2E_PEER_POLL_SECONDS` 设置 TCP 服务轮询间隔（默认 0.1 秒）；`LSM_ACCESS_CONTROL_XIAOO_E2E_POLICY_MAX_ENTRIES` 设置测试策略容量（默认 128，范围 32–128），同时决定超容量候选规模。命令与启动超时使用同前缀公共测试配置。工作目录不得含空白字符，以符合静态策略语法。

TODO：增加内核 map 发布故障注入和进程暂停恢复场景。

启动复用验收使用 `LSM_ACCESS_CONTROL_XIAOO_E2E_STARTUP_REUSE=1`，通过 `LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS=lsm,fanotify_seccomp` 比较有、无 LSM 的配置。每组默认刷新配置并启动三次 daemon，每次运行两次真实 Xiaoo，核对 BPF program/map/link ID 不变、一次准备及加载日志，并验证退出后程序释放。`LSM_ACCESS_CONTROL_XIAOO_E2E_STARTUP_SAMPLES` 可调整采样次数；`LSM_ACCESS_CONTROL_XIAOO_E2E_STARTUP_BASELINE=1` 用于不提供加载计数日志的基线二进制。输出 `STARTUP_REUSE_EVIDENCE` 包含原始采样；`start_ms` 是 start 命令完成耗时，后续另行确认 RPC 可用。

`LSM_ACCESS_CONTROL_XIAOO_E2E_STARTUP_BOUNDARIES=1` 单独验证无 BPF 需求、移除 BPF 权限后的失败缓存、重复请求拒绝、恢复权限后重启、真实 Xiaoo attach 复用、仅 TLS 的 BPF 配置双向明文采集，以及强制 LSM 初始化失败阻止启动。该场景额外需要 `setpriv`、`bpftool` 和 `openssl`。启动场景开关应单独使用。

失败时在清理前将 daemon 日志尾部写入测试输出；`LSM_ACCESS_CONTROL_XIAOO_E2E_DIAGNOSTIC_BYTES` 控制输出上限（默认 65536 字节）。

同一 WASM 动态策略在 seccomp 与 LSM 后端下的访问性能对照见 [性能实验](performance/README.zh.md)。该实验显式运行，从刷新配置启动真实 Xiaoo，单独统计规则发布与访问成本。

`LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS` 可指定逗号分隔的组名；默认运行全部五组。

设置 `LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS=lsm` 后，可选择以下独立场景：

- `LSM_ACCESS_CONTROL_XIAOO_E2E_IPV6=1`：真实 Xiaoo 通过 `connect(..., 24)` 连接 IPv6 服务，覆盖 allow/deny 与 `deny_on_error` 开关的四种组合，并核验实际连接效果和规则审计。
- `LSM_ACCESS_CONTROL_XIAOO_E2E_ALERTS=1`：真实 Xiaoo 分别触发 64 次文件、命令和网络拒绝，验证首次告警、增量次数、trace 结束及周期摘要、容量抑制，以及关闭审计后仍有告警。网络告警核对实际连接端点和规则归属。正常退出后再次检查告警没有重复，普通进程事件仍被保存。

告警计数表示收到并提交的拒绝事件，内核事件丢失和容量抑制不会被算作精确命中次数。

路径绑定场景：

```bash
LSM_ACCESS_CONTROL_XIAOO_E2E_PATHS=1 LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS=lsm,fanotify_seccomp python3 tests/v2/regression/lsm_access_control_xiaoo/run_e2e.py --color never
```

静态与动态规则分别以符号链接 A 声明文件、可执行路径和 mkdir/rmdir 的父目录递归范围，发布时 A 指向 B。真实 Xiaoo 批量执行 `path-probes.json` 中的文件写入、exec、mkdir 和 rmdir，验证：

- 初始访问 A 和 B 被拒绝，C 允许；发布时不存在的文件在创建后受控，未来 mkdir 目标仍被拒绝。另以悬空符号链接声明尚未创建的文件，创建目标后验证链接及直接目标均受控。LSM 另外覆盖发布时不存在的递归目录。
- A 改指 C 后，删除旧静态文件规则的父目录、改名旧静态命令目标，再发布网络拒绝规则；直接 B 仍受控，A/C 允许，真实网络连接被拒绝。
- 卸载网络规则 owner 后连接恢复，文件和命令绑定、声明及 revision 保持原状。
- 动态规则通过原样提交配置更新绑定；静态规则通过刷新默认配置并重启重新加载。随后 A/C 被拒绝，B 允许。

每阶段检查操作 errno、真实文件内容和目录状态、选中的后端、规则审计中的实际 B/C 路径以及正常观测。fanotify 的未来文件使用已存在的父目录。IPv6、告警、路径三个独立场景开关不能同时启用。

命令路径同时通过普通路径和继承的 ELF 文件描述符执行；后者由真实子进程访问 `/proc/self/fd/N`，核验两种后端均按实际 B/C 目标裁决。

fanotify/seccomp 组还由工作负载 fork 子进程，chroot 到测试空目录后，分别通过保留在新根外的 cwd 和 dirfd 发起相对 mkdir/rmdir。pipe 返回实际 errno，父进程等待退出并检查 B/C 目录效果，确认裁决使用实际目标。

同组另外预开 B/C 目录，在普通根目录环境通过 `/proc/self/fd/N/sub/<child>` 执行 mkdir/rmdir，检查 magic 目录链接后缀的解析和实际子目录效果。

已有文件描述符的动态权限验证：

```bash
LSM_ACCESS_CONTROL_XIAOO_E2E_CONTINUED=1 \
LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS=lsm \
python3 tests/v2/regression/lsm_access_control_xiaoo/run_e2e.py --color never
```

真实 Xiaoo 的 Bash 工作负载先打开受保护文件和对照文件，并保留两个文件描述符。测试控制进程通过实际 WASM 插件发布路径规则，依次验证允许、拒绝和恢复允许。每阶段核对读写 errno、文件内容和真实 agent 祖先。拒绝阶段检查 LSM 规则审计，并验证 pipe、socketpair 和 memfd 正常使用。

该场景使用现有 `open` 路径规则检查后续读写。测试通过 `actrailctl init -f` 刷新默认配置。`CONTINUED` 与 IPv6、告警、路径场景互斥，等待时间沿用公共启动超时和 peer 轮询配置。

同一场景还保留文件映射，使用原 FD 请求新的读、写、执行映射，并对原映射调用 `mprotect` 提升权限。动态拒绝期间这些操作返回权限错误，恢复允许后成功。权限降低和匿名映射始终允许。已有映射的内存访问不属于该撤权范围。

网络阶段保留 IPv4/IPv6 TCP 和 UDP socket，并建立独立的受保护端点与对照端点。发送操作覆盖 `send`、`write`、未连接 UDP 的 `sendto`/`sendmsg`、TCP 忽略显式地址时的真实 peer，以及 IPv4/IPv6 UDP 的 `AF_UNSPEC` 地址语义。全端口规则和精确端口例外共同参与判定。见证服务核对实际收到的每条消息，拒绝阶段只有对照端点收到数据，恢复允许后原连接继续发送。

发送检查复用现有 `connect` endpoint 规则，按本次发送地址或已连接 socket 的 peer 匹配。UDP cork/`MSG_MORE` 缓存中的目标地址未参与匹配，因此该场景的撤权控制存在缺口。

默认测试场景覆盖普通 socket 发送。TCP Fast Open 使用下述独立入口验证。

文件删除阶段分别通过完整路径和预先打开的目录 FD 调用 unlink。受保护目录使用现有 `open` 路径拒绝规则，控制操作通过 `path_unlink` 检查实际目录项。每阶段重新准备文件，验证允许时文件删除、拒绝时文件保留，以及对照目录始终可删除。

权限位修改阶段依次调用 chmod 和旧 FD 上的 fchmod，使用不同目标权限位验证两次修改。拒绝时权限位保持原值，恢复后达到目标值。父测试再次核对实际文件状态，memfd 的 fchmod 单独验证。

所有权修改阶段通过 chown 和旧 FD 上的 fchown 分别设置不同 UID/GID，验证两次真实修改。拒绝时所有权保持原值，恢复后改回测试进程的 UID。父测试核对最终 UID/GID，并验证 memfd 的 fchown 正常执行。

TCP Fast Open 使用独立网络命名空间运行同一真实 agent 场景：

```bash
python3 tests/v2/regression/lsm_access_control_xiaoo/continued/fastopen/run.py
```

启动器先创建并核验独立网络命名空间，再启用 loopback 和该命名空间的 TFO 客户端、服务端开关。服务端和客户端设置 socket 级 no-cookie 选项。客户端从全新、未连接的 socket 直接调用 `sendto(..., MSG_FASTOPEN, ...)`，并通过 `SO_SNDTIMEO` 限制阻塞时间。

允许和恢复阶段要求 IPv4/IPv6 服务端收到精确消息，并且 `TCP_INFO` 包含 `TCPI_OPT_SYN_DATA`，确认 SYN 数据被确认。拒绝阶段要求权限错误、socket 保持未连接、目标服务端没有对应消息。常规 TCP 回退不会通过该验收。该场景同时复验已有文件、映射、删除、属性和网络控制。

## 文件读写权限

文件语义测试还验证 truncate、ftruncate 和 O_TRUNC。各次调用后核验实际内容和长度，拒绝截断时核验文件保持完整。

LSM 场景还交替打开 240 字节路径、短路径、超出资源容量的路径和短路径。测试核验实际允许/拒绝结果、长短路径审计，以及解析失败审计不携带旧路径。

运行真实 Xiaoo，在保留同一 FD 的同时分别发布 open、write、read、delete、attr 拒绝规则，再撤销规则。测试核验打开门禁、已有 FD 读写、共享映射、私有写时复制映射、实际文件内容、unlink 后目录项及 chmod/chown 后模式和属主。

```bash
LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS=lsm \
LSM_ACCESS_CONTROL_XIAOO_E2E_SEMANTICS=1 \
python3 tests/v2/regression/lsm_access_control_xiaoo/run_e2e.py --color never
```

## 文件重命名和硬链接

真实 Xiaoo 分别执行普通重命名、覆盖重命名、RENAME_EXCHANGE 和硬链接。测试分别拒绝源路径、目标路径和覆盖目标的删除权限，并核验实际名字、内容和 inode。重命名和硬链接包含绝对路径及目录 FD 相对路径调用。

```bash
LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS=lsm \
LSM_ACCESS_CONTROL_XIAOO_E2E_NAMES=1 \
python3 tests/v2/regression/lsm_access_control_xiaoo/run_e2e.py --color never
```

## 网络操作和协议

使用真实 Xiaoo 验证 TCP/UDP、IPv4/IPv6、已有连接发送、新连接、bind 和 listen。测试核对接收端实际数据及本地 socket 状态。

```bash
LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS=lsm \
LSM_ACCESS_CONTROL_XIAOO_E2E_NETWORK_SEMANTICS=1 \
python3 tests/v2/regression/lsm_access_control_xiaoo/run_e2e.py --color never
```

将后端改为 fanotify_seccomp 可验证新规则的能力拒绝和原有访问行为。

## CIDR 和端口范围

真实 Xiaoo 在端口区间下界、中间、上界及区间外发起 TCP/UDP 操作。测试覆盖 IPv4/IPv6 网段、`/0`、更具体的地址与端口例外、带 IPv6 scope ID 的连接和规则撤销恢复。

发布验证覆盖授权包含关系、跨 owner 重叠拒绝、协议优先级和展开后的快照容量。容量拒绝后核对原配置与规则命中结果。

```bash
LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS=lsm \
LSM_ACCESS_CONTROL_XIAOO_E2E_NETWORK_RANGES=1 \
python3 tests/v2/regression/lsm_access_control_xiaoo/run_e2e.py --color never
```

将后端改为 `fanotify_seccomp` 可验证 connect 的 CIDR 与端口区间控制及拒绝审计。
