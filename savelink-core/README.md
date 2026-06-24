# savelink-core — 恢复/存储核心（已实现）

SaveLink 最高风险路径——**存档创建与恢复**——的纯 Rust 核心逻辑。
不依赖 Tauri，可独立 `cargo test`。被 `savelink-app` 的 Tauri 命令层薄壳包装后供前端调用。

> 历史：本 crate 最初是「测试先行骨架」（service/store 全 `todo!`、测试红灯），
> 用作客观验收尺。现已全部实现，33 个测试全绿。

## 当前状态

- 测试：**33 个全绿**（`cargo test --no-fail-fast`）。
- 依赖：`rusqlite 0.32`（`bundled`，SQLite 源码内编，用户无需安装；
  钉 0.32 是因为更高版本的 libsqlite3-sys 用了当前 rustc 未稳定的 `cfg_select`）。

## 模块地图

| 模块 | 状态 | 说明 |
| --- | --- | --- |
| `model.rs` | ✅ | 领域类型。**勿改字段语义**——前端 DTO、SQL schema、测试都依赖它 |
| `error.rs` | ✅ | 统一错误类型。`RestoreFailed{rolled_back}` 是对前端的契约，勿删字段 |
| `scan.rs` | ✅ | content_hash 权威算法（FNV-1a）。测试夹具复用它，**改哈希只改这一处** |
| `store.rs` | ✅ | `SnapshotStore` trait + `FsStore`（目录复制实现，zip 的零依赖等价替身） |
| `repo.rs` | ✅ | `Repository` trait + `InMemoryRepo`（测试用）+ 可注入 `Clock`/`IdGen` |
| `sqlite_repo.rs` | ✅ | `SqliteRepo`：生产用的落盘实现，与 InMemoryRepo 同 trait |
| `service.rs` | ✅ | `SnapshotService` / `RestoreService` / `startup_self_check` / `same_volume` |
| `testkit.rs` | ✅ | 故障注入 `FailingStore`、裁判 `dir_fingerprint`、`TempDir`、`corrupt_dir` |

## 改动这个 crate 的铁律（给后续开发者 / Codex）

1. **不要改测试来迁就实现。** 测试是验收基准。允许新增测试，不允许削弱既有断言。
2. **不要绕过故障注入。** `FailingStore`、损坏快照、Writing 残留等用例是整套规格存在的理由；
   让它们绿的唯一正道是把事务/回滚逻辑写对。
3. **存储抽象不可破。** 上层只认 `storage_key`（不透明）。换 zip/restic 时实现新的 `SnapshotStore`，
   不准让 service 去解析 key 的结构。换数据库同理（新实现 `Repository`）。
4. **恢复链路是命脉。** `RestoreService` 的「备份成功才覆盖 + 同盘原子 rename + rolled_back 语义」
   动了就要重证 B 组、E 组全绿。改这里前先读 `savelink-restore-test-spec.md` B/E 组。
5. **改完必须 `cargo test --no-fail-fast` 全绿才算数。**

## 运行

```bash
cargo test --no-fail-fast      # 全部（A 创建 / B 恢复 / C 删除 / D 存储 / E 自检 / F 持久化）
cargo test --test b_restore    # 只跑恢复组（命脉）
cargo test --test f_persistence # SQLite 落盘验证
```

## 与文档的对应

- 做什么：`../savelink-mvp-product-prototype.md`
- 怎么实现：`../savelink-tech-architecture.md`
- 验收基准：`../savelink-restore-test-spec.md`（本 crate 的测试是它的可执行版本）
- 整体进度与交接：`../PROGRESS.md`、`../HANDOFF-codex.md`
