# SaveLink 开发进度

> 持久化的路线图与当前位置。上下文压缩不影响此文件——以它为准。
> 最后更新：2026-06-24（第 6 步完成，全流程跑通）

## 当前位置

**全部 6 步完成。MVP 已打包成 Windows 安装包。可安装试用。**

进度条：

```
做什么(产品文档)        ████████████ 100%  ✅
长什么样(原型图)        ████████████ 100%  ✅ demo-front-1，纯前端 mock
怎么实现(架构文档)      ████████████ 100%  ✅
验收标准(测试 33 用例)  ████████████ 100%  ✅ 全绿（含 SQLite 持久化）
─────────────────────────────────────
第1步 核心逻辑          ████████████ 100%  ✅ A/B/C/D/E 组 31/31 绿
第2步 真 SQLite         ████████████ 100%  ✅ SqliteRepo 落盘；zip 后置
第3步 Tauri 外壳+命令层  ████████████ 100%  ✅ 8 命令接通 core
第4步 React 写前端      ████████████ 100%  ✅ 5 页面+交互，浏览器验证通过
第5步 前后端接线        ████████████ 100%  ✅ 真窗口验证：加游戏/快照/抽屉通过
第6步 打包 Windows 包    ████████████ 100%  ✅ MSI + NSIS + 独立 exe
```

## 安装包产物

```
savelink-app/src-tauri/target/release/
├── savelink-app.exe                         独立可执行(11M)，免安装直接双击
└── bundle/
    ├── msi/SaveLink_0.1.0_x64_en-US.msi      MSI 安装包(4.0M)
    └── nsis/SaveLink_0.1.0_x64-setup.exe     NSIS 安装程序(2.7M)
```

数据位置：`%APPDATA%/com.daiq.savelink/`（savelink.db + repository/）。

## 后续可做（非 MVP 必需）

- 真 zip 存储（当前 FsStore 是目录复制，不压缩）→ 接 zip crate 或 restic
- 阶段2自动化：文件监听、游戏退出自动快照、保留策略
- 阶段3游戏识别：复用 Ludusavi manifest
- 阶段4云端：百度网盘/WebDAV/NAS（只同步快照仓库，绝不碰真实存档）
- 应用图标换成自定义（当前是 Tauri 默认图标）

## 用户验证清单（第 5 步，在本机跑）

```
cd savelink-app
npm run tauri dev        # 弹出真 SaveLink 窗口
```
依次验证（建议先用一个无关紧要的测试目录，别拿真实游戏存档冒险）：
- [ ] 窗口打开，左栏为空（真数据库初始无游戏）
- [ ] 点"添加游戏"→"选择目录"弹出真系统目录选择器
- [ ] 选一个测试目录→"测试读取"显示真实文件数/大小
- [ ] 保存→游戏出现在左栏
- [ ] "创建快照"→时间线新增，且 savelink.db 真的写了记录
- [ ] "恢复"→走三步→真实目录被恢复，且生成 before_restore 备份
- [ ] 关闭重开 app→数据还在（SQLite 落盘验证）
数据库与仓库位置：系统 app_data_dir（Windows: %APPDATA%/<bundle-id>/）下 savelink.db + repository/

> 注：原"第4步 前后端接通"已拆成 **第4步 写 React 前端** + **第5步 接线**，
> 让"写前端"成为独立显眼的一步。打包顺延为第6步。

## 五步路线图

### 第 1 步：补完核心逻辑（B 组恢复 + E 组自检）✅ 已完成
- 实现了 `RestoreService::restore_snapshot`（备份→同盘原子 rename 替换→校验）、
  `restore_with_choice`、`startup_self_check`、`same_volume`。
- 之前已实现：`FsStore`（D 组）、`SnapshotService::create_snapshot`/`delete_snapshot`（A/C 组）。
- 结果：`cargo test --no-fail-fast` → 31/31 全绿，零警告。
- 关键安全设计：备份成功才允许覆盖；同盘原子 rename 三段式（.tmp→.old→替换）保证崩溃也不留半残；
  失败错误带 `rolled_back` 语义。均在 FailingStore 故障注入下挣得，未削弱测试。

### 第 2 步：把测试替身换成真实现 ✅ 已完成（SQLite 部分）
- `InMemoryRepo` → `SqliteRepo`：数据落盘、关机不丢。用 `rusqlite 0.32` + `bundled`
  （SQLite 源码一起编进来，用户无需手动安装任何东西；需本机有 C 工具链，已验证可用）。
- 注意版本：最新 `libsqlite3-sys 0.38` 用了 rustc 未稳定的 `cfg_select`，故钉 `rusqlite@0.32`。
- 新增 F 组持久化测试：写入→关闭连接→重开 .db→数据仍在。
- 结果：33 个测试全绿（A–E 的 31 个 + F 组 2 个），证明换数据库后端上层逻辑与测试一行不改。
- **zip 后置**：FsStore 已能正确存取/校验/恢复，zip 仅压缩优化，架构文档说 MVP 可接受不压缩。
  待真有压缩需求再接 zip crate 或 restic。

### 第 3 步：套上 Tauri 桌面外壳 ✅ 已完成
- `npm create tauri-app` 建 `savelink-app/`（React-TS 模板）。结构：`src/`=React 前端，
  `src-tauri/`=Rust 壳。
- `savelink-core` 作为路径依赖接入 `src-tauri`（保持 core 干净、可独立测试）。
- `src-tauri/src/commands.rs`：DTO 层 + AppState（系统数据目录下真 SqliteRepo + FsStore）
  + 8 个 Tauri command，全部真调用 core（非空壳）。
- 命令清单：list_games / list_snapshots / scan_path / add_game / create_snapshot /
  update_snapshot_meta / delete_snapshot / restore_snapshot。
- Rust 编译通过；前端依赖已装、`npm run build` 通过。
- 待用户本机验证：`npm run tauri dev` 弹真窗口（需 GUI 桌面，当前环境无法跑）。

### 第 4 步：用 React 实现前端 ✅ 已完成
- 照 `demo-front-1` 原型图，在 `savelink-app/src/` 用 React 实现全部页面与交互。
- 文件结构：`lib/types.ts`（与 Rust DTO 对齐）、`lib/api.ts`（**数据访问抽象层，当前 mock**，
  第 5 步的替换点）、`lib/format.ts`、`lib/icons.tsx`（内联 SVG）、`App.css`（设计令牌）、
  `App.tsx`（主壳）、`components/`（Toast / AddGameDialog / RestoreDialog / SnapshotDrawer）。
- **关键设计**：组件只调用 `api.ts`，不直接碰 invoke。第 5 步只改 api.ts 每个函数体
  （mock 数组 → await invoke），组件一行不用动。
- 浏览器验证（Vite dev + Preview 工具）：5 页面渲染正确、创建快照真新增时间线、
  恢复确认弹窗+三步进度+自动生成 before_restore 全部跑通，零控制台错误。
- `npm run build`（tsc + vite）通过。

### 第 5 步：前后端接线 ⬅ 下一步
- 把 `lib/api.ts` 里每个函数从操作 mock 数组，改成 `await invoke("命令名", {参数})`。
- 竖切逐条打通：添加游戏→看列表 → 创建快照 → 恢复。每通一条在真窗口验一次。
- 目录选择器（原型里"选择目录"假按钮）在此接系统对话框（tauri-plugin-dialog）。
- 处理真实边界：目录权限、大文件、游戏运行时文件锁。
- 注意：此步必须在 `npm run tauri dev`（真 Tauri 窗口）下验证，纯 Vite 没有 invoke。

### 第 6 步：打包 + 真机测试
- 打包 Windows .exe；用真实游戏存档走完整流程。
- 验收：能发给别人安装使用。

## 关键事实（防遗忘）

- 工作目录：`D:\door\codex_workspace\save_link`
- 核心 crate：`savelink-core/`（零依赖、可离线 `cargo test`）
- 原型图：`demo-front-1/index.html`（单文件、纯 mock，当高保真原型用，不是真前端）
- 五份文档：mvp-product-prototype / low-fidelity-wireframe / visual-interaction-guidelines
  / tech-architecture / restore-test-spec
- 已实现(真)：model / error / scan / service / store::FsStore
- 仍是替身(假)：repo::InMemoryRepo（内存）、store::FsStore（复制目录而非 zip）、testkit（仅测试）
- Codex 暂不参与开发，按本路线图由 Claude 推进。
- 进度的客观判据：`cargo test` 红绿灯 + 文件是否存在，不依赖记忆。
