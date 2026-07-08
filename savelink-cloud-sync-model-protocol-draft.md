# SaveLink 云同步数据模型与协议草案

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | 代强 | 2026-07-07 | 第一版：定义云同步数据边界、目录结构、增量协议与百度网盘可行性验证点 |
| 1.1 | 代强 | 2026-07-08 | 补充数据上云决策表，明确云端共享数据与本机私有数据 |

## 结论先行

SaveLink 云同步不应以 `latest.zip` 作为长期主线。`zip` 适合手动导出、迁移、离线备份，但不适合高频自动云同步，因为每次新增一个快照都可能导致整个数据包重新上传。

云同步主线应改为：

```text
云端目录结构 + 增量上传/下载 + 快照不可变约束
```

第一版云同步的目标不是“同步真实游戏存档目录”，而是同步 SaveLink 的云端 manifest、游戏/快照共享元数据，以及已校验完成的快照物理对象。真实游戏存档目录始终留在本机，只有用户主动点击恢复时才会写入。

## 背景

当前本地 MVP 已完成一轮正式回归验收，核心链路包括添加游戏、创建快照、恢复、恢复前自动备份、缺失目录处理、启动自检、移除游戏不删除真实存档等。

当前数据分布为：

```text
%APPDATA%\com.daiq.savelink\
├── savelink.db
└── repository\
    └── snapshots\
        ├── {snapshot_id}\
        └── {snapshot_id}.ok
```

`savelink.db` 存游戏和快照元数据；`repository/` 存快照实际文件。若只同步 `repository/`，另一台电脑无法完整展示时间线；若直接同步整个 `savelink.db`，又会把本机真实存档路径一起同步到另一台电脑，造成路径混乱。

因此，云同步前必须先拆清楚：

```text
哪些数据是云端共享数据
哪些数据是设备本地数据
```

## 目标

第一阶段云同步设计要支持：

- 公司电脑创建快照后，家里电脑能拉取并看到该快照。
- 家里电脑的真实存档路径可以与公司电脑不同。
- 用户在家里电脑可选择某个云端快照恢复到本机真实存档目录。
- 新增快照只上传新增快照文件和必要元数据，不重传整个历史包。
- 不直接同步真实游戏存档目录。
- 不依赖百度网盘的“文件夹同步”功能，走 SaveLink 自己的同步逻辑。

第一阶段不追求：

- 多设备复杂冲突自动合并。
- 云端历史 zip 版本。
- 按需懒下载单个快照。
- 端到端加密。
- 压缩/去重存储。
- 自动识别所有游戏路径。

## 核心原则

### 1. 真实存档目录不上云

云端保存的是 SaveLink 的共享元数据和快照物理对象，不是游戏运行时直接读写的真实存档目录。

错误方向：

```text
上传 D:\Games\EldenRing\save\
```

正确方向：

```text
上传 SaveLink 快照对象 snap_xxx
```

### 2. 快照内容不可变

一个 `snapshot_id` 一旦创建成功，其内容、文件数、大小、内容指纹都不应再改变。可变的只有备注、锁定、删除状态等元数据。

这个约束让增量同步成立：

- 旧快照不用反复上传。
- 新快照只需上传一次。
- 云端和本地可以通过 `snapshot_id`、`content_hash`、`file_count`、`total_size` 判断是否一致。

### 3. 本机路径是本机配置，不是云端共享事实

当前 `games.save_paths` 是真实存档路径，例如：

```text
D:\Games\EldenRing\save
```

但另一台电脑可能是：

```text
E:\SteamLibrary\EldenRing\save
```

因此，云端共享的游戏记录不能直接把某台设备的 `save_paths` 当成所有设备通用路径。第一版必须把“云端游戏身份”和“本机路径绑定”拆开。

### 4. 云同步优先 latest 状态，不做云端历史包

云端第一版只维护当前最新的 SaveLink 云端状态。游戏快照历史由 SaveLink 时间线负责，而不是由多个云端 zip 包负责。

## 当前 SQLite 数据分析

当前 `savelink.db` 有两张表：

```text
games
snapshots
```

`games` 当前字段：

```text
id
name
icon
repo_path
save_paths
created_at
updated_at
```

其中 `save_paths` 是本机真实存档路径，不适合原样云同步。

`snapshots` 当前字段：

```text
id
game_id
created_at
note
reason
locked
file_count
total_size
content_hash
storage_key
status
```

其中大部分适合作为云端共享元数据。需要注意：

- `storage_key` 当前等于 `snapshot_id`，但未来不应强依赖。
- `status = writing` 的快照不应上传云端。
- `status = complete` 且本地 `store.verify` 通过的快照才允许上传。

## 数据上云决策表

本表的“上云”指进入百度网盘/WebDAV/NAS 等云端后端的同步数据，不代表直接上传当前 `savelink.db`。第一版应优先把本机 SQLite 投影成云端 JSON 元数据，再与快照物理文件一起同步。

| 数据 | 当前来源 | 是否上云 | 原因 | 第一版策略 |
| --- | --- | --- | --- | --- |
| 云端仓库格式版本 | 新增 `cloud-manifest.json` | 上云 | 多设备需要判断协议兼容性 | 保存 `format_version`、`protocol_version`、`updated_at` |
| 云端最后更新时间 | 新增 `cloud-manifest.json` | 上云 | 用于判断云端是否有新状态 | 每次成功上传元数据后更新 |
| 最后修改设备 ID | 新增 `cloud-manifest.json` / 元数据项 | 上云 | 用于粗略判断是谁改了云端状态 | 上传匿名设备 ID，不上传设备名称或用户隐私 |
| 游戏 ID / 云端游戏 ID | 当前 `games.id`，后续可演进为 `cloud_game_id` | 上云 | 快照必须归属到同一个跨设备游戏身份 | 第一版可复用当前 `game_id`，但语义上视为云端游戏 ID |
| 游戏名称 | `games.name` | 上云 | 多设备需要展示同一个游戏名 | 同步到 `metadata/games.json` |
| 游戏图标 | `games.icon` | 可上云 | 图标是展示信息；当前实现未充分使用 | 第一版可同步字段，允许为空 |
| 游戏创建时间 | `games.created_at` | 上云 | 时间线和排序可用 | 同步到 `metadata/games.json` |
| 游戏更新时间 | `games.updated_at` | 上云 | 用于游戏元数据冲突判断 | 同步到 `metadata/games.json` |
| 游戏仓库路径 | `games.repo_path` | 不上云 | 当前是本机路径/历史字段，对云端无意义 | 不同步；云端路径由协议目录结构决定 |
| 真实存档路径 | `games.save_paths` | 不上云 | 每台电脑路径不同，原样同步会把公司路径带到家里 | 保存在本机绑定表；新设备拉取游戏后提示绑定本机存档目录 |
| 本机路径绑定时间 | 新增本机配置 | 不上云 | 只描述本机配置 | 本机保存，用于 UI 提示 |
| 本机是否启用某游戏同步 | 新增本机配置 | 不上云 | 不同设备可选择不同游戏同步 | 本机保存 |
| 快照 ID | `snapshots.id` | 上云 | 快照跨设备识别核心 ID | 同步到 `metadata/snapshots.json` |
| 快照所属游戏 ID | `snapshots.game_id` | 上云 | 需要挂到云端游戏身份下 | 同步到 `metadata/snapshots.json` |
| 快照创建时间 | `snapshots.created_at` | 上云 | 时间线核心展示数据 | 同步到 `metadata/snapshots.json` |
| 快照备注 | `snapshots.note` | 上云 | 备注是用户理解快照的关键元数据 | 同步；冲突先采用最后写入者胜出 |
| 快照原因 | `snapshots.reason` | 上云 | 区分手动、恢复前自动备份、未来自动快照 | 同步 `manual` / `before_restore` / `auto` |
| 快照锁定状态 | `snapshots.locked` | 上云 | 锁定用于保护关键快照，应跨设备生效 | 同步；锁定快照不允许删除 |
| 快照文件数量 | `snapshots.file_count` | 上云 | 校验和展示都需要 | 同步，并参与下载后校验 |
| 快照总大小 | `snapshots.total_size` | 上云 | 校验、展示、传输预估需要 | 同步，并参与下载后校验 |
| 快照内容指纹 | `snapshots.content_hash` | 上云 | 判断快照是否一致的核心校验值 | 同步，并作为恢复前/下载后校验依据 |
| 快照存储键 | `snapshots.storage_key` | 上云 | 云端需要定位快照物理对象 | 第一版可等于 `snapshot_id`，但协议不依赖二者永远相同 |
| 快照状态 | `snapshots.status` | 部分上云 | `writing` 是本机中间态，不应传播；`complete` 可作为上传前置条件 | 只上传 `complete` 且 verify 通过的快照；不上传 `writing` / `corrupt` |
| 快照实际文件 | `repository/snapshots/{snapshot_id}/` | 上云 | 这是可恢复存档的核心数据 | 增量上传；验证目录镜像和单快照 zip 两种物理格式 |
| `.ok` 完成标记 | `repository/snapshots/{snapshot_id}.ok` | 上云 | 校验快照完整性需要 | 与快照文件一起上传；下载后必须校验 |
| 删除墓碑 | 新增 `metadata/tombstones.json` | 上云 | 删除需要跨设备传播，否则无法区分“没上传过”和“已删除” | 第一版可设计并保留字段；是否实现删除同步可后置 |
| 快照元数据更新时间 | 新增字段 | 上云 | 备注/锁定冲突需要比较新旧 | 第一版新增 `updated_at`，最后写入者胜出 |
| 快照最后修改设备 ID | 新增字段 | 上云 | 冲突诊断需要 | 第一版保存匿名 `device_id` |
| 设备 ID | 新增本机配置 | 部分上云 | 本机需要稳定身份；云端只需要知道某条记录由哪个设备修改 | 本机生成并保存；作为 `updated_by_device` 写入元数据 |
| 设备名称 | 系统信息/用户输入 | 默认不上云 | 可能包含隐私，第一版不必要 | 第一版不上传；后续如展示设备来源再征求用户确认 |
| 百度 access token | 本机凭证 | 不上云 | 敏感凭证，绝不能进入云盘 | 存本机安全存储或配置；后续评估加密 |
| 百度 refresh token | 本机凭证 | 不上云 | 敏感凭证 | 同上 |
| AppKey / OAuth 配置 | 应用配置 | 不作为用户数据上云 | 属于应用接入配置，不是存档数据 | 随应用分发或本机配置 |
| 同步游标 / 上次同步时间 | 新增本机同步状态 | 不上云 | 描述本机同步进度 | 本机保存；可用于增量拉取 |
| 最近一次同步结果 | 新增本机同步状态 | 不上云 | 仅用于本机 UI 提示 | 本机保存 |
| UI 当前选中游戏/弹窗状态 | 前端状态 | 不上云 | 与跨设备存档无关 | 不同步 |
| 本机缓存/临时下载目录 | 新增缓存目录 | 不上云 | 可重建，不是事实数据 | 不同步 |
| 日志 | 本机日志 | 默认不上云 | 可能包含路径和错误信息 | 第一版不上云；如需诊断另做用户确认导出 |

### 决策摘要

第一版云同步要同步三类数据：

```text
1. 云端仓库 manifest
2. 游戏/快照共享元数据
3. 快照物理对象
```

第一版明确不上云：

```text
1. 真实游戏存档路径
2. 百度网盘 token
3. UI 状态和本机缓存
4. 某台电脑的完整 savelink.db 原文件
```

因此，百度网盘 API 最小实验只需要验证“上云数据”能否稳定上传、覆盖、列出、下载、校验。不上云的数据不要纳入 POC 范围。

## 云端数据模型草案

### 云端共享数据

云端应该保存：

- 云端游戏 ID。
- 游戏名称。
- 游戏图标或识别信息。
- 快照 ID。
- 快照所属游戏 ID。
- 快照创建时间。
- 快照备注。
- 快照原因：`manual` / `before_restore` / `auto`。
- 快照锁定状态。
- 文件数量。
- 总大小。
- 内容指纹。
- 存储键。
- 删除标记或删除墓碑。
- 元数据更新时间。
- 最后修改设备 ID。

### 本机私有数据

本机应该保存：

- 该云端游戏在本机绑定的真实存档路径。
- 本机设备 ID。
- 百度网盘 token。
- 本机同步状态。
- UI 选择状态。
- 本机临时目录、缓存目录。

### 建议新增概念

后续可以把当前 `games` 拆成两层概念：

```text
cloud_games
local_game_bindings
```

概念示意：

```text
cloud_games
  cloud_game_id
  name
  icon
  created_at
  updated_at

local_game_bindings
  cloud_game_id
  local_save_paths
  local_enabled
  last_bound_at
```

第一版不一定立刻重构 SQLite 表，但设计云同步时必须按这个边界思考。

## 云端目录结构草案

以百度网盘应用目录为例，云端可以长这样：

```text
/apps/SaveLink/
├── cloud-manifest.json
├── metadata/
│   ├── games.json
│   ├── snapshots.json
│   └── tombstones.json
└── repository/
    └── snapshots/
        ├── snap_aaa/
        │   └── ...
        ├── snap_aaa.ok
        ├── snap_bbb/
        │   └── ...
        └── snap_bbb.ok
```

### cloud-manifest.json

用途：标识云端仓库版本和同步协议版本。

示例：

```json
{
  "format_version": 1,
  "protocol_version": 1,
  "updated_at": "2026-07-07 17:30:00",
  "updated_by_device": "device_xxx"
}
```

### games.json

用途：保存云端共享游戏信息，不保存本机真实路径。

示例：

```json
[
  {
    "cloud_game_id": "game_xxx",
    "name": "EldenRing",
    "icon": null,
    "created_at": "2026-07-07 17:00:00",
    "updated_at": "2026-07-07 17:00:00"
  }
]
```

### snapshots.json

用途：保存快照时间线元数据。

示例：

```json
[
  {
    "snapshot_id": "snap_xxx",
    "cloud_game_id": "game_xxx",
    "created_at": "2026-07-07 17:10:00",
    "note": "Boss 前备份",
    "reason": "manual",
    "locked": false,
    "file_count": 12,
    "total_size": 20971520,
    "content_hash": "abcd1234",
    "storage_key": "snap_xxx",
    "updated_at": "2026-07-07 17:10:00",
    "updated_by_device": "device_xxx"
  }
]
```

### tombstones.json

用途：同步删除行为。直接从 `snapshots.json` 移除记录会让另一台设备难以判断“云端没这个快照”是没上传过，还是已经删除。

示例：

```json
[
  {
    "entity_type": "snapshot",
    "entity_id": "snap_old",
    "deleted_at": "2026-07-07 18:00:00",
    "deleted_by_device": "device_xxx"
  }
]
```

第一版可以先不实现复杂墓碑清理，但设计上要保留这个概念。

## 本地数据结构影响

当前如果直接同步 `savelink.db`，会带来两个问题：

1. 本机真实路径被同步到另一台电脑。
2. 多设备同时修改备注、锁定、删除时，整个数据库文件很难做字段级合并。

因此云同步更适合走“云端 JSON 元数据 + 本地 SQLite 投影”的方式。

也就是说：

- SQLite 仍然是本机运行数据库。
- 云端 JSON 是跨设备交换格式。
- 同步时把云端元数据导入/合并到本机 SQLite。
- 本机真实路径单独保存，不被云端覆盖。

这比直接上传/下载整个 `savelink.db` 更复杂，但更适合长期多设备。

## 增量同步协议草案

### 上传本机新增快照

条件：

- 快照 `status = complete`。
- `store.verify(storage_key) = true`。
- 云端没有该 `snapshot_id`。

上传内容：

```text
repository/snapshots/{snapshot_id}/
repository/snapshots/{snapshot_id}.ok
metadata/snapshots.json 中追加或更新该快照元数据
cloud-manifest.json updated_at
```

### 下载云端新增快照

条件：

- 云端 `snapshots.json` 有本机没有的快照。
- 云端快照文件存在。
- 云端 `.ok` 存在。

下载内容：

```text
repository/snapshots/{snapshot_id}/
repository/snapshots/{snapshot_id}.ok
```

然后写入本机 SQLite 的 `snapshots` 记录。

如果本机尚未绑定该游戏的真实存档路径，UI 应显示“需要在本机绑定存档目录后才能恢复”。

### 同步备注和锁定

备注和锁定是快照元数据，不改变快照内容。

第一版建议采用简单规则：

```text
updated_at 较新的元数据覆盖较旧元数据
```

若本机和云端同时修改同一快照备注，第一版可以接受“最后写入者胜出”，但必须在文档中承认这是简化策略。

### 同步删除

删除比新增更危险。

第一版建议：

- 删除快照时先写 tombstone。
- 云端看到 tombstone 后，其他设备同步删除本机对应快照记录和仓库文件。
- 锁定快照不允许删除，因此不会产生锁定快照的 tombstone。

第一版也可以先把“云端删除同步”延后，只同步新增和备注/锁定。若延后，UI 要说明“删除不会立刻同步到其他设备”。但长期必须设计 tombstone。

### 设备身份

每台设备应生成一个稳定 `device_id`，保存在本机配置中，不上传为用户身份，只用于同步冲突判断。

示例：

```text
device_20260707_xxx
```

## 冲突策略

第一版尽量避免复杂冲突，采用保守策略。

### 新增快照

不同设备新增不同 `snapshot_id`，可以合并。

### 同一快照内容冲突

如果本机和云端存在同一个 `snapshot_id`，但 `content_hash`、`file_count`、`total_size` 不一致，应视为严重冲突。

第一版处理：

- 不覆盖。
- 标记同步失败。
- 提示用户。

理论上这不应发生，因为 `snapshot_id` 应全局唯一且快照不可变。

### 备注/锁定冲突

第一版：

```text
updated_at 新者覆盖旧者
```

后续可增加冲突提示。

### 删除冲突

如果 A 设备删除快照，B 设备修改备注：

第一版可以规定：

```text
删除优先
```

但锁定快照不可删除，因此用户可通过锁定保护关键快照。

## 第一版建议范围

第一版云同步不要一次做满。建议最小闭环：

1. 设计本机设备 ID。
2. 设计云端目录结构。
3. 支持上传本机完整云端元数据。
4. 支持上传本机新增快照。
5. 支持下载云端新增快照。
6. 支持本机重新绑定真实存档目录。
7. 暂不做自动后台同步，先做手动“上传到云端 / 从云端拉取”。

第一版不做：

- 自动定时同步。
- 后台常驻。
- 云端历史 zip。
- 增量压缩。
- 多设备复杂冲突 UI。
- 按需下载单个快照。
- 删除墓碑清理。

## 百度网盘 API 可行性验证点

下一轮百度网盘 API 实验应验证这些能力：

1. 能否在应用目录下创建 SaveLink 根目录。
2. 能否创建多级目录。
3. 能否上传小文件：`cloud-manifest.json`、`games.json`、`snapshots.json`。
4. 能否上传快照目录中的普通文件。
5. 能否覆盖同名元数据文件。
6. 能否查询远端目录列表。
7. 能否读取文件大小、md5、修改时间。
8. 能否下载指定文件。
9. 能否删除文件或目录。
10. 大文件上传是否需要分片。
11. 上传速度是否受限到不可接受。
12. API 权限是否只限制在应用目录内。

百度网盘实验不应先做完整 UI。建议先做命令行或测试脚本，验证最小链路：

```text
创建目录 -> 上传 metadata -> 上传一个快照目录 -> 列表查询 -> 下载 -> 校验内容
```

## 待确认问题

- 百度网盘 API 是否能高效列出目录下大量文件。
- 百度网盘 API 的 md5 是否稳定可用。
- 大量小文件上传是否性能过差。
- 是否需要把单个快照目录打成单快照 zip，而不是整个 SaveLink 打成 latest zip。
- 云端元数据 JSON 变大后是否需要拆分。
- 当前 SQLite 是否立刻重构，还是先做同步适配层。
- 删除同步第一版是否纳入。

## 单快照 zip 的备选思路

虽然整个 `latest.zip` 不适合云同步，但“单个快照一个 zip”可能仍有价值。

例如：

```text
repository/
└── snapshots/
    ├── snap_xxx.zip
    └── snap_xxx.ok
```

优点：

- 新增快照时只上传一个新 zip。
- 避免百度网盘大量小文件上传效率差。
- 单快照仍然不可变，适合增量同步。

缺点：

- 当前 `FsStore` 是目录复制，需新增 `ZipStore` 或云端打包层。
- 恢复前需要解压。
- 需要重新评估校验策略。

这个方向值得在百度 API 实验后再定。如果百度网盘上传大量小文件体验很差，单快照 zip 可能比目录镜像更适合云端。

## 当前推荐路线

当前不建议优先做手动导出 `latest.zip`。

推荐下一步：

1. 做百度网盘 API 最小上传/下载实验。
2. 同时验证目录镜像和单快照 zip 两种快照上传方式的性能。
3. 根据实验结果决定云端快照物理格式。
4. 在代码层面先设计云同步适配层，不直接把 `savelink.db` 整库上传。

一句话：

```text
云同步要同步 SaveLink 的共享快照事实，而不是同步某台电脑的整个本地数据库。
```
