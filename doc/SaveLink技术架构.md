# SaveLink 技术架构

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | Claude | 2026-06-23 | 第一版：定义本地 MVP 技术栈、模块划分、数据模型、存储抽象与恢复事务设计 |
| 1.1 | Claude | 2026-06-24 | 增补实现现状与偏差：MVP 已落地并打包 |
| 1.2 | Codex | 2026-07-01 | 按当前代码重写现状：编辑/移除游戏、设置页、缺失目录创建恢复、opener 权限、34 个 core 测试 |
| 1.3 | 代强 | 2026-07-02 | 同步 Tauri 启动自检接入状态 |
| 1.4 | 代强 | 2026-07-08 | 同步恢复校验增强与云同步增量协议路线 |
| 1.5 | 代强 | 2026-07-14 | 同步百度 POC、协议 v1、云基础设施和 Fake 双设备闭环；明确百度适配器下一步 |
| 1.6 | 代强 | 2026-07-15 | 同步 BaiduNetdiskStore、OAuth 本机连接层、真实百度验证、62 个默认测试和真实上传下一步 |
| 1.7 | 代强 | 2026-07-16 | 同步百度真实双设备上传、接收、目录绑定和安全恢复完整闭环 |
| 1.8 | 代强 | 2026-07-28 | 第一版移除恢复前自动保护点并强化失败回滚；同步设置入口、自定义图标状态和 68 个默认测试 |
| 1.9 | 代强 | 2026-07-29 | 补充 doc 文档目录；同步中文文档名和引用 |
| 1.10 | 代强 | 2026-08-05 | 同步 v0.1.0 发布与 v0.2.0 专项验收；补充自动调度、联合清理及快照时间统一、旧库迁移和跨设备兼容架构 |
| 1.11 | 代强 | 2026-08-06 | 确认 v0.2.0 架构与主流程验收完成；同步版本元数据和发布后故障验收边界 |
| 1.12 | Codex | 2026-08-10 | 同步 Steam 自动发现、Manifest 打包资源、多目录存储/恢复边界、93 个默认测试及重叠目录防护 |
| 1.13 | Codex | 2026-08-11 | 同步 Elden Ring 修复后的真实开发版与绿色版候选回归结果 |
| 1.14 | 代强 | 2026-08-12 | 将自动备份、Steam 自动发现和多存档目录实现统一收口为 v0.3.0 架构 |
| 1.15 | 代强 | 2026-08-14 | 增加 v0.4.0 DeSmuME 精确 `.dsv` 来源、ROM 身份、跨设备映射；补目录链接环和 ROM 哈希栈溢出防护 |
| 1.16 | 代强 | 2026-08-15 | 确认 v0.4.0 ROM 扫描修复实测通过并统一发布版本元数据 |
| 1.17 | 代强 | 2026-08-17 | 同步 v0.4.0 发布及游戏程序/快捷方式识别、Manifest 复用和扫描边界 |
| 1.18 | Codex | 2026-08-18 | 收紧无 AppID 程序名称匹配；同步真实学习版三种入口及改动文件格式验收 |

## 文档用途

本文档描述 SaveLink v0.4.0 发布后开发主线的真实技术架构。若本文档与代码不一致，以代码和测试为准，并应回头更新本文档。

配套文档：

- `PROGRESS.md`：当前进度与下一步候选项。
- `HANDOFF-codex.md`：后续开发交接。
- `SaveLink恢复与存储测试规格.md`：恢复/存储关键路径验收基准。
- `SaveLink云端快照协议V1.md`：已定稿的云端目录、对象格式、同步流程和失败处理协议。
- `百度网盘API-POC报告-20260714.md`：百度 OAuth/文件 API POC、2MiB 基准数据和云端物理格式决策证据。
- `savelink-app/手动测试计划.md`：人工验收步骤。
- `SaveLink-MVP产品原型草案.md`、`SaveLink低保真原型图.md`、`SaveLink视觉与交互说明.md`：产品、页面、视觉气质。

## 当前实现概览

```text
save_link/
├── doc/              项目文档与阅读索引
├── savelink-core/    纯 Rust 核心逻辑，不依赖 Tauri
├── savelink-app/     Tauri 桌面应用：React 前端 + Rust 命令层
├── demo-front-1/     早期高保真 mock 原型，仅作参考
├── acceptance-data/  手动验收用假存档数据
└── design-drafts/    图标等视觉设计过程文件
```

当前能力：

- 添加游戏、编辑游戏、移除游戏。
- 创建快照、备注、锁定、删除快照。
- 恢复快照，当前已等于目标时跳过；其余情况直接恢复，不自动创建快照。
- 存档目录缺失时支持“创建目录并恢复”。
- 首页展示当前版本；设置齿轮打开全局自动备份开关，AppData 路径命令保留但当前设置页不展示。
- SaveLink 云链 V9 已接入网页、窗口、任务栏、托盘和安装包图标。
- Windows 绿色版 exe、绿色版 ZIP/SHA-256、NSIS、MSI 打包。
- 百度 OAuth、本机 Token 持久化与过期前自动刷新。
- 快照按钮手动上云，成功后持久化状态并显示绿色勾选。
- 顶栏云端存档窗口可发现并下载真实百度快照；接收成功后创建未绑定的本机游戏。
- 未绑定本机存档目录时禁止创建快照和恢复，下载与恢复保持为两个独立动作。
- 独立绑定弹窗复用 `scan_path` 做只读检测，扫描成功后复用 `update_game` 保存路径；绑定本身不触发快照、恢复或上传。
- v0.2.0 启动立即检查、10 分钟轮询、自动快照上云和 30 条未锁定记录联合清理已接入，并已通过绿色版及真实百度主流程验收。
- v0.4.0 DeSmuME 支持已接入：扫描 `desmume.ini`/ROM 目录、读取 NDS Header 身份、计算并缓存 ROM SHA-256、按目标 ROM 精确选择 `.dsv`，以及跨设备按目标设备 ROM 文件名恢复。
- 游戏程序识别已接入：用户选择 Windows `.lnk`、EXE 或安装目录后，优先读取 AppID；无 AppID 时只接受归一化后的完整名称相等，再展示真实存在的存档候选；手动添加继续保留。

当前验证状态：

- `savelink-core`：109 个默认测试全绿；N 组在 Steam 发现基础上增加程序 AppID、精确名称兜底、目录配置、未知程序、名称前缀拒绝和真实 `.lnk` 往返，O/P/Q 组覆盖多目录、精确文件和 DeSmuME。J/L 已按需执行通过，Q5 真实 DeSmuME 测试默认忽略且已执行通过。
- Tauri：自动上传状态选择测试 2 个全绿。
- `npm run build`：前端构建通过。
- `build-installer.bat`：打包通过。
- 旧设置对话框的“打开”目录代码路径曾通过绿色版实机验证；当前齿轮入口展示新的自动备份设置页。游戏程序识别已完成代码接线及真实学习版快捷方式、EXE、目录窗口验收。
- 设备 B 已完成云端接收、绑定假存档目录和手动安全恢复；恢复文件与仓库目标快照 SHA-256 一致。

## 第一性原则

SaveLink 的核心承诺是“不丢用户存档”。所有技术选择服从这几条：

1. 第一版不为成功恢复创建自动反悔点；确认页必须明确提醒用户可先手动创建快照。
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
| 快照存储 | `FsStore` 将目录树写入 `repository` | 当前不压缩；zip/restic 后置 |
| 云端 HTTP | `reqwest 0.12 blocking + rustls` | 百度适配器流式上传/下载；HTTP 细节不进入同步业务层 |
| 百度账号连接 | 系统浏览器 OAuth + loopback callback | `state` 校验后换 Token，凭据留在本机 AppData |
| 文件/目录选择 | `tauri-plugin-dialog` | 选择游戏快捷方式、EXE、安装目录或存档目录 |
| Windows 快捷方式 | `windows 0.61 / Shell COM` | 在独立 STA 线程只读解析 `.lnk` 目标，不启动游戏 |
| 打开路径 | `tauri-plugin-opener` | 保留的设置组件只允许打开 AppData 范围内路径；当前入口隐藏 |

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
│ CloudStateRepository(SqliteRepo)                       │
│ CloudObjectStore(FakeCloudObjectStore/BaiduNetdiskStore)│
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
get_baidu_connection_status
connect_baidu
upload_snapshot_to_baidu
discover_baidu_snapshots
receive_baidu_snapshot
list_snapshots
scan_path
scan_steam_games
scan_program_game
scan_desmume_games
register_desmume_game
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
├── credentials\baidu-oauth.json
├── cloud-work\
├── savelink.db
└── repository\
    └── snapshots\
        ├── {snapshot_id}\      # 原样复制的文件树
        └── {snapshot_id}.ok    # 内容哈希标记
```

绿色版 `savelink-app.exe` 和安装版使用同一个 identifier，因此共用同一个数据目录。

仅用于双设备验收时，可通过 `SAVELINK_TEST_DATA_DIR` 指向绝对路径启动隔离 profile。当前 `run-device-b-test.bat` 使用 `%APPDATA%\com.daiq.savelink-device-b-test`，并在顶栏显示“设备 B 隔离测试”；代码拒绝把测试变量指向正式数据目录。

## 数据模型现状

核心领域类型在 `savelink-core/src/model.rs`。

当前 SQLite 生产实现位于 `savelink-core/src/sqlite_repo.rs`。和早期设计不同，当前没有独立 `save_paths` 表；游戏路径以换行分隔文本存储在 `games` 表中。模型使用 `Vec<PathBuf>`，扫描、指纹、`FsStore`、恢复、云端协议、设备 B 绑定和前端展示均按完整路径数组运行；旧数据库中的单路径记录会迁移为一个来源。

Steam 自动发现位于 `savelink-core/src/steam_discovery.rs`：通过注册表和 Steam appmanifest 枚举已安装应用，再按 AppID 查询随包 `src-tauri/resources/manifest.db`。绿色版同时携带 Manifest 来源说明和 Ludusavi 许可证。精确文件规则会归一到父目录后交给 SaveLink 的目录型存储；末尾 `<storeUserId>` 目录占位符只接受目录，规则结果还会收敛相同或父子嵌套路径。添加/编辑游戏及创建/恢复快照前会再次调用 `validate_save_paths`，把重叠来源作为安全错误拒绝。

程序识别位于 `savelink-core/src/program_discovery.rs`：目录和 EXE 直接解析，Windows `.lnk` 通过 Shell COM 在独立 STA 线程读取目标。扫描严格限制在所选安装目录 3 层和最多 512 个目录，跳过目录链接，不遍历其他磁盘。身份优先读取 `steam_appid.txt` 及常见 AppID INI；没有 AppID 时，快捷方式、EXE 和目录名称会去除空格、标点并转为小写，只有与 Manifest 游戏名完整相等才匹配，不接受前缀关系。游戏身份确定后调用 `steam_discovery.rs` 的同一套 Manifest 查询、约束过滤、占位符展开和路径收敛逻辑；未匹配时返回空结果而不是猜测。

DeSmuME 自动发现位于 `savelink-core/src/desmume_discovery.rs`：检查模拟器目录和可执行文件，读取 `[PathSettings]` 中的 ROM 目录，以带 canonical 路径去重的迭代遍历发现 `.nds`，解析 NDS Header Title/Game Code，并分块计算 ROM SHA-256。哈希使用堆上的 1 MiB 流式缓冲区，避免占满 Tauri 工作线程栈。ROM 的文件大小和修改时间会写入当前设备绑定，用于后续缓存哈希；缓存失效时重新计算。扫描是只读的，配置路径失效时明确要求用户重新选择，不静默改写配置。

DeSmuME 使用 `SaveSource::Files` 表达共享 `Battery` 目录中的精确文件映射：例如设备 A 的 `zzjb2r ver0.99.dsv` 映射为快照内稳定的 `save.dsv`，设备 B 恢复时再映射为目标 ROM 文件名对应的 `.dsv`。`.dsv-01` 至 `.dsv-09` 不会冒充游戏内存档。SHA-256 相同是精确匹配；SHA 不同但 Header Title 与 Game Code 同时相同，只产生候选并要求用户确认。ROM 身份进入云端 `game.json`，模拟器根目录、ROM 路径和 Battery 路径只保存在本机绑定中。

快照时间采用统一口径：新记录持久化为固定秒精度 UTC RFC 3339（例如 `2026-08-05T13:25:00Z`）。打开旧数据库时，`YYYY-MM-DD HH:MM`、`YYYY-MM-DD HH:MM:SS` 和带偏移 RFC 3339 会自动转换为该格式；前端再按用户本地时区显示为 `YYYY-MM-DD HH:MM`。云端协议版本和目录结构不变，比较已有 `.ok` 时按真实时刻而非字符串表现形式判断。

主要概念：

- `Game`
  - `id`
  - `name`
  - `save_paths`：旧普通游戏的整目录来源兼容字段
  - `save_sources`：当前实际参与扫描、快照和恢复的目录或精确文件来源
  - `emulator_identity`：可跨设备同步的模拟器/ROM 身份
  - `emulator_binding`：仅当前设备有效的模拟器根目录、ROM 路径及哈希缓存
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

- `FsStore` 是实现 `SnapshotStore` 的代码类型。
- `repository` 是传给 `FsStore` 的运行时数据根目录，不是另一种代码实现。
- `Repository` / `SqliteRepo` 是元数据持久化接口和 SQLite 实现，与小写路径 `repository/` 不是同一个概念。
- 每条快照保存在 `repository/snapshots/{snapshot_id}/`。
- 本机 `repository/snapshots/{snapshot_id}.ok` 存内容哈希，用于 verify。

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
4. 扫描当前真实存档，比较 `content_hash + file_count + total_size`。
   - 已经等于目标：返回 `restored=false`，不覆盖、不发恢复进度。
   - 与目标不同：继续直接恢复，不创建或复用快照。
5. 将目标快照恢复到同盘临时目录。
6. 校验临时目录内容 hash、文件数和总大小。
7. rename 真实目录 -> .old。
8. rename 临时目录 -> 真实目录。
9. 校验恢复后真实目录 hash、文件数和总大小。
10. 校验成功后删除 .old；失败则删除错误的新目录并 rename .old -> 真实目录。
```

`RestoreOutcome` 只返回 `target_id` 和 `restored`。`Reason::BeforeRestore` 仅为历史数据库及云协议兼容保留。

缺失目录续走：

- `restore_snapshot_with_choice(..., "create")`：创建目录并恢复。
- `restore_snapshot_with_choice(..., "cancel")`：不写入。
- `restore_snapshot_with_choice(..., "reselect")`：后端枚举存在，但当前 UI 未接“重新选择并恢复”完整闭环。

## 错误处理

核心错误在 `savelink-core/src/error.rs`。

与用户安全强相关的错误：

- `SnapshotCorrupt`：目标快照损坏，恢复未触碰真实存档。
- `RestoreFailed { rolled_back }`
  - `rolled_back=true`：真实存档应已回到操作前状态。
  - `rolled_back=false`：极端情况下可能已经改动真实存档，UI 必须提醒用户核对。
- `SaveDirMissingNeedsChoice`：真实存档目录不存在，需要用户选择。
- `SnapshotLocked`：锁定快照不可删除。
- `OverlappingSavePaths`：多个存档根目录相同或互为父子目录，拒绝进入快照或恢复流程。

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

正式协议见 `SaveLink云端快照协议V1.md`。v1 按游戏目录列出 `.ok` 发现快照，不维护全局 `games.json`、`snapshots.json` 或 `tombstones.json`，避免多设备同时覆盖同一索引文件。

当前已实现的本机云同步基础：

- `cloud_model.rs`：云账号、游戏绑定、远端 `.ok` 缓存和同步状态模型。
- `cloud_repo.rs`：独立 `CloudStateRepository`，不污染现有本地 `Repository` 契约。
- `SqliteRepo`：包含 `app_settings`、`cloud_accounts`、`cloud_game_bindings`、`cloud_snapshot_sync`；旧数据库打开时自动补表，并在事务中无损升级云删除状态 CHECK 约束。
- `cloud_store.rs`：通用 `CloudObjectStore`、上传覆盖策略、云端条目模型和文件系统 `FakeCloudObjectStore`。
- `baidu_store.rs`：正式 `BaiduNetdiskStore`、逻辑/物理路径映射、Token 提供者边界、流式单步上传、分页列表、filemetas/dlink 下载、幂等删除和百度错误分类。
- F 组 4 个测试：SQLite 持久化、重开、游戏更新及旧快照混合时间迁移与真实时间排序。
- G 组 8 个测试：持久化、状态转换、目录排序、ignored 保留、CreateOnly/Overwrite、路径穿越防护、旧库补表和 v0.1.0 状态约束迁移。
- `cloud_protocol.rs`：manifest、game、云端 `.ok` JSON 和逻辑路径的序列化、解析与严格校验。
- `cloud_archive.rs`：单快照 zip、SHA-256、路径安全、防 zip slip 和解压后内容指纹校验。
- `cloud_service.rs`：上传、发现、下载、冲突、幂等、接收落地和“云端优先、本地最后”的联合删除编排。
- H 组 15 个测试：JSON、zip 往返、危险 entry、A/B 双设备闭环、孤儿 zip、篡改、内容不匹配、硬冲突、联合删除成功/失败重试、旧偏移时间云对象幂等兼容，以及已有云文档补写模拟器身份。
- 百度适配器内部单元测试 2 个，I 组本地 HTTP 契约测试 4 个；J 组真实百度对象存取冒烟默认忽略，2026-07-15 已使用环境变量注入 Token 执行通过。
- `baidu_oauth.rs`：OAuth URL、授权码换 Token、刷新方法、随机 `state`、本机回调监听和 Token 文件仓库；K 组 8 个测试保护。
- L 组真实百度设备 B 测试默认忽略，保护只读发现不创建游戏、下载后双重校验、接收落地及设备路径隔离；2026-07-16 已按需执行通过。
- P 组 2 个测试：共享 `Battery` 目录中只读取目标 `.dsv`、只恢复目标 `.dsv`，其他游戏存档保持不变。
- Q 组 5 个默认测试：失效 ROM 配置要求重选、NDS Header/SHA-256 解析与缓存、精确 `.dsv` 匹配及 `.dsv-01` 排除、目录链接环不会导致递归栈溢出，以及 512 KiB 小栈线程可完成 ROM 哈希；Q5 为真实 DeSmuME 只读发现测试，默认忽略。

OAuth、凭据持久化、自动刷新、真实上云和设备 B 发现/下载/接收已实现。上传通过 `CloudSyncService` 发布 `.zip + .ok`；下载先校验压缩包，再校验解压内容，通过后才写本机仓库。只读发现不创建本机游戏，接收后创建的游戏不含设备 A 路径。尚未完成解绑和凭据加密；`BaiduNetdiskStore` 当前按百度官方单步上传边界支持不超过 2 GiB 的对象，更大快照需要后续增加预上传/分片上传。

百度网盘 POC 已通过，云端快照物理格式确定为：

```text
本机 repository/snapshots/{snapshot_id}/（由 FsStore 管理）
  -> 临时打包 {snapshot_id}.zip
  -> 上传 zip
  -> 随后生成并上传云端 {snapshot_id}.ok
```

下载时先校验 `zip_size + zip_sha256`，解压后再校验 `content_hash + file_count + total_size`，通过后才能通过 `FsStore` 写入本机 `repository`。本机 `.ok` 与云端 `.ok` 内容不同；云端格式不要反向强制本地快照仓库改成 ZipStore。

云端同步的是 SaveLink 的共享快照事实和云端元数据：

- 游戏云端身份和名称。
- 快照时间线元数据。
- 快照内容文件。
- 快照校验信息。
- 发布快照时的备注和锁定状态。

v1 上传后的备注/锁定修改只保存在本机。v0.2.0 保留策略直接删除不可变快照对象，不引入墓碑文件：先删除远端 `.ok` 撤销发布，再幂等删除 `.zip`。云端删除失败时保留本地快照，并在 `cloud_snapshot_sync.sync_status` 写入 `delete_failed`。

## v0.2.0 自动备份与联合清理

桌面层新增 `auto_backup.rs`，职责是全局设置、启动即时检查、10 分钟调度、待上传自动快照重试和保留策略编排。核心层新增 `AutoBackupService`，只负责一次全游戏检查和计算超过 30 条的未锁定候选，不直接感知 Tauri 或百度网盘。

自动任务与手动创建、恢复、删除、编辑路径及云端接收共享 `snapshot_operation_lock`，防止后台扫描和用户写操作同时修改同一快照状态。云任务另使用 `baidu_sync_in_progress`，避免手动上传/下载与后台上传并发执行。

自动上传只处理 `reason=auto` 且状态为完整的快照。这样升级 v0.2.0 后不会把用户历史手动本地快照静默批量上传。未授权时后台不发起 OAuth；`uploading` 或 `error` 的自动快照在后续周期重试。

每个游戏的手动、自动、云端接收未锁定快照统一计入 30 条。锁定快照不计入且数量不限。删除状态分层如下：

- `snapshots.status`：`writing / complete / corrupt / deleting`，只描述本机物理生命周期。
- `cloud_snapshot_sync.sync_status`：原上传/下载状态之外，增加 `delete_pending / deleting / delete_failed / remote_deleted`。

联合清理顺序：

```text
delete_pending
-> deleting
-> 删除云端 .ok
-> 删除云端 .zip
-> remote_deleted
-> snapshots.status=deleting
-> 删除本机快照文件
-> 删除 snapshots 记录
-> 删除 cloud_snapshot_sync 缓存记录
```

云删除失败时不得进入本地删除。显式本地删除失败会恢复快照原状态；进程在本地删除中断时留下 `snapshots.status=deleting`，下次启动由 `startup_self_check` 幂等续做。

本机私有数据不应被云端覆盖：

- 真实游戏存档路径。
- 本机设备 ID。
- 百度网盘 token。
- 本机设置和缓存。

当前账号连接实现把 Token 保存到 `%APPDATA%\com.daiq.savelink\credentials\baidu-oauth.json`，SQLite `cloud_accounts.token_ref` 只保存相对引用。Token 不传 React、不写日志、不上传云端。v0.1.0 已按明文 JSON 文件状态发布，迁移到 Windows DPAPI、Credential Manager 或 Tauri Stronghold 是 v0.1.x 的优先安全加固项。

手动 zip 导出/导入仍可作为备份迁移工具，但当前优先级低，不作为百度网盘自动云同步的技术主线。

## 设置组件与 Tauri 权限

当前齿轮入口渲染 `SettingsDialog`，通过 `get_auto_backup_settings` / `set_auto_backup_enabled` 读取和修改全局自动备份开关。默认值由 Tauri 初始化时写入 `app_settings.auto_backup_enabled=true`，设置页同时展示固定的 10 分钟检查间隔。

旧 `get_app_info` 命令仍保留，可返回：

- version
- data_dir
- repository_dir
- database_path

内部路径当前不在设置页展示。若以后重新开放，路径复制使用浏览器 Clipboard API，路径打开使用 `@tauri-apps/plugin-opener` 的 `openPath()`。

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
- v0.4.0 已正式发布；程序识别已完成真实学习版快捷方式、EXE 和安装目录窗口验收，名称前缀误判已修复。
- Steam 自动发现的 Elden Ring 父子候选问题已修复，并通过自动测试、真实开发版和绿色版候选回归；真实多 Steam 游戏库机器仍待以后验收。
- 真实恢复进度事件未接入前端；当前 Tauri 命令传空 progress 回调。
- `startup_self_check` 已在 Tauri setup 中显式调用；真实窗口残留清理场景可在正式回归时补验收。
- 设置入口已提供自动备份开关；帮助入口仍是占位。
- 百度首次授权、过期前自动刷新和授权失效后重新连接已实现；解绑和凭据加密尚未接入。
- 真实百度上传、发现、下载、接收、独立目录绑定和绑定后的安全恢复均已通过设备 B 实机验收。
- 百度适配器当前只实现不超过 2 GiB 的单步上传；断点续传和分片上传后置。
- 家中验收环境已安装 `rustfmt 1.9.0-stable`，本轮修改的两个 Rust 文件已通过 `rustfmt --check`；全仓检查仍会报告既有文件的历史格式差异，尚未单独收口。

## 路线图对应

| 阶段 | 技术面 |
| --- | --- |
| 阶段 1 本地 MVP | Tauri + React + Rust + SQLite + FsStore + 安全恢复 |
| MVP 后补齐 | 编辑/移除游戏、缺失目录创建恢复、首页版本号、设置组件实现、打包脚本、权限修复；设置入口当前隐藏 |
| 阶段 2 自动化 | notify 文件监听、游戏退出后快照、保留策略 |
| 阶段 3 游戏识别 | Ludusavi manifest / 常见游戏路径库 |
| 阶段 4 云端 | POC、协议 v1、Fake/真实双设备传输、OAuth、上传/接收、独立绑定及安全恢复均已完成实机验收 |
| 阶段 5 DeSmuME | ROM 发现、身份匹配、精确 `.dsv` 快照/恢复和云身份兼容已实现并随 v0.4.0 发布；Yuzu 后置到 v0.5.0 |
| 阶段 6 游戏程序识别 | `.lnk`/EXE/目录身份和 Manifest 存档候选已实现，自动测试及真实学习版样本只读验收通过 |

## 当前结论

SaveLink 当前架构的重心不是功能花哨，而是把“创建快照”和“恢复快照”这条高风险链路做成可测试、可回滚、可替换存储实现的结构。

目录复制不是最终形态，但它让 MVP 足够简单；`SnapshotStore` 抽象和恢复事务才是长期价值所在。

