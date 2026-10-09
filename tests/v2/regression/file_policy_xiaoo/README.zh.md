# 真实 Xiaoo 文件访问治理

使用真实 Xiaoo 和本地确定性 streaming provider，要求 Bash 向预先创建的目标文件写入标记。

测试分别使用默认 complete 与显式 fanotify 配置运行静态 deny 和 allow。每个阶段通过 `actrailctl init -f --patch` 刷新默认配置。默认 complete 仅隔离路径并指定测试规则文件，继承默认 LSM 后端、能力和完整观测设置；同时核验五项默认后端为 LSM 且 daemon 没有 fanotify 句柄。显式 fanotify 组采用精简观测并关闭 seccomp。拒绝阶段断言文件内容保持为空且工具返回权限错误；允许阶段断言标记完整写入。两个阶段均通过 viewer 验证实际存储中的文件治理事件。

```bash
python3 tests/v2/regression/file_policy_xiaoo/run_e2e.py --color never
```

需要 root、BPF LSM、fanotify、eBPF 及可执行的 Xiaoo。runner 会构建并安装 release 产物，默认清理测试服务、配置和数据。

`FILE_POLICY_XIAOO_E2E_BINARY` 可指定 Xiaoo 路径；`FILE_POLICY_XIAOO_E2E_READY_TIMEOUT_SECONDS` 配置 provider 就绪及审计等待时间，默认 15 秒。命令与启动超时沿用公共测试配置。
