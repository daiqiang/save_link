# SaveLink 技术架构

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | Claude | 2026-06-23 | 第一版：定义本地 MVP 技术栈、模块划分、数据模型、存储抽象与恢复事务设计 |
| 1.1 | Claude | 2026-06-24 | 增补实现现状与偏差：MVP 已落地并打包 |
| 1.2 | Codex | 2026-07-01 | 按当前代码重写现状：编辑/移除游戏、设置页、缺失目录创建恢复、opener 权限、34 个 core 测试 |
| 1.3 | 代强 | 2026-07-02 | 同步 Tauri 启动自检接入状态 |
| 1.4 | 代强 | 2026-07-08 | 同步恢复校验增强与云同步增量协议路线 |

## 文档用途

本文档描述 SaveLink 当前本地 MVP 的真实技术架构。若本文档与代码不一致，以代码和测试为准，并应回头更新本文档。

配套文档：

- `PROGRESS.md`：当前进度与下一步候选项。
- `HANDOFF-codex.md`：后续开发交接。
- `savelink-restore-test-spec.md`：恢复/存储关键路径验收基准。
- `savelink-cloud-sync-model-protocol-draft.md`：云同步数据边界、目录结构和增量协议草案。
- `savelink-app/手动测试计划.md`：人工验收步骤。
- `savelink-mvp-product-prototype.md`、`savelink-low-fidelity-wireframe.md`、`savelink-visual-interaction-guidelines.md`：产品、页面、视觉气质。

## 当前实现概览

```text
save_link_workspace/
├── savelink-core/    纯 Rust 核心逻辑，不依赖 Tauri
├── savelink-app/     Tauri 桌面应用：React 前端 + Rust 命令层
├── demo-front-1/     早期高保真 mock 原型，仅作参考
└── acceptance-data/  手动验收用假存档数据
```

当前能力：

- 添加游戏、编辑游戏、移除游戏。
- 创建快照、备注、锁定、删除快照。
- 恢复快照，恢复前自动备份。
- 存档目录缺失时支持“创建目录并恢复”。
- 设置页展示并打开/复制数据目录、仓库目录、数据库文件。
- Windows 绿色版 exe、NSIS、MSI 打包。

当前验证状态：

- `savelink-core`：35 个测试全绿。
- `npm run build`：前端构建通过。
- `build-installer.bat`：打包通过。
- 绿色版实机验证过设置页“打开”目录。

## 第一性原则

SaveLink 的核心承诺是“不丢用户存档”。所有技术选择服从这几条：

1. 任何覆盖真实存档的操作，前面必须有一个已成功落盘的回退点。
2. 写操作要么完整成功，要么完整回滚，不允许半残态。
3. 快照内容一旦写入即不可变；可变的只有备注、锁定等元数据。
4. 存储格式与上层逻辑解耦，未来可从目录复制换成 zip/restic。
5. 前端不直接碰文件系统，不直接散落 `invoke`，统一走 Rust 命令和 `api.ts`。

## 技术栈

| 层 | 当前选择 | 说明 |
| --- | --- | --- |
| 桌面壳 | Tauri 2.x | Rust 原生后端 + WebView2 前端 |
| 前端 | React + TypeScript + Vite | 当前没有引入 Tailwind/shadcn/TanStack Query |
| 样式 | `src/App.css` | 手写设计令牌和组件样式 |
| 图标 | `src/lib/icons.tsx` | 内联 SVG，当前没有引入 lucide-react |
| 核心逻辑 | Rust | 文件扫描、哈希、快照、恢复 |
| 元数据 | SQLite via `rusqlite 0.32 bundled` | 用户无需单独安装 SQLite |
| 快照存储 | `FsStore` 目录复制 | 当前不压缩；zip/restic 后置 |
| 目录选择 | `tauri-plugin-dialog` | 添加/编辑游戏选择目录 |
| 打开路径 | `tauri-plugin-opener` | 设置页打开 AppData 范围内路径 |

## 分层结构

```text
┌──────────────────────────────────────────────────────┐
│ React 前端                                            │
│ App.tsx + components + lib/api.ts                     │
│ 只负责渲染、交互、状态；后端调用统一走 api.ts           │
└───────────────────────────┬──────────────────────────┘
                            │ Tauri invoke
┌───────────────────────────┴──────────────────────────┐
│ savelink-app/src-tauri/src/commands.rs                │
│ DTO + 参数校验 + Tauri command 薄壳                    │
└───────────────────────────┬──────────────────────────┘
                            │
┌───────────────────────────┴──────────────────────────┐
│ savelink-core                                         │
│ SnapshotService / RestoreService                      │
│ Repository(SqliteRepo) + SnapshotStore(FsStore)        │
└──────────────────────────────────────────────────────┘
```

## 前端结构

```text
savelink-app/src/
├── App.tsx
├── App.css
├── lib/
│   ├── api.ts          invoke 收口层
│   ├── types.ts        与 Rust DTO 对齐
│   ├── format.ts
│   └── icons.tsx
└── components/
    ├── AddGameDialog.tsx
    ├── EditGameDialog.tsx
    ├── RestoreDialog.tsx
    ├── SettingsDialog.tsx
    ├── SnapshotDrawer.tsx
    └── Toast.tsx
```

前端状态当前使用 React hooks：

- `games`：游戏列表。
- `selectedId`：当前选中游戏。
- `snapshots`：当前加载的快照列表。
- 弹窗/抽屉/menu 状态：组件内 `useState`。

命令成功后通常重拉游戏或快照列表，避免本地状态与 SQLite 状态漂移。

## Tauri 命令清单

当前 `generate_handler!` 注册：

```text
list_games
get_repository_path
get_app_info
list_snapshots
scan_path
add_game
update_game
create_snapshot
update_snapshot_meta
delete_snapshot
delete_game
restore_snapshot
restore_snapshot_with_choice
```

前端只通过 `savelink-app/src/lib/api.ts` 调用这些命令。

## 数据目录与仓库

Tauri 配置：

```text
productName = SaveLink
identifier  = com.daiq.savelink
```

Windows 运行时数据：

```text
%APPDATA%\com.daiq.savelink\
├── savelink.db
└── repository\
    └── snapshots\
        ├── {snapshot_id}\      # 原样复制的文件树
        └── {snapshot_id}.ok    # 内容哈希标记
```

绿色版 `savelink-app.exe` 和安装版使用同一个 identifier，因此共用同一个数据目录。

## 数据模型现状

核心领域类型在 `savelink-core/src/model.rs`。

当前 SQLite 生产实现位于 `savelink-core/src/sqlite_repo.rs`。和早期设计不同，当前没有独立 `save_paths` 表；游戏路径以换行分隔文本存储在 games 表中。模型仍是 `Vec<PathBuf>`，但 UI 和恢复流程目前按第一个目录运行。

主要概念：

- `Game`
  - `id`
  - `name`
  - `save_paths`
  - `created_at`
  - `updated_at`
- `Snapshot`
  - `id`
  - `game_id`
  - `created_at`
  - `note`
  - `reason`: `manual` / `before_restore` / `auto`
  - `locked`
  - `file_count`
  - `total_size`
  - `content_hash`
  - `storage_key`
  - `status`: `writing` / `complete` / `corrupt`

## SnapshotStore 抽象

```rust
pub trait SnapshotStore: Send + Sync {
    fn create(&self, snapshot_id: &str, sources: &[PathBuf], ctx: &ScanResult)
        -> Result<StoredSnapshot>;
    fn restore(&self, key: &str, target: &Path) -> Result<()>;
    fn verify(&self, key: &str) -> Result<bool>;
    fn delete(&self, key: &str) -> Result<()>;
}
```

当前实现：

- `FsStore`
- 一快照一个目录。
- `{snapshot_id}.ok` 存内容哈希，用于 verify。

重要纪律：

- service 只传递 `storage_key`，不得解析 key 的内部结构。
- 未来换 `ZipStore` / `ResticStore` 时，应新实现 `SnapshotStore`，而不是改业务服务去理解 zip/restic。

## 创建快照流程

```text
1. 根据 game_id 读取游戏与 save_paths。
2. 扫描目录：文件数、总大小、content_hash。
3. 与最近快照 content_hash 比对。
   - 相同：返回 NoChange，不写库不写仓库。
4. 插入 status=Writing 的快照记录。
5. SnapshotStore.create 写入快照文件。
6. SnapshotStore.verify 校验快照。
7. 更新记录为 Complete。
8. 任一步失败：清理半成品文件，删除记录，不碰真实存档。
```

## 恢复快照流程

```text
1. 读取游戏和目标快照。
2. verify 目标快照。
   - 损坏：返回 SnapshotCorrupt，不碰真实存档。
3. 检查真实存档目录是否存在。
   - 不存在：返回 SaveDirMissingNeedsChoice，不自动写入。
4. 强制创建 before_restore 快照。
   - 失败：返回 BackupFailed，不覆盖真实存档。
5. 将目标快照恢复到同盘临时目录。
6. 校验临时目录内容 hash、文件数和总大小。
7. rename 真实目录 -> .old。
8. rename 临时目录 -> 真实目录。
9. 删除 .old。
10. 校验恢复后真实目录 hash、文件数和总大小。
```

缺失目录续走：

- `restore_snapshot_with_choice(..., "create")`：创建目录并恢复。
- `restore_snapshot_with_choice(..., "cancel")`：不写入。
- `restore_snapshot_with_choice(..., "reselect")`：后端枚举存在，但当前 UI 未接“重新选择并恢复”完整闭环。

## 错误处理

核心错误在 `savelink-core/src/error.rs`。

与用户安全强相关的错误：

- `SnapshotCorrupt`：目标快照损坏，恢复未触碰真实存档。
- `BackupFailed`：恢复前备份失败，恢复中止，真实存档未被覆盖。
- `RestoreFailed { rolled_back }`
  - `rolled_back=true`：真实存档应已回到操作前状态。
  - `rolled_back=false`：极端情况下可能已经改动真实存档，UI 必须提醒用户核对。
- `SaveDirMissingNeedsChoice`：真实存档目录不存在，需要用户选择。
- `SnapshotLocked`：锁定快照不可删除。

前端恢复失败页会根据错误类型给出“本次恢复未修改真实存档”或“请核对存档目录”的提示。

## 云同步数据边界

云同步主线不应是 `latest.zip` 覆盖上传，也不应把某台电脑的 `savelink.db` 整库原样上云。

原因：

- `latest.zip` 每次新增快照都可能重传整个历史包，不适合高频自动同步。
- 当前 `games.save_paths` 保存本机真实存档路径，不能作为跨设备共享事实。
- 直接同步整库难以做字段级合并，也容易让家里电脑拿到公司电脑的本地路径。

当前推荐方向：

```text
云端目录结构 + 增量上传/下载 + 快照不可变约束
```

云端同步的是 SaveLink 的共享快照事实和云端元数据：

- 游戏云端身份和名称。
- 快照时间线元数据。
- 快照内容文件。
- 快照校验信息。
- 备注、锁定、删除墓碑等可同步状态。

本机私有数据不应被云端覆盖：

- 真实游戏存档路径。
- 本机设备 ID。
- 百度网盘 token。
- 本机设置和缓存。

手动 zip 导出/导入仍可作为备份迁移工具，但当前优先级低，不作为百度网盘自动云同步的技术主线。

## 设置页与 Tauri 权限

设置页通过 `get_app_info` 获取：

- version
- data_dir
- repository_dir
- database_path

路径复制使用浏览器 Clipboard API。路径打开使用 `@tauri-apps/plugin-opener` 的 `openPath()`。

Tauri 2 capability 需要：

```json
{
  "identifier": "opener:allow-open-path",
  "allow": [
    { "path": "$APPDATA" },
    { "path": "$APPDATA/**" }
  ]
}
```

这个范围足够打开 SaveLink 自己的数据目录和仓库目录。不要扩大到 `$HOME/**` 或整盘，除非总规划明确要求并重新评估安全边界。

## 当前技术债

- `FsStore` 目录复制不压缩，空间占用和文件数量后续可能成为问题。
- 多存档目录尚未完成；模型支持数组，但 UI/恢复按第一个目录运行。
- 真实恢复进度事件未接入前端；当前 Tauri 命令传空 progress 回调。
- `startup_self_check` 已在 Tauri setup 中显式调用；真实窗口残留清理场景可在正式回归时补验收。
- 帮助入口仍是占位。
- 应用图标仍是 Tauri 默认图标。
- 本机此前缺 `rustfmt`，`cargo fmt` 未验证。

## 路线图对应

| 阶段 | 技术面 |
| --- | --- |
| 阶段 1 本地 MVP | Tauri + React + Rust + SQLite + FsStore + 安全恢复 |
| MVP 后补齐 | 编辑/移除游戏、缺失目录创建恢复、设置页、打包脚本、权限修复 |
| 阶段 2 自动化 | notify 文件监听、游戏退出后快照、保留策略 |
| 阶段 3 游戏识别 | Ludusavi manifest / 常见游戏路径库 |
| 阶段 4 云端 | 云端目录结构 + 增量同步；同步共享快照事实和云端元数据，真实存档路径保留为本机绑定 |

## 当前结论

SaveLink 当前架构的重心不是功能花哨，而是把“创建快照”和“恢复快照”这条高风险链路做成可测试、可回滚、可替换存储实现的结构。

目录复制不是最终形态，但它让 MVP 足够简单；`SnapshotStore` 抽象和恢复事务才是长期价值所在。

