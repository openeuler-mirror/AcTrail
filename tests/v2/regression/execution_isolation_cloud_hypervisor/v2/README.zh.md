# Cloud Hypervisor 可选执行隔离测试

该 case 使用 Kata 部署与生命周期 profile，
验证 Cloud Hypervisor backend 承载的 sandbox link 二进制观测通路：

```text
actrail-sb → actrail-vsock-gateway → actraild
```

OTLP `vsock-bridge` 由 virtual-container 出境链路独立使用。

测试使用现有 V2 内容寻址资产链。执行前必须重新安装当前 release，并通过
`prepare-v2-test-artifacts.py --backend cloud-hypervisor --xiaoo ...` 刷新
`local/kata/v2-test-profile.json`。manifest、release、runtime config 或 xiaoO 不一致
属于部署错误并返回 `FAILED`；KVM、containerd、Kata、shim 或 Cloud Hypervisor
缺失属于外部条件并返回 `SKIPPED`。

`CloudHypervisorSocketInventory` 在创建 VM 前后分别读取 `/run/vc/vm/*/clh.sock`，
只接受本轮恰好新增一个 base socket，并为 VSOCK port 生成
`<base>_<port>` endpoint。它不会扫描后随意挑选已有 VM，也不会覆盖已有 socket。

完整场景通过 `execution_isolation_cloud_hypervisor` 注册到 V2 runner，
并按以下顺序执行：

1. 校验 manifest 中当前 release、Guest bundle 内 `actrail-sb`、Host bundle 内
   `actrail-vsock-gateway`、sandbox resource alert package 和 xiaoO 的摘要；
2. 从刷新后的默认 operator 配置启用 `hand_observation` listener 和独立的
   `sandbox-alerts.sqlite`；
3. 配置并由 daemon 拉起 `actraild-alert-proxy`，外部 subscriber 完成握手、心跳，
   并订阅 CPU、OOM killed、可用内存、进程读和进程写五类 sandbox 告警；
4. 加载真实 `actrail.sandbox-resource-alert` consumer，并将 CPU、可用内存、读和写
   阈值降低到场景可稳定触发的值；
5. 创建一台 Cloud Hypervisor Kata VM，解析本轮唯一新增的 `clh.sock`；
6. 分别调用当前 release 的 SB/gateway `init --output` 生成完整默认配置；SB 静态配置
   只覆盖根进程名、Guest-local control socket 和实例锁，不保存 VSOCK CID 或 port；
   gateway 配置覆盖 VSOCK port、Cloud Hypervisor socket 和 daemon address；
7. 启动 Host gateway，并使用生成配置中的有界 per-SB/global queue；正常
   observation 持续刷新连接活性，只有超过最大静默时间才发送兜底 Heartbeat；
8. Guest 内执行 `actrail-sb daemon --config`，等待 Guest-local control socket ready；
   随后执行 `actrail-sb connect --control-socket ... --host-cid ... --port ...` 注入本轮
   Cloud Hypervisor VSOCK endpoint，握手成功后再启动 `comm=actrail-root` 的命名根和其
   真实 xiaoO 子进程；
9. 本地 provider 要求真实 xiaoO 使用 Bash 工具完成一次文件读取和一次文件写入；
10. 在独立 memory cgroup 中运行受限 Python 子进程，确认该子进程被 Guest kernel OOM
    killer终止且 `/proc/vmstat` 的 `oom_kill` 真实增长，不消耗宿主机或其他 Guest 进程的
    内存边界；
11. 要求独立数据库内存在 high-CPU、OOM-killed、OOM-risk、high-read 和 high-write 结构化记录，
    两类 I/O 记录的 PID 必须等于命名谱系根 PID；
12. 要求 subscriber 收到与数据库记录相同的 sandbox source 和 extras，source
    不得伪造 `trid`；
13. 结束时只清理本轮 OOM cgroup、subscriber、VM、gateway、SB、daemon 与 alert proxy
    资源。

仓库根目录运行：

```bash
deploy/virtual-container/host/run-v2-tests.sh \
  --case virtual_container \
  --case execution_isolation_cloud_hypervisor \
  --color never
```

manifest 或 bundle 缺失、陈旧、backend 不匹配时先返回 `FAILED`；只有这些仓库资产
完整后，KVM、containerd、Kata、shim 或 Cloud Hypervisor 等外部能力缺失才返回
`SKIPPED`。
