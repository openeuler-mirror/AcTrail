# 真实 Xiaoo 按操作选择访问控制后端

使用真实 Xiaoo、本地 streaming provider 和实际文件、ELF、TCP 操作，验证独立 BPF LSM 访问控制。

每次启动 daemon 都通过 `actrailctl init -f --patch` 刷新默认配置，开启 socket `bpf-copy` 与 L4 payload 保留，关闭 process seccomp 观测。默认运行五组操作配置：

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

配置项：`LSM_ACCESS_CONTROL_XIAOO_E2E_BINARY` 指定 Xiaoo；`LSM_ACCESS_CONTROL_XIAOO_E2E_READY_TIMEOUT_SECONDS` 设置服务就绪、操作及审计超时（默认 15 秒）；`LSM_ACCESS_CONTROL_XIAOO_E2E_PEER_POLL_SECONDS` 设置 TCP 服务轮询间隔（默认 0.1 秒）；`LSM_ACCESS_CONTROL_XIAOO_E2E_POLICY_MAX_ENTRIES` 设置测试策略容量（默认 64，范围 32–128），同时决定超容量候选规模。命令与启动超时使用同前缀公共测试配置。工作目录不得含空白字符，以符合静态策略语法。

TODO：增加内核 map 发布故障注入和进程暂停恢复场景。

失败时在清理前将 daemon 日志尾部写入测试输出；`LSM_ACCESS_CONTROL_XIAOO_E2E_DIAGNOSTIC_BYTES` 控制输出上限（默认 65536 字节）。

同一 WASM 动态策略在 seccomp 与 LSM 后端下的访问性能对照见 [性能实验](performance/README.zh.md)。该实验显式运行，从刷新配置启动真实 Xiaoo，单独统计规则发布与访问成本。

`LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS` 可指定逗号分隔的组名；默认运行全部五组。

设置 `LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS=lsm` 后，可选择以下独立场景：

- `LSM_ACCESS_CONTROL_XIAOO_E2E_IPV6=1`：真实 Xiaoo 通过 `connect(..., 24)` 连接 IPv6 服务，覆盖 allow/deny 与 `deny_on_error` 开关的四种组合，并核验实际连接效果和规则审计。
- `LSM_ACCESS_CONTROL_XIAOO_E2E_ALERTS=1`：真实 Xiaoo 分别触发 64 次文件拒绝和 64 次命令拒绝，验证首次告警、增量次数、trace 结束及周期摘要、容量抑制，以及关闭审计后仍有告警。正常退出后再次检查告警没有重复，普通进程事件仍被保存。

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
