# savelink-core — 恢复/存储核心（已实现）

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | 代强 | 2026-07-14 | 补齐版本历史；同步云状态仓库、协议 JSON、zip 编解码、CloudSyncService 与 G/H 组测试 |

SaveLink 最高风险路径——**存档创建与恢复**——的纯 Rust 核心逻辑。不依赖 Tauri，可独立 `cargo test`。`savelink-app` 的 Tauri 命令层只做 DTO 和调用包装。

> 历史：本 crate 最初是测试先行骨架，用作客观验收尺。现已实现，并由 A-F 组测试保护。

## 当前状态

- 测试：**50 个全绿**（`cargo test --no-fail-fast`）。
- 主要依赖：`rusqlite 0.32`、`serde/serde_json`、`zip 2`、`sha2`、`chrono`；SQLite 使用 `bundled`，用户无需单独安装。
- 生产 repo：`SqliteRepo`。
- 生产 store：`FsStore`，即目录复制实现；zip/restic 是后续优化。

## 测试分组

| 分组 | 数量 | 说明 |
| --- | ---: | --- |
| A 创建快照 | 8 | 正常创建、未变化、变化检测、失败回滚、空目录、不可读目录 |
| B 恢复 | 9 | 恢复前备份、目标损坏、替换恢复、回滚语义、缺失目录、进度顺序 |
| C 删除/锁定/游戏删除 | 6 | 锁定不可删、删除回滚、元数据可变、移除游戏 |
| D 存储/扫描 | 6 | create/restore 往返、verify、content_hash、storage_key 不透明 |
| E 启动自检/同盘检测 | 2 | writing 残留清理、同卷判断 |
| F SQLite 持久化 | 3 | 数据重开仍在、枚举往返、游戏更新持久化 |
| G 云同步基础 | 7 | 云状态持久化、状态转换、旧库补表、Fake 云对象操作和路径安全 |
| H Fake 云同步闭环 | 8 | 协议 JSON、zip 安全、A/B 双设备往返、幂等、孤儿与损坏拒绝 |

## 模块地图

| 模块 | 状态 | 说明 |
| --- | --- | --- |
| `model.rs` | 已实现 | 领域类型。字段语义会影响 DTO、SQL schema、测试 |
| `error.rs` | 已实现 | 统一错误类型。`RestoreFailed { rolled_back }` 是 UI 文案契约 |
| `scan.rs` | 已实现 | content_hash 权威算法（FNV-1a） |
| `store.rs` | 已实现 | `SnapshotStore` trait + `FsStore` 目录复制实现 |
| `repo.rs` | 已实现 | `Repository` trait + `InMemoryRepo` 测试替身 + Clock/IdGen |
| `sqlite_repo.rs` | 已实现 | `SqliteRepo` 落盘实现 |
| `cloud_model.rs` | 已实现 | 云账号、游戏绑定、远端快照缓存和同步状态模型 |
| `cloud_repo.rs` | 已实现 | `CloudStateRepository` 本机云状态持久化接口 |
| `cloud_store.rs` | 已实现 | `CloudObjectStore` 与文件系统 `FakeCloudObjectStore` |
| `cloud_protocol.rs` | 已实现 | v1 JSON 契约、逻辑路径、ID/hash/时间校验 |
| `cloud_archive.rs` | 已实现 | 单快照 zip、SHA-256 和安全解压 |
| `cloud_service.rs` | 已实现 | Fake/真实云后端共用的上传、发现、下载和接收落地编排 |
| `service.rs` | 已实现 | `SnapshotService`、`RestoreService`、`startup_self_check`、`same_volume` |
| `testkit.rs` | 已实现 | 故障注入、临时目录、指纹裁判、损坏模拟 |

## 核心安全契约

1. 恢复前必须先校验目标快照。
2. 目标损坏时不碰真实存档。
3. 恢复前必须强制生成 before_restore 备份。
4. 备份失败则中止恢复，不覆盖真实存档。
5. 覆盖恢复采用同盘临时目录 + rename 替换。
6. 失败时 `RestoreFailed { rolled_back }` 的语义必须准确。
7. 存档目录缺失时，默认返回“需要用户选择”，不静默创建。
8. 选择“创建并恢复”时才创建目录并恢复。
9. 快照内容不可变，可变的只有备注和锁定状态。
10. 上层只认 `storage_key`，不得解析 store 的内部布局。

## 改动这个 crate 的铁律

1. 不要改测试来迁就实现。允许新增测试，不允许削弱既有断言。
2. 不要绕过故障注入。`FailingStore` 是事务/回滚逻辑的验收工具。
3. 存储抽象不可破。换 zip/restic 时实现新的 `SnapshotStore`，不要让 service 理解 zip 文件名。
4. 恢复链路是命脉。动 `RestoreService` 前先读 `../savelink-restore-test-spec.md` 的 B/E 组。
5. 改完必须跑：

```bash
cargo test --no-fail-fast
```

## 常用命令

```bash
cargo test --no-fail-fast       # 全部 A-F 组
cargo test --test b_restore     # 只跑恢复组
cargo test --test c_delete_lock # 删除/锁定/删除游戏
cargo test --test f_persistence # SQLite 落盘验证
cargo test --test g_cloud_foundation # 云同步本机状态与假云端基础设施
cargo test --test h_cloud_sync # 设备 A -> Fake 云端 -> 设备 B 闭环
```

## 与文档的对应

- 做什么：`../savelink-mvp-product-prototype.md`
- 怎么实现：`../savelink-tech-architecture.md`
- 验收基准：`../savelink-restore-test-spec.md`
- 整体进度与交接：`../PROGRESS.md`、`../HANDOFF-codex.md`
