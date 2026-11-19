# Checkpoint Btrfs 验收

## 测试目标

验证 Btrfs checkpoint 能在任务启动、手动保存和 LLM 请求边界保存文件状态，并在恢复、删除、trace 结束和 daemon 重启时遵守生命周期规则。

## 场景设计

测例在独立工作目录中创建 1 GiB 回环 Btrfs 文件系统和受保护子卷，配置 Btrfs checkpoint backend 动态库，启动隔离配置的 daemon，用 `actrailctl launch --checkpoint` 运行任务（使用 shell 模拟 Agent）。LLM 请求发往测例进程内的本地 HTTPS 服务，经 TLS-sync 产生请求边界。测例只写自己的运行目录，结束时停止 daemon 和 Web 并卸载文件系统。


### 场景检查


- `setup`：回环文件系统、backend 配置和 daemon 就绪。
- `initial_checkpoint`：launch 在 Agent 运行前完成初始保存。
- `manual_save`：Agent 运行期间手动保存成功。
- `delete`：删除 checkpoint 后 backend 释放对应快照。
- `trace_end`：Trace 结束时释放全部快照，之后恢复被拒绝。
- `connect_failure`：backend 不接受的 scope 使 launch 失败，Agent 没有运行。
- `llm_boundary`：三次 LLM 请求均在响应前保存，包括此前没有文件修改的一次；普通 HTTPS 请求不触发保存。
- `forced_restore`：Agent 保持运行，等待 LLM 请求的恢复经 `force` 立即执行。
- `web`：Web 把三次 LLM 边界匹配到 ActionTree 中的 LLM 调用，并能删除 checkpoint。
- `running_restore`：运行中的 Agent 在下一次 LLM 请求时被恢复，收到响应后读到的是恢复后的文件。
- `daemon_restart`：daemon 重启后历史仍可查询，恢复被拒绝。

前一步失败时后续步骤不再执行。


## 测试依赖

| 依赖 | 要求 | 用途 |
| --- | --- | --- |
| Python | Python ≥ 3.11 | 运行测试执行器 |
| 主机环境 | Linux、root 权限和可工作的 eBPF 环境 | 启动采集与治理 |
| AcTrail | release 产物和测例使用的官方插件包 | 执行、保存和查询测试结果 |
| 文件系统 | Btrfs 内核支持与回环挂载权限 | 创建独立子卷和快照 |
| 工具 | mkfs.btrfs、btrfs、findmnt、mount、curl、openssl | 准备文件系统与请求边界 |
| Backend | Btrfs checkpoint 动态库及 TLS-sync runtime | 执行快照和识别 LLM 边界 |

## 测试步骤摘要

1. 创建隔离 Btrfs 文件系统，配置 backend 并启动 daemon。
2. 启动任务，检查初始保存，再验证手动保存和删除。
3. 发起 LLM 与普通 HTTPS 请求，核对保存时点和 Web 调用关联。
4. 在任务运行期间分别触发强制恢复和下一次 LLM 边界恢复，核对实际文件内容。
5. 结束 trace 并重启 daemon，核对快照释放、历史查询和恢复限制。
6. 停止服务，卸载文件系统并清理目录。

| 检查项 | 条件 | 预期结果 |
| --- | --- | --- |
| 保存边界 | 任务启动、手动保存或 LLM 请求 | 在对应边界保存，普通 HTTPS 请求不触发 |
| 运行中恢复 | 强制或下一次 LLM 边界恢复 | 任务继续运行并读到恢复后的文件 |
| 生命周期 | 删除、trace 结束或 daemon 重启 | 按场景释放快照并拒绝失效恢复，历史可查询 |

## 快速运行

- 公共入口先执行 `scripts/install-release.sh`，其中的 `cargo build --release` 会构建本仓库的 Btrfs backend 动态库。测例默认读取仓库的 `target/release/libactrail_ckpt_fs_btrfs.so`；设置 `CARGO_TARGET_DIR` 时读取对应构建目录。需要使用其他位置的动态库时，可用 `ACTRAIL_CHECKPOINT_BTRFS_LIBRARY` 指定其绝对路径。
- 内核能加载 Btrfs，并具备 `mkfs.btrfs`、`btrfs`、`findmnt`、`mount`、`curl` 和 `openssl`。curl 需链接 OpenSSL，TLS-sync runtime 库与 `actrailctl` 位于同一目录。

缺少工具或内核不支持 Btrfs 时判定为 `SKIPPED`；动态库缺失时判定为 `FAILED`。运行参数在 `constants.py`。

在仓库根目录、已配置 Rust 构建工具链的 root 终端中直接运行，无须预先单独构建或设置动态库路径：

```bash
python3.11 tests/e2e/regression/test_all.py --case checkpoint_btrfs
```
