# SaveLink 恢复与存储测试规格

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | Claude | 2026-06-23 | 第一版：定义 SaveLink 本地 MVP 中「快照创建 / 恢复 / 删除 / 存储层」关键路径的测试用例，作为测试先行（TDD）的客观验收基准 |
| 1.1 | Codex | 2026-07-01 | 同步当前可执行测试：34 个全绿；补充删除游戏、游戏更新持久化、缺失目录创建恢复的实现状态 |
| 1.2 | Codex | 2026-07-02 | 基线收口：同步当前已实现状态，保留本规格作为后续回归验收基准 |
| 1.3 | 代强 | 2026-07-14 | 同步当前 50 个 core 测试；G/H 组云同步测试不改变既有恢复验收基准 |
| 1.4 | 代强 | 2026-07-28 | 第一版移除恢复前自动保护点；补充无新增快照、历史兼容和最终校验失败回滚；同步 68 个默认测试 |
| 1.5 | 代强 | 2026-07-29 | 迁入 doc 目录并改为中文文件名；同步文档引用 |
| 1.6 | 代强 | 2026-08-05 | 补充 Deleting 中断续做、旧云状态迁移、联合删除失败重试和自动备份 M 组；同步 75 个默认测试 |

> **实现状态（2026-08-05）**：本规格已落成 `savelink-core/tests/` 下的可执行 Rust 测试并**全部转绿**。
> A-F 组继续保护创建、恢复、删除、存储、自检和 SQLite 持久化；G-L 组及内部单元测试保护云同步基础设施、百度适配和 OAuth；M 组保护自动检查与保留候选。当前共 75 个默认测试，另有 2 个真实百度测试默认忽略。
> 本文档继续作为**验收基准**：改动 `savelink-core` 后须保持这些断言全绿；
> 加新功能须按同样标准补测试。详见 `savelink-core/README.md`。

## 文档用途

本文档是 SaveLink 最高风险路径——**存档的创建与恢复**——的测试规格。

它的目的只有一个：

> 提供一把不依赖任何 AI 主观判断的尺子，客观回答「用户存档到底有没有丢」。

配套文档：

- `SaveLink技术架构.md`：定义被测对象（`SnapshotStore`、`RestoreService`、事务设计、错误类型）。
- `SaveLink-MVP产品原型草案.md`：定义被测对象必须遵守的 5 条安全规则。

**测试先行原则**：新增或重写核心能力时，应先把对应用例写成可执行测试，再让实现转绿。当前 MVP 已实现，通过这套测试仍是后续合并的前提。

## 被测对象与边界

本规格覆盖 Rust 核心服务层，**不经过 Tauri、不经过前端**：

```text
SnapshotService.create_snapshot
RestoreService.restore_snapshot
SnapshotService.delete_snapshot
SnapshotStore（当前 FsStore 目录复制实现；ZipStore/ResticStore 是后续替换项）
Scanner（扫描 / 哈希 / content_hash）
```

不在本规格范围内（另行测试）：前端渲染、IPC 序列化、UI 状态、网络/云端。

## 测试环境约定

- 测试框架：Rust 内置 `#[test]` + `tempfile` crate 提供隔离临时目录。每个用例自带 setup/teardown，互不影响。
- 所有「真实存档目录」「快照仓库」均在 `tempfile::tempdir()` 下创建，测试结束自动清理。不得触碰用户真实路径。
- 时间、ID 等不确定量通过可注入的 `Clock` / `IdGen` 控制，使断言可重复。
- 结构遵循 Arrange-Act-Assert。

### 共用测试夹具

```text
fn make_save_dir(files: &[(相对路径, 字节内容)]) -> 临时目录
fn make_repo() -> 临时仓库目录 + 干净的 SQLite
fn dir_fingerprint(dir) -> 对目录做与 content_hash 同算法的指纹，用于"内容一致"断言
fn corrupt_snapshot(key) -> 故意破坏某快照文件或 ok 标记，模拟损坏
fn read_snapshots(db, game_id) -> 时间线列表
```

`dir_fingerprint` 是关键裁判工具：恢复后用它比对「恢复目录」与「目标快照解出内容」是否逐字节一致。

## 用例命名与优先级

- `P0`：丢失/损坏用户存档的风险，必须全绿才能合并。
- `P1`：核心功能正确性。
- `P2`：体验与边界。

---

## A 组：创建快照（SnapshotService.create_snapshot）

### A1 [P1] 正常创建：内容与统计正确

```text
Arrange: 存档目录含 3 个文件，总计 12,345 字节
Act:     create_snapshot(game, note="Boss 前")
Assert:
  · 返回快照 file_count == 3
  · total_size == 12345
  · 时间线新增 1 条，reason == "manual"
  · 该快照解出后的指纹 == 原存档目录指纹（内容无损）
```

### A2 [P1] 存档未变化：不创建重复快照

```text
Arrange: 已有一个快照；存档目录文件未做任何改动
Act:     再次 create_snapshot
Assert:
  · 返回 NoChange 分支（非错误）
  · 时间线数量不变
  · 对应原型提示「存档未变化，未创建新快照」
```

### A3 [P1] 任一文件变化即视为有变化

```text
Arrange: 已有快照后，修改其中 1 个文件 1 个字节 / 或新增 1 文件 / 或删除 1 文件（三个子用例）
Act:     create_snapshot
Assert:  每个子用例都成功创建新快照，content_hash 与上一快照不同
```

### A4 [P0] 创建中途失败不留半成品

```text
Arrange: 注入 SnapshotStore.create 在写入过程中抛 IO 错
Act:     create_snapshot 预期失败
Assert:
  · 仓库中不存在该快照的残留 zip 文件
  · snapshots 表中无该记录（或被回滚）
  · 数据库中不存在 status=="writing" 的悬挂记录
  · 真实存档目录完全未被触碰
```

### A5 [P2] 空目录可创建但有提示

```text
Arrange: 存档目录存在但为空
Act:     create_snapshot
Assert:  允许创建，file_count == 0，total_size == 0（对应原型「目录为空仍可保存」）
```

### A6 [P0] 存档目录不可读时不写任何状态

```text
Arrange: 存档目录不存在 / 无读取权限
Act:     create_snapshot 预期返回 SaveDirMissing / SaveDirUnreadable
Assert:  不创建记录、不创建文件、不修改时间线
```

---

## B 组：恢复（RestoreService.restore_snapshot）—— 本规格的核心

每条都以「真实存档目录的内容」为最终裁判，因为这才是用户真正在乎的东西。

### B1 [P0] 恢复不自动创建快照

```text
Arrange: 时间线有目标快照 T；真实存档当前为不同且尚未保存的状态 S(current)
Act:     restore_snapshot(target=T)
Assert:
  · 返回 restored==true
  · 时间线条数不变，不产生 reason=="before_restore" 的新记录
  · 真实存档 == T
```

这是第一版明确的产品取舍：成功恢复后若要回到 S(current)，用户必须事先手动创建快照。

### B2 [P0] 目标文件准备失败 → 真实存档原封不动

```text
Arrange: 注入 SnapshotStore.restore 失败
Act:     restore_snapshot 预期返回 RestoreFailed{rolled_back=true}
Assert:
  · 真实存档目录指纹 == 操作前指纹（一字节未改）
  · 未执行任何覆盖
  · 时间线不新增快照
```

### B3 [P0] 恢复成功后真实存档 == 目标快照内容

```text
Arrange: 目标快照 T 内容已知；当前真实存档为不同的 S(current)
Act:     restore_snapshot(target=T) 成功
Assert:
  · 真实存档目录指纹 == T 解出内容指纹（逐字节一致）
  · 真实存档中不残留任何只属于 S(current) 的文件（即覆盖是"替换"而非"合并"）
  · restore_logs result == "success"
```

「不残留旧文件」单列断言：合并式恢复是常见隐藏 bug，必须显式排除。

### B4 [P0] 目标快照损坏 → 不动真实存档

```text
Arrange: corrupt_zip(T) 破坏目标快照；真实存档为 S(current)
Act:     restore_snapshot(target=T) 预期返回 SnapshotCorrupt
Assert:
  · 因恢复前 verify 不通过而中止
  · 真实存档目录指纹 == 操作前指纹
  · 时间线不新增快照
```

### B5 [P0] 覆盖阶段中途崩溃 → 不留下半残存档

```text
Arrange: 注入「解包/替换」阶段在中途失败或被中断
Act:     重新进入应用并触发一致性检查
Assert:
  · 真实存档目录要么是完整的 S(current)（回滚成功），要么是完整的 T
  · 不允许出现"一半 S 一半 T"的混合态
  · 时间线不新增快照
```

B5 是 rename 原子替换策略存在的全部理由，必须有用例钉死。

### B6 [P0] 恢复失败的错误必须携带 rolled_back 语义

```text
Arrange: 注入覆盖阶段失败
Act:     restore_snapshot 返回 RestoreFailed
Assert:
  · 错误携带 rolled_back: bool
  · rolled_back==true  时：真实存档指纹 == 操作前指纹
  · rolled_back==false 时：UI 必须明确要求用户核对真实存档目录
```

### B7 [P0] 真实存档目录不存在 → 不自动创建恢复

```text
Arrange: 真实存档目录被删除/不存在
Act:     restore_snapshot
Assert:
  · 返回需用户决策的状态（创建并恢复 / 重选目录 / 取消），而非静默创建并写入
  · 在用户未确认前，不在该路径写入任何文件（安全规则 5）
```

当前实现状态：

- `restore_snapshot` 返回 `SaveDirMissingNeedsChoice`。
- 前端已接“创建目录并恢复”和“取消”。
- 后端 `restore_with_choice(CreateAndRestore)` 会创建目录并恢复，不创建任何快照。
- “重新选择目录并恢复”后端枚举存在，但 UI 未做完整闭环。

### B8 [P1] 可在两个已有快照之间往返恢复

```text
Arrange: 时间线上事先存在内容不同的快照 A 和 B
Act:     当前为 B 时恢复 A，再恢复 B
Assert:
  · 最终真实存档 == B
  · 两次恢复均不新增快照
```

### B9 [P1] 恢复进度事件顺序正确

```text
Act:    restore_snapshot 监听进度
Assert: 事件依次为 restore_target → verify
        已经等于目标的无操作恢复不发出虚假进度
```

### B10 [P1] 从空目录恢复也不创建快照

```text
Arrange: 当前真实存档目录存在但为空，目标快照 T 非空
Act:     restore_snapshot(target=T)
Assert:
  · 返回 restored==true
  · 时间线条数不增加
  · 恢复后真实存档 == T
```

### B11 [P1] 当前已经等于目标版本时不执行恢复

```text
Arrange: 当前真实存档指纹 == 目标快照 T 的指纹
Act:     restore_snapshot(target=T)
Assert:
  · 返回 restored==false
  · 不调用 store.restore，不创建快照，不发出恢复进度
  · 真实存档和时间线均不变化
```

### B12 [P0] 旧版 before_restore 快照仍可恢复

```text
Arrange: 数据库存在旧版本生成的 reason=="before_restore" 快照 B
Act:     restore_snapshot(target=B)
Assert:
  · B 可正常读取和恢复
  · 不新增快照
```

### B13 [P0] 最终校验失败时必须换回原目录

```text
Arrange: 目标临时目录已校验，rename 替换完成；在最终校验前篡改新目录
Act:     最终校验失败
Assert:
  · 删除错误的新目录并 rename .old -> 真实存档目录
  · 返回 RestoreFailed{rolled_back=true}
  · 真实存档指纹 == 操作前 S(current)
  · 时间线不新增快照
```

---

## C 组：删除与锁定（安全规则 4）

### C1 [P0] 锁定快照不可删除

```text
Arrange: 快照 locked == true
Act:     delete_snapshot 预期返回 SnapshotLocked
Assert:  记录与文件均仍存在；时间线不变
```

### C2 [P1] 取消锁定后可删除

```text
Arrange: 先 update_snapshot_meta(locked=false)
Act:     delete_snapshot 成功
Assert:  记录与对应 zip 文件均被删除
```

### C3 [P0] 删除时先删文件再删记录，失败回滚不留悬挂

```text
Arrange: 注入 SnapshotStore.delete 失败
Act:     delete_snapshot 预期失败
Assert:
  · snapshots 记录仍存在（未出现"有记录无文件"或"有文件无记录"的悬挂）
  · 时间线与操作前一致
```

### C4 [P2] 元数据可变、内容不可变（安全规则 3）

```text
Act:    update_snapshot_meta 修改 note / locked
Assert: note、locked 改变；created_at、content_hash、file_count、total_size 不变
        且对应 zip 文件字节不变
```

### C5 [P1] 删除游戏会移除元数据和仓库快照，但保留真实存档

```text
Arrange: 某游戏已有快照，真实存档目录存在
Act:     delete_game(game_id)
Assert:
  · 游戏记录被删除
  · 该游戏快照记录被删除
  · 对应快照仓库文件被删除
  · 真实存档目录仍存在，内容不被删除
```

### C6 [P0] 删除游戏时快照文件删除失败则中止

```text
Arrange: 注入 SnapshotStore.delete 失败
Act:     delete_game(game_id) 预期失败
Assert:
  · 游戏记录仍存在
  · 尚未成功删除的快照记录仍存在
  · 不出现数据库与仓库大面积不一致
```

---

## D 组：存储层与扫描（SnapshotStore / Scanner）

### D1 [P1] create → restore 往返内容无损

```text
Arrange: 源目录含含子目录、空文件、中文文件名、较大文件
Act:     store.create(sources) 得到 key；store.restore(key, 新目录)
Assert:  新目录指纹 == 源目录指纹（含目录结构、空文件、文件名编码）
```

### D2 [P1] verify 能识别完好与损坏

```text
Act/Assert:
  · 正常快照 verify(key) == true
  · corrupt_zip 后 verify(key) == false
```

### D3 [P1] content_hash 对顺序/路径稳定

```text
Arrange: 同一组文件以不同枚举顺序构造两次
Act:     分别计算 content_hash
Assert:  两次 content_hash 相同（排序后再哈希，保证可重复，支撑 A2 未变化判断）
```

### D4 [P2] storage_key 对上层不透明

```text
Assert: 上层（service / db）从不解析 storage_key 的内部结构，仅原样传回 store
        —— 通过代码审查 + 一个"换 key 命名规则不影响 service 测试"的用例保证
        这是未来替换为 ResticStore 的解耦保证
```

### D5 [P2] 跨平台路径与权限

```text
Assert:
  · Windows 反斜杠路径与 glob 处理正确
  · 恢复后文件可读（权限位不破坏存档可用性）
```

---

## E 组：中断恢复与启动自检

### E1 [P0] 启动时清理残留 writing 快照

```text
Arrange: 数据库存在 status=="writing" 的快照（模拟上次创建中崩溃）
Act:     应用启动自检
Assert:
  · 该悬挂记录被清理或标记 corrupt，不出现在正常时间线
  · 对应的半成品文件被清除
```

### E2 [P0] 同盘 rename 替换的原子性前提成立

```text
Assert:
  · 临时目录 target.tmp 与真实存档目录处于同一卷（保证 rename 原子）
  · 若检测到跨卷（无法原子 rename），实现必须改用"先备份后复制 + 失败回滚"的安全降级，
    且该降级路径同样满足 B5（不留半残态）
```

E2 钉死架构文档的关键假设：rename 原子性依赖同卷。跨卷必须有安全降级，否则 B5 在某些用户机器上会悄悄失效。

### E3 [P0] 启动时续做 deleting 快照

```text
Arrange: 数据库存在 status=="deleting" 的快照（模拟云端已删、本机删除阶段崩溃）
Act:     应用启动自检
Assert:
  · 幂等删除对应本机快照文件
  · 删除 snapshots 悬挂记录
  · 不触碰真实游戏存档目录
```

---

## 验收矩阵（合并前必须全绿）

| 风险 | 用例 |
| --- | --- |
| 恢复意外创建快照 | B1, B2, B7, B8, B10, B12, B13 |
| 恢复后存档不等于目标 | B3 |
| 恢复出"半残/混合"存档 | B5, B6, B13, E2 |
| 损坏快照污染真实存档 | B4 |
| 静默创建/覆盖不存在的目录 | B7 |
| 创建或删除中断留垃圾/悬挂 | A4, E1, E3, C3 |
| 锁定快照被误删 | C1 |
| 移除游戏误删真实存档 | C5 |
| 移除游戏中途失败导致悬挂 | C6 |
| 存档目录不可读仍写状态 | A6 |
| 云删除失败却误删本地 | H10 |
| 锁定记录被计入可配置上限淘汰 | M3 |

## F 组：SQLite 持久化

### F1 [P1] 数据关闭重开仍在

```text
Arrange: 写入游戏与快照
Act:     关闭连接，重新打开同一个 .db
Assert:  游戏、快照仍能读回
```

### F2 [P1] 枚举字段往返正确

```text
Assert: reason / status / locked 等字段经过 SQLite 持久化后语义不变
```

### F3 [P1] 游戏更新持久化

```text
Arrange: 已有游戏
Act:     update_game 修改名称与 save_paths，关闭并重新打开 .db
Assert:  修改后的名称和路径仍在
```

**P0 全部为绿，是「这版可以碰用户真实存档」的最低门槛。** 任一 P0 红灯，MVP 不得发布、不得指向真实存档目录。

## 给两个实现者的统一约定

- 无论 Claude 还是 Codex 实现 `restore_service` / `snapshot_service`，都先让本规格的 P0 用例存在并保持红灯，再写实现。
- 关键路径（B 组、E 组）由**另一个 AI 交叉评审**，只回答一个问题：「在什么输入下，这段代码会让用户存档进入不可恢复状态？」
- 最终对错由本规格判定，不由任何一方的主观说明判定。

## 当前结论

这套规格把「不丢存档」这句承诺，翻译成了一组可以亮红绿灯的断言。

> 谁的实现能让 B 组、E 组的 P0 用例全部转绿，谁的实现就是对的——
> 这比任何一个 AI 嘴上说"我写得更稳"都更可信。

当前下一步不是生成测试骨架，而是在每次改动后继续保持 `savelink-core/tests/` 全绿，并为新增核心能力补同级别测试。


