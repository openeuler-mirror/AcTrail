# Probe Detector 当前实现路径地图

本文件直接分解当前源码路径及其职责。TLS Probe Detector 用于定位目标程序在 TLS 加密前写出请求明文、以及在 TLS 解密后读入响应明文的运行时挂载点；这些明文随后才会被解析为 HTTP、WebSocket 和 LLM request/response。

```text
crates/core/model/src/
├── binary_identity.rs
│   ├── BinaryIdentityTypeCode：1 = GNU build-id；2 = ELF executable-segment sample SHA-256 v1
│   ├── BinaryIdentity：identity type code 与 identity 的不可分割值对象
│   └── 统一执行十六进制规范化和类型专属长度校验
└── lib.rs
    └── 暴露跨层共享的 binary_identity contract

crates/tools/tls_probe_point_finder/src/
├── lib.rs
│   ├── crate 公共入口与 CLI dispatch
│   ├── 保留 fast::resolve(FastProbeRequest) 的 PlanOnly 兼容 API
│   ├── 新增 fast::resolve_for_consumer(request, consumer) 供真实采集端声明执行能力
│   ├── 暴露 BinaryIdentity、identity type code 与已读 ELF bytes identity 入口
│   └── 保留 Go pclntab 公共辅助 API，但实现直接委托 GoTlsProbeDetector
│
├── binary_identity/
│   ├── resolver.rs
│   │   └── build-id 优先、固定 executable segment 窗口采样 fallback
│   └── mod.rs
│       └── 最小 re-export 共享 model contract 与 resolver
│
├── fast.rs
│   ├── 为一次请求构造一棵 TlsProbeDetector 对象树
│   ├── 按 first-complete 启动顺序调用树中的 provider detector
│   ├── 只有完整且当前 consumer 可执行的 candidate 才能提前结束
│   └── 将 detector 私有结果投影为 ProbePointPlan
│
├── detect/
│   ├── command.rs
│   │   ├── 为一次诊断构造一棵 TlsProbeDetector 对象树
│   │   ├── 使用 CollectAll 执行并保留全部 provider outcomes
│   │   ├── 消费与 fast 完全相同的 DetectionOutcome/DetectionEvidence
│   │   └── OutcomeProjector 只投影 matched/failed/ambiguous leaf 摘要，不执行 provider 检测；
│   │       当前不会把 Inapplicable 与所有父级 evidence 完整投影到 CandidateReport
│   ├── report.rs
│   │   └── CandidateReport、symbol/pattern/offset 等诊断输出模型
│   └── mod.rs
│       └── detect 内部最小 re-export
│
├── plan.rs
│   └── 采集 consumer 使用的 ProbePointPlan、BinaryIdentity、ProbePoint、方向与挂载策略
│
└── probe_detector/
    ├── mod.rs
    │   └── contract 与 detector 两个 namespace；不含全局 dead-code allowance
    │
    ├── contract/
    │   ├── detector/
    │   │   ├── config.rs
    │   │   │   └── ProbeDetectorConfig 校验边界与 DetectorConfigError
    │   │   └── probe_detector.rs
    │   │       └── 递归 detector 共同 contract
    │   ├── detection/
    │   │   └── context、outcome、evidence、error；Collected outcome 保留当前节点的直接子 outcomes
    │   ├── candidate/
    │   │   └── candidate、closure、verification；ProbePoint 值对象由 plan.rs 统一定义
    │   ├── identity/
    │   │   └── DetectorId 与递归 DetectorPath
    │   ├── selection/
    │   │   └── FirstComplete、UniqueMatch、UniqueClosure、CollectAll、SelectApplicable 均有真实节点
    │   └── capability/
    │       ├── detector/consumer capability models
    │       ├── Standalone/Sync 的 provider 与 hook-symbol 结构能力矩阵
    │       └── Daemon 按目标 PT_INTERP 区分 dynamic-sync 与 static-direct
    │
    └── detector/tls/
        ├── candidate.rs
        │   └── TlsProbeCandidateFactory：统一构造 points、closure 与 consumer capability
        ├── config.rs
        │   └── TlsProbeDetectorConfig：独立拥有所有直接子 detector config；统一下发 match_limit
        ├── probe_detector.rs
        │   └── TlsProbeDetector：fast 按 FirstComplete 短路；detect 按 CollectAll 保留全部子树
        ├── mod.rs
        │   └── 声明 provider namespace，并最小 re-export TLS 根类型
        │
        ├── rustls/
        │   ├── config.rs + probe_detector.rs
        │   │   └── Rustls 根节点按 fast/detect 配置使用 FirstComplete 或 CollectAll
        │   ├── symbol/
        │   │   └── 从 demangled CommonState symbols 建立明文读写闭包
        │   └── static_pattern/
        │       ├── probe_detector.rs
        │       │   └── SelectApplicable 调用架构分支；非目标分支在扫描前返回 Inapplicable，
        │       │       只有实际 ELF 架构分支执行机器码扫描
        │       ├── x86_64/
        │       │   ├── common_state_pair_27_32/
        │       │   ├── common_state_pair_41_32/
        │       │   └── UniqueClosure 比较所有命中 signature family 的规范化闭包
        │       └── aarch64/
        │           ├── common_state_pair_52_64/
        │           ├── common_state_pair_48_56/
        │           └── UniqueClosure 比较所有命中 signature family 的规范化闭包
        │               每个 signature family 均含 config.rs、probe_detector.rs 与 verified_targets.rs
        │
        ├── boringssl/
        │   ├── config.rs + probe_detector.rs
        │   │   └── 通过 SelectApplicable 分发到唯一适用的独立架构子树
        │   ├── common/
        │   │   └── 两架构共同的 symbol evidence、static-pattern 数据模型与 outcome factory
        │   ├── x86_64/
        │   │   └── symbol、static_pattern、shared_library detector 及各自 config
        │   └── aarch64/
        │       └── symbol、static_pattern、shared_library detector 及各自 config
        │
        ├── openssl/
        │   ├── config.rs + probe_detector.rs
        │   │   ├── 持有 x86_64/aarch64 两个实际 branch，并通过 SelectApplicable 分派
        │   │   └── 聚合 executable/shared_library；多个 libssl 命中由 UniqueMatch 拒绝歧义
        │   ├── common/
        │   │   ├── executable/
        │   │   │   └── 实现 ProbeDetector 并生成 executable symbol candidate
        │   │   └── shared_library/
        │   │       ├── discovery/
        │   │       │   └── user/direct/Python-_ssl/transitive library discovery
        │   │       └── symbol/
        │   │           └── libssl SSL_read/write closure 检测
        │   ├── x86_64/mod.rs
        │   │   └── re-export common 实现；构造时绑定 x86_64 并写入 detector path
        │   └── aarch64/mod.rs
        │       └── re-export common 实现；构造时绑定 aarch64 并写入 detector path
        │
        ├── go_tls/
        │   ├── config.rs + probe_detector.rs
        │   └── pclntab/config.rs + pclntab/probe_detector.rs
        │       └── 实现 ProbeDetector，定位 crypto/tls Conn.Read、Conn.Write 与 runtime.memmove
        │
        ├── gnutls/
        │   ├── config.rs + probe_detector.rs
        │   └── shared_library/config.rs + shared_library/probe_detector.rs
        │       └── 实现 ProbeDetector，定位 gnutls_record_send/recv
        │
        └── nss/
            ├── config.rs + probe_detector.rs
            └── shared_library/config.rs + shared_library/probe_detector.rs
                └── 实现 ProbeDetector，定位 NSPR PR_Write/Send/Read/Recv
```

旧的 `src/providers.rs`、`src/providers/*.rs`、`fast/tests.rs` 与 `detect/assemble.rs` 已删除。provider 检测只有 `probe_detector/detector/tls/` 一条实现路径；`fast.rs` 与 `detect/command.rs` 只负责输入和 outcome projection，不再包含第二套 detector。

真实端到端路径：

```text
tests/agent-trace/
├── xiaoo-rustls/
│   └── 验证真实 xiaoO 的 Rustls 双向明文和 LLM spans
├── opencode-bun/
│   └── 验证真实 Bun/OpenCode BoringSSL 采集
├── dynamic-tls/
│   └── 验证 OpenSSL executable/shared-library 路径
└── gnutls-nss-llm/ 等 workload
    └── 验证对应 shared-library detector 与采集闭包

tests/v2/regression/
├── probe_codex_llm/
│   └── 验证 static ELF → Daemon direct Rustls，并校验 LLM request/response 成对
├── probe_xiaoo_llm/
│   └── 验证 dynamic ELF → Daemon sync Rustls，并校验 LLM request/response 成对
└── probe_claude_llm/
    └── 验证 dynamic ELF → Daemon sync BoringSSL，并校验 LLM request/response 成对
```

当前 diagnostic 配置在 TLS 根和 Rustls provider 根使用 `CollectAll`：因此会收集全部 provider outcome，并展开 Rustls 的 symbol/static-pattern 两类 outcome。OpenSSL、BoringSSL、GnuTLS 与 NSS 的后代节点仍保留各自启动导向的选择策略；当前实现不声称已经递归展开所有 provider 的全部 leaf evidence。

受当前 aarch64 主机环境限制，x86_64 detector 已 release 编译但未在本机执行真实运行时 E2E；Go toolchain 与 Python `langgraph` 包缺失的工作负载也保留为环境待验证项。
