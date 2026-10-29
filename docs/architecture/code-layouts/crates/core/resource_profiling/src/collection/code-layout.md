状态标记：`[ ]` 待实现，`[/]` 修改中，`[x]` 已完成，`[-]` 待删除或迁移。

```text
[x] crates/core/resource_profiling/src/collection/ # 资源采集子模块；拥有采集契约、Host/Workload 后端、采样时钟与采集生命周期。
├── [x] config/ # 组合宿主与工作负载采集配置。
│   ├── [x] mod.rs # 声明并导出采集配置。
│   └── [x] resource_collection_config.rs # 将 Host/Workload 子表交给各自模块解析。
│       ├── [x] struct RawConfig # 接收 Host/Workload 原始配置子表。
│       └── [x] struct ResourceCollectionConfig # 组合独立 Host 与 Workload 采集配置。
├── [x] contracts/ # 定义采样上下文、后端接口及组件内错误。
│   ├── [x] mod.rs # 声明模块并 re-export。
│   ├── [x] profiling_error.rs # 组件错误，不依赖 daemon 控制协议。
│   │   └── [x] struct ProfilingError # 组件错误，不依赖 daemon 控制协议。
│   ├── [x] resource_profiling_backend.rs # 对已绑定目标执行采样；关联类型声明输入和输出。
│   │   └── [x] trait ResourceProfilingBackend # 对已绑定目标执行采样；关联类型声明输入和输出。
│   └── [x] sampling_context.rs # 单调采样时间与事件墙钟时间。
│       └── [x] struct SamplingContext # 单调采样时间与事件墙钟时间。
├── [x] host/ # 调度整机资源采集，拥有宿主后端与 CPU 差分基线。
│   ├── [x] config/ # 配置宿主采集开关、周期及后端选择。
│   │   ├── [x] host_backend_mode.rs # linux、noop。
│   │   │   └── [x] enum HostBackendMode # linux、noop。
│   │   ├── [x] host_profiling_config.rs # 宿主开关、周期、后端枚举、默认值和校验。
│   │   │   ├── [x] struct HostProfilingConfig # 宿主开关、周期、后端枚举、默认值和校验。
│   │   │   └── [x] struct RawConfig # 接收本模块配置的反序列化输入。
│   │   └── [x] mod.rs # 声明模块并 re-export。
│   ├── [x] host_backend_factory.rs # 按 enum 构造 linux/noop；仅 Linux 分支读取 boot identity。
│   │   └── [x] struct HostBackendFactory # 按 enum 构造 linux/noop；仅 Linux 分支读取 boot identity。
│   ├── [x] host_profiler.rs # 持有 Box<dyn HostResourceProfilingBackend>，只管理周期和调度。
│   │   └── [x] struct HostProfiler # 持有 Box<dyn HostResourceProfilingBackend>，只管理周期和调度。
│   ├── [x] host_resource_profiling_backend.rs # 约束宿主输入和宿主样本。
│   │   └── [x] trait HostResourceProfilingBackend # 约束宿主输入和宿主样本。
│   ├── [x] linux/ # 从 Linux procfs 读取宿主 CPU、内存与负载。
│   │   ├── [x] host_counter_reader.rs # 读取 /proc/stat、meminfo 和 loadavg。
│   │   │   ├── [x] struct HostCounterReader # 读取 /proc/stat、meminfo 和 loadavg。
│   │   │   └── [x] struct HostCounters # 宿主 CPU 累计计数、内存与负载读数。
│   │   ├── [x] linux_host_resource_profiling_backend.rs # 真实 boot identity 和整机 CPU 差分基线。
│   │   │   └── [x] struct LinuxHostResourceProfilingBackend # 真实 boot identity 和整机 CPU 差分基线。
│   │   └── [x] mod.rs # 声明模块并 re-export。
│   ├── [x] mod.rs # 声明模块并 re-export。
│   └── [x] noop/ # 提供不访问内核的空宿主采样实现。
│       ├── [x] mod.rs # 声明模块并 re-export。
│       └── [x] noop_host_resource_profiling_backend.rs # 通过宿主采样接口返回空样本，无采集 I/O。
│           └── [x] struct NoOpHostResourceProfilingBackend # 通过宿主采样接口返回空样本，无采集 I/O。
├── [x] mod.rs # 声明并导出采集接口、Host/Workload 配置与采样器。
└── [x] workload/ # 按 trace 的权威目标执行资源准入、采样和最终化。
    ├── [x] assembly/ # 根据配置装配 procfs、cgroup、noop 及自动候选链。
    │   ├── [x] mod.rs # 声明模块并 re-export。
    │   ├── [x] workload_backend.rs # 装配后端族；cgroup 版本选择委托 CgroupBackendFactory。
    │   │   ├── [x] enum SelectedBackend # procfs、cgroup、auto 候选链、noop 的具体装配。
    │   │   └── [x] struct WorkloadBackend # 装配后端族；cgroup 版本选择委托 CgroupBackendFactory。
    │   └── [x] workload_backend_factory.rs # 按 mode 装配 procfs/cgroup/noop；auto 组合 cgroup 与 procfs 候选链。
    │       └── [x] struct WorkloadBackendFactory # 按 mode 装配 procfs/cgroup/noop；auto 组合 cgroup 与 procfs 候选链。
    ├── [x] cgroup/ # interface、common contract、factory 与 implementations 分层；各版本只实现自身操作。
    │   ├── [x] cgroup_backend.rs # 版本后端共同接口与 v2/v1 枚举静态分派；版本专属绑定操作仅在 cgroup 模块内使用。
    │   │   ├── [x] trait CgroupBackend # 两版本共同的准入、采样、托管收尾与屏障操作。
    │   │   └── [x] enum VersionBackend # 静态分派到 v2/v1 实现。
    │   ├── [x] cgroup_backend_factory.rs # factory：按配置构造版本后端；有 live 托管范围时优先选其版本恢复，否则按 v2→v1 尝试。
    │   │   └── [x] struct CgroupBackendFactory # 构造 CgroupSamplingBackend；无法安全恢复 live 范围或 cgroup 模式下无可用路径时报错。
    │   ├── [x] cgroup_persistence.rs # 组合 managed、v1、v2 持久化端口，校验选定版本能恢复已有绑定。
    │   │   └── [x] trait CgroupPersistence # 组合 managed、v1、v2 持久化端口，校验选定版本能恢复已有绑定。
    │   ├── [x] cgroup_resource_profiling_backend.rs # cgroup 后端族能力；版本私有计数与绑定结构留在所属后端。
    │   │   └── [x] trait CgroupResourceProfilingBackend # cgroup 后端族能力；版本私有计数与绑定结构留在所属后端。
    │   ├── [x] cgroup_sampling_backend.rs # 门面：依次执行托管、v2 容器外部绑定、v1 现有 cgroup 绑定采样；统一终止屏障与回退原因。
    │   │   └── [x] struct CgroupSamplingBackend # 仅持有版本后端，不向服务层暴露 v1/v2 实现对象。
    │   ├── [x] common/ # common contract：与版本无关的自建范围生命周期与 cgroup 计数采样。
    │   │   ├── [x] availability.rs # 区分能力缺失与必须报错的故障，映射 cgroup 错误。
    │   │   │   └── [x] struct CgroupAvailability # 区分能力缺失与必须报错的故障。
    │   │   ├── [x] managed/ # 自建 trace 范围：准入、采样、最终化与重启恢复只实现一次。
    │   │   │   ├── [x] completion.rs # managed 最终样本与范围清理结果。
    │   │   │   │   ├── [x] struct ResourceFinalizationDraft # managed 最终样本及范围清理超时结果。
    │   │   │   │   └── [x] struct ResourceFinalizationPoll # 待清理范围与 managed 最终样本批次。
    │   │   │   ├── [x] hierarchy.rs # 各 cgroup 版本向生命周期提供的文件系统操作与样本口径。
    │   │   │   │   └── [x] trait ManagedHierarchy # 准备层级、建/移/核验/扫描/删除范围、计数读取器及版本元数据。
    │   │   │   ├── [x] manager.rs # 创建、激活、恢复和清理自建范围，拥有范围及计数句柄。
    │   │   │   │   ├── [x] enum ManagedScopeAdmission # 托管绑定或允许的接入降级结果。
    │   │   │   │   └── [x] struct ManagedScopeManager # 创建、激活、恢复和清理自建范围，拥有范围及计数句柄。
    │   │   │   ├── [x] mod.rs # 声明模块并 re-export。
    │   │   │   ├── [x] payload.rs # cgroup 计数到资源事件字段映射（managed 与 external 共用）。
    │   │   │   ├── [x] persistence.rs # 准备恢复记录、持久化丢失范围的最终状态。
    │   │   │   │   ├── [x] struct ManagedRecoveryRecord # 已持久化范围及权威根身份。
    │   │   │   │   └── [x] trait ManagedScopePersistence # 准备恢复记录、持久化丢失范围的最终状态。
    │   │   │   ├── [x] recovery.rs # 对照所属版本的范围登记与内核目录，分类恢复和清理候选。
    │   │   │   │   └── [x] struct ResourceScopeRecovery # 对照范围登记与内核目录，分类恢复和清理候选。
    │   │   │   └── [x] runtime.rs # 按版本恢复记录初始化共同生命周期，并执行周期采样、最终化轮询、超时与完成屏障。
    │   │   │       └── [x] struct ManagedScopeRuntime # 由各版本后端分别持有的共同托管生命周期。
    │   │   ├── [x] mod.rs # 声明模块并 re-export。
    │   │   └── [x] sampler/ # 把单个 cgroup 的计数转换为工作负载样本，供 managed 与 external 共用。
    │   │       ├── [x] cgroup_counter_source.rs # 读取同一目录对象的计数和子树 PID。
    │   │       │   └── [x] trait CgroupCounterSource # 读取同一目录对象的计数和子树 PID；各版本在自身目录实现。
    │   │       ├── [x] cgroup_scope_sampler.rs # 持有来源句柄、CPU 基线和独立 RSS 读取器。
    │   │       │   └── [x] struct CgroupScopeSampler # 持有来源句柄、CPU 基线和独立 RSS 读取器；managed v1/v2 与 external 共用。
    │   │       ├── [x] cgroup_workload_sample.rs # 主计数、CPU 差分和可选 RSS 的独立有效性。
    │   │       │   └── [x] struct CgroupWorkloadSample # 主计数、CPU 差分和可选 RSS 的独立有效性。
    │   │       ├── [x] mod.rs # 声明模块并 re-export。
    │   │       ├── [x] process_rss/ # 仅按绑定 cgroup 子树读取 RSS，核验 PID 代际和成员范围；含 RSS 开关配置。
    │   │       └── [x] sampler_factory.rs # 向已绑定来源注入 RSS 读取配置并构造采样后端。
    │   │           └── [x] struct CgroupSamplerFactory # 向已绑定来源注入 RSS 读取配置并构造采样后端。
    │   ├── [x] config/
    │   │   ├── [x] cgroup_backend_mode.rs # auto、v1、v2。
    │   │   │   └── [x] enum CgroupBackendMode # auto、v1、v2。
    │   │   ├── [x] cgroup_profiling_config.rs # mode 及独立 v1/v2 子表，委托候选分支解析与校验。
    │   │   │   ├── [x] struct CgroupProfilingConfig # mode 及独立 v1/v2 子表，委托候选分支解析与校验。
    │   │   │   └── [x] struct RawConfig # 接收本模块配置的反序列化输入。
    │   │   └── [x] mod.rs # 声明模块并 re-export。
    │   ├── [x] implementations/ # 各 cgroup 版本自己的实现。
    │   │   ├── [x] hybrid/ # 跨 cgroup v1/v2 的资源采集（占位，未编译）。
    │   │   │   └── [x] hybrid_backend.rs # TODO：组合 v1/v2 指标能力的 Hybrid 采集后端。
    │   │   ├── [x] mod.rs # 声明 v1、v2 模块。
    │   │   ├── [x] v1/ # v1 自建范围层级，以及只读绑定目标进程已有的 cpuacct/memory 分组；两个 controller 边界独立保存。
    │   │   │   ├── [x] backend.rs # 组合 v1 托管生命周期与 v1 绑定，实现 CgroupBackend。
    │   │   │   │   └── [x] struct CgroupV1SamplingBackend # v1 版本后端。
    │   │   │   ├── [x] binding/ # v1 只读绑定：进程当前所在的 cpuacct/memory 分组。
    │   │   │   │   ├── [x] handle/ # 发现、固定并重开 v1 双 controller 目录，核验进程代际与成员关系。
    │   │   │   │   │   ├── [x] directory.rs # 固定目录句柄，核验文件系统类型，以受限相对路径读取计数。
    │   │   │   │   │   │   └── [x] struct V1Directory # 固定目录句柄，核验文件系统类型，以受限相对路径读取计数。
    │   │   │   │   │   ├── [x] discovery.rs # 按 mountinfo 发现 controller 层级并解析目标成员路径。
    │   │   │   │   │   │   └── [x] struct V1Discovery # 按 mountinfo 发现 controller 层级并解析目标成员路径。
    │   │   │   │   │   ├── [x] handle.rs # 绑定和重开双 controller 边界，核验进程代际、成员与目录身份。
    │   │   │   │   │   │   └── [x] struct CgroupV1BindingHandle # 按记录打开的双 controller 目录句柄。
    │   │   │   │   │   └── [x] mod.rs # 声明模块并 re-export。
    │   │   │   │   ├── [x] manager.rs # 本机全部 v1 绑定的恢复、接入、逐目标采样、最终样本输出与关闭。
    │   │   │   │   │   ├── [x] struct CgroupV1BindingManager # 本机全部 v1 绑定的恢复、接入、逐目标采样、最终样本输出与关闭。
    │   │   │   │   │   └── [x] struct CgroupV1BindingSession # 单个 trace 的绑定记录、采样器、准入状态和等待确认的失效状态。
    │   │   │   │   ├── [x] persistence.rs # v1 绑定记录的恢复、持久化失效与恢复 trace 收尾。
    │   │   │   │   │   ├── [x] struct CgroupV1BindingRecovery # 绑定、权威根身份与终止状态。
    │   │   │   │   │   └── [x] trait CgroupV1BindingPersistence # 恢复绑定、持久化失效与恢复 trace 收尾。
    │   │   │   │   ├── [x] sampler.rs # v1 绑定计数、CPU 差分基线与样本字段映射。
    │   │   │   │   │   └── [x] struct CgroupV1BindingSampler # v1 绑定计数、CPU 差分基线与样本字段映射。
    │   │   │   │   └── [x] mod.rs # 声明模块并 re-export。
    │   │   │   ├── [x] config/
    │   │   │   │   └── [x] cgroup_v1_config.rs # 绑定尝试次数、层级根接入许可、自建范围子表、默认值与校验。
    │   │   │   │       └── [x] struct CgroupV1Config # 绑定尝试次数、层级根接入许可、自建范围子表、默认值与校验。
    │   │   │   ├── [x] managed/ # v1 自建范围：在每个层级顶层建 `<root_name>/traces/trace-<id>-<nonce>/workload`。
    │   │   │   │   ├── [x] config/managed_v1_scope_config.rs # 开关、根目录名、扫描上限、最终化等待期限与校验。
    │   │   │   │   │   └── [x] struct ManagedV1ScopeConfig # 开关、根目录名、扫描上限、最终化等待期限与校验。
    │   │   │   │   ├── [x] hierarchy.rs # 以 v1 多层级实现 ManagedHierarchy，含 v1 计数口径元数据。
    │   │   │   │   │   └── [x] struct ManagedV1Hierarchy # 以 v1 多层级实现 ManagedHierarchy，含 v1 计数口径元数据。
    │   │   │   │   └── [x] preflight.rs # 发现 v1 层级，自动创建根目录并用一次性范围验证可用。
    │   │   │   │       └── [x] struct ManagedV1Preflight # 发现 v1 层级，自动创建根目录并用一次性范围验证可用。
    │   │   │   └── [x] mod.rs # 声明模块并 re-export。
    │   │   └── [x] v2/ # v2 自建范围层级，以及只读绑定容器运行时自有的 cgroup。
    │   │       ├── [x] binding/ # v2 只读绑定：容器的 cgroup；接入、恢复、核验、失效和最终采样。
    │   │       │   ├── [x] config/binding_config.rs # 接入策略、失败阈值、绑定重试上限及校验（TOML 键仍为 `cgroup.v2.external`）。
    │   │       │   │   ├── [x] enum CgroupV2BindingPolicy # disabled、prefer、require。
    │   │       │   │   └── [x] struct CgroupV2BindingConfig # 接入策略、失败阈值、绑定重试上限及校验。
    │   │       │   ├── [x] handle/ # 解析容器范围并固定目录身份，防止成员或路径变化造成误绑定。
    │   │       │   │   ├── [x] container_cgroup_identity.rs # 解析完整容器身份及 host cgroup 边界。
    │   │       │   │   ├── [x] container_cgroup_resolver.rs # 使用 openat2 在已知层级下绑定或重开目录。
    │   │       │   │   ├── [x] error.rs # 身份、成员变动和目录访问错误。
    │   │       │   │   │   └── [x] enum CgroupV2BindingError # 身份、成员变动和目录访问错误。
    │   │       │   │   ├── [x] handle.rs # 绑定目录句柄和成员核验能力。
    │   │       │   │   │   ├── [x] struct CgroupDirectoryIdentity # 目录 device/inode 身份。
    │   │       │   │   │   └── [x] struct CgroupV2BindingHandle # 按记录打开的容器 cgroup 目录句柄。
    │   │       │   │   ├── [x] host_binding_environment.rs # 真实 boot identity 和绑定错误分类。
    │   │       │   │   ├── [x] process_verifier.rs # 绑定前后验证期望的进程代际。
    │   │       │   │   └── [x] tests.rs # 绑定实现的身份与目录绑定测试。
    │   │       │   ├── [x] manager.rs # 本机全部 v2 绑定的恢复、接入、核验、失效和最终采样。
    │   │       │   │   ├── [x] enum CgroupV2BindingAdmission # 绑定成功或允许的接入降级结果。
    │   │       │   │   ├── [x] enum CgroupV2BindingRead # 计数、失效或可重试读取失败。
    │   │       │   │   ├── [x] struct CgroupV2BindingFinalRead # 持久化绑定事实、最终计数和局部读取错误。
    │   │       │   │   ├── [x] struct CgroupV2BindingSession # 单个 trace 的绑定记录与采样器。
    │   │       │   │   └── [x] struct CgroupV2BindingManager # 本机全部 v2 绑定的恢复、接入、核验、失效和最终采样。
    │   │       │   ├── [x] persistence.rs # 恢复、成功/失败计数、失效和恢复 trace 收尾。
    │   │       │   │   ├── [x] struct CgroupV2BindingRecovery # 持久化绑定、根身份和终止状态。
    │   │       │   │   └── [x] trait CgroupV2BindingPersistence # 恢复、成功/失败计数、失效和恢复 trace 收尾。
    │   │       │   └── [x] mod.rs # 声明模块并 re-export。
    │   │       ├── [x] config/cgroup_v2_config.rs # 委托 managed/external（绑定）/process_rss 各自解析。
    │   │       │   └── [x] struct CgroupV2Config # 委托 managed/external（绑定）/process_rss 各自解析。
    │   │       ├── [x] managed/
    │   │       │   ├── [x] config/managed_v2_scope_config.rs # 根路径、扫描上限、最终化等待期限、默认值和校验。
    │   │       │   │   └── [x] struct ManagedV2ScopeConfig # 根路径、扫描上限、最终化等待期限、默认值和校验。
    │   │       │   └── [x] hierarchy.rs # 以 systemd 委托的 v2 统一层级实现 ManagedHierarchy。
    │   │       │       └── [x] struct ManagedV2Hierarchy # 以 systemd 委托的 v2 统一层级实现 ManagedHierarchy。
    │   │       ├── [x] mod.rs # 声明模块并 re-export。
    │   │       └── [x] runtime/ # v2 托管与绑定的准入、采样与最终化。
    │   │           ├── [x] backend.rs # 实现 CgroupBackend，组合 v2 托管与绑定采样。
    │   │           ├── [x] binding_sampling.rs # 绑定计数、绑定失效与局部读取失败。
    │   │           ├── [x] cgroup_v2_sampling_backend.rs # 持有 v2 托管生命周期、绑定管理器、回退原因与诊断状态。
    │   │           │   └── [x] struct CgroupV2SamplingBackend # v2 版本后端。
    │   │           ├── [x] completion.rs # 绑定最终样本与单次采样结果。
    │   │           │   ├── [x] enum CgroupV2BindingSampleOutcome # 已采样、绑定失效或局部读取失败。
    │   │           │   └── [x] struct CgroupV2BindingFinalization # 绑定最终样本。
    │   │           ├── [x] lifecycle.rs # 绑定接入、最终样本输出与屏障。
    │   │           ├── [x] payload.rs # 绑定最终样本字段映射。
    │   │           └── [x] sampling.rs # 实现批量采样接口，处理绑定及恢复目标，逐目标隔离失败。
    │   └── [x] mod.rs # 声明模块并 re-export。
    │   ├── [ ] hybrid/ # 跨 cgroup v1/v2 的资源采集。
    │   │   └── [ ] hybrid_backend.rs # TODO：组合 v1/v2 指标能力的 Hybrid 采集后端。
    │   ├── [x] mod.rs # 声明模块并 re-export。
    │   ├── [x] v1/ # 只读绑定已有 cpuacct/memory 分组；两个 controller 边界独立保存。
    │   │   ├── [x] binding/ # 发现并固定 v1 双 controller 目录，核验进程代际与成员关系。
    │   │   │   ├── [x] directory.rs # 固定目录句柄，核验文件系统类型，以受限相对路径读取计数。
    │   │   │   │   ├── [x] struct OpenHow # openat2 的打开标志与路径解析约束。
    │   │   │   │   └── [x] struct V1Directory # 固定目录句柄，核验文件系统类型，以受限相对路径读取计数。
    │   │   │   ├── [x] discovery.rs # 按 mountinfo 发现 controller 层级并解析目标成员路径。
    │   │   │   │   ├── [x] struct V1Discovery # 按 mountinfo 发现 controller 层级并解析目标成员路径。
    │   │   │   │   └── [x] struct V1Mount # controller 挂载点与可见层级根路径。
    │   │   │   ├── [x] mod.rs # 声明模块并 re-export。
    │   │   │   └── [x] handle.rs # 绑定和重开双 controller 边界，核验进程代际、成员与目录身份。
    │   │   │       └── [x] struct CgroupV1BindingHandle # 绑定和重开双 controller 边界，核验进程代际、成员与目录身份。
    │   │   ├── [x] sampler.rs # v1 计数、CPU 差分基线与样本字段映射。
    │   │   │   └── [x] struct CgroupV1BindingSampler # v1 计数、CPU 差分基线与样本字段映射。
    │   │   ├── [x] config/ # 配置 v1 绑定重试与层级根接入许可。
    │   │   │   ├── [x] cgroup_v1_config.rs # 绑定尝试次数、层级根接入许可、默认值与校验。
    │   │   │   │   └── [x] struct CgroupV1Config # 绑定尝试次数、层级根接入许可、默认值与校验。
    │   │   │   └── [x] mod.rs # 声明模块并 re-export。
    │   │   ├── [x] mod.rs # 声明模块并 re-export。
    │   │   ├── [x] persistence.rs # v1 只读绑定：恢复、持久化失效与恢复 trace 收尾。
    │   │   │   ├── [x] struct CgroupV1BindingRecovery # 绑定、权威根身份与终止状态。
    │   │   │   └── [x] trait CgroupV1BindingPersistence # 恢复绑定、持久化失效与恢复 trace 收尾。
    │   │   └── [x] runtime.rs # 接入、恢复、逐目标采样、最终样本输出与绑定关闭。
    │   │       ├── [x] struct CgroupV1BindingManager # 接入、恢复、逐目标采样、最终样本输出与绑定关闭。
    │   │       └── [x] struct CgroupV1BindingSession # 绑定句柄、准入状态和等待确认的失效状态。
    │   └── [x] v2/ # v2 配置、采样、准入、恢复与最终化的完整边界。
    │       ├── [x] cgroup_counter_source.rs # 读取同一目录对象的计数和子树 PID。
    │       │   └── [x] trait CgroupCounterSource # 读取同一目录对象的计数和子树 PID。
    │       ├── [x] cgroup_v2_backend.rs # 持有来源句柄、CPU 基线和独立 RSS 读取器。
    │       │   └── [x] struct CgroupV2Backend # 持有来源句柄、CPU 基线和独立 RSS 读取器。
    │       ├── [x] cgroup_v2_backend_factory.rs # 消费 v2 配置，装配 v2 managed/external 组件。
    │       │   ├── [x] struct CgroupAvailability # 区分能力缺失和必须报错的配置、结构故障。
    │       │   ├── [x] struct CgroupSamplerFactory # 向已绑定来源注入 RSS 读取配置并构造采样后端。
    │       │   └── [x] struct CgroupV2BackendFactory # 消费 v2 配置，装配 v2 managed/external 组件。
    │       ├── [x] cgroup_workload_sample.rs # 主计数、CPU 差分和可选 RSS 的独立有效性。
    │       │   └── [x] struct CgroupWorkloadSample # 主计数、CPU 差分和可选 RSS 的独立有效性。
    │       ├── [x] config/ # 组合 v2 受管范围、外部范围及可选 RSS 配置。
    │       │   ├── [x] cgroup_v2_config.rs # 委托 managed/external/process_rss 各自解析。
    │       │   │   ├── [x] struct CgroupV2Config # 委托 managed/external/process_rss 各自解析。
    │       │   │   └── [x] struct RawConfig # 接收本模块配置的反序列化输入。
    │       │   └── [x] mod.rs # 声明模块并 re-export。
    │       ├── [x] external/ # 只读绑定已有容器 cgroup，维护核验、失效与最终采样。
    │       │   ├── [x] binding/ # 解析容器范围并固定目录身份，防止成员或路径变化造成误绑定。
    │       │   │   ├── [x] container_cgroup_identity.rs # 解析完整容器身份及 host cgroup 边界。
    │       │   │   │   └── [x] struct ContainerCgroupIdentity # 解析完整容器身份及 host cgroup 边界。
    │       │   │   ├── [x] container_cgroup_resolver.rs # 使用 openat2 在已知层级下绑定或重开目录。
    │       │   │   │   ├── [x] struct ContainerCgroupResolver # 使用 openat2 在已知层级下绑定或重开目录。
    │       │   │   │   └── [x] struct OpenHow # openat2 系统调用参数。
    │       │   │   ├── [x] error.rs # 身份、成员变动和目录访问错误。
    │       │   │   │   └── [x] enum CgroupV2BindingError # 身份、成员变动和目录访问错误。
    │       │   │   ├── [x] handle.rs # 绑定目录句柄和成员核验能力。
    │       │   │   │   ├── [x] struct CgroupDirectoryIdentity # 目录 device/inode 身份。
    │       │   │   │   └── [x] struct CgroupV2BindingHandle # 绑定目录句柄和成员核验能力。
    │       │   │   ├── [x] host_binding_environment.rs # 真实 boot identity 和绑定错误分类。
    │       │   │   │   └── [x] struct HostBindingEnvironment # 真实 boot identity 和绑定错误分类。
    │       │   │   ├── [x] mod.rs # 声明模块并 re-export。
    │       │   │   ├── [x] process_verifier.rs # 绑定前后验证期望的进程代际。
    │       │   │   │   └── [x] struct ProcfsProcessVerifier # 绑定前后验证期望的进程代际。
    │       │   │   └── [x] tests.rs # 绑定实现的身份与目录绑定测试。
    │       │   ├── [x] config/ # 定义外部范围接入策略、重试和局部失败阈值。
    │       │   │   ├── [x] binding_config.rs # disabled、prefer、require。
    │       │   │   │   ├── [x] enum CgroupV2BindingPolicy # disabled、prefer、require。
    │       │   │   │   └── [x] struct CgroupV2BindingConfig # 接入策略、失败阈值、绑定重试上限及校验。
    │       │   │   └── [x] mod.rs # 声明模块并 re-export。
    │       │   ├── [x] manager.rs # 只读接入、恢复、核验、失效和最终采样。
    │       │   │   ├── [x] enum CgroupV2BindingAdmission # 绑定成功或允许的接入降级结果。
    │       │   │   ├── [x] enum CgroupV2BindingRead # 计数、失效或可重试读取失败。
    │       │   │   ├── [x] struct CgroupV2BindingSession # 准备接入的绑定与采样后端。
    │       │   │   ├── [x] struct CgroupV2BindingFinalRead # 持久化绑定事实、最终计数和局部读取错误。
    │       │   │   └── [x] struct CgroupV2BindingManager # 只读接入、恢复、核验、失效和最终采样。
    │       │   ├── [x] persistence.rs # 恢复、成功/失败计数、失效和恢复 trace 收尾。
    │       │   │   ├── [x] struct CgroupV2BindingRecovery # 持久化绑定、根身份和终止状态。
    │       │   │   └── [x] trait CgroupV2BindingPersistence # 恢复、成功/失败计数、失效和恢复 trace 收尾。
    │       │   └── [x] mod.rs # 声明模块并 re-export。
    │       ├── [x] managed/ # 创建、恢复和清理由 AcTrail 管理的 trace cgroup。
    │       │   ├── [x] config/ # 配置受管根路径、恢复扫描预算及最终化等待期限。
    │       │   │   ├── [x] managed_v2_scope_config.rs # 根路径、扫描上限、最终化等待期限、默认值和校验。
    │       │   │   │   └── [x] struct ManagedV2ScopeConfig # 根路径、扫描上限、最终化等待期限、默认值和校验。
    │       │   │   └── [x] mod.rs # 声明模块并 re-export。
    │       │   ├── [x] managed_scope_manager.rs # 创建、激活、恢复和清理自建范围，拥有范围及计数句柄。
    │       │   │   ├── [x] enum ManagedScopeAdmission # 托管绑定或允许的接入降级结果。
    │       │   │   └── [x] struct ManagedScopeManager # 创建、激活、恢复和清理自建范围，拥有范围及计数句柄。
    │       │   ├── [x] managed_scope_persistence.rs # 准备恢复记录、持久化丢失范围的最终状态。
    │       │   │   ├── [x] struct ManagedRecoveryRecord # 已持久化范围及权威根身份。
    │       │   │   └── [x] trait ManagedScopePersistence # 准备恢复记录、持久化丢失范围的最终状态。
    │       │   ├── [x] mod.rs # 声明模块并 re-export。
    │       │   └── [x] scope_recovery.rs # 对照范围登记与内核目录，分类恢复和清理候选。
    │       │       └── [x] struct ResourceScopeRecovery # 对照范围登记与内核目录，分类恢复和清理候选。
    │       ├── [x] mod.rs # 声明模块并 re-export。
    │       ├── [x] process_rss/ # 在已绑定 cgroup 子树内核验进程代际并汇总可选 RSS。
    │       │   ├── [x] cgroup_process_rss_reader.rs # 仅按绑定 cgroup 子树读取 RSS，核验 PID 代际和成员范围。
    │       │   │   ├── [x] struct CgroupProcessRss # RSS 总量、候选进程数和采样进程数。
    │       │   │   └── [x] struct CgroupProcessRssReader # 仅按绑定 cgroup 子树读取 RSS，核验 PID 代际和成员范围。
    │       │   ├── [x] config/ # 控制 cgroup 样本中的额外进程 RSS 读取。
    │       │   │   ├── [x] cgroup_process_rss_config.rs # 本分支 RSS 开关、默认值和校验。
    │       │   │   │   └── [x] struct CgroupProcessRssConfig # 本分支 RSS 开关、默认值和校验。
    │       │   │   └── [x] mod.rs # 声明模块并 re-export。
    │       │   └── [x] mod.rs # 声明模块并 re-export。
    │       └── [x] runtime/ # 编排 v2 准入、周期采样、最终样本和资源完成屏障。
    │           ├── [x] cgroup_sampling.rs # managed 计数及覆盖范围映射。
    │           ├── [x] cgroup_v2_sampling_backend.rs # 拥有 cgroup 准入、恢复、最终化状态和完成屏障。
    │           │   └── [x] struct CgroupV2SamplingBackend # 拥有 cgroup 准入、恢复、最终化状态和完成屏障。
    │           ├── [x] completion.rs # managed/external 最终样本与范围清理结果。
    │           │   ├── [x] enum CgroupV2BindingSampleOutcome # 已采样、绑定失效或局部读取失败。
    │           │   ├── [x] struct CgroupV2BindingFinalization # external 最终样本。
    │           │   ├── [x] struct ResourceFinalizationDraft # managed 最终样本及范围清理超时结果。
    │           │   └── [x] struct ResourceFinalizationPoll # 待清理范围与 managed 最终样本批次。
    │           ├── [x] binding_sampling.rs # external 计数、绑定失效与局部读取失败。
    │           ├── [x] lifecycle.rs # 准入、最终样本输出、范围清理与控制状态持久化。
    │           ├── [x] mod.rs # 声明模块并 re-export。
    │           ├── [x] payload.rs # cgroup 计数到资源事件字段映射。
    │           └── [x] sampling.rs # 实现批量采样接口，处理 managed/external 及恢复目标，逐目标隔离失败。
    ├── [x] config/ # 配置工作负载采集周期，并将后端参数交所属分支解析。
    │   ├── [x] mod.rs # 声明模块并 re-export。
    │   ├── [x] workload_backend_mode.rs # auto、procfs、cgroup、noop。
    │   │   └── [x] enum WorkloadBackendMode # auto、procfs、cgroup、noop。
    │   └── [x] workload_profiling_config.rs # 工作负载开关、周期、mode；按 mode 委托分支解析。
    │       ├── [x] struct RawConfig # 接收本模块配置的反序列化输入。
    │       └── [x] struct WorkloadProfilingConfig # 工作负载开关、周期、mode；按 mode 委托分支解析。
    ├── [x] mod.rs # 声明模块并 re-export。
    ├── [x] noop/ # 提供无资源准入和内核读取的空批量采样后端。
    │   ├── [x] mod.rs # 声明模块并 re-export。
    │   └── [x] noop_sampling_backend.rs # 通过批量采样接口返回空批次，无准入、采集和最终化资源。
    │       └── [x] struct NoOpSamplingBackend # 通过批量采样接口返回空批次，无准入、采集和最终化资源。
    ├── [x] procfs/ # 按核验后的进程坐标读取计数，维护每个 trace 的采样会话。
    │   ├── [x] config/ # 配置 procfs 后代进程采集行为。
    │   │   ├── [x] constants.rs # procfs 默认值。
    │   │   ├── [x] mod.rs # 声明模块并 re-export。
    │   │   └── [x] procfs_workload_profiling_config.rs # 本分支的后代进程采集开关。
    │   │       └── [x] struct ProcfsWorkloadProfilingConfig # 本分支的后代进程采集开关。
    │   ├── [x] mod.rs # 声明模块并 re-export。
    │   ├── [x] process_counter_reader.rs # stat/statm 读前读后代际核验及进程存活读取。
    │   │   ├── [x] struct ProcessCounterReader # stat/statm 读前读后代际核验及进程存活读取。
    │   │   ├── [x] struct ProcessCounters # CPU、RSS、虚拟内存、线程和名称。
    │   │   └── [x] struct ProcessStat # 读取器内部的 stat 解析结果。
    │   ├── [x] procfs_process_liveness.rs # 使用已核验坐标检查根进程存活。
    │   │   └── [x] struct ProcfsProcessLiveness # 使用已核验坐标检查根进程存活。
    │   ├── [x] procfs_sampling_sessions.rs # 按 trace 持有后端并释放结束的采样实例。
    │   │   └── [x] struct ProcfsSamplingSessions # 按 trace 持有后端并释放结束的采样实例。
    │   ├── [x] procfs_workload_resource_profiling_backend.rs # 拥有本分支配置和每个进程的 CPU 基线。
    │   │   ├── [x] struct CpuBaseline # 进程 CPU ticks 与单调采样时间。
    │   │   └── [x] struct ProcfsWorkloadResourceProfilingBackend # 拥有本分支配置和每个进程的 CPU 基线。
    │   ├── [x] procfs_workload_sample.rs # procfs 工作负载计数与基线有效性。
    │   │   └── [x] struct ProcfsWorkloadSample # procfs 工作负载计数与基线有效性。
    │   └── [x] sampling.rs # 实现通用批量采样接口，跳过已接管目标，输出 procfs 样本并释放结束会话。
    ├── [x] profiler/ # 提供通用采样时钟、权威目标接口及样本草稿。
    │   ├── [x] mod.rs # 声明模块并 re-export。
    │   ├── [x] workload_profiler.rs # 注入实现采样契约的 B，管理时钟与周期并透传上下文。
    │   │   └── [x] struct WorkloadProfiler # 注入实现采样契约的 B，管理时钟与周期并透传上下文。
    │   ├── [x] workload_sample.rs # 带 trace、进程身份、时间、覆盖范围的样本。
    │   │   ├── [x] struct ResourceMetricsDrain # 本轮样本与失败批次。
    │   │   ├── [x] struct ResourceSampleDraft # 带 trace、进程身份、时间、覆盖范围的样本。
    │   │   └── [x] struct ResourceSamplingFailure # 单个目标的局部失败。
    │   └── [x] workload_target.rs # 本轮目标身份、资源能力和生命周期事实。
    │       ├── [x] struct WorkloadTarget # 本轮目标身份、资源能力和生命周期事实。
    │       └── [x] trait WorkloadTargets # 获取权威目标、成员坐标和根进程存活状态。
    ├── [x] sampling/ # 定义批量采样接口及单轮候选路由，隔离各目标失败。
    │   ├── [x] fallback_chain.rs # 先执行主后端，再把本轮路由结果交给候选后端。
    │   │   └── [x] struct FallbackChain # 先执行主后端，再把本轮路由结果交给候选后端。
    │   ├── [x] mod.rs # 声明模块并 re-export。
    │   ├── [x] sampling_selection.rs # 本轮已接管目标和降级元数据，采样结束即释放。
    │   │   └── [x] struct SamplingSelection # 本轮已接管目标和降级元数据，采样结束即释放。
    │   ├── [x] verified_process_target.rs # 权威身份服务提供的稳定身份、observer PID 和期望 start ticks。
    │   │   └── [x] struct VerifiedProcessTarget # 权威身份服务提供的稳定身份、observer PID 和期望 start ticks。
    │   └── [x] workload_sampling_backend.rs # 维护与批量采样契约；上下文由装配层确定。
    │       └── [x] trait WorkloadSamplingBackend # 维护与批量采样契约；上下文由装配层确定。
    └── [x] workload_resource_profiling_backend.rs # 绑定稳定根进程身份的后端能力。
        └── [x] trait WorkloadResourceProfilingBackend # 绑定稳定根进程身份的后端能力。
```
