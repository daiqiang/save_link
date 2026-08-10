# savelink-app — SaveLink 桌面应用（Tauri + React）

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | 代强 | 2026-07-16 | 补齐版本历史；同步百度真实双设备上传、接收、目录绑定和安全恢复闭环 |
| 1.1 | 代强 | 2026-07-23 | 根据真实用户反馈，将快照上传按钮改为图标加文字并明确各状态 |
| 1.2 | 代强 | 2026-07-28 | 第一版移除恢复前自动保护点；同步首页版本号、设置占位入口和自定义应用图标状态 |
| 1.3 | 代强 | 2026-07-29 | 同步 doc 目录及中文文档名引用 |
| 1.4 | 代强 | 2026-08-05 | 同步 v0.1.0 发布；补充 v0.2.0 自动备份设置、后台调度、自动上云和 30 条联合清理 |
| 1.5 | 代强 | 2026-08-06 | 同步 v0.2.0 当前版本和发布产物名称 |
| 1.6 | Codex | 2026-08-10 | 同步 Steam 自动发现、多存档目录和真实绿色版验收状态 |

SaveLink 的桌面外壳。前端 React 在 `src/`，Rust 命令层在 `src-tauri/`，核心逻辑在隔壁 `../savelink-core`（路径依赖，保持纯净、可独立测试）。

## 当前能力

- 添加游戏时默认从 Steam 与随包 Manifest 数据库发现候选，同时保留手动添加入口。
- 一个游戏可绑定多个存档目录；扫描、快照、恢复、编辑、详情和设备 B 绑定均保留完整目录列表。
- 创建快照，未变化时不重复创建。
- 时间线展示快照，支持备注、锁定、删除。
- 恢复指定快照，当前已等于目标时不重复写入；第一版不自动创建恢复前快照。
- 存档目录缺失时支持“创建目录并恢复”或取消。
- 编辑游戏名称和存档路径。
- 移除游戏（删除 SaveLink 内部记录与仓库快照，不删除真实存档目录）。
- 首页标题旁显示当前版本；齿轮入口打开设置页，第一个正式设置项是默认开启的全局自动备份开关。
- SaveLink 云链 V9 已用于网页 favicon、窗口、任务栏、托盘和安装包图标。
- 快照可通过带云图标和文字的 `上传` 按钮手动保存到百度网盘；按钮会显示 `上传中`、`已上云` 或 `重试`，未授权时授权后自动续传。
- access token 临近过期时自动通过 refresh token 刷新。
- 顶栏云端存档窗口可按游戏发现百度快照并手动下载；接收成功后显示在本机时间线。
- 云端接收的游戏不继承其他设备的存档路径；绑定本机目录前禁止创建快照和恢复。
- 未绑定游戏提供独立“绑定存档目录”入口；目录扫描成功后才可绑定，绑定不会自动创建快照、恢复或上云。
- `run-device-b-test.bat` 可用独立 AppData 目录启动“设备 B 隔离测试”profile。
- 可通过 `build-portable.bat` 生成绿色版目录、ZIP 和 SHA-256，通过 `build-installer.bat` 生成绿色 exe、NSIS、MSI。
- v0.1.0 已发布到 GitHub Releases；v0.2.0 已完成开发，准备发布；GitHub 为主仓库，Gitee 为国内镜像。
- 启动时立即检查存档变化，之后每 10 分钟检查；自动快照校验成功后立即尝试上传，未授权时后台不会弹浏览器。
- 每个游戏保留 30 条未锁定记录，所有来源统一计数；锁定记录不限量。淘汰时云端删除成功后才删除本地快照。

## 结构

```text
savelink-app/
├── src/                         React 前端（TS）
│   ├── App.tsx                  主壳：顶栏 + 左栏游戏列表 + 右栏时间线
│   ├── App.css                  设计令牌 + 全部样式
│   ├── lib/
│   │   ├── types.ts             与 Rust DTO 对齐的类型
│   │   ├── api.ts               数据访问层：前端唯一 invoke 收口
│   │   ├── format.ts            大小/标签格式化
│   │   └── icons.tsx            内联 SVG 图标
│   └── components/
│       ├── AddGameDialog.tsx
│       ├── BindSavePathDialog.tsx
│       ├── EditGameDialog.tsx
│       ├── RestoreDialog.tsx
│       ├── SettingsDialog.tsx
│       ├── CloudSnapshotsDialog.tsx
│       ├── SnapshotDrawer.tsx
│       └── Toast.tsx
└── src-tauri/
    ├── src/
    │   ├── lib.rs               Tauri 入口：注册插件、初始化 AppState、注册命令
    │   ├── auto_backup.rs       全局设置、10 分钟调度、自动上传重试与 30 条清理编排
    │   └── commands.rs          命令层（薄壳）：DTO + AppState + #[tauri::command]
    ├── Cargo.toml               依赖 savelink-core、tauri-plugin-dialog/opener、chrono、rusqlite
    ├── tauri.conf.json          productName=SaveLink，identifier=com.daiq.savelink
    └── capabilities/
        └── default.json         权限：dialog + opener scoped 到 AppData
```

## Tauri 命令清单

当前前端通过 `src/lib/api.ts` 调用这些命令：

```text
list_games
get_repository_path
get_app_info
get_auto_backup_settings
set_auto_backup_enabled
get_baidu_connection_status
connect_baidu
upload_snapshot_to_baidu
discover_baidu_snapshots
receive_baidu_snapshot
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

## 开发 / 构建

```bash
npm install
npm run tauri dev      # 真桌面窗口（开发）。纯 npm run dev 没有 Tauri invoke
npm run build          # 前端 tsc + vite 构建
npm run tauri build    # Tauri 打包
```

面向普通用户发布绿色版：

```bat
build-portable.bat
```

生成：

```text
src-tauri/target/release/bundle/portable/SaveLink_0.2.0_windows_x64_portable.zip
src-tauri/target/release/bundle/portable/SaveLink_0.2.0_windows_x64_portable.zip.sha256.txt
```

生成安装包：

```bat
build-installer.bat
```

它会自动定位 MSVC 环境、关闭正在运行的 SaveLink、防止 exe 占用，并生成：

```text
src-tauri/target/release/savelink-app.exe
src-tauri/target/release/bundle/nsis/SaveLink_0.2.0_x64-setup.exe
src-tauri/target/release/bundle/msi/SaveLink_0.2.0_x64_en-US.msi
```

运行时数据：

```text
%APPDATA%\com.daiq.savelink\
├── credentials\baidu-oauth.json
├── cloud-work\
├── savelink.db
└── repository\
```

绿色版和安装版共用同一个 Tauri identifier，所以共用同一个用户数据目录。

设备 B 验收使用独立目录：

```text
%APPDATA%\com.daiq.savelink-device-b-test\
```

先通过 `build-installer.bat` 生成 release exe，再运行 `run-device-b-test.bat`。脚本只设置隔离数据目录并启动同一份程序，不复制或覆盖正式数据。

## 权限注意

当前 `SettingsDialog` 展示自动备份开关，不展示内部数据路径。旧路径打开能力仍按最小范围保留，供后续明确需要时复用。

Tauri 2 需要同时满足：

- 命令权限：`opener:allow-open-path`
- 路径 scope：当前只允许 `$APPDATA` 和 `$APPDATA/**`

不要为了省事开放整个磁盘。该组件只需要打开 SaveLink 自己的数据目录和仓库目录。

## 给后续开发者 / Codex 的要点

- **前端只通过 `lib/api.ts` 访问后端，不在组件里直接 `invoke`。**
- **新增端到端功能的链路**：
  1. `savelink-core` 加/改逻辑并补测试。
  2. `src-tauri/src/commands.rs` 包一层 DTO 命令。
  3. `src-tauri/src/lib.rs` 的 `generate_handler!` 注册。
  4. `src/lib/api.ts` 加调用。
  5. 组件调用 `api.ts`。
- DTO 必须与前端 `types.ts` 对齐。
- 涉及真实存档写入/覆盖/删除时，先看 `../doc/SaveLink恢复与存储测试规格.md`，再动代码。
- 详细交接见 `../doc/HANDOFF-codex.md` 与 `../doc/PROGRESS.md`。
