# SaveLink 开发交接（给 Codex / 后续开发者）

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | 代强 | 2026-07-02 | 更新当前状态与待办，记录总规划会话注意点；同步基线收口要求；接入启动自检 |
| 1.1 | 代强 | 2026-07-08 | 同步恢复校验增强、35 个 core 测试与云同步增量协议路线 |
| 1.2 | 代强 | 2026-07-14 | 同步百度网盘 POC、协议 v1、本机云基础设施和 Fake 双设备闭环；core 增至 50 个测试 |

> 角色约定：总规划会话负责方向、范围和验收判断；本地开发会话负责按文档实现、验证、打包。
> 开工前先读完本文档 + `PROGRESS.md`，再动代码。
> 最后更新：2026-07-14（Codex）

---

## 一、现在是什么状态

SaveLink 当前是一个可运行、可打包、可用测试数据验收的 Windows 本地桌面 MVP。

核心闭环已经可用：

```text
添加游戏 -> 选择存档目录 -> 创建快照 -> 时间线 -> 恢复 -> 恢复前自动备份
```

MVP 后第一轮补齐也已完成：

- 编辑游戏名称和存档路径。
- 移除游戏（不删除真实存档目录）。
- 删除/锁定/备注快照。
- 存档目录缺失时支持“创建目录并恢复”。
- 恢复失败页按错误类型给出更准确文案。
- 设置页展示版本、数据目录、仓库目录、数据库文件。
- 设置页支持打开/复制路径。
- 修复 Tauri opener ACL 权限问题。
- Tauri 启动时执行 `startup_self_check`，清理上次异常中断留下的 Writing 半成品快照。
- 打包脚本可构建绿色版、NSIS、MSI。
- 百度网盘 OAuth、应用目录、元数据/快照上传下载和 sha256 校验 POC 已通过。
- 2MiB 基准测试已确认百度后端使用单快照 zip + `.ok`；本地快照仍由 `FsStore` 以目录形式保存在 `repository` 中。
- 云同步本机基础设施已实现：4 张新表、`CloudStateRepository`、`CloudObjectStore`、文件系统 `FakeCloudObjectStore` 和 G 组测试。
- 协议 JSON、`ZipCloudArchiveCodec` 和 `CloudSyncService` 已实现；设备 A -> Fake 云端 -> 设备 B 的上传、发现、下载、接收落地闭环已通过 H 组测试。

当前客观状态：

- `savelink-core`：50 个测试全绿，其中 G/H 组 15 个覆盖云同步基础设施和无网络双设备闭环。
- `savelink-app`：Tauri 桌面应用，React 前端 + Rust 命令薄壳。
- 数据位置：`%APPDATA%\com.daiq.savelink\`。
- 产物位置：`savelink-app/src-tauri/target/release/`。

**这是一个能跑、被测试保护、已经经过多轮人工验收的代码库，不是空白项目。不要推倒重来。**

## 二、必须先读的文档（按顺序）

1. `PROGRESS.md`：当前进度、已完成能力、产物位置、后续候选项。
2. `SaveLink-current-status-for-planning-20260630.md`：给总规划会话看的当前状态摘要。
3. `savelink-tech-architecture.md`：架构、数据模型、恢复事务设计、实现现状与偏差。
4. `savelink-restore-test-spec.md`：恢复/存储的验收基准。
5. `savelink-core/README.md` + `savelink-app/README.md`：两个模块的结构与铁律。
6. 产品三件套：
   - `savelink-mvp-product-prototype.md`
   - `savelink-low-fidelity-wireframe.md`
   - `savelink-visual-interaction-guidelines.md`
7. `savelink-cloud-sync-protocol-v1.md`：已定稿的云端目录、对象格式、发现、上传、下载、冲突和失败恢复协议。
8. `baidu-netdisk-api-poc-report-20260714.md`：OAuth/文件 API 实测、2MiB 基准数据和单快照 zip 决策证据。

## 三、不可触碰的红线

1. **不丢存档是最高优先级。**
   任何会写、覆盖、删除真实存档目录的逻辑，改动前后都要让 `savelink-core` 的 B 组、E 组测试全绿。恢复必须遵守“备份成功才覆盖 + 同盘原子 rename”。

2. **不准改测试来迁就实现。**
   测试是验收尺。允许新增测试，不允许削弱既有断言。不准绕过 `FailingStore` 故障注入。

3. **不准破坏存储抽象。**
   上层只认不透明 `storage_key`。换 zip/restic 就新实现 `SnapshotStore`，换数据库就新实现 `Repository`，service 与测试不动。

4. **前端不准在组件里直接 `invoke`。**
   一律走 `savelink-app/src/lib/api.ts`。

5. **不准把内部实现概念暴露给用户。**
   Kopia、Ludusavi、repository、manifest、storage_key 等不应直接出现在用户界面。

6. **不要向用户暴露内部错误堆栈。**
   面向用户的文案要说清楚“发生了什么、有没有动真实存档、下一步怎么办”。

7. **Tauri 权限要最小化。**
   例如 opener 目前只允许打开 `$APPDATA` 与 `$APPDATA/**`。不要为了方便开放整个磁盘。

## 四、每次改动的标准动作

- 改了 `savelink-core`：
  ```text
  cd savelink-core
  cargo test --no-fail-fast
  ```
  必须全绿。

- 改了前端：
  ```text
  cd savelink-app
  npm run build
  ```
  必须通过。

- 加了端到端功能：
  1. `savelink-core` 加逻辑并补测试。
  2. `savelink-app/src-tauri/src/commands.rs` 包 DTO / Tauri 命令。
  3. `savelink-app/src-tauri/src/lib.rs` 的 `generate_handler!` 注册命令。
  4. `savelink-app/src/lib/api.ts` 加调用函数。
  5. 组件只调用 `api.ts`。
  6. 用真实 Tauri 窗口或绿色版验证。

- 改了打包相关：
  ```text
  cd savelink-app
  .\build-installer.bat
  ```

- 完成后更新：
  - `PROGRESS.md`
  - 必要时更新 `savelink-app/手动测试计划.md`
  - 如果影响架构或安全边界，更新 `savelink-tech-architecture.md` / `savelink-restore-test-spec.md`

## 五、当前后续候选项（总规划会话重排后）

这些都不是当前阻塞项，需要总规划会话排优先级后再做。

### P0：基线收口

- 同步 `PROGRESS.md`、当前状态文档、交接文档和测试计划。
- 修正代码顶部残留的 “todo/红灯” 旧注释。
- 跑 `cargo test --no-fail-fast`、`npm run build`。
- 做一次轻量端到端冒烟验证。
- 无阻塞问题时提交基线收口。

### P1：正式回归验收

把当前绿色版按 `savelink-app/手动测试计划.md` 跑一轮，输出通过/失败清单。

### P2：已完成 - 启动自检接入 Tauri 启动路径

Tauri setup 已在 `AppState::init` 后、`app.manage(state)` 前调用 `startup_self_check`。后续正式回归时，可补一条真实窗口场景下的 Writing 残留清理验收。

### P3：真实恢复进度事件

core 已有 progress 回调概念，当前 Tauri 命令传空回调。可改为通过 Tauri event 向前端推送步骤进度。

### P4：真 zip / restic 存储

当前 `FsStore` 是把快照目录树写入 `repository` 的 `SnapshotStore` 实现，不压缩。若要降低空间占用和快照文件数量，新增 `ZipStore` 或接 restic。不要让 service 解析具体存储结构。

### P5：应用图标和安装体验

当前仍可能显示 Tauri 默认图标。换 SaveLink 自定义图标时，需要重新生成 `src-tauri/icons/` 并打包验证。

### P6：多存档目录

模型有 `save_paths` 数组，但 UI 和恢复目前按第一个目录运行。多目录会影响扫描、恢复原子性、错误提示，必须单独设计。

### P7：自动化 / 游戏识别 / 云端

- 文件监听、游戏退出后自动快照、保留策略。
- 游戏识别可考虑复用 Ludusavi manifest。
- 云端原则：同步 SaveLink 的共享快照事实和云端元数据，绝不直接同步真实存档目录。
- 不把 `latest.zip` 或整份 `savelink.db` 原样同步作为云端主线；主线是云端目录结构 + 增量同步。
- 百度网盘快照物理格式已定为每条快照一个 `{snapshot_id}.zip`，zip 上传成功后再生成并上传云端 `{snapshot_id}.ok`。
- 云端快照协议 v1 已定稿：按游戏目录发现，不使用全局 `snapshots.json`；有效 `.ok` 是不可变发布记录。
- v1 本机删除不删除云端副本，上传后备注/锁定修改暂不跨设备同步，避免第一版引入误删和可变元数据冲突。
- `app_settings`、`cloud_accounts`、`cloud_game_bindings`、`cloud_snapshot_sync` 已落库；旧数据库打开时自动补表。
- 协议 JSON、`CloudArchiveCodec` 和 `CloudSyncService` 已实现，Fake 云端已完成“上传 -> 发现 -> 下载 -> 双重校验 -> 写入另一套本机 repository -> 登记 SQLite”闭环。
- 云同步术语统一为“接收云端快照并落地到本机快照仓库”；“导出/导入”只用于未来独立的手动备份功能。
- 下一步实现 `BaiduNetdiskStore`，用真实百度网盘替换 Fake 适配器，并在同一 `CloudSyncService` 上复验闭环。
- 手动 zip 导出/导入只作为备份迁移工具，当前优先级低。

## 六、有疑问时

- 涉及“会不会丢用户存档”的任何判断：停下来问总规划会话。
- 拿不准是不是该改测试：默认不改，问总规划会话。
- 产品行为/文案拿不准：先查产品三件套；仍不清楚再问。
- 文档与代码不一致：以代码和测试为准，同时更新文档。
