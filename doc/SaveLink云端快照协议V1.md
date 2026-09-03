# SaveLink 云端快照协议 v1

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | 代强 | 2026-07-07 | 第一版：定义云同步数据边界、目录结构、增量协议与百度网盘可行性验证点 |
| 1.1 | 代强 | 2026-07-08 | 补充数据上云决策表，明确云端共享数据与本机私有数据 |
| 1.2 | 代强 | 2026-07-14 | 根据百度 POC 定稿协议 v1；同步 Fake 双设备实现，并统一“接收落地”术语 |
| 1.3 | 代强 | 2026-07-15 | 同步 BaiduNetdiskStore、OAuth/Token 本机连接层和真实百度验证状态，不改变协议 v1 语义 |
| 1.4 | 代强 | 2026-07-16 | 记录真实上云及设备 B 发现、下载、双重校验和接收落地验收，不改变协议 v1 语义 |
| 1.5 | 代强 | 2026-07-28 | 同步第一版直接恢复策略；保留旧 `before_restore` 协议兼容，不改变云端协议 v1 语义 |
| 1.6 | 代强 | 2026-07-29 | 迁入 doc 目录并改为中文文件名；同步文档引用 |
| 1.7 | Codex | 2026-08-29 | 兼容扩展：增加独立快照元数据对象，支持名称/锁定状态按字段跨设备同步、离线重试和清理保护 |
| 1.8 | Codex | 2026-09-02 | 标注 v0.6.0 已批准的云端删除语义；当前 v0.5.0 实现规则不变 |
| 1.9 | Codex | 2026-09-03 | 同步 v0.6.0 六轮评审修订：补齐真实账号/单快照 owner、分账号凭据、远端仓库 ID、可重试 manifest 和永久墓碑点查门槛；实现前仍保持 v0.5.0 行为 |

## 文档状态

本文是 SaveLink 云端快照协议 v1 的正式实现依据，不再是方向草案。

协议中的目录、字段、校验顺序和不可变约束，在实现第一条云同步闭环时应直接遵守。若以后需要破坏兼容性的调整，必须增加新的 `protocol_version`，不能静默改变 v1 文件含义。

## 结论先行

SaveLink v1 云同步采用以下模型：

```text
本机：FsStore 将快照以目录形式保存在 repository
云端：按游戏分目录，每条快照保存为 {snapshot_id}.zip + {snapshot_id}.ok
发现：列出 games/ 和各游戏 snapshots/，不维护全局 snapshots.json
同步：支持手动上传/下载、自动快照上传和十分钟元数据维护
恢复：下载只写入 SaveLink 本机快照仓库，用户主动恢复时才写真实游戏存档目录
```

云端有效快照的判定条件是：

```text
存在合法的 {snapshot_id}.ok
+ 存在对应 {snapshot_id}.zip
+ zip_size 与 zip_sha256 校验通过
+ 解压后 file_count、total_size、content_hash 校验通过
```

任何条件不满足，都不能写入本机时间线，更不能写入真实游戏存档目录。

## v1 目标

v1 必须支持以下闭环：

```text
设备 A 创建本地快照
-> 手动上传该快照
-> 设备 B 连接同一个云端仓库
-> 发现云端游戏和快照
-> 手动下载指定快照
-> 双重校验
-> 通过 FsStore 写入设备 B 的 repository
-> 登记设备 B 的 SQLite
-> 绑定设备 B 的真实存档目录
-> 使用现有安全恢复流程恢复
```

v1 不包含：

- 游戏启动前强制同步。
- 删除墓碑和跨设备删除传播。
- 多设备复杂冲突合并界面。
- 端到端加密、内容去重和增量块传输。
- 同时接入多个网盘账号。

## 术语与边界

- `SnapshotStore`：Rust 快照存储接口，定义创建、恢复、校验和删除行为。
- `FsStore`：当前实现 `SnapshotStore` 的代码类型，把目录树写入本机 `repository`。
- `repository/`：SaveLink 本机实际保存快照文件的数据目录，不是代码实现。
- `Repository` / `SqliteRepo`：元数据持久化接口和 SQLite 实现，与磁盘目录 `repository/` 不是同一个概念。
- 本机快照：`repository/snapshots/{snapshot_id}/` 及同级本机 `{snapshot_id}.ok`。
- 云端快照：云端 `{snapshot_id}.zip` 及云端 `{snapshot_id}.ok`。
- `cloud_game_id`：跨设备识别同一个游戏时间线的云端游戏 ID。
- `device_id`：本机随机生成的匿名设备 ID，只用于记录云端对象来源和诊断冲突。

本机与云端都使用 `.ok` 后缀，但不是同一个文件：

- 本机 `.ok` 由 `FsStore` 创建，当前内容是 `content_hash`。
- 云端 `.ok` 由云同步层创建，是 JSON 格式的不可变快照发布记录。

云同步层不能把本机 `.ok` 原样上传为云端 `.ok`。

## 核心不变量

### 1. 真实存档目录不上云

云端只保存 SaveLink 快照和共享元数据。`games.save_paths`、本机目录绑定和真实存档路径不得写入任何云端 JSON、zip、日志或对象路径。

### 2. 快照逻辑内容不可变

一个 `snapshot_id` 发布成功后，以下字段不得改变：

- `cloud_game_id`
- `created_at`
- `reason`
- `file_count`
- `total_size`
- `content_hash`

如果同一个 `snapshot_id` 在本机和云端对应不同内容，必须报告硬冲突，不得自动覆盖任意一方。

### 3. 云端快照以 `.ok` 为提交记录

zip 可以先存在，但只有合法云端 `.ok` 出现后，该快照才算发布成功。只有 zip、没有 `.ok` 的对象是未完成上传，不进入时间线。

### 4. SQLite 只做本机投影

`savelink.db` 不整库上传。云端对象是跨设备交换格式，本机 SQLite 是当前设备的运行数据库。

### 5. 拉取不等于恢复

拉取只允许写入 SaveLink 的临时目录、`repository` 和 SQLite。拉取过程绝不能写入 `games.save_paths` 指向的真实游戏存档目录。

### 6. 数据安全优先于自动清理

自动清理只处理本机明确识别的超额未锁定快照。删除前必须拉取可变元数据，任何同步失败都暂停清理；无法确认来源的云端对象仍不得自动删除。

## 标识符与时间格式

### 标识符

`repository_id`、`device_id`、`cloud_game_id` 和 `snapshot_id` 都是不可解析的稳定字符串。

要求：

- 长度为 1 到 128 个字符。
- 只允许 ASCII 字母、数字、下划线和连字符。
- 一旦写入云端不得修改。
- 业务代码不得从 ID 中解析时间、设备或游戏名称。

当前生产代码生成的 `game_{timestamp}_{counter}` 和 `snap_{timestamp}_{counter}` 满足 v1 要求，可以直接作为第一版 `cloud_game_id` 和 `snapshot_id`。未来可以改用 UUID，但协议不依赖具体生成算法。

### 设备 ID

第一次启用云同步时生成随机 UUID v4，保存为本机 `device_id`。不得使用计算机名、用户名、MAC 地址或硬盘序列号生成设备 ID。

### 时间

云端 JSON 时间统一使用 RFC 3339，并携带时区偏移，例如：

```text
2026-07-14T18:30:00+08:00
```

`game.json` 更新先比较显式 `revision`。快照可变元数据没有共享 revision，名称和锁定状态分别使用 `changed_at` 合并；同一时刻名称保留当前云端值，锁定状态取 `true`，保证结果稳定并优先防止误删。

当前 SQLite 中没有时区的旧时间，在首次上传时按该设备对应日期的本地时区解释并转换为 RFC 3339。另一台设备接收落地时，再转换为该设备本地展示时间。

## 云端逻辑目录

协议使用逻辑根目录 `savelink/v1/`。不同后端负责映射到自己的实际路径。

百度网盘映射为：

```text
/apps/savelink/v1/
```

完整结构：

```text
savelink/v1/
├── manifest.json
└── games/
    └── {cloud_game_id}/
        ├── game.json
        ├── snapshots/
            ├── snap_aaa.zip
            ├── snap_aaa.ok
            ├── snap_bbb.zip
            └── snap_bbb.ok
        └── snapshot-meta/
            ├── snap_aaa.json
            └── snap_bbb.json
```

路径规则：

- 游戏名称、备注和本机路径不得出现在对象路径中。
- 云端目录只使用经过校验的 ID。
- v1 不创建全局 `games.json`、`snapshots.json` 或 `tombstones.json`。
- 云端目录名 `repository` 不再作为 v1 协议路径，避免与本机 `repository/` 混淆。

## 为什么不使用全局索引

v1 以目录列表和每条快照自己的 `.ok` 作为事实来源：

```text
列出 games/ 子目录
-> 读取每个 game.json
-> 列出该游戏 snapshots/ 下的 .ok
-> 只下载本机尚未见过的 .ok
```

不使用全局 `snapshots.json` 的原因：

- 两台设备同时上传时，不会共同覆盖一个大 JSON 文件。
- 一条快照损坏不会让所有游戏的索引不可读。
- 新增快照只新增自己的 zip 和 `.ok`，符合增量同步目标。
- 云端目录就是最低限度的索引，第一版不需要额外一致性协议。

当单个游戏拥有大量快照、首次拉取 `.ok` 请求数过多时，可以在后续协议中增加“非权威缓存索引”。缓存索引只能优化速度，不能取代 `.ok` 作为事实来源。

## manifest.json

`manifest.json` 标识这是一个 SaveLink 云端仓库。它只在首次初始化时创建，不在每次上传后覆盖，因此不是多设备写入热点。

```json
{
  "protocol": "savelink-cloud-snapshot",
  "protocol_version": 1,
  "repository_id": "repo_550e8400-e29b-41d4-a716-446655440000",
  "created_at": "2026-07-14T18:30:00+08:00",
  "created_by_device_id": "device_550e8400-e29b-41d4-a716-446655440001"
}
```

规则：

- `protocol` 必须严格等于 `savelink-cloud-snapshot`。
- `protocol_version` 必须等于 `1`。
- `repository_id` 创建后不得改变。
- 已存在 manifest 时，客户端只能校验，不能用本机新值覆盖。
- 每台设备本机生成的 repository ID 只是在远端 manifest 缺失时用于 `CreateOnly` 的候选值；已有远端仓库以实际读取到的 `manifest.repository_id` 为准，并按内部云账号持久化为 `remote_repository_id`。本机候选值不同不能用于判断设备或账号不匹配。
- 并发初始化可以 single-flight，但瞬时失败只结束当前一轮并允许下次重试；成功缓存必须绑定真实账号和 credential generation。
- manifest 缺失但 `games/` 已存在数据时，必须提示仓库异常，不得自动重建并接管未知数据。

## game.json

每个云端游戏目录包含一个 `game.json`。它只保存跨设备展示信息，不保存本机真实路径。

```json
{
  "schema_version": 1,
  "object_type": "game",
  "cloud_game_id": "game_1752489000000000000_0",
  "name": "Elden Ring",
  "created_at": "2026-07-14T18:30:00+08:00",
  "revision": 1,
  "updated_at": "2026-07-14T18:30:00+08:00",
  "updated_by_device_id": "device_550e8400-e29b-41d4-a716-446655440001"
}
```

v1 字段规则：

- `schema_version` 固定为 `1`。
- `object_type` 固定为 `game`。
- `cloud_game_id` 必须与所在目录名一致。
- `name` 是非空展示名称，最大 256 个 Unicode 字符。
- `revision` 从 1 开始，只能递增。
- v1 不同步 `save_paths`、`repo_path` 和本机图标路径。

游戏名称更新采用读远端、递增 `revision`、覆盖 `game.json` 的方式。百度网盘不提供通用跨后端 CAS，极少数并发改名场景采用最后一次成功写入生效。该冲突只影响展示名称，不影响快照内容和本机路径。

第一条正式闭环只要求创建和读取 `game.json`，游戏名称跨设备编辑可以在同一协议下后续实现。

## 云端 `{snapshot_id}.ok`

云端 `.ok` 是一条快照的不可变发布记录，同时提供时间线所需元数据和两层校验信息。

```json
{
  "schema_version": 1,
  "object_type": "snapshot_commit",
  "snapshot_id": "snap_1752489060000000000_0",
  "cloud_game_id": "game_1752489000000000000_0",
  "created_at": "2026-07-14T18:31:00+08:00",
  "reason": "manual",
  "note": "Boss 前备份",
  "locked": true,
  "file_count": 12,
  "total_size": 20971520,
  "content_hash": {
    "algorithm": "savelink-fnv1a64-tree-v1",
    "value": "0123456789abcdef"
  },
  "archive": {
    "file_name": "snap_1752489060000000000_0.zip",
    "format": "zip",
    "layout_version": 1,
    "size": 20980000,
    "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  },
  "published_at": "2026-07-14T18:32:00+08:00",
  "created_by_device_id": "device_550e8400-e29b-41d4-a716-446655440001"
}
```

字段规则：

- `schema_version` 固定为 `1`。
- `object_type` 固定为 `snapshot_commit`。
- `snapshot_id` 必须与 `.ok` 文件名和 zip 文件名一致。
- `cloud_game_id` 必须与所在游戏目录一致。
- `reason` 只允许 `manual`、`before_restore`、`auto`。
- `note` 允许为 `null`，非空时最大 2000 个 Unicode 字符。
- `locked` 保存发布快照时的锁定状态。
- `file_count`、`total_size` 和内容哈希描述解压后的快照内容。
- `content_hash.value` 必须是 16 位小写十六进制字符串。
- `archive.size` 是 zip 文件字节数。
- `archive.sha256` 是 zip 文件的小写十六进制 SHA-256，共 64 个字符。
- `.ok` JSON UTF-8 编码，文件大小不得超过 64KiB。

v1 中 `.ok` 一旦发布就不再覆盖。上传后的名称和锁定状态由独立 `snapshot-meta/{snapshot_id}.json` 保存；缺少该文件的旧云端快照继续使用 `.ok` 中的发布值。

这样既保持内容发布记录不可变，也允许两个可变字段独立演进，不改变现有 `.ok` 的兼容语义。

## 云端 `snapshot-meta/{snapshot_id}.json`

```json
{
  "schema_version": 1,
  "object_type": "snapshot_metadata",
  "snapshot_id": "snap_1752489060000000000_0",
  "cloud_game_id": "game_1752489000000000000_0",
  "note": {
    "value": "Boss 前备份",
    "changed_at": "2026-08-29T12:00:00Z"
  },
  "locked": {
    "value": true,
    "changed_at": "2026-08-29T12:01:00Z"
  }
}
```

规则：

- 名称和锁定状态分别比较 `changed_at`，较新的字段获胜。
- 时间完全相同且名称不同，保留读取到的云端名称；锁定状态取 `true`。
- `device_id` 不参与胜负判断，避免设备间结果反复变化。
- 写入执行“读取、按字段合并、覆盖、重新读取验证”；失败写入本机独立元数据错误状态并在后续维护周期重试。
- 自动生成的“存档1”“锁定存档1”是本机展示名称，不写入 `note.value`；`display_zone` 也不上云。

## zip 内容格式

### 目录布局

zip 根目录直接保存快照内容，不额外包一层 `{snapshot_id}/`：

```text
{snapshot_id}.zip
├── ER0000.sl2
└── ER0000.sl2.bak
```

zip 中不得包含：

- 本机或云端 `.ok`。
- `savelink.db`。
- `game.json`、token、日志和本机配置。
- 真实存档目录的绝对路径。
- 符号链接、junction 或其他重解析点。

### 路径安全

每个 zip entry 必须满足：

- 使用相对路径。
- 路径分隔符统一为 `/`。
- 不允许空路径、绝对路径、盘符、UNC 路径和 `..`。
- 归一化后不得出现重复路径。
- 在 Windows 大小写不敏感规则下不得产生路径碰撞。
- 只允许普通文件和目录。

任一 entry 不满足要求，整个快照必须拒绝接收落地。

### 内容指纹

`savelink-fnv1a64-tree-v1` 与当前 `savelink-core/src/scan.rs` 保持一致：

1. 收集所有普通文件。
2. 相对路径使用 `/` 归一化并排序。
3. 对每个文件依次哈希：相对路径 UTF-8 字节、`0x00`、文件内容、`0xFF`。
4. 使用 FNV-1a 64-bit，offset 为 `0xcbf29ce484222325`，prime 为 `0x100000001b3`。
5. 输出 16 位小写十六进制字符串。

该内容指纹用于判断快照逻辑内容是否一致，不承担恶意篡改防护。zip 传输对象另用 SHA-256 校验。

### 解压限制

解压时必须边写边累计文件数和未压缩大小：

- 实际文件数不得超过 `.ok.file_count`。
- 实际未压缩大小不得超过 `.ok.total_size`。
- 解压完成后必须严格等于 `.ok.file_count` 和 `.ok.total_size`。
- 最终内容指纹必须等于 `.ok.content_hash.value`。

这些规则用于阻止 zip slip、路径碰撞和明显的 zip bomb。实现还应允许设置产品级单快照大小上限，但该上限不属于协议兼容字段。

## 数据上云边界

| 数据 | v1 是否上云 | 规则 |
| --- | --- | --- |
| 仓库协议和 repository ID | 是 | `manifest.json` |
| 云端游戏 ID 和名称 | 是 | `game.json` |
| 快照创建时间、原因 | 是 | 云端 `.ok` |
| 名称、锁定状态 | 是 | `.ok` 保存发布基线；后续修改写 `snapshot-meta/{snapshot_id}.json` |
| 文件数、总大小、内容指纹 | 是 | 云端 `.ok` |
| 单快照实际文件 | 是 | 单快照 zip |
| 真实游戏存档路径 | 否 | 只保存在本机 |
| `games.repo_path` | 否 | 本机历史字段 |
| 本机 `storage_key` | 否 | 云端物理路径由协议确定；接收落地后本机可使用 `snapshot_id` |
| `status = writing/corrupt` | 否 | 只有本机 complete 且 verify 通过才可上传 |
| 整份 `savelink.db` | 否 | SQLite 是本机投影 |
| access token / refresh token | 否 | 只保存在本机凭据存储 |
| 设备名称、用户名、计算机名 | 否 | v1 只上传随机 `device_id` |
| UI 状态、缓存和日志 | 否 | 可重建或可能包含隐私 |

## 本机数据模型要求

v1 不要求立刻拆掉现有 `games` 和 `snapshots` 表，但正式实现必须增加云同步状态，不能只靠内存判断。

建议新增以下语义。具体 SQL 迁移可以遵循当前 `SqliteRepo` 的命名习惯，但字段含义必须保留。

### app_settings

```text
key                TEXT PRIMARY KEY
value              TEXT NOT NULL
```

至少保存：

```text
device_id
```

### cloud_accounts

```text
id                 TEXT PRIMARY KEY
provider           TEXT NOT NULL
account_identity   TEXT
display_name       TEXT
token_ref          TEXT
created_at         TEXT NOT NULL
updated_at         TEXT NOT NULL
```

`token_ref` 只引用本机凭据位置，不得把 token 上传到云端。第一版只有一个百度账号，也仍然使用 `account_id` 作为隔离键，为后续多后端保留边界。

### cloud_game_bindings

```text
account_id          TEXT NOT NULL
cloud_game_id       TEXT NOT NULL
local_game_id       TEXT NOT NULL
remote_revision     INTEGER NOT NULL DEFAULT 0
sync_enabled        INTEGER NOT NULL DEFAULT 1
last_scanned_at     TEXT
PRIMARY KEY (account_id, cloud_game_id)
```

上传现有本机游戏时，v1 默认令 `cloud_game_id = games.id`。新设备首次拉取云端游戏时，也优先使用 `cloud_game_id` 创建同 ID 的本机游戏记录。

如果本机已经存在同 ID 游戏：

- 保留本机 `save_paths`，云端绝不能覆盖。
- 可以按 `game.json.revision` 更新名称。

如果只有名称相同但 ID 不同，v1 不自动合并，避免把两个不同游戏或版本误判为同一个游戏。

### cloud_snapshot_sync

```text
account_id          TEXT NOT NULL
cloud_game_id       TEXT NOT NULL
snapshot_id         TEXT NOT NULL
created_at          TEXT NOT NULL
reason              TEXT NOT NULL
note                TEXT
locked              INTEGER NOT NULL
file_count          INTEGER NOT NULL
total_size          INTEGER NOT NULL
content_hash        TEXT NOT NULL
archive_size        INTEGER NOT NULL
archive_sha256      TEXT NOT NULL
published_at        TEXT NOT NULL
created_by_device_id TEXT NOT NULL
sync_status         TEXT NOT NULL
last_synced_at      TEXT
last_error_code     TEXT
metadata_sync_status TEXT NOT NULL
metadata_last_synced_at TEXT
metadata_last_error_code TEXT
remote_note_updated_at TEXT NOT NULL
remote_locked_updated_at TEXT NOT NULL
PRIMARY KEY (account_id, snapshot_id)
```

`sync_status` 允许：

```text
uploading
uploaded
remote_only
downloading
downloaded
ignored
error
```

启动时发现残留 `uploading` 或 `downloading`，应转为可重试的 `error`，并清理对应本机临时文件。不得因此删除云端有效快照。

`metadata_sync_status` 独立允许 `synced / pending / error`，不能复用 zip 内容的 `sync_status`。`snapshots` 另保存 `note_updated_at` 和 `locked_updated_at`；旧库以创建时间初始化，若本机值与旧云缓存不同，则迁移时保留本机值并标为 `pending`。

## 云端发现协议

1. 下载并校验 `manifest.json`。
2. 列出 `games/` 的直接子目录。
3. 校验目录名是合法 `cloud_game_id`。
4. 下载并校验每个 `game.json`；游戏数量通常较少，v1 不为这一步增加额外索引。
5. 列出 `games/{cloud_game_id}/snapshots/`。
6. 只把以 `.ok` 结尾且 ID 合法的文件视为候选快照。
7. 对本机 `cloud_snapshot_sync` 中没有记录的候选下载 `.ok`。
8. 校验 `.ok` JSON、所在目录和字段之间的一致性。
9. 检查对应 zip 是否存在。
10. 读取可选的 `snapshot-meta/{snapshot_id}.json`；不存在时使用 `.ok` 的发布值。
11. 合法候选记为 `remote_only`；只有 zip 和 `.ok` 都合法才允许用户下载。

只有 zip、没有 `.ok` 的文件不进入时间线，也不需要下载 zip 内容。

## 上传协议

### 前置条件

只有同时满足以下条件的本机快照可以上传：

- SQLite `status = complete`。
- `SnapshotStore.verify(storage_key) = true`。
- 重新扫描本机快照目录得到的 `file_count`、`total_size`、`content_hash` 与 SQLite 一致。
- 所属游戏存在且名称非空。

上传必须从本机 `repository` 中已经完成的快照读取，不得直接从正在被游戏写入的真实存档目录打包。

### 上传步骤

1. 初始化或校验云端 `manifest.json`。
2. 创建或校验 `games/{cloud_game_id}/game.json`。
3. 在本机将同步状态写为 `uploading`。
4. 检查云端同名 `.ok`。
5. 若 `.ok` 已存在，先确认对应 zip 存在且远端大小与 `.ok.archive.size` 一致；缺失或大小不符时立即返回远端损坏错误。
6. 若 `.ok` 中的游戏 ID、快照 ID、创建时间、原因和内容统计与本机一致，视为幂等成功，不重复上传；本机后来修改的备注和锁定不参与该判断。
7. 若 `.ok` 已存在但上述不可变字段不同，返回 `snapshot_id_conflict`，不得覆盖。
8. 将本机 `repository/snapshots/{snapshot_id}/` 打包到本机临时 zip。
9. 计算 zip 字节数和 SHA-256。
10. 上传 `{snapshot_id}.zip`。当云端只有同名孤儿 zip、没有 `.ok` 时允许覆盖该 zip 并重试。
11. 上传完成后重新查询远端文件大小；大小不符则失败，不生成 `.ok`。
12. 根据本机快照元数据、zip 大小和 SHA-256 生成云端 `.ok`。
13. 以“文件不存在时创建”的方式上传 `{snapshot_id}.ok`；若并发操作导致文件已存在，则下载并按第 5 到 7 步重新判断。
14. 重新下载或读取云端 `.ok`，确认内容与本机生成值一致。
15. 将本机同步状态写为 `uploaded`，记录 `archive_sha256` 和时间。
16. 读取并按字段合并 `snapshot-meta/{snapshot_id}.json`，覆盖写入并回读校验；失败只标记元数据同步错误，不回滚已经成功发布的 zip/`.ok`。
17. 删除本机临时 zip。

上传 `.ok` 成功但本机状态尚未来得及更新时，下次重试会在第 5 步识别为幂等成功。

## 下载与本机接收落地协议

### 下载步骤

1. 用户选择一个 `remote_only` 快照。
2. 将本机同步状态写为 `downloading`。
3. 再次下载并校验 `.ok`，避免使用过期缓存。
4. 将 zip 下载到本机临时文件 `{snapshot_id}.zip.part`。
5. 下载完成后校验文件大小和 SHA-256。
6. 将 zip 解压到独立临时目录，执行路径安全检查。
7. 对解压目录重新计算 `file_count`、`total_size` 和 `content_hash`。
8. 任一校验失败，删除临时数据，写入 `error`，不得改动本机快照仓库和时间线。
9. 确保本机存在对应游戏记录和 `cloud_game_bindings`。
10. 本机不存在该游戏时，用 `cloud_game_id` 创建游戏，`save_paths` 为空，状态为“未绑定本机存档目录”。
11. 在本机 SQLite 插入 `status = writing` 的快照记录。
12. 以解压目录为输入，通过 `SnapshotStore.create` 写入本机 `repository`。
13. 调用 `SnapshotStore.verify`。
14. 校验通过后把 SQLite 快照状态更新为 `complete`。
15. 将同步状态更新为 `downloaded`，记录 `archive_sha256`。
16. 删除 zip 和解压临时目录。

接收并写入本机 `Snapshot` 时：

- `id = snapshot_id`
- `game_id = 本机绑定的 local_game_id`
- `storage_key = SnapshotStore.create` 返回值，当前 `FsStore` 下等于 `snapshot_id`
- `reason`、创建时间和内容统计来自云端 `.ok`；`note`、`locked` 优先来自独立元数据对象
- `status` 只有完成本机写入和 verify 后才变为 `complete`

如果进程在第 11 到 14 步之间崩溃，现有 `startup_self_check` 必须能够按 `writing` 记录清理半成品。云同步实现不得绕过这条安全链路直接复制进正式目录。

### 重复下载

本机已经存在同 ID 快照时：

- 内容字段完全一致：视为幂等成功，更新同步状态，不重复写入。
- 内容字段不同：返回 `snapshot_id_conflict`，不覆盖本机快照。
- SQLite 有记录但 `SnapshotStore.verify` 失败：先标记本机快照损坏，再由用户明确选择重新下载；不能静默覆盖。

## 本机绑定与恢复规则

新设备可以先发现云端游戏和下载快照，但在绑定本机真实存档目录前不能恢复。

UI 应区分：

```text
云端游戏已发现
快照已下载到本机
尚未绑定本机存档目录
```

用户绑定目录后，恢复继续使用现有安全规则：

1. 确认页提醒用户当前存档不会自动备份。
2. 校验目标快照。
3. 先恢复到临时目录并验证。
4. 将真实目录暂存为 `.old`，再换入目标目录。
5. 最终校验通过后删除 `.old`；失败时换回原目录。

云同步协议不新增绕过安全恢复流程的写入入口。

## 备注和锁定状态

当前兼容扩展的明确行为：

- 上传时把当前 `note` 和 `locked` 写入不可变云端 `.ok`。
- 另一台设备先读取独立元数据；不存在时回退到 `.ok`。
- 上传后修改名称或锁定状态，立即尝试同步；断网时保存 `pending/error`，由十分钟维护周期重试。
- 两个字段独立合并，某台设备只修改锁定状态时不会覆盖另一台设备更新的名称。
- v1 仍不覆盖云端 `.ok`。

元数据未同步成功时，该快照不得执行云端保留清理。

## 删除规则

### 删除本机快照

v1 删除本机快照时：

> 以下是 v0.5.0 当前实现。v0.6.0 已决定改为：已上云快照先持久化删除意图并立即从正常界面隐藏，再发布不可变的逐快照 tombstone，然后清理 metadata、`.ok`、ZIP，远端成功后才删除本机。尚未运行或等待中的同快照上传由删除取代，运行中的请求不强制中断。详细对象格式、状态机、旧库迁移和验收标准见 `SaveLink-v0.6.0云任务调度与冲突治理方案.md`。在对应代码完成前，不得把计划行为写成已经实现。

- 删除本机 SQLite 快照记录和 `repository` 中的物理文件。
- 不调用云端删除 API。
- 对已经存在于云端的快照保留 `cloud_snapshot_sync` 记录，并标记为 `ignored`。
- 后续扫描云端时不自动重新下载该快照。
- 用户可以通过“重新下载”显式清除 `ignored` 并再次拉取。

### 保留策略删除云端快照

当前保留策略只清理本机已知的未锁定超额快照。删除前必须先完成元数据拉取和合并；元数据状态不是 `synced` 时暂停。删除顺序为独立元数据对象、`.ok`、`.zip`、本机物理快照和 SQLite 记录。任一云端删除失败都保留本机数据等待重试。

v0.6.0 目标顺序调整为：创建或确认 snapshot tombstone，再删除独立元数据对象、`.ok`、`.zip` 和本机快照。tombstone 长期保留；所有 v0.6+ 客户端上传前后、接收下载前与本机提交前、metadata 写入前后、上传恢复补记成功前都必须检查。旧仓库缺少该可选目录等价于空集合；缓存只有在本地与远端修改时间都存在且相等时命中。并发删除只比较 schema、对象类型和 repository/game/snapshot 身份，审计字段采用远端先创建版本。合法 tombstone 缓存后，本机副本仍可编辑，但不再产生 metadata 云任务。一个本机快照只归属一个百度账号：删除只面向其 owner，owner 未激活时等待原账号，自动保留策略跳过，禁止把同一快照上传到第二账号。平铺 `snapshot-tombstones/{snapshot_id}.json` 是实现前候选布局；若 10,000 条下单对象检查不满足发布门槛，则在第一批正式对象写入前改为按 `sha256(snapshot_id)` 首个十六进制字符分成 16 桶。旧客户端不理解该扩展，因此发布说明必须要求同仓库设备一起升级。详细竞态规则和规模门槛以 v0.6.0 权威设计为准。

## 幂等与冲突规则

| 场景 | v1 处理 |
| --- | --- |
| 重复上传同 ID、同内容 | 成功，不重复上传 |
| 重复上传同 ID、不同内容 | 硬冲突，禁止覆盖 |
| 重复下载同 ID、同内容 | 成功，不重复写入本机快照仓库 |
| 重复下载同 ID、不同内容 | 硬冲突，禁止覆盖 |
| 不同 ID、相同内容 | 作为两条独立快照保留 |
| 同名游戏、不同 `cloud_game_id` | 不自动合并 |
| 同 ID 游戏、不同本机路径 | 保留各自本机路径 |
| 云端 game.json 并发改名 | 最后成功写入生效，只影响名称 |
| 上传后名称或锁定修改 | 按字段时间合并到独立元数据对象；平局时名称保留云端、锁定取 `true` |
| 本机删除云端快照副本 | 云端保留，本机标记 ignored |

## 中断、孤儿对象和临时文件

### 云端只有 zip、没有 `.ok`

视为孤儿 zip：

- 其他设备忽略。
- 不进入时间线。
- 同一 `snapshot_id` 的后续上传允许覆盖并继续发布。
- v1 不自动清理，以免误删尚可重试的数据。

### 云端有 `.ok`、没有 zip

视为损坏的已发布快照：

- 标记远端错误。
- 不允许下载或接收落地。
- v1 不自动覆盖 `.ok` 或重新发布。
- 后续可提供显式“从本机副本修复云端快照”工具。

### zip 与 `.ok` 不匹配

大小或 SHA-256 不一致时视为远端损坏。不得尝试解压，不得覆盖本机数据。

### 本机临时目录

建议使用：

```text
{data_dir}/cloud/tmp/{operation_id}/
```

启动时可以删除没有活跃操作引用的本机临时目录。清理本机临时文件不影响本机正式快照和云端对象。

## 错误码

云同步核心层至少应提供以下稳定错误码，UI 再映射为中文说明：

| 错误码 | 含义 |
| --- | --- |
| `auth_required` | 未连接云账号或授权失效 |
| `protocol_not_supported` | 云端协议版本不受支持 |
| `manifest_invalid` | manifest 缺失、损坏或字段不一致 |
| `game_metadata_invalid` | game.json 非法 |
| `snapshot_marker_invalid` | 云端 `.ok` 非法 |
| `snapshot_id_conflict` | 同一 ID 对应不同逻辑内容 |
| `remote_zip_missing` | `.ok` 存在但 zip 缺失 |
| `archive_size_mismatch` | zip 大小不一致 |
| `archive_hash_mismatch` | zip SHA-256 不一致 |
| `unsafe_archive_entry` | zip 路径或 entry 类型不安全 |
| `snapshot_content_mismatch` | 解压后内容统计或指纹不一致 |
| `rate_limited` | 云后端限流 |
| `network_unavailable` | 网络不可用或传输中断 |
| `local_store_failed` | 写入或校验本机快照仓库失败 |

错误信息和日志不得输出 access token、refresh token、SecretKey 或授权码。

## 云存储适配层

云同步业务层不能直接依赖百度网盘接口名、dlink 或应用目录细节。`CloudObjectStore` 只抽象逻辑对象操作：

```text
put_file(logical_path, local_file, overwrite_policy)
get_file(logical_path, local_file)
list_directory(logical_path)
stat_file(logical_path)
delete_file(logical_path)
```

v0.6.0 起，上述接口的远端存在性语义必须是新鲜的：`list_directory/stat_file` 每次查询后端，`get_file/delete_file` 不得根据旧父目录缓存判断目标，`put_file(CreateOnly)` 以服务端创建结果为权威。Store 不保存无 TTL 的跨任务 `directory_cache`；SQLite 缓存只用于跳过未变化对象的内容下载，不能替代目录枚举。共享范围限制为 HTTP Client 和限流器；Token Provider 只在同一账号凭据 generation 内共享，每个任务创建绑定 `account_id + generation` 的短生命周期 Store，目录确保集合也仅在任务内有效。

职责划分：

- `CloudObjectStore`：逻辑文件上传、下载、列目录、查询和删除。
- `BaiduNetdiskStore`：把逻辑路径映射到 `/apps/savelink/v1/`，处理 mkdir、upload、filemetas、dlink、限流和错误码，并通过 `BaiduAccessTokenProvider` 获取当前 token。
- `CloudArchiveCodec`：创建 zip、安全解压、计算 zip SHA-256。
- `CloudSyncService`：初始化/校验 manifest，并执行发现、上传、下载、冲突判断和本机状态更新。
- `SnapshotStore`：管理本机正式快照文件。
- `Repository`：管理本机游戏和本机完整快照元数据。
- `CloudStateRepository`：管理云账号、游戏绑定、远端快照缓存和同步状态。

OAuth 登录和 token 刷新属于账号连接层，不属于快照协议。其他网盘和 NAS 后端只需实现同一逻辑对象操作，不应改变 v1 云端对象语义。

清除授权或重新授权时必须先使旧 Runtime generation 失效，再安装新凭据并重建网络运行时。每一次百度 HTTP 请求都要取得与任务绑定账号和 generation 一致的请求租约；旧任务不能重新读取共享 Token 文件取得新账号凭据，也不能在 generation 变化后继续发出后续请求。旧 generation 的 Token 刷新、OAuth 回调和授权错误均采用 compare-and-apply，不能覆盖或清除新 Token；旧网络结果不得提交到新账号状态，任何目录和目录确保状态也不能跨账号继承。OAuth 成功后必须先取得稳定的真实账号身份：Token 按内部账号分别原子保存，最后才更新 SQLite 活跃账号；进程启动再次核对 Token 身份。同账号重新授权可以恢复该账号义务，切换账号则使用独立本机命名空间，旧账号义务保持暂停。

## 百度网盘映射

百度网盘后端必须遵守：

- 所有数据位于 `/apps/savelink/v1/`。
- 不依赖百度网盘客户端的文件夹同步或文件夹备份。
- 上传 zip 时由适配层处理普通上传或分片上传差异。
- 下载速度、会员限速和 dlink 获取细节不得泄漏到 `CloudSyncService`。
- API 返回的 md5 可以用于诊断，但不能替代协议要求的 zip SHA-256。

百度网盘 POC 已验证 OAuth、应用目录、上传、覆盖、列表、下载、中文路径和 sha256 完整性。尚未验证的上线审核、普通用户账号授权、长期 token 轮换、限流和大文件分片属于产品化工作，不改变本协议的数据模型。

2026-07-15 已实现正式 Rust `BaiduNetdiskStore`，并通过本地 HTTP 契约测试和真实百度对象上传、列表、下载、校验、删除冒烟。同日已跑通桌面端系统浏览器 OAuth、本机回调、Token 文件持久化和 SQLite `token_ref` 登记；Token 不进入云端对象。

2026-07-16 正式客户端已通过 `CloudSyncService` 将一条本机快照真实上传为同 ID 的 `.zip` 与 `.ok`，并在 SQLite 记录 `uploaded`；设备 B 随后在隔离数据目录中完成只读发现、下载、双重校验和接收落地。发现阶段不创建本机游戏，接收成功后才创建未绑定游戏；协议 v1 的对象格式无需修改。

## 第一条正式实现闭环

第一条工程任务只实现以下纵向切片：

```text
1. 在设备 A 的测试数据目录创建一个本地游戏和快照
2. 连接百度网盘测试账号
3. 初始化 manifest.json
4. 上传 game.json
5. 上传一条 snapshot zip
6. 上传该 snapshot .ok
7. 使用独立的设备 B 数据目录扫描云端
8. 发现 game.json 和 snapshot .ok
9. 下载指定 zip
10. 校验 zip 和解压后内容
11. 通过 FsStore 写入设备 B repository
12. 写入设备 B SQLite
13. 在设备 B 时间线看到该快照
14. 绑定设备 B 测试存档目录
15. 使用现有安全恢复流程恢复
```

第一条闭环可以先通过后端命令或测试入口驱动，但最终必须接入同一个 Tauri 应用。不能把 POC 工程直接复制进产品仓库充当正式实现。

截至 2026-07-16，第 1 至 13 步已通过真实百度网盘和设备 B 隔离模式验收。第 14 至 15 步属于写入本机目标目录的恢复阶段；必须使用假存档目录，并复用现有“同盘替换 + 恢复后校验 + 失败换回原目录”流程。

## 验收标准

### 正常链路

- A 上传后，云端目录严格符合 v1。
- B 不知道 A 的真实存档路径。
- B 能发现云端游戏和快照。
- B 下载后本机 `SnapshotStore.verify` 通过。
- B SQLite 元数据与云端 `.ok` 一致。
- B 未绑定路径时不能恢复。
- 绑定后恢复不自动创建快照；确认页提醒用户如需保留当前状态，应先手动创建快照。

### 幂等

- A 重复上传同一快照不会创建重复对象或覆盖不同内容。
- B 重复扫描不会创建重复游戏和快照。
- B 重复下载不会重复写入时间线。

### 故障

- 删除云端 `.ok` 后，B 忽略孤儿 zip。
- 修改 zip 任意字节后，B 在解压前因 SHA-256 失败而停止。
- 构造危险 zip entry 时，B 返回 `unsafe_archive_entry`。
- 修改解压后文件时，内容指纹校验失败，SQLite 不出现 complete 快照。
- 下载或接收落地中断后，启动自检不会把半成品显示为正常快照。
- 同 ID 不同内容时，两端数据均不被自动覆盖。

### 数据边界

- 云端对象中不存在本机真实存档路径。
- 云端对象中不存在 token、SecretKey、数据库文件和日志。
- 拉取阶段不写入真实游戏存档目录。
- 删除本机云端快照副本不会删除云端有效对象。

## 后续演进

以下能力必须通过兼容扩展或新协议版本实现：

- 非权威的按游戏快照缓存索引，减少大量 `.ok` 下载请求。
- 云端删除、墓碑、回收站和多设备删除传播。
- 游戏启动前云端同步提醒。
- 多账号、多后端和同一快照多副本。
- 端到端加密、分块、去重和断点续传优化。
- 云端损坏快照的显式修复流程。

这些扩展不得改变已发布 v1 `.ok` 对逻辑快照内容的含义。

## POC 证据

百度网盘 POC 结论见 `百度网盘API-POC报告-20260714.md`。

2MiB 实测结果：

| 场景 | 目录上传 | 单快照 zip 上传 | 目录下载 | 单快照 zip 下载 |
| --- | ---: | ---: | ---: | ---: |
| 1 个文件，合计 2MiB | 5.28s | 5.17s | 26.49s | 27.63s |
| 100 个文件，合计 2MiB | 46.76s | 4.88s | 61.12s | 27.55s |

该结果支持 v1 使用“一条快照一个 zip”，而不是逐文件目录镜像，也不是包含全部历史的 `latest.zip`。

## 最终决定

SaveLink 云端同步的是不可变快照事实，不是某台电脑的整个数据目录。

v1 的最终数据路径是：

```text
本机 repository 目录快照
-> 单快照 zip
-> 云端 .ok 发布
-> 另一台设备按游戏目录发现
-> 下载并双重校验
-> 通过 FsStore 写入另一台设备 repository
-> 登记本机 SQLite
-> 用户主动恢复时才写真实存档目录
```
