# savelink-core — 恢复/存储核心（测试先行骨架）

这是 SaveLink 最高风险路径——**存档创建与恢复**——的 Rust 核心骨架。
它把 `savelink-restore-test-spec.md` 的用例落成了**可编译、可运行、当前为红灯**的真实测试。

目的：给任何实现者（Claude 或 Codex）一把客观的尺子。
> 谁把这些测试转绿，谁的实现就是对的——不靠嘴上说"我写得更稳"。

## 当前状态

- **零外部依赖**，可离线 `cargo test`。
- 测试现状：**29 红 / 2 绿**。
  - 2 个绿的是 `d3_*`：测试夹具自身的 content-hash 裁判（oracle）自检——它们必须一开始就绿，证明裁判可靠。
  - 其余 29 个红：等待实现 `service` 与 `store::FsStore` 后逐条转绿。

## 已实现 vs 待实现

| 模块 | 状态 | 说明 |
| --- | --- | --- |
| `model.rs` | ✅ 完成 | 领域类型，勿改字段语义 |
| `error.rs` | ✅ 完成 | 统一错误类型；`RestoreFailed{rolled_back}` 是契约的一部分 |
| `scan.rs` | ✅ 完成 | content_hash 权威算法。测试夹具复用它，**改哈希算法只改这一处** |
| `repo.rs` | ✅ 完成 | `InMemoryRepo` + 可注入 `Clock`/`IdGen`（测试用） |
| `testkit.rs` | ✅ 完成 | 故障注入 `FailingStore`、裁判 `dir_fingerprint`、`TempDir`、`corrupt_dir` |
| `store.rs` `FsStore` | ⛔ todo! | **实现这个**（ZipStore 的零依赖等价替身） |
| `service.rs` | ⛔ todo! | **实现这个**：`SnapshotService` / `RestoreService` / `startup_self_check` / `same_volume` |

## 给实现者的规则（重要）

1. **不要改测试来迁就实现。** 测试是验收基准。允许新增测试，不允许削弱既有断言。
2. **不要绕过故障注入。** `FailingStore`、损坏快照、Writing 残留等用例是整套规格存在的理由；让它们变绿的唯一正道是把事务/回滚逻辑写对。
3. **先红后绿。** 实现前确认 `cargo test` 是红的；逐模块实现，逐条转绿。
4. **P0 全绿才算可发布。** 见 `savelink-restore-test-spec.md` 验收矩阵。任一 P0 红灯，不得指向用户真实存档目录。
5. **关键路径交叉评审。** B 组、E 组实现完，由另一个 AI 审一个问题：「什么输入会让存档进入不可恢复状态？」

## 运行

```bash
cargo test --no-fail-fast          # 跑全部（看完整红绿图）
cargo test --test b_restore        # 只跑恢复组
cargo test d3_                     # 跑 oracle 自检（应为绿）
```

## 落地顺序建议

对应 `savelink-tech-architecture.md`「落地顺序」：

```
1. 实现 FsStore::create/restore/verify/delete   → D 组转绿
2. 实现 SnapshotService::create_snapshot         → A 组转绿
3. 实现 update_meta / delete_snapshot            → C 组转绿
4. 实现 RestoreService（备份→原子替换→校验）     → B 组转绿（最难，最后啃）
5. 实现 startup_self_check / same_volume         → E 组转绿
```

先打通"创建"链路，再啃"恢复"链路。恢复是命脉，留到地基稳了再动。

## 与文档的对应

- 做什么：`savelink-mvp-product-prototype.md`
- 怎么实现：`savelink-tech-architecture.md`
- 验收基准：`savelink-restore-test-spec.md`（本骨架是它的可执行版本）
