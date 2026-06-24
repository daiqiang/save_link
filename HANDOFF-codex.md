# SaveLink 开发交接（给 Codex）

> 角色约定：Claude 是主管（定方向、定规则、审验收），Codex 是开发（按本文档执行）。
> 本文档是给 Codex 的工作说明。开工前**先读完本文档 + `PROGRESS.md`**，再动代码。
> 最后更新：2026-06-24（Claude）

---

## 一、现在是什么状态

本地 MVP **已完成并打包**成 Windows 安装包。核心闭环可用：
添加游戏 → 选存档目录 → 创建快照 → 时间线 → 恢复（恢复前自动备份）。

- `savelink-core`：纯逻辑，33 个测试全绿。
- `savelink-app`：Tauri 桌面应用，前端 React + 后端 Rust 薄壳。
- 已产出 MSI / NSIS 安装包 + 独立 exe。

**你接手的是一个能跑、被测试保护的代码库，不是空白项目。** 不要推倒重来。

## 二、必须先读的文档（按顺序）

1. `PROGRESS.md` —— 进度、6 步路线、产物位置。**每完成一项，回来更新它。**
2. `savelink-tech-architecture.md` —— 架构、数据模型、恢复事务设计（看「实现现状与偏差」节）。
3. `savelink-restore-test-spec.md` —— 恢复/存储的验收基准（你改 core 必须守它）。
4. `savelink-core/README.md` + `savelink-app/README.md` —— 两个 crate 的结构与铁律。
5. 产品三件套（做什么/页面/气质）：`savelink-mvp-product-prototype.md`、
   `savelink-low-fidelity-wireframe.md`、`savelink-visual-interaction-guidelines.md`。

## 三、不可触碰的红线（违反即打回）

1. **不丢存档是最高优先级。** 任何会写/覆盖/删真实存档目录的逻辑，改动前后都要让
   `savelink-core` 的 B 组、E 组测试全绿。恢复必须「备份成功才覆盖 + 同盘原子 rename」。
2. **不准改测试来迁就实现。** 测试是验收尺。允许加测试、不允许削弱既有断言。
   不准绕过 `FailingStore` 故障注入（那是测试存在的理由）。
3. **不准破坏存储抽象。** 上层只认不透明 `storage_key`；换 zip/restic 就新实现 `SnapshotStore`，
   换数据库就新实现 `Repository`，service 与测试不动。
4. **前端不准在组件里直接 `invoke`。** 一律走 `savelink-app/src/lib/api.ts`。
5. **不准把 Kopia / Ludusavi / repository / manifest 等内部概念暴露到用户界面。**（见视觉规范）
6. **不向用户界面暴露内部错误细节**；面向用户的文案要说「下一步怎么办」。

## 四、每次改动的标准动作

- 改了 `savelink-core`：`cd savelink-core && cargo test --no-fail-fast` 必须全绿。
- 加了新功能（端到端）：按这条链补全 ——
  ① core 加逻辑 + 补测试 → ② `savelink-app/src-tauri/commands.rs` 包 DTO 命令
  → ③ `src-tauri/src/lib.rs` 的 `generate_handler!` 注册 → ④ 前端 `lib/api.ts` 加调用 → ⑤ 组件用它。
- 改了前端：`cd savelink-app && npm run build` 要过；涉及 invoke 的真验证用 `npm run tauri dev`。
- 完成后更新 `PROGRESS.md`，并在需要时回报主管（Claude）。

## 五、待办（优先级从高到低）

这些都是 **MVP 之后**的增强，不是 bug。按序做，每项做完测试全绿 + 更新 PROGRESS。

### P1：真 zip 存储（替换 FsStore）
- 现状：`store.rs` 的 `FsStore` 是「目录复制」，不压缩、占空间。
- 做法：新增 `ZipStore implements SnapshotStore`（引 `zip` crate），`storage_key` 存 zip 文件名。
  **service 与 D 组测试不许改**——只是把 `AppState` 里 new 的实现换掉，D 组测试对新实现重跑应仍全绿。
- 验收：D 组（含往返无损 D1、损坏检测 D2）对 ZipStore 全绿。

### P2：编辑游戏 / 删除游戏
- 现状：详情页「编辑游戏」按钮是占位 toast。
- 做法：core 加 `update_game`/`delete_game`（删游戏要级联删其快照与物理文件，走确认）；
  补命令 + 前端弹窗。删除是高风险，参照删快照的确认与回滚纪律。

### P3：多存档目录
- 现状：一个游戏一个目录（数据结构已支持多个，`save_paths` 是数组）。
- 做法：添加游戏弹窗支持「添加另一个目录」；`scan`/快照/恢复对多目录聚合。
  注意 content_hash 与恢复的原子性要覆盖全部目录。

### P4：阶段 2 自动化（见产品文档路线图阶段 2）
- 文件变化检测（`notify` crate）、游戏退出后自动快照、保留策略（清理时跳过 locked）。

### P5：应用图标
- 现在是 Tauri 默认图标。换成 SaveLink 自己的图标（`src-tauri/icons/`，用 `tauri icon` 生成）。

> 阶段 3（游戏识别，复用 Ludusavi manifest）、阶段 4（云端，只同步快照仓库绝不碰真实存档）
> 属更后期，开工前先找主管确认范围。

## 六、有疑问时

- 涉及「会不会丢用户存档」的任何判断 —— **停下来问主管，不要自己拍板**。
- 拿不准是不是该改测试 —— 默认「不改」，问主管。
- 产品行为/文案拿不准 —— 以产品三件套文档为准；仍不清楚再问。
