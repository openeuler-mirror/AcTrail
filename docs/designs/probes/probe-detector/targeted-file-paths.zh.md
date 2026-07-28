# Probe Detector 目标文件路径

本文件定义 Probe Detector 重构完成后的目标路径。路径本身承担 namespace；每个 detector category 目录包含当前层 detector，并可以继续包含任意层级的子 detector。每个 `probe_detector.rs` 必须在同目录拥有一一对应的 `config.rs`，由后者定义该 detector 的构造配置结构体；即使当前没有可配置字段，也必须保留显式空配置结构体。Rustls static candidate 按实际 signature/codegen shape 命名，适用版本作为验证证据记录，不进入 detector 路径。

目标二进制和实际挂载二进制统一使用 `BinaryIdentity { identity_type_code, identity }`。任何 detector、plan、诊断报告、symbol map 或本机缓存都不得把 GNU build-id 本身作为必填 identity；它只是 identity provider 之一。identity type code 与取值规则共同版本化，比较时必须同时比较二者。

当前 provider 选择顺序：

1. `identity_type_code = 1`：GNU build-id，`identity` 是规范化的小写十六进制原始 build-id 字节。
2. `identity_type_code = 2`：ELF executable-sample SHA-256 v1。仅在 GNU build-id 缺失时使用；输入包括 ELF machine、文件长度、所有 executable `PT_LOAD` 的布局元数据，以及每个 executable segment 开头、中间、结尾最多 4 KiB 的确定性窗口。哈希数据量受 executable segment 数量约束，不随整个 ELF 文件大小线性增长。

无法构造任一 identity 属于启动期输入错误并 fail-fast；不得写入 `not_found`、空 identity 或无类型 fallback。运行中某个下游 identity 校验失败时只使该采集路径失效，不能导致无关采集器退出。

```text
crates/core/model/src/
├── binary_identity.rs
│   └── 定义跨 finder、control、daemon、runtime 与 collector 共享的值 contract
│       ├── BinaryIdentityTypeCode：稳定、版本化的 identity provider code
│       ├── BinaryIdentity：identity_type_code + identity 不可分割值对象
│       └── 规范化、类型专属长度校验与成对比较
└── lib.rs
    └── 暴露 binary_identity contract namespace

crates/tools/tls_probe_point_finder/src/
├── binary_identity/
│   ├── mod.rs
│   │   └── 仅最小 re-export 共享值 contract 与 finder resolver
│   └── resolver.rs
│       └── 从同一份已读 ELF bytes 选择 GNU build-id 或低开销 executable-segment sample identity
│
└── probe_detector/
    ├── mod.rs
    │   └── 仅声明 contract 与 detector 子树并执行最小 re-export
    │
    ├── contract/
    │   ├── mod.rs
    │   │   └── 仅声明 contract namespace 并执行最小 re-export
    │   │
    │   ├── detector/
    │   │   ├── mod.rs
    │   │   ├── config.rs
    │   │   │   └── 定义 detector 配置的共同边界、校验语义和空配置约定
    │   │   └── probe_detector.rs
    │   │       └── 定义所有分支和叶子共同实现的 ProbeDetector trait
    │   │           ├── detector identity
    │   │           └── detect(context) -> DetectionOutcome
    │   │
    │   ├── detection/
    │   │   ├── mod.rs
    │   │   │
    │   │   ├── context/
    │   │   │   ├── mod.rs
    │   │   │   └── probe_context.rs
    │   │   │       └── 一次检测共享的只读事实
    │   │   │           ├── target identity
    │   │   │           ├── parsed ELF image
    │   │   │           ├── actual/requested architecture
    │   │   │           ├── provider/source filters
    │   │   │           ├── shared-library candidates
    │   │   │           └── target consumer
    │   │   │
    │   │   ├── outcome/
    │   │   │   ├── mod.rs
    │   │   │   └── detection_outcome.rs
    │   │   │       └── 明确区分检测结果
    │   │   │           ├── Inapplicable
    │   │   │           ├── NoMatch
    │   │   │           ├── Matched
    │   │   │           ├── Ambiguous
    │   │   │           └── Collected：诊断模式保留全部独立子结果，不把多个命中伪装成歧义
    │   │   │
    │   │   ├── evidence/
    │   │   │   ├── mod.rs
    │   │   │   └── detection_evidence.rs
    │   │   │       └── 记录 detector path、架构、symbols、patterns、offsets 与拒绝原因
    │   │   │
    │   │   └── error/
    │   │       ├── mod.rs
    │   │       └── detection_error.rs
    │   │           └── 表达损坏输入、解析失败、溢出和 detector 内部错误
    │   │
    │   ├── candidate/
    │   │   ├── mod.rs
    │   │   ├── probe_candidate.rs
    │   │   │   └── 完整 detector 匹配结果
    │   │   │       ├── detector path
    │   │   │       ├── provider/source/architecture
    │   │   │       ├── probe points
    │   │   │       ├── evidence
    │   │   │       └── capability claim
    │   │   │
    │   │   ├── closure/
    │   │   │   ├── mod.rs
    │   │   │   └── probe_closure.rs
    │   │   │       └── 验证 candidate 是否形成完整 TLS plaintext 观测闭包
    │   │   │           ├── outbound plaintext request point
    │   │   │           └── inbound plaintext response point
    │   │   │
    │   │   └── verification/
    │   │       ├── mod.rs
    │   │       └── verified_target.rs
    │   │           └── 与 detector identity 分离的真实验证元数据
    │   │               ├── library/runtime version
    │   │               ├── compiler and optimization shape
    │   │               ├── target BinaryIdentity
    │   │               └── E2E evidence source
    │   │
    │   ├── selection/
    │   │   ├── mod.rs
    │   │   ├── selection_policy.rs
    │   │   │   └── FirstComplete、UniqueMatch、UniqueClosure、CollectAll、SelectApplicable
    │   │   └── detection_selector.rs
    │   │       └── 按 policy 聚合多个子 ProbeDetector 的 DetectionOutcome
    │   │
    │   ├── identity/
    │   │   ├── mod.rs
    │   │   ├── detector_id.rs
    │   │   │   └── 当前 namespace 内稳定 detector identity
    │   │   └── detector_path.rs
    │   │       └── 从根 detector 到叶子 detector 的完整 namespace path
    │   │
    │   └── capability/
    │       ├── mod.rs
    │       ├── key/
    │       │   ├── mod.rs
    │       │   └── capability_key.rs
    │       │       └── architecture × provider × source × resolver × consumer 复合键
    │       ├── detector/
    │       │   ├── mod.rs
    │       │   └── detector_capability.rs
    │       │       └── finder 是否能生成可信完整 plan
    │       └── consumer/
    │           ├── mod.rs
    │           └── consumer_capability.rs
    │               ├── standalone/sync/daemon consumer 是否能执行 plan
    │               ├── 校验 provider、hook symbol 与本机构建架构
    │               └── daemon 根据目标 PT_INTERP 分为 dynamic-sync 与 static-direct 两种能力
    │
    └── detector/
        ├── mod.rs
        │   └── 仅声明具体 detector namespace
        │
        └── tls/
            ├── mod.rs
            ├── candidate.rs
            │   └── TLS detector 共用的 candidate factory；ProbePoint 值对象继续由顶层 plan.rs 唯一定义
            ├── config.rs
            │   └── TlsProbeDetectorConfig，仅组合 TLS 根节点参数与直接子 detector configs
            ├── probe_detector.rs
            │   └── TLS plaintext 根 detector
            │       ├── 一次构造并持有各 TLS provider 子 detector
            │       ├── fast 使用 FirstComplete，只对当前 consumer 可执行的完整 candidate 短路
            │       ├── detect 使用 CollectAll，执行并保留全部 provider 子结果
            │       └── fast/detect 消费同一 DetectionOutcome，分别投影启动 plan 与诊断报告
            │
            ├── rustls/
            │   ├── mod.rs
            │   ├── config.rs
            │   ├── probe_detector.rs
            │   │   └── Rustls provider detector
            │   │       ├── fast 按 FirstComplete 调用 symbol/static-pattern
            │   │       └── detect 按 CollectAll 保留 symbol/static-pattern 两类证据
            │   │
            │   ├── symbol/
            │   │   ├── mod.rs
            │   │   ├── config.rs
            │   │   └── probe_detector.rs
            │   │       └── 从未剥离 ELF 的 demangled symbols 建立双向闭包
            │   │           ├── CommonState::buffer_plaintext
            │   │           └── CommonState::take_received_plaintext
            │   │
            │   └── static_pattern/
            │       ├── mod.rs
            │       ├── config.rs
            │       ├── probe_detector.rs
            │       │   └── 使用 SelectApplicable 根据 target ELF 实际架构选择唯一适用的子 detector
            │       │
            │       ├── x86_64/
            │       │   ├── mod.rs
            │       │   ├── config.rs
            │       │   ├── probe_detector.rs
            │       │   │   └── 聚合 x86_64 Rustls signature/codegen candidates
            │       │   │       └── 使用 UniqueClosure policy
            │       │   │
            │       │   ├── common_state_pair_27_32/
            │       │   │   ├── mod.rs
            │       │   │   ├── config.rs
            │       │   │   ├── probe_detector.rs
            │       │   │   │   └── 匹配 27-byte buffer 与 32-byte take signature family
            │       │   │   └── verified_targets.rs
            │       │   │       └── 记录该 signature family 已验证的版本和真实 binary
            │       │   │
            │       │   └── common_state_pair_41_32/
            │       │       ├── mod.rs
            │       │       ├── config.rs
            │       │       ├── probe_detector.rs
            │       │       │   └── 匹配 41-byte buffer 与 32-byte take signature family
            │       │       └── verified_targets.rs
            │       │           └── 记录该 signature family 已验证的版本和真实 binary
            │       │
            │       └── aarch64/
            │           ├── mod.rs
            │           ├── config.rs
            │           ├── probe_detector.rs
            │           │   └── 聚合 aarch64 Rustls signature/codegen candidates
            │           │       └── 使用 UniqueClosure policy
            │           │
            │           ├── common_state_pair_52_64/
            │           │   ├── mod.rs
            │           │   ├── config.rs
            │           │   ├── probe_detector.rs
            │           │   │   └── 匹配 52-byte buffer 与 64-byte take signature family
            │           │   └── verified_targets.rs
            │           │       └── 记录 Codex/xiaoO/Rustls 构建验证证据，不参与 detector 命名
            │           │
            │           └── common_state_pair_48_56/
            │               ├── mod.rs
            │               ├── config.rs
            │               ├── probe_detector.rs
            │               │   └── 匹配 48-byte buffer 与 56-byte take signature family
            │               └── verified_targets.rs
            │                   └── 记录 xiaoO/Rustls 构建验证证据，不参与 detector 命名
            │
            ├── openssl/
            │   ├── mod.rs
            │   ├── config.rs
            │   ├── probe_detector.rs
            │   │   ├── OpenSSL provider detector，通过 SelectApplicable 进入显式架构 namespace
            │   │   └── 对多个实际 libssl 命中使用 UniqueMatch，禁止静默选择第一个
            │   │
            │   ├── common/
            │   │   ├── mod.rs
            │   │   │   └── 仅暴露经确认在 x86_64/aarch64 完全相同的实现
            │   │   │
            │   │   ├── executable/
            │   │   │   ├── mod.rs
            │   │   │   ├── config.rs
            │   │   │   └── probe_detector.rs
            │   │   │       └── 从 executable exported SSL_* symbols 建立双向闭包
            │   │   │
            │   │   └── shared_library/
            │   │       ├── mod.rs
            │   │       ├── config.rs
            │   │       ├── probe_detector.rs
            │   │       │   └── 聚合具体 libssl candidate detectors
            │   │       ├── discovery/
            │   │       │   ├── mod.rs
            │   │       │   ├── config.rs
            │   │       │   │   └── Python _ssl query 默认启用；dependency node 上限默认 4096，均允许外部覆盖
            │   │       │   └── probe_detector.rs
            │   │       │       └── 发现 user/direct/Python-_ssl/transitive library candidates
            │   │       └── symbol/
            │   │           ├── mod.rs
            │   │           ├── config.rs
            │   │           └── probe_detector.rs
            │   │               └── 验证具体 libssl 与 SSL read/write symbol closure
            │   │
            │   ├── x86_64/
            │   │   └── mod.rs
            │   │       └── 在 x86_64 namespace 下 re-export common executable/shared_library detectors 及其 configs
            │   │
            │   └── aarch64/
            │       └── mod.rs
            │           └── 在 aarch64 namespace 下 re-export common executable/shared_library detectors 及其 configs
            │
            ├── boringssl/
            │   ├── mod.rs
            │   ├── config.rs
            │   ├── probe_detector.rs
            │   │   └── BoringSSL provider detector，根据 image.arch() 进入显式架构 namespace
            │   │
            │   ├── common/
            │   │   ├── mod.rs
            │   │   └── symbol_evidence.rs
            │   │       └── 两个架构共同使用的 ELF symbol 查询和 evidence 构造，不决定 closure
            │   │
            │   ├── x86_64/
            │   │   ├── mod.rs
            │   │   ├── config.rs
            │   │   ├── probe_detector.rs
            │   │   │   └── 聚合 x86_64 BoringSSL detectors
            │   │   ├── symbol/
            │   │   │   ├── mod.rs
            │   │   │   ├── config.rs
            │   │   │   └── probe_detector.rs
            │   │   │       └── 要求 handshake + read + write symbol closure
            │   │   ├── static_pattern/
            │   │   │   ├── mod.rs
            │   │   │   ├── config.rs
            │   │   │   └── probe_detector.rs
            │   │   │       └── read + write 构成采集闭包；handshake 是可选增强证据，缺失时要求 BoringSSL identity marker
            │   │   └── shared_library/
            │   │       ├── mod.rs
            │   │       ├── config.rs
            │   │       └── probe_detector.rs
            │   │           └── 验证 x86_64 shared object 与 handshake/read/write closure
            │   │
            │   └── aarch64/
            │       ├── mod.rs
            │       ├── config.rs
            │       ├── probe_detector.rs
            │       │   └── 聚合 aarch64 BoringSSL detectors
            │       ├── symbol/
            │       │   ├── mod.rs
            │       │   ├── config.rs
            │       │   └── probe_detector.rs
            │       │       └── 要求 read + write symbol closure
            │       ├── static_pattern/
            │       │   ├── mod.rs
            │       │   ├── config.rs
            │       │   └── probe_detector.rs
            │       │       └── read/read_internal/write related-entry detection
            │       └── shared_library/
            │           ├── mod.rs
            │           ├── config.rs
            │           └── probe_detector.rs
            │               └── 验证 aarch64 shared object 与 read/write closure
            │
            ├── go_tls/
            │   ├── mod.rs
            │   ├── config.rs
            │   ├── probe_detector.rs
            │   │   └── Go crypto/tls provider detector
            │   └── pclntab/
            │       ├── mod.rs
            │       ├── config.rs
            │       └── probe_detector.rs
            │           └── 从 .gopclntab 建立 Conn.Write/Conn.Read/runtime.memmove 闭包
            │
            ├── gnutls/
            │   ├── mod.rs
            │   ├── config.rs
            │   ├── probe_detector.rs
            │   │   └── GnuTLS provider detector
            │   └── shared_library/
            │       ├── mod.rs
            │       ├── config.rs
            │       └── probe_detector.rs
            │           └── 验证 libgnutls architecture 与 record_send/record_recv closure
            │
            └── nss/
                ├── mod.rs
                ├── config.rs
                ├── probe_detector.rs
                │   └── NSS/NSPR provider detector
                └── shared_library/
                    ├── mod.rs
                    ├── config.rs
                    └── probe_detector.rs
                        └── 验证 libnspr4 architecture 与 PR write/read closure
```

`config.rs` 与 `probe_detector.rs` 是同一个 namespace 内不可拆分的一对：

```text
category/
├── mod.rs
├── config.rs
│   └── CategoryProbeDetectorConfig
├── probe_detector.rs
│   └── CategoryProbeDetector::try_new(CategoryProbeDetectorConfig)
└── child/
    ├── mod.rs
    ├── config.rs
    │   └── ChildProbeDetectorConfig
    └── probe_detector.rs
        └── ChildProbeDetector::try_new(ChildProbeDetectorConfig)
```

每个 config 只拥有当前 detector 的构造参数和直接子 detector configs，不得读取兄弟或祖先的内部配置。配置默认值必须由对应 config 类型定义，并预留外部覆盖入口；构造时完成校验，非法值在启动阶段 fail-fast。空配置也必须定义具名结构体，不能用 `()`、全局配置或父配置中的隐式常量代替。匹配上限、聚合策略和 candidate enablement 属于相应 detector config，不属于 `ProbeContext` 中的目标事实。

目标 namespace 示例：

```text
probe_detector::contract::detector
probe_detector::contract::detection::outcome
probe_detector::contract::candidate::closure
probe_detector::contract::capability::consumer

probe_detector::detector::tls::rustls
probe_detector::detector::tls::rustls::static_pattern::aarch64
probe_detector::detector::tls::rustls::static_pattern::aarch64::common_state_pair_52_64
probe_detector::detector::tls::openssl::x86_64::shared_library::symbol
probe_detector::detector::tls::openssl::aarch64::shared_library::symbol
probe_detector::detector::tls::boringssl::x86_64::static_pattern
probe_detector::detector::tls::boringssl::aarch64::static_pattern
```

即使两个架构当前使用相同 detector，也必须保留显式架构 namespace：

```text
provider/
├── common/
│   └── shared implementation
├── x86_64/
│   └── mod.rs  → re-export common implementation
└── aarch64/
    └── mod.rs  → re-export common implementation
```

只有行为、闭包要求、ABI 解释和 evidence 构造均相同的实现才能进入 `common/`。任一条件存在架构差异时，必须在 `x86_64/` 与 `aarch64/` 下分别实现。

禁止使用下列版本绑定路径：

```text
rustls_0_23_40/
rustls_0_23_42/
```

版本适用范围必须记录在对应 signature family 的 `verified_targets.rs`，使同一个 detector 可以覆盖多个产生相同机器码形态的 Rustls 版本和构建。
