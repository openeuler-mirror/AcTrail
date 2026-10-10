# 访问控制的代码归属与抽象边界

## 目标

按控制能力组织策略，按执行机制组织后端，使配置、代码位置和运行职责保持一致。文件打开、目录操作、命令执行和网络连接分别选择实现；观测按自身配置选择采集机制。

本文说明按操作选择后端的配置与运行边界，以及后端代码归属的目标布局。配置模型已在代码中实现；目标目录布局用于后续职责迁移。

## 现有职责分布

| 位置 | 职责 |
| --- | --- |
| `crates/access_control/src/policy/file/` | 文件规则、授权、优先级、owner 与 revision |
| `crates/access_control/src/policy/command/` | 命令规则与匹配语义 |
| `crates/access_control/src/policy/network/` | 网络端点规则与匹配语义 |
| `crates/access_control/src/lsm/` | LSM 策略编译、map 发布、运行时和插件 policy host |
| `crates/access_control/bpf/` | LSM hooks、控制范围、内核规则匹配、信号与审计 |
| `crates/apps/daemon/src/services/enforcement/` | 唯一文件策略 store、文件 policy host、fanotify 打开控制与 seccomp 目录控制，以及 LSM 文件快照发布对接 |
| `crates/apps/daemon/src/services/command_control/` | 原有 seccomp 命令控制执行路径 |
| `crates/apps/daemon/src/services/network_control/` | 原有 seccomp 网络控制执行路径 |

独立 `access_control` crate 已包含共享策略和 LSM 后端。原有执行后端仍分布在 daemon 的服务目录中。

## 命名范围与代码归属问题

### LSM 实现使用了整个模块的名称

`crates/access_control/bpf/access_control.bpf.c` 实际实现 LSM 后端。文件名表达的是整个访问控制模块，父目录也没有标明 LSM，容易让读者误判其他后端的归属。

LSM 的 BPF 程序、专用 ABI、控制范围和策略匹配头文件应归入明确的 `bpf/lsm/` 目录。入口文件命名为 `lsm.bpf.c`，Rust 侧编译器、publisher、runtime 和这些内核代码共同构成一个后端。

### 执行后端的组织不对称

LSM 在独立 crate 内具有完整的编译与执行结构，seccomp 和 fanotify 的控制逻辑仍放在 daemon。阅读者需要跨多个服务目录才能理解访问控制的完整实现，后端选择也容易散落在装配代码中。

控制专属的通知处理、规则执行和响应逻辑应归入对应后端。迁移时按职责确认边界：进程启动、共享 seccomp 通道、观测采集、插件运行时和持久化基础设施由各自模块管理，通过窄接口供控制后端使用。

## 抽象边界问题

### 配置按实际执行机制表达能力

`access_control.file.open`、`file.mkdir`、`file.rmdir`、`command.exec` 和 `network.connect` 分别声明 `enabled` 与 `backend`。文件打开可选 fanotify 或 LSM；其余四项可选 seccomp 或 LSM。

fanotify 文件打开与 seccomp 命令、目录和网络控制分别装配。文件打开使用 fanotify、命令和网络使用 LSM 时，只启动配置需要的执行资源。全局 `access_control.backend`、`enforcement.backend` 和 `enforcement.seccomp_syscalls` 已移除；旧字段会在配置解析时被拒绝。

文件观测与控制分别按自身配置选择机制。配置中的 seccomp 仅表示相应操作的 seccomp user notification 执行路径。

### 文件控制需要明确操作范围

| 控制能力 | 当前已有实现基础 | 覆盖说明 |
| --- | --- | --- |
| 文件打开 | fanotify / LSM | 当前 LSM 使用 `file_open` |
| 创建目录 | seccomp / LSM | 当前 LSM 使用 `path_mkdir` |
| 删除目录 | seccomp / LSM | 当前 LSM 使用 `path_rmdir` |
| 命令执行 | seccomp / LSM | 当前 LSM 按可执行路径裁决；具体 argv 条件需要额外能力 |
| 网络连接 | seccomp / LSM | 当前 LSM 覆盖 IP `connect` |

各项能力独立选择执行后端。文件删除、重命名、已打开 FD 的逐次读写控制需要各自的能力定义。拒绝后续打开不会自动撤销已有 FD 的使用权限。

## 目标目录布局

```text
crates/access_control/
  build.rs
  bpf/
    lsm/
      lsm.bpf.c                 # LSM hooks 入口
      abi.h                     # LSM 专属内核/用户态 ABI
      scope.h                   # 控制范围与任务继承
      policy.h                  # 内核策略查询与裁决
      directory.h               # 目录路径解析
  src/
    lib.rs
    policy/
      file/                     # 按能力组织的共享规则语义
      command/
      network/
    hosts/                      # WASM policy host 授权与发布入口
    backends/
      lsm/                      # compiler、publisher、runtime、audit
      seccomp/                  # 控制通知、参数读取、裁决与响应
      fanotify/                 # permission events、marks、裁决与响应

crates/core/config/src/daemon/access_control/
  ...                           # 能力选择和各后端子配置

crates/apps/daemon/src/services/access_control/
  ...                           # 装配、生命周期协调和审计对接
```

目录树表达职责边界，模块按实际规模拆分。Rust 行为聚合到具备明确生命周期的结构体中，模块导出遵循最小可见性原则。

## 各层职责与状态归属

### 共享策略与 WASM 接口

共享策略层统一规则语义、授权范围、owner、优先级和 revision。WASM 按控制能力提交声明式规则，host 根据能力配置将规则变更交给对应后端。

文件服务持有唯一文件 store，文件 policy host 和 owner 撤销均经过该服务。命令与网络的权威 store 由各自选中的执行路径持有。

LSM controller 缓存从文件 store 编译出的 entries，供命令或网络发布时合并，避免重新编译文件规则。文件规则成功发布时替换这些 entries；它们与内核 map 均为派生快照。文件全部使用 fanotify/seccomp 时，文件更新直接作用于文件服务。

混合文件更新先准备新增 fanotify marks，再编译、发布 LSM 快照，最后替换文件 store 并清理旧 marks。准备或发布失败保留旧规则与旧快照，并清理本次新增 marks。该发布边界保证更新返回成功后各执行路径使用新策略；跨机制访问不具有同时切换的承诺。

后端能力校验覆盖具体操作和规则条件。例如，选择当前 LSM 命令执行实现时，发布包含具体 argv 条件的规则应明确失败，并保留已生效规则。

### 执行后端

- LSM：编译快照、创建并冻结 map、原子发布、管理 links/maps，在内核直接裁决。
- seccomp：处理控制所需的 syscall 通知、读取参数、执行策略并发送响应。
- fanotify：维护控制所需的 marks、处理权限事件、执行策略并发送响应。

统一接口覆盖配置校验、规则生效和生命周期等共同职责。后端内部保留符合自身机制的执行路径。LSM 访问热路径保持内核查询与裁决，审计异步消费。

### daemon 装配与观测

daemon 根据配置构造策略域和后端，连接启动协调、插件 host、身份服务与审计存储。具体访问规则匹配归入访问控制模块。

观测按自身配置选择 bpf-copy 等机制。fanotify 用于权限裁决或文件观测时，需要分别声明用途和生命周期。同一进程中共享的机制资源通过所属模块接口管理。

## 配置模型

以下配置使用 fanotify 控制文件打开，使用 LSM 控制目录、命令和网络：

```toml
[access_control.file.open]
# backend options are: fanotify/lsm
enabled = true
backend = "fanotify"

[access_control.file.mkdir]
# backend options are: seccomp/lsm
enabled = true
backend = "lsm"

[access_control.file.rmdir]
# backend options are: seccomp/lsm
enabled = true
backend = "lsm"

[access_control.command.exec]
# backend options are: seccomp/lsm
enabled = true
backend = "lsm"

[access_control.network.connect]
# backend options are: seccomp/lsm
enabled = true
backend = "lsm"

[access_control.backends.lsm]
policy_max_entries = 4096
```

五项操作的默认后端均为 LSM。默认启用文件打开控制；`mkdir/rmdir` 操作及命令、网络控制域默认关闭。`actrailctl init` 与 `init --mode complete` 继承这些设置；`profile` 关闭控制域并移除相应能力声明。Complete 的 socket seccomp 通知用于完整正文观测，控制仍由 LSM 执行。

`enforcement.enabled`、`command_control.enabled`、`network_control.enabled` 是各域总开关；相应操作还需自身 `enabled=true` 才运行。原始分层配置保留用于导出，装配前派生一次有效操作配置。规则路径、默认裁决和审计参数继续位于三个域配置中。LSM 容量、路径和故障参数位于 `access_control.backends.lsm`。

`capture.capabilities` 声明实际使用的机制：fanotify open 对应 `enforcement-file-permission-fanotify`；seccomp 目录分别对应 `enforcement-file-mkdir-seccomp` 和 `enforcement-file-rmdir-seccomp`；LSM 文件操作聚合为 `enforcement-file-permission-lsm`。命令与网络使用各自 `...-seccomp` 或 `...-lsm` 能力。混合文件配置可以同时声明 fanotify、目录 seccomp 与文件 LSM 能力。

LSM 和目录 seccomp 控制要求必选 `proc-lifecycle` 与相应控制能力。选择 seccomp 目录操作还需启用 `seccomp_notify`；启动过滤器根据配置安装，因此空规则启动后仍可动态发布目录规则。

每项能力默认选择一个裁决后端。多个后端共同裁决同一操作时，需要明确的组合语义和独立配置，避免重复拦截及允许/拒绝结果歧义。

## 状态与故障约束

- 启动时检查操作覆盖、规则条件、内核环境和所需能力；配置无法兑现时明确失败。
- 更新规则时保留权威 store 和生效快照的对应关系；失败不得静默清空已有策略。
- 当前 LSM 的单次 host 更新采用完整快照原子替换，访问使用取得的快照完成裁决。
- 多个域连续更新、多个执行机制共同接收更新时，需要明确提交边界；单个 LSM map 的原子性不自动扩展为跨后端原子性。
- 插件卸载撤销其 owner 规则，并使相关执行后端生效。
- 控制范围、PID 身份、进程继承和生命周期使用明确的权威来源，避免重复维护关联状态。
- 运行中审计和存储故障局部处理；逐访问裁决保持各后端既定的故障语义。

## 验收要求

使用刷新的默认配置运行真实 agent，分别验证全 LSM、现有 seccomp/fanotify 组合，以及文件打开 fanotify 配合命令/网络 LSM 的混合组合。

验证操作效果、动态规则发布、拒绝更新后的既有策略、owner 卸载、后代进程控制范围及实际启动的后端资源。全 LSM 配置应验证 `Seccomp=0` 和无 fanotify 权限控制实例；混合配置应验证各能力由所选后端执行。

性能对照保持规则、观测和审计设置一致，分别记录规则发布成本与访问成本。结构调整后的 LSM 热路径需继续满足内核直接裁决的目标。

## 参考

- [现有 LSM 设计与实现说明](../lsm_access_control/design.md)
- [WASM 动态策略性能实验](../lsm_access_control/wasm-performance.md)
- [Linux BPF LSM 文档](https://docs.kernel.org/bpf/prog_lsm.html)
- [fanotify 接口文档](https://man7.org/linux/man-pages/man7/fanotify.7.html)
- [seccomp 接口文档](https://kernel.org/doc/html/latest/userspace-api/seccomp_filter.html)
