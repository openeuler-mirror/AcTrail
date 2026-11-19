# 真实 Xiaoo 文件访问治理

## 测试目标

验证真实 XiaoO 文件写入受 deny/allow 策略控制，并验证治理期间资源存储正常工作。

## 场景设计

使用本地流式 provider 驱动真实 XiaoO 写入预创建文件。对默认 complete 和显式 fanotify 配置分别执行拒绝与允许阶段，并覆盖资源库独立存储与共享存储。

### 场景检查

使用真实 Xiaoo 和本地确定性 streaming provider，要求 Bash 向预先创建的目标文件写入标记。

测试分别使用默认 complete 与显式 fanotify 配置运行静态 deny 和 allow。每个阶段通过 `actrailctl init -f --patch` 刷新默认配置。默认 complete 仅隔离路径并指定测试规则文件，继承默认 LSM 后端、能力和完整观测设置；同时核验五项默认后端为 LSM 且 daemon 没有 fanotify 句柄。显式 fanotify 组采用精简观测并关闭 seccomp。拒绝阶段断言文件内容保持为空且工具返回权限错误；允许阶段断言标记完整写入。两个阶段均通过 viewer 验证实际存储中的文件治理事件。

fanotify 分别使用独立资源 SQLite 和与主库存储共享文件的配置，共执行六轮真实 Xiaoo。数据库与受管控文件处于同一监听目录，覆盖首次创建和重启，验证 daemon 启动及内部数据库访问不被自身权限拦截阻塞。每轮还检查资源样本已写入；共享文件组同时检查主库事件和资源样本。

每轮将文件策略结果和 `*_resource_storage` 分别显示。存储小项检查五张列存储表存在、旧表不存在、
样本没有缺失的 profile/boot 引用，且 extra 使用 BLOB；host 的 boot ID、时间戳、CPU、内存和 load
通过真实 viewer 读回，与 SQL 逐行核对。短任务允许只有 host 样本；共享库组还要求主库 events 非空。

## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、root 权限和可工作的 eBPF 环境 | 启动采集与治理 |
| AcTrail | release 产物和测例使用的官方插件包 | 执行、保存和查询测试结果 |

需要 root、BPF LSM、fanotify、eBPF 及可执行的 Xiaoo。

## 测试步骤摘要

1. 准备受保护文件和本地 provider，为默认 complete 与显式 fanotify 两组分别刷新默认配置。
2. 在 deny 阶段驱动真实 XiaoO 向目标文件写入，核对权限错误以及文件保持为空的实际效果。
3. 在 allow 阶段重新执行写入，核对完整文件内容，并通过 viewer 查询两阶段的文件治理事件。
4. 对 fanotify 分别使用独立资源库和共享数据库，覆盖首次创建及重启，确认数据库访问和 daemon 启动正常。
5. 读回资源样本，核对存储表、profile/boot 引用和 host 指标，共享库组同时检查主库事件。
6. 停止测试服务，清理文件、配置与存储。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 拒绝 | 真实工具尝试写入受保护文件 | 返回权限错误，文件保持为空 |
| 允许 | 改为允许后写入 | 完整写入任务标记 |
| 治理证据 | 每轮任务结束 | viewer 中有对应文件治理记录 |
| 内部存储 | 数据库与受控文件位于同一监听目录 | daemon 正常启动，主库与资源样本可读且关联有效 |

## 快速运行

```bash
python3 tests/e2e/regression/file_policy_xiaoo/run_e2e.py --color never
```

需要 root、BPF LSM、fanotify、eBPF 及可执行的 Xiaoo。runner 会构建并安装 release 产物，默认清理测试服务、配置和数据。

`FILE_POLICY_XIAOO_E2E_BINARY` 可指定 Xiaoo 路径；`FILE_POLICY_XIAOO_E2E_READY_TIMEOUT_SECONDS` 配置 provider 就绪及审计等待时间，默认 15 秒。命令与启动超时沿用公共测试配置。
