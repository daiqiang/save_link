# SaveLink v0.4.0 DeSmuME 模拟器支持方案

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | 代强 | 2026-08-14 | 定义 v0.4.0 的 DeSmuME 范围、ROM 身份、精确存档文件保护、跨设备恢复和验收边界；补目录环及 ROM 哈希栈溢出防护 |

## 1. 版本目标

v0.4.0 只支持 **DeSmuME 0.9.x**，不试图一次覆盖所有模拟器。目标是让用户可以：

```text
选择 DeSmuME 目录
  -> 自动读取 ROM 目录和 Battery 存档
  -> 选择一个已有 .dsv 存档的游戏
  -> 创建本地快照
  -> 按快照时间线恢复
  -> 在另一台电脑重新选择 ROM 后继续恢复
```

Yuzu 延后到 v0.5.0。`.ds0` 至 `.ds9` 即时存档也不在 v0.4.0 保护范围内。

## 2. DeSmuME 存档事实

DeSmuME 绿色版通常让多个游戏共用一个 `Battery/` 目录，并按 ROM 文件名保存游戏内存档：

```text
ROM：zzjb2r ver0.99.nds
存档：Battery/zzjb2r ver0.99.dsv
历史即时存档：Battery/zzjb2r ver0.99.dsv-01
```

因此不能把整个 `Battery/` 当成一个游戏的存档。SaveLink 只保护当前 ROM 对应的精确 `.dsv` 文件，其他游戏的 `.dsv` 和 `.dsv-01` 等文件保持不动。

## 3. ROM 身份与匹配

SaveLink 不上传 ROM，也不保存 ROM 内容，只记录少量身份信息：

| 信息 | 用途 |
| --- | --- |
| ROM 文件名 | 第一次扫描时展示和寻找对应 `.dsv` |
| ROM SHA-256 | 内容相同即认定为同一 ROM，支持改名和跨设备确认 |
| NDS Header Title | SHA 不同时作为弱匹配依据 |
| Game Code | SHA 不同时作为弱匹配依据，例如 `TMXJ` |

匹配规则：

1. 模拟器相同且 SHA-256 相同：精确匹配，可直接绑定。
2. SHA-256 不同但 Header Title 与 Game Code 都相同：候选匹配，显示风险并要求用户确认。
3. 其他情况：不自动绑定。

公司电脑真实样本的 ROM 信息为：`METALMAX2R`、`TMXJ`，SHA-256 为 `189754a134b0919a0a837f5a5453f217a5f2b5a897c3d8232422a5ab81228f1c`。

## 4. 数据模型

### 4.1 普通游戏

普通游戏继续使用已有的整目录来源：

```text
Game.save_sources = Directory { path }
```

这保证 v0.1-v0.3 的数据库和行为兼容。

### 4.2 DeSmuME 游戏

DeSmuME 游戏使用精确文件来源和逻辑文件映射：

```text
Game.save_sources = Files {
  root: ".../Battery",
  files: [{
    local_relative_path: "zzjb2r ver0.99.dsv",
    snapshot_relative_path: "save.dsv"
  }]
}
```

快照只保存逻辑文件 `save.dsv`，不把设备 A 的绝对路径写入快照或云端协议。设备 B 绑定新 ROM 后，恢复流程把 `save.dsv` 写入设备 B 当前 ROM 对应的 `.dsv` 文件名。

## 5. 发现流程

1. 用户在“添加游戏”中选择 DeSmuME 根目录。
2. SaveLink 检查目录和 DeSmuME 可执行文件。
3. 读取 `desmume.ini` 的 `PathSettings`。
4. 如果配置的 ROM 目录失效，明确提示用户重新选择 ROM 目录；不静默使用错误路径。
5. 迭代扫描 `.nds` 文件，跳过目录链接并对 canonical 路径去重；按需计算 SHA-256，读取 NDS Header Title 和 Game Code。
6. 用 ROM 文件名寻找 `Battery/<ROM 文件名>.dsv`，不把 `.dsv-01` 当作游戏内存档。
7. 没有 `.dsv` 的 ROM 可以展示，但 v0.4.0 暂不能添加，因为当前版本只保护已有游戏内存档。

扫描是只读操作，不修改 ROM、`desmume.ini`、`Battery` 或 `StateSlots`。

## 6. 快照、恢复与安全边界

- 创建快照只读取目标 `.dsv`，共享 `Battery` 中其他文件变化不会触发本游戏快照。
- 恢复只替换目标 `.dsv`，不会清空或替换整个 `Battery` 目录。
- 恢复前不自动创建保护点；用户需要先手动创建快照。
- 恢复使用临时文件、备份和校验，失败时回滚目标文件。
- 目标 `.dsv` 不存在时允许创建其父目录并恢复；未得到用户确认前不写入。
- `.ds0` 至 `.ds9` 暂不扫描、快照或恢复。

## 7. 云同步边界

云端仍使用现有协议 v1、单条快照 zip 和 `.ok` 提交标记：

- `game.json` 增加可选 `emulator_identity`，旧 v1 文档没有该字段时仍可读取。
- 云端不保存 ROM 路径、DeSmuME 根目录、Battery 路径或设备 A 的本地路径。
- 设备 B 下载云快照后只得到 ROM 身份，必须重新扫描并绑定本机 ROM 才能恢复。
- 旧云端 `game.json` 没有 ROM 身份、而本机已有身份时，上传流程会以递增 revision 补写身份并校验发布结果。

## 8. 已实现代码入口

| 层次 | 入口 |
| --- | --- |
| 领域模型 | `savelink-core/src/model.rs` |
| DeSmuME 发现与 ROM 身份 | `savelink-core/src/desmume_discovery.rs` |
| 精确文件扫描与恢复 | `savelink-core/src/scan.rs`、`service.rs`、`store.rs` |
| SQLite 兼容迁移 | `savelink-core/src/sqlite_repo.rs` |
| 云协议与升级 | `savelink-core/src/cloud_protocol.rs`、`cloud_service.rs` |
| Tauri 命令 | `savelink-app/src-tauri/src/commands.rs` |
| React 添加流程 | `savelink-app/src/components/AddGameDialog.tsx` |

## 9. 验收范围

自动化验收覆盖：

- 失效 `desmume.ini` ROM 路径要求用户重选。
- 精确 `.dsv` 发现，排除 `.dsv-01`。
- Header Title、Game Code、SHA-256 解析。
- 512 KiB 小栈线程可完成 ROM 哈希，1 MiB 流式缓冲区不得放在线程栈上。
- ROM 目录存在链接环时不会递归耗尽线程栈。
- ROM 改名后身份保持一致。
- 共享 Battery 中只对目标文件创建快照和恢复。
- 目标 ROM 改名后跨设备逻辑文件映射恢复。
- 云端身份到达设备 B，但本地路径不跨设备传播。
- 旧云端 `game.json` 补写 ROM 身份。

真实 DeSmuME 样本的只读发现测试通过。真实可见 Tauri 窗口的添加、绑定和恢复仍需在用户桌面或家里电脑完成一次人工验收；这不改变核心实现已经通过的结果。

## 10. 明确不做

- v0.4.0 不支持 Yuzu；Yuzu 计划放在 v0.5.0。
- 不上传 ROM，不尝试从 ROM 内容推导游戏名称。
- 不自动修改 DeSmuME 配置，不启动或关闭模拟器。
- 不保护即时存档 `.ds0` 至 `.ds9`。
- 不在本版本实现所有模拟器通用的自动路径识别。
