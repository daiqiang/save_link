# SaveLink Steam 自动发现与多存档目录验收报告

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | Codex | 2026-08-10 | 首次真实 Steam 发现和隔离多目录快照/恢复验收 |

## 验收范围

- 代码提交：`3a832bd feat: 接入 Steam 自动发现与多目录存档`
- 应用版本：`0.2.0`
- 真实 Steam：`C:\Program Files (x86)\Steam`
- 隔离 profile：`SAVELINK_TEST_DATA_DIR=acceptance-data/steam-multidir-20260810/profile`
- 多目录写入测试全部使用工作区假存档；未对真实 Steam 存档创建快照或执行恢复。
- 真实百度网盘不在本次范围内。

## 自动验证

- `savelink-core cargo test --no-fail-fast`：90 个默认测试通过，0 失败；J/L 两个真实百度测试按设计忽略。
- `savelink-app/src-tauri cargo test --no-fail-fast`：2 个测试通过。
- `npm.cmd run build`：通过。
- `build-portable.bat --no-open`：通过。
- 绿色版 ZIP SHA-256：`633BFB13EB2DC88103BBE2F3962384DE2DAADAE5C9490CCA62EEFE8DA1F9E2CB`。
- ZIP 包含 `SaveLink.exe`、README、`manifest.db`、Manifest 来源 JSON 和 Ludusavi 许可证；本机搜狗压缩产生的空 `log/` 目录属于已确认的外部环境现象。

## 真实窗口结果

### Steam 自动发现

- Steam 关闭状态下完成自动发现。
- 识别 1 个 Steam 游戏库、13 个 appmanifest、8 个 Manifest 候选。
- 候选包含 Elden Ring、Goose Goose Duck、Liar's Bar、《杀戮尖塔》1/2、Street Fighter 6、Super Auto Pets、The Binding of Isaac: Rebirth。
- 《杀戮尖塔》显示 4 个保护目录：`betaPreferences`、`preferences`、`runs`、`saves`。
- 两个纯配置文件归一为 1 个安装父目录，并显示在“不纳入快照的纯配置路径”。
- 当前机器只有 1 个 Steam 游戏库，真实多库扫描尚未验收；N 组自动测试已覆盖多库枚举和分组。

### 多存档目录

- 从 Steam 候选添加《杀戮尖塔》后，首页和编辑页均保留 4 个目录。
- 四个目录可分别测试读取，再整体保存到游戏记录。
- 快照 A：5 个文件、129 B、4 个存档目录。
- 跨目录修改、增加和删除后创建快照 B：6 个文件、155 B、4 个存档目录。
- 从未备份状态 C 恢复快照 A：5 个目标文件的集合与内容全部匹配，额外文件被删除，被删文件恢复，无 `.old` 或 staging 残留。
- 再恢复快照 B：6 个目标文件的集合与内容全部匹配。
- 恢复确认、快照详情、编辑游戏和移除确认均显示全部 4 个目录，长路径没有遮挡按钮或正文。
- 移除游戏后，四个假存档目录及内容保持不变；隔离 SQLite 中 `games=0`、`snapshots=0`，仓库快照目录为空。

## 发现的问题

### BUG-STEAM-01：纯配置父目录误入保护路径

Elden Ring 的 Manifest 规则包含：

```text
[save]   <winAppData>/EldenRing/<storeUserId>
[config] <winAppData>/EldenRing/GraphicsConfig.xml
```

主工程会把精确文件规则归一到父目录。实际候选结果为：

```text
将保护的存档目录：
- .../EldenRing
- .../EldenRing/76561198820991451

不纳入快照的纯配置路径：
- .../EldenRing
```

纯配置父目录同时进入了保护路径，并与真实数字用户存档目录形成父子重叠来源。该问题在真实绿色版、自动发现和单游戏复核中可稳定复现。

预期：纯 `config` 规则只能进入 `config_paths`；`save_paths` 只保留真正带 `save` 标签的路径。修复后应增加“纯配置归一父目录不进入 recommended”的自动回归，并重新验证 Elden Ring 候选只显示 1 个保护目录。

## 结论

- 多存档目录：通过。扫描、编辑、快照、指纹变化、A/B 往返恢复、详情展示和移除安全均符合预期。
- Steam 自动发现：有条件通过。真实候选发现和《杀戮尖塔》添加可用，但 `BUG-STEAM-01` 修复前不能认为候选分类完全验收通过。
- 数据安全：本轮未写入真实 Steam 存档；所有恢复和删除测试均在隔离假目录及隔离 profile 中完成。
