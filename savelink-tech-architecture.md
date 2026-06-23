# SaveLink 技术架构草案

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | Claude | 2026-06-23 | 第一版：定义 SaveLink 本地 MVP 的技术栈、模块划分、数据模型、存储层抽象与恢复流程的事务/安全设计 |

## 文档用途

本文档定义 SaveLink 第一阶段（本地 MVP）的技术实现架构。

配套文档：

- `savelink-mvp-product-prototype.md`：做什么（产品流程、功能范围、安全规则）。
- `savelink-low-fidelity-wireframe.md`：页面怎么组织。
- `savelink-visual-interaction-guidelines.md`：界面什么气质。
- `demo-front-1/index.html`：高保真原型（纯前端、mock 数据）。

本文档回答最后一个问题：**怎么实现**。

## 设计第一性原则

SaveLink 的核心承诺是「不丢用户存档」。所有技术选择都服从这一条：

1. **任何会覆盖真实存档的操作，前面必须有一个已成功落盘的回退点。**
2. **写操作要么完整成功，要么完整回滚，不允许中间态。**
3. **快照内容一旦写入即不可变；可变的只有元数据（备注、锁定）。**
4. **存储格式与上层逻辑解耦，以便 zip 起步、未来无痛替换为去重引擎。**

性能、美观、功能丰富度都排在这四条之后。

## 技术栈定稿

```text
桌面壳     Tauri 2.x
前端       React 18 + TypeScript + Vite
样式       Tailwind CSS + shadcn/ui 风格组件
图标       lucide-react
核心逻辑   Rust（文件扫描、哈希、打包、恢复）
元数据     SQLite（rusqlite，bundled 模式）
快照存储   zip 起步，存储层抽象隔离，未来可换 restic
文件监听   notify crate（阶段 2 自动快照才引入）
```

选型理由见本文档末「选型取舍」一节。先用 mock 数据完成前端，再接 Tauri command。

## 总体分层

```text
┌─────────────────────────────────────────────────────────┐
│  前端 (React + TS)  —— 渲染、交互、状态、确认弹窗          │
│  · 不碰文件系统  · 不做存档逻辑  · 只通过 invoke 调命令     │
└───────────────────────────┬─────────────────────────────┘
                            │ Tauri IPC (invoke / event)
┌───────────────────────────┴─────────────────────────────┐
│  Tauri Command 层 (Rust)  —— 边界、参数校验、错误归一化     │
└───────────────────────────┬─────────────────────────────┘
                            │
┌───────────────────────────┴─────────────────────────────┐
│  核心服务层 (Rust)                                         │
│  ┌────────────┐ ┌────────────┐ ┌────────────────────┐    │
│  │ GameService│ │SnapshotSvc │ │  RestoreService    │    │
│  └─────┬──────┘ └─────┬──────┘ └─────────┬──────────┘    │
│        │              │                  │               │
│  ┌─────┴──────────────┴──────────────────┴─────────┐     │
│  │  Repository (元数据)        SnapshotStore (文件)  │     │
│  │  └ SQLite                   └ trait, zip 实现     │     │
│  └──────────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────────┘
```

关键纪律：**前端永远不直接触碰文件系统或真实存档目录**。所有读写都经过 Rust 命令层。这样安全规则只需在一处保证。

## 前端架构

### 目录结构

```text
src/
├── app/
│   ├── App.tsx
│   └── routes.tsx              // 单窗口，视图切换而非真路由
├── features/
│   ├── games/
│   │   ├── GameSidebar.tsx
│   │   ├── GameDetail.tsx
│   │   └── AddGameDialog.tsx
│   ├── snapshots/
│   │   ├── Timeline.tsx
│   │   ├── SnapshotCard.tsx
│   │   └── SnapshotDrawer.tsx
│   └── restore/
│       ├── RestoreConfirmDialog.tsx
│       └── RestoreProgress.tsx
├── lib/
│   ├── ipc.ts                  // 对 invoke 的薄封装 + 类型
│   ├── format.ts               // 大小、时间格式化
│   └── types.ts                // 与 Rust 对齐的 DTO 类型
├── hooks/
│   ├── useGames.ts
│   └── useSnapshots.ts
└── styles/
    └── tokens.css
```

### 状态管理

| 关注点 | 方案 |
| --- | --- |
| 服务端（Rust）状态 | TanStack Query —— 缓存游戏/快照列表，命令成功后 invalidate 重拉 |
| 客户端 UI 状态 | 组件内 useState / 轻量 Zustand（当前选中游戏、抽屉开关） |
| 长任务进度 | Tauri event 监听 —— 创建快照、恢复进度通过事件推送 |

原则：**不把 Rust 返回的数据复制进客户端 store**。列表数据始终来自 Query 缓存，命令执行后让 Query 失效重取，避免前后端状态漂移。

### 进度反馈

创建快照和恢复是耗时操作。前端 `invoke` 发起后，Rust 通过 `app.emit` 推送进度事件：

```text
invoke("restore_snapshot", { gameId, snapshotId })
  → 监听 event "restore:progress"  { step, status }
  → 步骤：backup_current → restore_target → verify
  → 完成后命令 resolve，前端 invalidate 时间线
```

这对应原型里恢复弹窗的三步进度条。

## 后端架构（Rust）

### Crate / 模块结构

```text
src-tauri/src/
├── main.rs
├── commands/              // Tauri command，唯一对前端暴露的入口
│   ├── games.rs
│   ├── snapshots.rs
│   └── restore.rs
├── services/              // 业务逻辑，不感知 Tauri
│   ├── game_service.rs
│   ├── snapshot_service.rs
│   └── restore_service.rs
├── store/                 // 存储抽象
│   ├── mod.rs             // SnapshotStore trait
│   └── zip_store.rs       // zip 实现
├── repo/                  // 元数据持久化
│   ├── mod.rs             // Repository trait
│   └── sqlite_repo.rs
├── scan/                  // 目录扫描、哈希、大小统计
│   └── scanner.rs
├── model.rs               // 领域类型
└── error.rs               // 统一错误类型 SaveLinkError
```

分层依赖方向单向向下：`commands → services → (store + repo + scan)`。services 不依赖 Tauri，便于单元测试。

### 数据模型

与原型 mock 字段一致，落到 SQLite：

```sql
CREATE TABLE games (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  icon        TEXT,
  repo_path   TEXT NOT NULL,        -- 该游戏快照仓库根目录
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE save_paths (
  id          TEXT PRIMARY KEY,
  game_id     TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
  path        TEXT NOT NULL,
  label       TEXT,
  include_glob TEXT,                -- 预留，MVP 不出 UI
  exclude_glob TEXT
);

CREATE TABLE snapshots (
  id           TEXT PRIMARY KEY,
  game_id      TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
  created_at   TEXT NOT NULL,
  note         TEXT,
  reason       TEXT NOT NULL,        -- manual | before_restore | auto
  locked       INTEGER NOT NULL DEFAULT 0,
  file_count   INTEGER NOT NULL,
  total_size   INTEGER NOT NULL,     -- 字节
  content_hash TEXT NOT NULL,        -- 整快照内容指纹，用于"未变化"判断
  storage_key  TEXT NOT NULL,        -- 交给 SnapshotStore 解析，不假设是路径
  status       TEXT NOT NULL DEFAULT 'complete'  -- complete | writing | corrupt
);

CREATE TABLE restore_logs (
  id            TEXT PRIMARY KEY,
  game_id       TEXT NOT NULL,
  target_id     TEXT NOT NULL,       -- 恢复到哪个快照
  backup_id     TEXT NOT NULL,       -- 恢复前自动备份的快照
  started_at    TEXT NOT NULL,
  finished_at   TEXT,
  result        TEXT NOT NULL        -- success | aborted | failed
);
```

注意 `storage_key` 而非 `storage_path`：上层只持有一个不透明 key，由 `SnapshotStore` 决定它是 zip 文件名还是 restic 的 snapshot id。这是替换存储引擎的关键解耦点。

`status` 字段实现「写入中」防护：快照记录先以 `writing` 落库，文件写完校验通过才置为 `complete`。启动时扫到残留的 `writing` 记录即判定为上次中断，清理掉。

### 存储层抽象（架构成败手）

```rust
/// 快照文件的存与取。上层只认 storage_key，不关心底层是 zip 还是去重仓库。
pub trait SnapshotStore: Send + Sync {
    /// 把若干源目录打包成一个快照，返回 storage_key 与统计信息。
    fn create(&self, sources: &[PathBuf], ctx: &ScanResult) -> Result<StoredSnapshot>;

    /// 把指定快照解出到目标目录（恢复用）。必须是覆盖式、可中断后可识别。
    fn restore(&self, key: &str, target: &Path) -> Result<()>;

    /// 校验快照完整性（恢复前/创建后调用）。
    fn verify(&self, key: &str) -> Result<bool>;

    /// 删除快照物理文件。
    fn delete(&self, key: &str) -> Result<()>;
}
```

- **MVP 实现 `ZipStore`**：一快照一个 zip，`storage_key` 即 `{snapshot_id}.zip`。
- **未来 `ResticStore`**：`create` 调 restic CLI 备份，`storage_key` 存 restic snapshot id；去重、压缩、加密全部由 restic 负责。
- 关键：**不要自研去重/增量**。自研内容寻址一旦写错就是损坏用户数据，restic 已经为此磨了多年。

仓库目录布局（zip 阶段）：

```text
{repo_path}/
  games/{game_id}/
    snapshots/{snapshot_id}.zip
    metadata/{snapshot_id}.json   // 冗余一份元数据，便于仓库自描述/迁移
  savelink.db                     // 也可放用户数据目录，二选一见下
```

数据库位置二选一：放仓库内便于整体迁移；放系统 AppData 便于多仓库共享。MVP 建议放仓库内，保持「一个仓库自包含」的心智。

## 核心流程的事务设计

### 创建快照

```text
1. 校验存档目录存在且可读           （失败 → 直接报错，不写库）
2. 扫描：枚举文件、累计大小、算 content_hash
3. 与上一快照 content_hash 比对
     相同 → 返回 "存档未变化"，不创建      （对应原型提示）
4. 在事务内插入 snapshots 记录，status = 'writing'
5. SnapshotStore.create() 打包落盘
6. SnapshotStore.verify() 校验
     失败 → 删除半成品文件 + 删除记录，报错
7. 事务内更新 status = 'complete'
8. 返回新快照
```

`content_hash` 怎么算：对所有文件按相对路径排序，逐个累计「相对路径 + 文件内容 hash」得到整体指纹。只要任一文件变化或增删，指纹就变。这支撑「存档未变化则不重复创建」。

### 恢复快照（最高风险路径）

这是整个产品的生命线，对应安全规则 2「恢复前必须自动备份，备份失败则中止」。

```text
前置检查
  · 目标快照存在且 verify 通过      （损坏 → 中止，不动真实存档）
  · 真实存档目录存在
       不存在 → 不自动创建，回前端让用户选择
                （创建并恢复 / 重选目录 / 取消）—— 安全规则 5

第 1 步：备份当前存档  ← 必须成功才继续
  · 以 reason = 'before_restore' 走一遍"创建快照"
  · 失败 → 整个恢复中止，真实存档原封不动
  · 成功 → 记下 backup_id

第 2 步：恢复目标版本（覆盖写，需可回滚）
  · 不直接清空真实目录再解包（中途崩溃 = 数据全没）
  · 改用"旁路 + 原子替换"：
      a. 解包目标快照到同盘临时目录 target.tmp
      b. 校验 target.tmp 内容
      c. 把现有真实目录 rename 为 target.old
      d. 把 target.tmp rename 为真实目录
      e. 删除 target.old
  · 任一步失败 → 用已 rename 的 target.old 回滚，并保留 before_restore 快照

第 3 步：校验恢复结果
  · 比对恢复后目录 hash 与目标快照 hash

写 restore_logs：result = success / aborted / failed
前端时间线顶部出现新的 "恢复前自动备份"
```

为什么用 rename 原子替换而不是「清空再写入」：`rename` 在同一磁盘分区上是原子操作，进程崩溃也不会留下"删了一半、写了一半"的真实存档。这是把「不静默损坏存档」从口号落到代码的关键。代价是恢复瞬间需要约 2 倍存档大小的磁盘空间——对存档（MB 级）完全可接受。

### 删除快照

```text
· locked = 1 → 直接拒绝（安全规则 4 / 视觉文档：先取消锁定）
· 事务内先删 SnapshotStore 文件，再删 snapshots 记录
· 删文件失败 → 回滚，记录不删，避免出现"有记录无文件"的悬空快照
```

## 错误处理与状态归一

Rust 侧统一错误类型，前端按 kind 渲染对应状态（对应视觉文档「状态设计」清单）：

```rust
pub enum SaveLinkError {
    SaveDirMissing,      // 存档目录不存在
    SaveDirUnreadable,   // 无法访问
    NoChange,            // 存档未变化（非错误，作正常分支返回）
    BackupFailed,        // 恢复前备份失败 → 恢复中止
    RestoreFailed { rolled_back: bool },  // 是否已回滚，前端要据此提示
    SnapshotCorrupt,     // 快照校验不通过
    SnapshotLocked,      // 锁定快照不可删
    Io(String),
}
```

`RestoreFailed.rolled_back` 直接对应视觉文档要求的「恢复失败必须说明：当前存档是否已备份、是否已开始覆盖、下一步怎么办」。前端据此给出准确而非含糊的失败文案。

## Tauri Command 清单（前端契约）

```text
list_games() -> Game[]
add_game(name, save_paths[], repo_path?) -> Game
update_game(game_id, patch) -> Game
delete_game(game_id) -> void

list_snapshots(game_id) -> Snapshot[]
create_snapshot(game_id, note?) -> CreateResult   // 含 NoChange 分支
update_snapshot_meta(snapshot_id, { note?, locked? }) -> Snapshot
delete_snapshot(snapshot_id) -> void

scan_path(path) -> { file_count, total_size, readable }   // 添加游戏的"测试读取"
restore_snapshot(game_id, snapshot_id) -> RestoreResult   // 进度走 event
```

事件：`snapshot:progress`、`restore:progress`。

前端 `lib/types.ts` 的 DTO 必须与这些返回类型逐字段对齐，这也是当前原型 mock 数据已经预演过的结构。

## 选型取舍

| 决策 | 选择 | 取舍说明 |
| --- | --- | --- |
| 壳 | Tauri 而非 Electron | 体积/内存更小、后端 Rust。唯一前提：团队接受 Rust 学习成本。若是纯 JS 团队且追求最快出活，Electron + Node 也成立 |
| 核心逻辑 | Rust 而非 Node | 覆盖/恢复是会毁存档的操作，Rust 的正确性与原子文件操作能力在此是真实价值，不是炫技 |
| 存储 | zip 起步，trait 隔离 | 简单、可移植、用户可理解"一个快照包"。代价是无去重；用接口隔离，撑不住时换 restic |
| 去重引擎 | 未来接 restic，不自研 | 自研内容寻址风险极高（写错=损坏数据）；restic 是单静态二进制，易打包进 Tauri |
| 游戏识别 | 复用 Ludusavi manifest | 阶段 3 的存档路径库直接消费其开放 YAML，不依赖 Ludusavi 二进制，零成本吸收能力 |
| 恢复策略 | rename 原子替换 | 防止中途崩溃损坏真实存档；代价是临时 2x 磁盘占用，对 MB 级存档可忽略 |

## 与路线图的对应

| 阶段 | 新增技术面 |
| --- | --- |
| 阶段 1 本地 MVP | 本文档全部内容：Tauri + React + Rust + SQLite + ZipStore + 安全恢复 |
| 阶段 2 自动化 | notify crate 监听文件变化；进程检测；退出后自动 `create_snapshot`；保留策略（清理时跳过 locked） |
| 阶段 3 游戏识别 | 引入 Ludusavi manifest，预设常见游戏/模拟器路径 |
| 阶段 4 云端 | 新增 `ResticStore` 或仓库同步层；只同步快照仓库，绝不同步真实存档目录（安全规则 1） |

## 落地顺序建议

```text
1. 定 DTO 类型（前后端共享契约）
2. Rust：Repository + SQLite 建表、跑通 list/add_game
3. Rust：scan_path → 接通"测试读取"
4. Rust：ZipStore + create_snapshot（含 NoChange、writing 防护）
5. 前端：把原型 mock 换成真实 invoke，跑通添加游戏 + 创建快照
6. Rust：restore_service 的 rename 原子替换 + before_restore 备份
7. 前端：接通恢复进度事件，跑通完整闭环
8. 删除、锁定、备注 等次级操作
9. 错误状态全覆盖（对照视觉文档状态清单逐一验收）
```

先把第 1~5 步的"创建"链路打通，再啃第 6~7 步的"恢复"链路——恢复是最难也最关键的，留到地基稳了再动。

## 当前结论

技术架构不追求一步到位，而是守住一条主线：

> 用最简单的 zip 起步，但从第一天就把存储层和恢复事务做对，让「不丢存档」这个承诺落在代码结构里，而不是落在口号里。

前端框架、UI 库都可替换，不会让产品失败；**存档恢复的可靠性会**。架构的重心因此压在存储抽象与恢复事务上。



