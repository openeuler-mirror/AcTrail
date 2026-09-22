# 采集开销矩阵 benchmark

用相同工作量比较 **0（不采集）、P、C × fork、exec、read、write、agent TPOT=0、agent TPOT=3、空任务**，输出 21 个测试单元的 CPU 和耗时。默认每个单元预热一轮、正式测量一轮，每次执行上限 10 秒；较轻的负载会提前完成。daemon 启动、采集收尾和编译分别处理，不算进负载执行时间。

P 使用 `bpf-copy`，通过 `payload.tls.direct_dynamic_discovery_enabled = false` 关闭 daemon 侧动态探针发现，启动探针发现开启。动态发现专项验收需要通过专用配置显式开启动态发现。

## 运行

在仓库根目录执行：

```bash
sudo -E python3 scripts/bench/payload --agent-bin "$(command -v xiaoo)"
```

需要 Linux、Python 3.11+、C 编译器 `cc`、OpenSSL、可用的真实 `xiaoo` 或 `opencode` 和 AcTrail 的主机采集权限；OpenCode 还需要 Git、npm，首次准备依赖需要可用的 npm 缓存或 registry 连接。默认先执行 `cargo fmt`、`cargo build --release`；使用 `sudo` 时需保留可找到 Rust 工具链的 PATH，或在已具备权限的开发 shell 中直接执行。benchmark 保存开始和结束时的并发 daemon 信息，只停止本次隔离实例；其他实例存在不直接使样本无效。

纯 TLS `bpf-copy` 的启动预挂功能验收使用独立入口：

```bash
python3 -m scripts.bench.payload.direct_acceptance --out local/bench/payload/bpf-functional
```

先构建 `cargo build --release -p daemon -p ctl -p view`。该入口用刷新后的默认配置运行真实 xiaoo，覆盖 P/C、TPOT 0/3、实际工具输出及 viewer 读回。P 叠加本目录 `configs/tls-bpf-copy.toml`，C 使用原配置；有效配置与产物哈希随结果保存。进程映射按 trace root 的 PID 和启动代际核对，P 要求无 runtime 注入，C 要求有注入。`--agent-bin`、`--bin-dir` 和 `--config-dir` 可显式指定。

此入口仅进行功能验收，允许其他 daemon 共存，每个测试实例使用独立配置、socket、数据库和 BPF maps，退出时只停止自身实例。CPU 对照由正式 benchmark 统计 task 及本次 daemon 自身的 CPU，结合样本范围及并发环境评估变化；不能仅凭 wall 变长认定 CPU 样本无效。新 executable/共享库的运行中发现尚不属于该启动预挂验收范围。

动态发现专项通过 `--config-dir` 指定显式开启动态发现的配置，再使用 `--modes P --via-bash-exec`，让启动预挂只看到 bash，由实际 exec 发现 xiaoo，并保存实际启动命令。真实外层 agent 工具调用、不同 inode、同路径替换及短执行后的复用使用独立入口：

```bash
python3 -m scripts.bench.payload.nested_acceptance --out local/bench/payload/bpf-nested
```

首次异步发现允许出现明确记录的覆盖缺口；已有挂载就绪证据的后续调用要求完整请求、响应及命令关联。调试日志中的 `attachment ready` 包括复用成功，不能当作新挂载次数。短进程未能及时取得文件引用时，不声称已验证退出后继续解析。共享库映射发现独立验收。

长历史限采验收使用相同真实 agent 入口：

```bash
python3 -m scripts.bench.payload.direct_acceptance --modes P \
  --turns 100 --input-bytes 65536 --limited-acceptance \
  --out local/bench/payload/bpf-long
```

`--limited-acceptance` 要求实际出现缺失字节证据，核验请求的 `capture_limited`、响应协议完成及逐一关联；完整请求仍须通过 JSON 完整性检查。该专用断言只把“声明的请求体已经大于整个捕获报文”作为缺失的充分证据，其他不完整且长度不明确的情况会失败。真实 OpenSSL 短写、限采及失败的操作级验证见 [tls_write/README.md](tls_write/README.md)。

使用已编译的 release 产物进行快速测量：

```bash
sudo -E python3 scripts/bench/payload \
  --skip-build --agent-bin "$(command -v xiaoo)" \
  --out local/bench/payload/current
```

输出目录必须不存在。`--skip-build` 会明确记录没有核验二进制与源码 commit 的对应关系，并保存二进制路径、大小及修改时间。`--bin-dir` 可以指定另一套完整 release 产物目录；与 `--skip-build` 一起使用。

缩小范围或调整工作量：

```bash
python3 scripts/bench/payload --skip-build --workloads fork exec
python3 scripts/bench/payload --skip-build --workloads read write --read-operations 4096 --write-operations 4096
python3 scripts/bench/payload --skip-build --workloads agent --agent-turns 6
python3 scripts/bench/payload --skip-build --workloads agent --agent-tpot-ms 0 3
python3 scripts/bench/payload --skip-build --workloads agent --agent-kind opencode --rounds 3
python3 scripts/bench/payload --skip-build --workloads idle
python3 scripts/bench/payload --skip-build --workloads stdio --stdio-operations 100000
python3 scripts/bench/payload --skip-build --workloads agent --agent-turns 12 --agent-input-bytes 65536
```

`--modes 0 P` 可只比较两组；缺少 0 组时仅输出绝对值。`--warmups` 控制预热轮数，`--rounds` 控制正式采样轮数，每轮仍受 `--timeout-seconds` 限制。`--warmups 0` 可观察未经任务预热的结果。默认参数集中在 [configs/benchmark.toml](configs/benchmark.toml)，可用 `--config-dir` 指向同结构目录。

`--workloads stdio` 单独执行固定次数的 10 字节 stdout 写入，记录实际 syscall 次数并核验完整输出；默认矩阵不包含该专项。`--agent-input-bytes` 设置每次真实工具读取的文件长度，配合 `--agent-turns` 增加请求历史。agent 可能限制工具返回内容，输入文件大小不能直接当作 LLM 请求大小，需以实际采集证据为准。

stdio 专项要求有效配置开启 stdout 采集及 `stdio-chunk`，并验证 trace clean。默认 stdout 为 drop，数据库没有每次写入的 payload；报告的 `payload_bytes_verified=false` 明确该专项的验证范围。变长事件的逐字节采集验收需使用保留 stdout/L4 的独立运行和真实 MCP agent。

MaaS 请求体上限由 `maas_max_request_bytes` 或 `--maas-max-request-bytes` 控制，默认16MiB。长历史超过上限会导致服务拒绝请求及客户端失败，应保留失败证据并为对照双方固定同一上限。

`--perf` 对每个观测样本的 daemon 采集 `cpu-clock:u` 99 Hz DWARF 调用栈，保存原始 data、日志和文本报告。带该选项的结果标记 `diagnostic_perf=true`，仅用于热点定位；正式 CPU 对照应单独运行且不启用该选项。

工具结果配置专项使用真实 OpenCode、HTTPS MaaS 和正式 viewer 验证动作身份、关系、未导出/足额/超限三态及配置冲突：

```bash
python3 -m scripts.bench.payload.tool_results_acceptance --out local/bench/tr-accept
```

运行前执行 `cargo fmt`、`cargo build --release`。输出路径应保持简短，以满足Unix socket路径长度上限。

带 `--keep-runtime` 完成真实 agent benchmark 后，可运行 `python3 -m scripts.bench.payload.profile_acceptance <结果目录>`，核验P的内容确实缺省、C的请求内容仍保留，以及两组请求/响应的结束状态、时间戳和call关系。`llm.call` 沿用现有推断分组语义，协议结束证据来自request/response动作。

关系与离线 OTel 验收使用 `python3 -m scripts.bench.payload.lineage_acceptance <结果目录> --bin-dir <release产物目录>`。它通过指定 viewer 读取真实 trace，核验 command/agent 的五类实时关系、命令关系图和 LLM command 归属，并实际导出 OTel 检查动作、span 及父关系。每个模式需要实际覆盖五类关系；关闭文件采集等配置可能无法满足该覆盖要求。报告明确列出未覆盖的迟到事件、异常退出、网页和在线 OTLP 场景。

`lineage_acceptance --functional` 读取 `direct_acceptance` 的真实 agent 结果。`profile_acceptance --bin-dir <release产物目录>` 使用对应构建的 viewer，适合 schema 不同的固定版本对照。运行时状态拆分的真实 MCP、关系及网页 API 验收见 [action_state/README.md](action_state/README.md)。

`--workloads agent` 按配置中的 `agent_tpot_ms = [0, 3]` 展开为 `agent-tpot0` 和 `agent-tpot3` 两行。`--agent-tpot-ms` 接受一个或多个非负毫秒值，例如 `--agent-tpot-ms 3` 只测 3 ms。每个 TPOT 值独立预热，观测开销仅与相同 TPOT 的裸跑结果比较。

`--agent-kind xiaoo|opencode` 选择真实 agent，默认为 xiaoo；自动从 PATH 查找所选程序，`--agent-bin` 可覆盖路径。每次运行只测一种 agent，报告记录其名称，额外开销使用该 agent 自己的裸跑基线。比较时同时看额外 CPU 毫秒数与百分比：agent 自身 CPU 越高，相同采集增量对应的百分比越低。

## 配置组

| 项目 | 0 | [P.toml](configs/P.toml) | [C.toml](configs/C.toml) |
| --- | --- | --- | --- |
| daemon 与采集器 | 停止 | 开启 | 开启 |
| TLS 后端 | — | `bpf-copy`（uprobe） | `tls-sync` |
| TLS 单段 / 单操作捕获上限 | — | 65535 / 65535 字节 | 默认配置 |
| TLS `sync_flow_control_enabled` | — | 不适用 | `false` |
| socket 后端 | — | `bpf-copy` | `bpf-copy-seccomp-fallback` |
| `seccomp_notify.enabled` | — | `false` | `true` |
| 治理控制、进程 seccomp | — | 关闭 | 关闭 |
| LLM 工具结果投影 | — | 关闭 | 开启 |
| LLM 请求/响应内容、工具声明、usage、trajectory | — | 关闭 | 刷新后的默认值 |
| L2 body、独立 SSE、HTTP2 帧明细、L4 payload 存储 | — | 关闭 | 刷新后的默认值 |
| stdio | — | 刷新后的默认值 | 刷新后的默认值 |
| MCP | — | 关闭 | 刷新后的默认值 |

P/C 是独立开关组成的配置组。P 保留 LLM 识别、关联、时间及结束状态，显式关闭内容、usage、工具声明及结果投影；进程、文件和 stdio 采集独立配置。现有协议解析仍参与识别与组装，关闭输出不代表已经省掉解析过程中的全部内容构建。C 的完整采集可能使用同步 seccomp 后端。两组均按现有 profiling 配置去掉 `fanotify` capability。

每组通过 `actrailctl init -f --patch` 生成新的默认配置，再应用组内开关与独立运行路径。输出保留原始 patch、解析后的完整 P/C 配置和实际负载参数，方便检查默认值变化。benchmark 不依赖 `local/` 中的配置或历史录制。

## 七种负载

| 负载 | 默认工作量 | 实际执行与验收 |
| --- | --- | --- |
| fork | 1,000 次 | 原生程序逐次 fork，子进程直接 `_exit`，父进程逐个 wait；检查所有退出码 |
| exec | 100 次 | 子进程 exec 同一原生程序，父子各执行 1,000,000 次整数迭代，父进程 wait |
| read | 8,192 × 4 KiB | 顺序读取预先准备的 32 MiB 文件；检查操作数、字节数和每块首尾字节累计值 |
| write | 8,192 × 4 KiB | 顺序覆盖预先准备的 32 MiB 文件；检查操作数、字节数和文件长度 |
| agent-tpot0 | 4 次 LLM 请求，TPOT=0 ms | 所选真实 agent 连接本地 HTTPS MaaS，执行 3 次 bash 工具读取并复制各 16 KiB 的内容，最后输出完成标记 |
| agent-tpot3 | 4 次 LLM 请求，TPOT=3 ms | 使用相同剧本、工具步骤、工具文件内容及轮数，按 3 ms 的 SSE 帧间隔发送响应 |
| idle | `sleep 1` | 直接启动系统 sleep，等待 1 秒后正常退出；观测组检查进程事件和 trace 完整性 |

空任务用于观察一次采集任务的基础成本，包括启动受控进程、程序加载、等待期间的 daemon 活动和 trace 收尾。时长由 `idle_seconds` 或 `--idle-seconds` 控制。sleep 自身 CPU 很少，应优先比较额外 CPU 的绝对毫秒数；该结果包含整个任务生命周期，不等于 daemon 单独空转的 CPU，也不自动从其他负载中扣除。

fork 不为每个子进程启动 shell 或解释器。exec 包含父任务和子任务，覆盖生命周期、执行上下文及实际 CPU 工作。read/write 处理短 I/O 和 EINTR，并报告实际 I/O 调用次数。

读写文件在计时前准备并进入 page cache；不清空主机缓存，不调用 fsync，因此衡量的是热缓存下系统调用及采集处理的成本，不是磁盘持久化吞吐。每次负载使用独立目录、相同字节内容和参数。文件准备不属于 trace。

真实 agent 的提示、工具命令、正文大小和终止标记放在 [maas/agent.json](maas/agent.json)。本地 MaaS 复用仓库的 `tests/v2/common/test_suites/local_maas_server/server.py`；每种 TPOT 使用独立的本地服务实例，每次 agent 运行前重置回放，并创建独立 agent 配置和工具目录。没有远程模型调用，也不使用 HTTP 客户端脚本替代 agent。验收检查成功请求数、工具输出文件内容和最终响应标记。

OpenCode 使用显式 `--dir` 和单元内预先初始化的空 Git 仓库，工具执行与快照以该单元为项目范围。每个 TPOT 的 XDG 数据、配置和缓存目录独立于用户环境，并在 0/P/C 的预热与正式测量之间复用。每次测量启动新 agent 进程。显式 provider 配置指向本地 MaaS，禁用项目配置读取、外部插件、模型目录更新、自动更新与自动生成会话标题；HTTPS 使用本地 CA。两种 agent 使用同一工具剧本，各自的系统提示、工具 schema、内部状态维护和进程行为仍有差异。

OpenCode 配置加载仍可能触发后台依赖安装。准备阶段按 `opencode --version` 在各 TPOT 的共享配置目录同步安装对应版本的 `@opencode-ai/plugin`，并核验实际包、manifest 和 lock。准备受 `build_timeout_seconds` 限制，发生在 daemon 启动与负载计时前；安装日志位于 MaaS 运行目录的 `npm-setup.log`。预热和正式运行设置 `npm_config_offline=true`，避免 registry 检查或下载进入采样。

分析固定成本时，可固定 `--agent-tpot-ms 0 --rounds 3`，分别测 `--agent-turns 2` 和 `--agent-turns 8`，并用 `--workloads idle` 测无 LLM 的任务基础成本。轮数增加同时带来工具执行和累计请求历史增长，因此差值表示这些工作合计的增量。空任务成本只能作为参照，不能精确替代 agent 自身的启动成本。

这里的 TPOT 按本地 MaaS 的实现作用于 SSE 帧：首帧使用 TTFT，后续每帧发送前等待指定毫秒数，0 表示不执行这次等待。工具参数或文本整块发送；当前四轮短剧本的 3 ms 配置增加约 45～57 ms 的人工等待。两组用于比较不同发送节奏下的采集开销及分片影响。各次真实 agent 的上下文可能包含不同工作目录。

## 测量口径

- **预热**：0/P/C 各自先按相同参数运行全部选定负载的预热轮，再进入正式轮。预热也检查负载结果、采集完整性并等待 trace 收尾，真实 agent 每次使用独立目录并重置 MaaS。预热不进入均值、裸跑分母或额外开销比例；默认共有 21 个预热记录和 21 个正式样本。
- **任务 CPU**：使用 `wait4` 回收负载进程的 user+system CPU，包含它已回收的子孙进程。所有原生负载等待其子进程结束；观测组还包括 `actrailctl launch` 的开销。避免通过轮询存活 PID 漏掉快速退出的 fork/exec 子进程。
- **daemon CPU**：读取 `/proc/<pid>/stat` 的累计值差，分别记录负载运行和退出后收尾两段。等待该 trace 的 `trace_finalization completed` 日志后结束计费；不在负载刚退出时停止计费。
- **额外 CPU**：`(观测任务 CPU + daemon CPU − 裸跑任务 CPU) / 裸跑任务 CPU`。同时输出绝对增加的毫秒数，便于判断很小分母产生的大百分比。
- **耗时**：从启动命令到命令退出；采集收尾时间单列。daemon 启动耗时保存在 JSON 中，启动和停止的 CPU 不纳入单元结果。
- **完整性**：观测单元要求产生一个 trace、完成收尾、trace health 为 clean。fork/exec 检查进程事件数至少达到操作数，read/write 通过文件路径索引检查测试文件的对应语义动作，agent 检查采到的 LLM 请求和响应数均等于配置轮数；报告按 kind code 汇总事件和语义动作。事件数量不直接等于用户操作数量。自定义配置若关闭这些能力或将单文件读写合并为其他动作类型，验收会明确失败。
- **排除项**：benchmark 控制进程和本地 MaaS server 的 CPU。daemon 空闲背景 CPU 不额外扣除，报告的是测量窗口内实际消耗。

0 组运行时没有 daemon，之后顺序运行 P、C；每组复用一个隔离 daemon，各单元使用独立 trace。默认一轮用于快速识别影响范围，短负载、CPU tick 精度、缓存、运行顺序和主机其他任务都会影响比例，不能据此宣称小幅差异具有统计显著性。

## 输出与失败

默认结果写入仓库的 `local/bench/payload/<时间>/`，由仓库的 `local/` 忽略规则排除出 Git。`scripts/bench/payload/` 保存 benchmark 源码、输入配置和说明。

- `results.md`：21 单元对照，含任务 CPU、daemon CPU、总 CPU、额外 CPU 比例、耗时及收尾时间。
- `results.json`：原始样本、均值、工作量验收、采集数量、构建与环境信息；每个样本的 `phase` 区分 `warmup` 和 `measured`，agent 样本还记录 `agent_tpot_ms`。
- `configs/`：输入配置、完整 P/C 配置和 agent fixture 快照；CLI 覆盖后的实际参数在 JSON 的 `settings` 中。
- 各单元目录：负载或真实 agent 的标准输出、错误日志及 agent 配置。

预热或正式执行中出现超时、请求失败、工具结果错误、trace 不完整或收尾超时，都会使运行返回非零，并在结果中保留失败状态；失败单元不进入均值。排空单独受 `drain_timeout_seconds` 限制，默认 10 秒。

结束时停止本次 daemon 和 MaaS。成功后清理测试数据文件、编译的负载驱动、daemon 数据库和证书等运行目录，保留结果和配置；`--keep-runtime` 可保留运行目录用于排查。失败时保留运行证据。基准锁复用仓库其他采集测试的锁，避免同时运行导致污染。

目录内包含负载实现、配置、MaaS 剧本和报告生成；daemon 生命周期、release 构建与测试锁复用仓库现有 benchmark 基础设施。

## Provider 保留验收

`python3 -m scripts.bench.payload.validation.providers --out local/bench/providers`
使用当前 release、刷新默认配置、真实 OpenCode 与 Anthropic Messages HTTPS MaaS。
四轮请求包含 reasoning、正文和三次真实 bash 工具执行；验证 P/C 和独立关闭响应正文、工具声明、usage 的组合。
动作与关系通过正式 viewer 读回，核对 provider、完成状态、chunk 证据和独立保留行为。
该命令用于功能验收，性能对照使用上面的固定工作量 benchmark。

`python3 -m scripts.bench.payload.validation.network --out local/bench/network`
通过真实Python HTTPS客户端、TLS server和正式viewer核验选择字段解析，包括字段顺序、重复字段、背景请求、大字段及Unicode/数值/深度/尾随数据错误。
同时验证Chat、Responses、Structured SSE的provider证据、内容/usage/工具保留、普通SSE负例和截断HTTP响应。
`--bin-dir`可指定冻结的release对照；`--verify-existing`重新核验已完成真实网络运行的provider证据。
Responses目前没有reasoning提取，Structured SSE没有工具提取；相应真实agent端到端能力尚未覆盖。

`python3 -m scripts.bench.payload.validation.http2 --out local/bench/http2`
使用真实Node HTTP/2客户端和TLS ALPN服务端，在一条连接中并发请求，交错发送JSON/SSE响应。
核验P/C按需保留、真实stream ID和call关系、普通SSE负例，以及同次TLS写入DATA+RST和收到PING确认后RST的部分响应。
服务端保存实际收到的请求长度与流编号，客户端核对全部响应字节和RST码；重置只结束对应流。
重置用例预期产生HTTP2流重置诊断；原始诊断、动作、关系及两批实际发送的frame字节均保存到输出目录。
该专项是实际网络协议验收，真实agent验收另由local MaaS benchmark执行。

`python3 -m scripts.bench.payload.validation.failures --out local/bench/provider-failures`
通过完整HTTP 200报文验证Responses failed/incomplete/error、Anthropic和Chat错误，覆盖无正文失败及失败后的完成标记。
同时验证有/无LLM请求关联的通用error负例。每例包含一次写入和逐行TLS写入两档，正式动作中的operation IDs核验实际多次读取；间隔由`--event-pause-seconds`控制。
已观察provider终态时`llm.response.done=true`；完整采集的失败响应为error/partial。未收到provider终态的传输截断与RST仍为done=false；限采继续使用现有capture_limited完整性规则。
