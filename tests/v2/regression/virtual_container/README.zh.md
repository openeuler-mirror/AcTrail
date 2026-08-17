# 虚拟容器回归测试

本功能验证 AcTrail 在 Kata 虚拟容器中的两条关键链路：

- guest-root `actraild` 与非 root openEuler workload 的接口、权限和身份边界；
- guest 内 TLS 明文与 eBPF 数据的采集、落库和 viewer 查询。

当前公共 case 名称为 `virtual_container`，并由统一 V2 runner 管理。测试不会在运行时
安装 Kata、替换宿主内核或修改 containerd；这些属于显式部署准备步骤。

contracts scope 还会离线验证 Guest OTLP endpoint 必须显式注入、占位/loopback
地址会在写镜像前失败，以及主机侧 Collector 的版本固定、资源上限、只读根文件系统、
落盘轮转和非 WAL 边界。

## 快速运行

公共 runner 默认使用 `VIRTUAL_CONTAINER_E2E_SCOPE=auto`。没有 `/dev/kvm` 的 Linux
开发机可直接运行下面的基础 case；包装脚本会在缺少本地 profile 时自动使用
`--no-profile`，contracts 通过后以明确的 `SKIPPED` 结束，不启动 VM：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container \
  --color never
```

contracts 对 ELF 元数据的读取固定使用稳定的 C locale，因此宿主界面语言不会把有效的
release 误判为不可读；ELF、bundle 或静态契约本身损坏仍然属于 `FAILED`。只有
contracts 通过后确认 `/dev/kvm` 或 Kata backend 外部能力不可用时才是 `SKIPPED`。

检测到 `/dev/kvm` 时，包装脚本仍要求先在同一 checkout 生成 profile 和 artifact，
然后继续完整 Kata runtime 验收。可显式设置 scope 为 `contracts` 或 `all` 覆盖自动选择。

首次部署必须先把目标分支拉到最终 checkout，并在该 checkout 中完成 release 构建和
`prepare-v2-test-artifacts.py`。`local/kata/` 是 Git 忽略的 checkout-local 资产；
新 worktree 不会继承旧目录的 profile、manifest 或 bundle，也不应复制旧 profile 来
绕过准备。完整首次部署命令见
[`deploy/virtual-container/README.md`](../../../../deploy/virtual-container/README.md)。
preparer 应由部署用户通过 `sudo -E` 启动，而不是在 root shell 中直接运行；它完成需要
特权的准备后会把 checkout-local artifact 和 profile 的属主还原为该部署用户。

完成准备后，在同一个仓库根目录执行：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container \
  --color never
```

同时运行基础和双 xiaoO 并发 case 时省略 `--case`：

```bash
deploy/virtual-container/host/run-v2-tests.sh --color never
```

脚本会读取机器本地的 `local/kata/v2-test-profile.json`，并在需要时申请一次 sudo。
默认 profile 缺失时，脚本会在 sudo 和 runner 启动前给出 preparer 提示并退出。密码、
API Token 和私钥不会写入 profile。

## 覆盖范围

单个 backend 使用两台顺序启动的长驻 Kata VM：

1. base VM 依次执行 `verify`、`deny`、`launch`、`namespace`；
2. data VM 依次执行 `tls-only`、`ebpf-only`、`combo`。

因此单 backend 共启动约 2 台 VM，最大同时运行 1 台。某个外部 backend/VMM 缺失
只会使该 backend 为 `SKIPPED`；部署产物过期、配置错误或数据断言失败为 `FAILED`。

## 详细说明

- [V2 测试、矩阵与排障](v2/README.zh.md)
- [Kata 部署与支持边界](../../../../deploy/virtual-container/README.md)
- [openEuler guest 制作](../../../../deploy/virtual-container/guest/OPENEULER.md)
- [workload 接入契约](../../../../deploy/virtual-container/workload/README.md)

普通 Docker 容器由 `container_auto` 和 `container_agent_xiaoo` 独立验证；其结果不能
替代 Kata guest 内独立内核、guest daemon 和 runtime Profile 的证据。
