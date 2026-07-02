# SaveLink 开发交接（给 Codex）

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | 代强 | 2026-07-02 | 更新当前状态与待办，记录总规划会话注意点 |

> 角色约定：Claude 是主管（定方向、定规则、审验收），Codex 是开发（按本文档执行）。
> 本文档是给 Codex 的工作说明。开工前**先读完本文档 + `PROGRESS.md`**，再动代码。
> 最后更新：2026-07-02（Codex）

---

## 一、现在是什么状态

本地 MVP **已完成并打包**成 Windows 安装包。核心闭环可用：
添加游戏 → 选存档目录 → 创建快照 → 时间线 → 恢复（恢复前自动备份）。

- `savelink-core`：纯逻辑，34 个集成测试全绿。
- `savelink-app`：Tauri 桌面应用，前端 React + 后端 Rust 薄壳。
- 已产出 MSI / NSIS 安装包 + 独立 exe。
- 已补齐编辑游戏、移除游戏、设置页、缺失目录创建恢复、设置页打开/复制路径等 MVP 实用功能。
- 当前更细的阶段状态见 `SaveLink-current-status-for-planning-20260630.md`；`PROGRESS.md` 可能仍落后，若冲突以当前状态文档和代码为准。

**你接手的是一个能跑、被测试保护的代码库，不是空白项目。** 不要推倒重来。

## 二、必须先读的文档（按顺序）

1. `SaveLink-current-status-for-planning-20260630.md` —— 当前真实功能完成情况与已知未完成点。
2. `PROGRESS.md` —— 进度、6 步路线、产物位置。**如果发现落后，先更新它再继续开发。**
3. `savelink-tech-architecture.md` —— 架构、数据模型、恢复事务设计（看「实现现状与偏差」节）。
4. `savelink-restore-test-spec.md` —— 恢复/存储的验收基准（你改 core 必须守它）。
5. `savelink-core/README.md` + `savelink-app/README.md` —— 两个 crate 的结构与铁律。
6. 产品三件套（做什么/页面/气质）：`savelink-mvp-product-prototype.md`、
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

## 五、待办（总规划会话重排后）

这些待办按当前总规划判断排序。完成每项后：测试全绿、前端 build 通过、更新 `PROGRESS.md` 与当前状态文档。

### P0：同步文档与交接状态
- `PROGRESS.md` 仍停留在 2026-06-24 的老状态，落后于 6 月 30 日后的实际功能。
- `SaveLink-current-status-for-planning-20260630.md` 已记录当前真实状态，但没有版本历史；若继续作为正式规划文档，需要补上。
- `savelink-core/src/service.rs` 顶部注释仍有“todo/红灯”旧说法，容易误导后续 agent。

### P1：可靠性加固
- `startup_self_check` 已在 core 中实现并有测试，但 Tauri 启动时还没有调用。
- 恢复进度事件尚未从后端 emit 到前端；现在前端只有整体进行中提示。
- 继续补真实边界：目录权限、大文件、游戏运行时文件锁、错误文案与回滚提示。

### P2：正式回归验收
- 基于 `savelink-app/手动测试计划.md` 和 `acceptance-data/` 做一次完整回归。
- 特别关注：添加/编辑/移除游戏、恢复前自动备份、缺失目录创建恢复、设置页打开权限、打包产物运行。
- 回归结果要落文档，失败项进入明确待办。

### P3：快照仓库存储优化
- 当前快照仓库仍是 `FsStore` 目录复制，不是 zip。
- 如果空间占用、文件数量、云同步效率开始成为问题，再新增 `ZipStore` 或 `ResticStore` 实现 `SnapshotStore`。
- 换存储实现时不准破坏 `storage_key` 不透明抽象，D 组存储测试必须全绿。

### P4：应用图标和安装体验
- 当前仍是 Tauri 默认图标。
- 换 SaveLink 自定义图标，重新生成 `src-tauri/icons/` 并打包验证。

### P5：多存档目录
- 当前 UI 和恢复流程主要按单目录使用，虽然模型已有 `save_paths: Vec<PathBuf>`。
- 多目录会影响扫描、快照、恢复原子性、错误提示与回滚语义；必须单独立项设计。

### P6：阶段 2 自动化
- 文件变化检测（`notify` crate）、游戏退出后自动快照、保留策略。
- 自动清理必须跳过 locked 快照；自动快照不能制造用户难以理解的大量版本。

> 阶段 3（游戏识别，复用 Ludusavi manifest）、阶段 4（云端，只同步快照仓库绝不碰真实存档）
> 属更后期，开工前先找主管确认范围。

## 六、有疑问时

- 涉及「会不会丢用户存档」的任何判断 —— **停下来问主管，不要自己拍板**。
- 拿不准是不是该改测试 —— 默认「不改」，问主管。
- 产品行为/文案拿不准 —— 以产品三件套文档为准；仍不清楚再问。
