# 探针检测器

> 本文展示当前 TLS 探针发现实现及 provider 变更对应的检测器子树。

TLS 探针检测器发现明文捕获点，即采集器可在 TLS 加密前或解密后读取数据的函数位置。它不安装或运行探针。当前实现在 `crates/tools/tls_probe_point_finder` 下。

`fast` 是面向运行时的解析器；`detect` 是诊断命令。两者构造相同的 `TlsProbeDetector` 对象树，并使用相同的检测结果。`fast` 使用 first-complete 选择并投影出可执行的 `ProbePointPlan`；`detect` 收集诊断结果并投影出报告。Provider 检测在 `probe_detector/detector/tls` 下只有一条实现路径。

检测器契约分为：

- 递归的检测器配置和行为；
- 上下文、结果、证据和错误；
- 候选项、完整的探针闭包和验证；
- 稳定的检测器 ID 和递归路径；
- first-complete、unique-match、unique-closure、collect-all 和 select-applicable 等显式选择节点；
- 检测器和使用方的能力模型。

当前 provider 树覆盖 Rustls、BoringSSL、OpenSSL、Go `crypto/tls`、GnuTLS 和 NSS/NSPR。叶节点使用适合各 provider 的证据：导出符号和依赖发现、Go pclntab 元数据，或架构特定的静态机器码模式。

二进制身份是用于识别同一可执行构建的共享值对象。优先使用 GNU build-id；否则使用固定的可执行段 SHA-256 采样作为身份形式。检测证据说明候选项为何匹配；使用方能力则决定特定采集器能否执行该候选项。

诊断路径会完整展开所配置的 TLS 和 Rustls 收集选项。其他 provider 的后代节点使用各自面向启动的选择策略，因此其报告止于这些策略选中的节点。`fast` 返回一个 provider 方案，daemon 挂载这一个方案。
