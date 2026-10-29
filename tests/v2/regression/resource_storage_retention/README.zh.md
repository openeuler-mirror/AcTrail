# 资源样本过期清理专项

该清理验收在独立 SQLite 中预置两张样本表各 200,001 条过期样本、四条未过期样本和旧原型表，
随后启动真实 release daemon，让 50 ms 周期的 host 写入触发正式清理路径。
测试等待 `actrailctl list-traces` 成功响应后才开始读取资源库，保证 daemon 已完成数据库初始化，
避免测试读取与启动时日志模式转换竞争；测试不显式设置 WAL。
每批仅删除 32 行，清理间隔设为一小时，验证循环删除及积压续扫可以完成，而不是等待下一轮间隔。
测试只使用自己的运行目录，不需要真实 agent、Docker、cgroup 托管或浏览器。

默认回归的 `resource_metrics_cgroup` 在生命周期、存储和 API 检查通过后，将此验收作为 `retention`
小项执行，使用独立的子工作目录。下面的入口保留用于单独执行清理验收。

结果分别显示：

- `expired_samples`：两张表的过期积压均被清空；打印耗时，不把机器特定的性能值作为断言。
- `fresh_samples`：四条未过期样本逐字段保持不变。
- `profiles_and_boots`：仅有过期样本的 profile/boot 被回收；同时有新旧样本的和仅有新样本的保留。
- `metadata_keys`：保留 metadata 键字典。
- `legacy_table`：打开新存储时直接删除旧 `resource_samples` 表。

运行参数在 `constants.py`。30 秒等待上限用于判断清理是否完成，不断言每次 sweep 的墙钟耗时。

```bash
python3.11 tests/v2/regression/resource_storage_retention/run_e2e.py
```

也可以通过 `test_all.py --case resource_storage_retention` 显式选择。这个独立 case 不加入默认集合；
默认集合通过 `resource_metrics_cgroup` 的 `retention` 小项执行相同检查。
