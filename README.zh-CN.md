<div align="center">

# AcTrail: Agent Credible Trail

面向 AI Agent 治理、调度和管理控制的通用底座

[![版本：0.9.0](https://img.shields.io/badge/version-0.9.0-blue.svg)](#4-安装)
[![许可证：MulanPSL-2.0](https://img.shields.io/badge/License-MulanPSL--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.90%2B-orange.svg)](https://www.rust-lang.org/)
[![状态：持续开发中](https://img.shields.io/badge/status-active%20development-green.svg)](https://gitcode.com/openEuler/AcTrail/tree/dev)
[![最新代码：openEuler/dev](https://img.shields.io/badge/latest-openEuler%2Fdev-blue.svg)](https://gitcode.com/openEuler/AcTrail/tree/dev)

[English](README.md) | [中文](README.zh-CN.md)

</div>

---

## 1 核心亮点

| 亮点 | 价值 |
| --- | --- |
| **独立观测** | 在 Agent 日志之外，可靠观测命令、文件和网络活动 |
| **模型细节洞察** | 在同一条轨迹中查看提示词、响应、推理内容、工具调用和 token 用量 |
| **行为可追溯** | 从行为快速定位支撑证据，加快调查 |
| **主动治理** | 执行文件与命令策略，并追溯决策过程 |
| **受控执行** | 统一管理 Agent 启动并实时跟踪进程 |
| **灵活定制** | 在运行时加载或卸载策略与分析插件 |
| **可视化调查** | 通过 Web UI、CLI 和轨迹导出简化诊断 |
| **支持的 Agent** | xiaoO、Pi、Claude Code、Codex CLI、OpenCode、Grok、deepagents-code、Qoder CLI、zcode、openclaw |

## 2 为什么需要 AcTrail

Agent 可能在修改了错误文件或执行了意外命令后，仍然报告成功。AcTrail 提供独立证据，帮助你诊断错误，并在执行过程中落实策略。

<p align="center">
  <img src="images/figure1-agent-log-gap-zh-cn.drawio.svg" width="49%" alt="Agent 日志与系统活动">
  <img src="images/actrail-readme__evidence-to-action__candidate.drawio.svg" width="49%" alt="AcTrail 从证据到行为的轨迹">
</p>

## 3 前置依赖

已勾选 = 必需，未勾选 = 可选

### 3.1 主机

- [x] Linux / WSL（x86_64 / ARM64）
- [x] Root 权限或等效内核 capabilities
- [x] 内核 BTF、可写 tracefs、perf tracepoints、uprobes

### 3.2 组件

- [x] `actraild`：采集与存储
- [x] `actrailctl`：配置与轨迹控制
- [ ] `actrailweb`：Web UI（快速运行中使用）
- [ ] `actrailviewer`：CLI 查询与导出
- [ ] 策略与分析插件
- [ ] `actrail-sb`、`actrail-vsock-gateway`：隔离执行

[平台与构建要求](docs/reference/platform-support.md)

## 4 安装

### 4.1 交给 AI Agent

在仓库根目录向 Agent 提供以下提示词：

```text
阅读 README.md 和 scripts/install-release.sh。先使用
scripts/install-build-deps.sh --install 安装构建依赖，再运行
scripts/install-release.sh /usr/local/bin。验证 actraild、actrailctl、
actrailviewer 和 actrailweb 已在 PATH 中可用。
```

### 4.2 从源码安装

在仓库根目录执行：

```bash
./scripts/install-build-deps.sh --install  # 安装构建依赖（dnf / apt-get）
./scripts/install-release.sh /usr/local/bin  # 构建并安装 release 产物
```

### 4.3 从预构建 RPM 安装

<details>
<summary>📥 官方构建包</summary>

| 软件包 | 版本 | openEuler 版本 |
| --- | --- | --- |
| [AcTrail（Releases）](https://gitcode.com/openeuler/AcTrail/releases) | 最新 | — |
| [AcTrail（EulerMaker）](https://eulermaker.openeuler.openatom.cn/package/download?osProject=openEuler-26.09-DevStation:epol&packageName=AcTrail) | 0.7.2-2.oe2609 | 26.09-DevStation |

</details>

下载适合当前架构的软件包，然后安装：

```bash
sudo rpm -Uvh ./AcTrail-*.rpm  # 安装下载的软件包
```

## 5 快速运行

### 5.1 初始化

生成完整采集配置：

```bash
sudo actrailctl init --mode complete
```

### 5.2 启动守护进程

```bash
sudo actraild start
```

### 5.3 启动 Agent

示例：[xiaoO](https://atomgit.com/openEuler/xiaoO)（已安装并配置）

使用其他[支持的 Agent](#1-核心亮点)时，将下面的 `xiaoo` 替换为该 Agent 的启动命令。

```bash
sudo --preserve-env=HOME,PATH actrailctl launch -- xiaoo
```

### 5.4 执行任务

在 Agent 会话中输入：

```text
读取 /etc/os-release，并用三点概括操作系统信息。不要修改任何文件。
```

总结完成后，退出 Agent 会话。

### 5.5 启动 Web UI

```bash
sudo actrailweb
```

可通过 `--addr` 和 `--port` 修改监听地址与端口。

打开 [Web UI](http://127.0.0.1:18080)，查看：

- Agent 进程树
- `/etc/os-release` 文件活动
- 模型请求与总结响应

<!-- TODO: 添加展示 Agent 轨迹及关联证据的 Web UI 截图。 -->

### 5.6 停止

按 `Ctrl-C` 停止 Web UI，然后停止守护进程：

```bash
sudo actraild stop
```

[CLI 与故障排查](docs/getting-started/quickstart.md)

## 6 部署视图

| 本地执行 | 远端隔离执行 |
| --- | --- |
| 本地开发，以及主机、VM 或容器工作负载 | 需要隔离沙箱的远端工作负载 |
| ![本地部署](images/readme-deployment-local.svg) | ![远端部署](images/readme-deployment-hand-brain.svg) |

[部署指南](docs/operations/deployment/choose-a-mode.md)

## 7 文档

| 主题 | 链接 |
| --- | --- |
| 能力 | [概览](docs/concepts/capabilities.md)、[覆盖范围](docs/concepts/collection-observation-checklist.md) |
| 配置 | [采集](docs/reference/configuration/collection.md) |
| 运维 | [指南](docs/operations/README.md) |
| 开发 | [架构](docs/architecture/README.md) |

[全部文档](docs/README.md)

## 8 参与贡献

- [Issues](https://gitcode.com/openeuler/AcTrail/issues)：提供问题复现步骤或功能使用场景
- [Pull requests](https://gitcode.com/openeuler/AcTrail/pulls)：提交聚焦的改动及验证结果
- 验证：`cargo fmt`、release 构建、[真实 Agent 端到端测试](tests/README.md)

## 9 许可证

[木兰宽松许可证，第 2 版](LICENSE)
