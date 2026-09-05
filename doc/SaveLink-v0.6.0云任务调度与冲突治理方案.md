# SaveLink v0.6.0 云任务调度与冲突治理方案

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | Codex | 2026-09-02 | 第一版：确定云任务类型、资源键、优先级、并发规则、状态机、删除语义、未读提醒和实施顺序 |
| 1.1 | Codex | 2026-09-03 | 根据七轮评审完成最终收口：冻结任务、迁移和 tombstone 契约；补齐仓库 ID 不一致、账号核验分类与跨账号接收边界 |

> 本文是 v0.6.0 云同步改造的权威设计。文中标为“目标状态”的内容尚未实现；当前 v0.5.0 的真实行为仍以代码、`SaveLink技术架构.md` 和 `SaveLink云端快照协议V1.md` 为准。

## 一、决策摘要

v0.6.0 在继续增加云功能前，先正式引入统一的云任务协调器。核心决策如下：

1. 同一快照的云操作严格串行；通常按受理顺序执行，但删除是终止性操作，可以取代尚未运行或正在等待的同快照任务。已经运行的任务不强制中断，结束后再删除。
2. 不同快照允许有限并发；同一游戏的上传先保守串行，避免并发改写共享的 `game.json`。
3. 云端目录刷新是账号级一致性屏障：刷新期间不与该账号的快照写任务交叉执行，但所有请求进入队列等待，不再直接报“正在进行后台云同步”。
4. 顶层云任务最多同时执行 3 个；百度 HTTP 请求全局最多 8 个，其中后台任务最多占 6 个，始终给用户任务保留容量。
5. 队列保存在 Tauri 进程内存中，不新增通用 `cloud_tasks` 持久化表。上传、元数据、删除等跨重启义务继续由 SQLite 业务状态恢复。
6. HTTP Client 和限流器改为应用级共享；Token Provider 按账号凭据 generation 建立不可变会话并采用 single-flight 刷新。任务绑定真实账号身份和创建时的 generation，每次 HTTP 请求取得同 generation 的请求租约，旧任务不能读取或操作后来连接的新账号；Token 按内部账号分别原子落盘，带目录状态的 Store 不跨任务共享。
7. 用户删除已上云快照时，立即写入独立的删除意图并从正常界面隐藏；同一快照的 `Queued/WaitingRetry/WaitingAuth` 上传由删除取代，`Running` 上传结束后再执行删除。
8. 内容同步状态、元数据同步状态和删除状态必须拆开：删除意图在 `snapshots`，跨设备删除生命周期在专用 `cloud_snapshot_tombstones`，不能继续让 `sync_status` 同时表达上传和删除。
9. 没有合法 tombstone 时，本地备注或锁定状态先立即落库，再异步同步；使用本地元数据 revision 防止旧任务覆盖用户的新修改。已有合法 tombstone 的本机副本仍可编辑，但只改本地，不再产生 metadata 云任务。
10. 云端删除先发布不可变 tombstone，再删除 metadata、`.ok` 和 ZIP；v0.6+ 客户端上传前后都检查 tombstone，防止离线任务重新发布同一 snapshot ID。
11. 后台发现远端新增、有效元数据变化或 tombstone 时增加持久化 generation；云端入口显示小红点，窗口成功展示对应 generation 后才标记已读。
12. 后台发现只读取目录、tombstone、`.ok` 和发生变化时必要的 metadata，不自动下载 ZIP；ZIP 仍只在用户明确点击下载时传输。
13. 操作排队、内容冲突和网络失败是三类不同问题，界面和状态不能再统一显示成“云冲突”。
14. tombstone 不只约束上传：接收、metadata 写入和上传中断恢复都必须在关键提交前复查；发现删除事实后不得发布或落地新的同 ID 对象。
15. `snapshot-tombstones/` 是 v0.6.0 可选扩展目录；旧仓库中该目录不存在等价于空集合，不能导致刷新失败。
16. tombstone 缓存只有在本地和远端修改时间都存在且相等时才能命中；并发创建只比较协议身份字段，审计字段采用远端先创建成功的版本。
17. 远端存在性属于实时事实：`CatalogRefresh` 及上传、接收、metadata、恢复、删除的对象查询都必须访问当时的云端状态，不得使用无 TTL 的进程级目录缓存。
18. 永久 tombstone 的规模验收不仅覆盖全量刷新，还覆盖单条上传、接收、metadata 和恢复检查的点查请求数；10,000 条下出现随目录总量线性放大的点查时阻止发布，再根据实测选择百度直接查询能力或哈希前缀分片。
19. 一个本机快照最多归属一个百度账号；首次上传或接收时确定 `cloud_owner_account_id`，之后禁止向第二个账号上传。未激活账号拥有的快照不参与自动淘汰，也不在当前账号执行云删除或 metadata 同步。
20. 远端 manifest 的 `repository_id` 必须按账号持久化为 `remote_repository_id`；每台设备随机生成的本机 `repository_id` 只在创建空远端仓库时作为候选，绝不作为账号或 legacy 归属证据。
21. manifest single-flight 只共享当前一轮结果：成功缓存绑定账号和 credential generation；失败唤醒等待者后回到可重试状态，不能永久缓存第一次网络错误。
22. `remote_repository_id` 只允许首次绑定：为空时原子写入、相同则继续、不同则进入派生的 `repository_mismatch` 并停止该账号全部仓库同步，绝不覆盖旧值。
23. 百度账号身份只取 `uinfo` 的稳定 `uk`；401/Token 无效、网络/429/5xx、成功但 `uk` 不一致分别进入等待授权、等待重试和安全异常，不能都当成重新授权。
24. 重复接收同 ID、同内容只有在来源账号等于本机 owner，或 owner 仍为空并由本次事务成功认领时才幂等成功；另一账号无论内容是否相同都不得落地或建立第二条同步关系。

本轮适合直接引入队列。继续在现有全局互斥上叠加例外，会让上传、刷新、元数据和删除之间的关系越来越难以证明正确。

## 二、目标与非目标

### 2.1 目标

- 用户云操作不再因为十分钟后台维护而立即失败。
- 同一对象不会被上传、更新元数据、下载和删除同时修改。
- 多个互不相关的快照可以利用百度网盘允许的并发能力。
- 删除、自动上传和元数据修改在断网或应用重启后能够继续收尾。
- 所有已升级到 v0.6.0 或更高版本的设备都不能自动复活已经写入 tombstone 的快照 ID。
- 远端变化对用户可见，但不把本机自己刚完成的上传误报为新消息。
- 对硬冲突、授权失效、限流、断网和普通排队给出不同结果。
- 为未来两分钟游戏运行期快照提供可复用调度基础，但 v0.6.0 不在本方案中实现两分钟快照。

### 2.2 非目标

- 不做可手动排序、暂停、继续或取消传输的完整任务中心。
- 不强制中断已经发出的 HTTP 请求。
- 不自动恢复真实游戏存档。
- 不在后台自动下载远端 ZIP。
- 不让云队列阻塞本地快照创建；断网时本地快照仍应正常产生。
- 不在第一版实现多个云账号同时在线，但必须识别并持久化真实提供方账号身份，所有业务状态和资源键按内部 `account_id` 隔离；切换账号不复用旧账号义务。
- 不保证仍在运行 v0.5.0 或更早版本的客户端理解 tombstone；启用 v0.6.0 强删除语义前应升级同一云仓库的所有 SaveLink 设备。

## 三、当前实现为什么需要改

v0.5.0 使用 `baidu_sync_in_progress: AtomicU8` 表示空闲、后台任务或用户任务。任一云流程取得它后，其他流程直接失败。十分钟维护又把以下操作串成一个大批次：

```text
待同步 metadata
  -> 云端目录发现
  -> 自动快照上传
  -> 超额快照联合删除
```

因此，即使用户只想读取云端列表，也可能碰到后台批次；即使后台只处理一条 metadata，也会占用与完整发现相同的全局互斥。当前 `BaiduCloudRuntime::build()` 还会为每次任务创建新的 Token Provider。若直接放开并发，各 Provider 的刷新锁互不共享，会产生重复刷新和 Token 覆盖风险。

另一个结构性问题是 `CloudSyncStatus` 同时包含上传、下载和 `delete_pending/deleting/delete_failed/remote_deleted`。例如上传正在执行时用户请求删除：如果界面为了立即隐藏而先写 `delete_pending`，旧上传完成后又可能把状态写回 `uploaded`，删除意图就丢失了。该问题必须通过独立状态机解决，不能只靠调整调用顺序。

## 四、术语与职责边界

### 4.1 云任务

云任务是一项可排队、可去重、会占用网络槽位的工作。例如上传一个快照、刷新一次目录。它的状态只存在于当前进程。

### 4.2 业务义务

业务义务是必须跨重启保留的事实。例如“这个自动快照尚未上传”“这条 metadata 等待同步”“用户已经要求删除”。它保存在 SQLite，由启动和十分钟维护重新生成云任务。

### 4.3 维护扫描

维护扫描只读取 SQLite、整理本地快照区域并向协调器提交任务，本身不占百度 HTTP 槽位，也不持有云资源锁等待网络。它不是一个长时间运行的网络任务。

### 4.4 协调器与执行器

- `CloudTaskCoordinator`：受理、合并、排序、资源冲突判断、状态广播和工作线程调度。
- `CloudTaskExecutor`：执行具体百度网盘和本地提交步骤，不自行决定优先级。
- `CloudRuntime`：持有共享 HTTP Client、HTTP 限流器、账号凭据会话/generation、manifest 初始化 single-flight 和工作目录；Token Provider 只在当前 generation 内共享，不持有长期目录结果。
- `BaiduNetdiskStore`：由每个任务从 `CloudRuntime` 创建的短生命周期适配器，复用共享网络组件；父目录确保集合只在本任务内有效，不保留跨任务目录缓存。
- `CloudSyncService`：每个任务创建，持有该任务的 Store，继续负责协议校验、打包、上传、下载、metadata 合并和幂等逻辑。

协调器属于 Tauri 应用层，因为它涉及应用生命周期、前端事件和任务来源；纯调度算法应隔离成不依赖 Tauri 的模块，用 Fake 执行器做确定性测试。

## 五、云任务类型

| 类型 | 触发来源 | 主要动作 | 是否可靠恢复 |
| --- | --- | --- | --- |
| `CatalogRefresh` | 用户打开/刷新云端窗口；十分钟后台发现 | 增量枚举远端目录；可选 tombstone 目录不存在按空集合处理，存在则先完整读取并验证，再处理新增 `.ok` 和变化的 metadata | 后台刷新无需恢复；下个周期会重做 |
| `SnapshotUpload` | 用户手动上传；新自动快照；自动上传失败重试 | 检查 tombstone，打包并校验 ZIP，上传 ZIP、metadata、`.ok`，再次检查 tombstone 并更新内容同步状态 | 自动快照上传需要恢复；普通手动上传由用户重试 |
| `SnapshotReceive` | 用户点击下载 | 下载前和本机提交前检查 tombstone，下载 ZIP，校验归档和内容，写入本机快照仓库 | 第一版不自动跨重启续传；用户重新点击 |
| `SnapshotMetadataSync` | 无合法 tombstone 时本地修改备注/锁定；失败重试 | 远端写入前后检查 tombstone，读取 metadata、字段级合并、覆盖并校验；若删除并发发生则清理孤儿 metadata | 仅无合法 tombstone 的 `pending/error` 必须恢复 |
| `SnapshotDelete` | 用户删除已上云快照；保留策略；tombstone 清理；失败重试 | 创建或确认 tombstone，删除 metadata、`.ok`、ZIP；仅本机存在删除意图时继续删除本机仓库和 SQLite 记录 | 本机删除意图和远端清理都必须恢复 |
| `SnapshotUploadReconcile` | 启动发现遗留 `uploading` | 开始和成功落库前检查 tombstone，只读检查 `.ok` 和 ZIP；补记成功或标记中断失败，不直接重新上传 | 每次启动和重新授权后恢复 |
| `MaintenanceSweep` | 启动；每十分钟；重新授权成功 | 读取业务状态并提交上述任务，执行本地布局整理 | 自身无需恢复 |

OAuth Token 刷新不是普通队列任务。它是所有 HTTP 请求共享的内部前置条件，使用应用级 single-flight：同一时刻只有一个刷新请求，其余请求等待同一结果。

`CatalogRefresh` 不下载 ZIP。十分钟看到远端新增快照，只让它出现在云端列表并产生未读提醒；用户明确点击下载后才创建 `SnapshotReceive`。

## 六、资源键

一个任务可以声明多个资源及访问模式。只有所有资源都可用时才能进入 `Running`。

### 6.1 资源定义

```text
AccountCatalog(account_id)
Game(account_id, cloud_game_id)
Snapshot(account_id, cloud_game_id, snapshot_id)
```

- `AccountCatalog` 支持共享/独占两种模式。快照任务持有共享模式，目录刷新持有独占模式。
- `Game` 第一版使用独占模式，保护同一游戏共享的 `game.json` 和上传初始化；因此同一游戏的两个上传串行，不同游戏仍可并行。
- `Snapshot` 使用独占模式，保证同一快照的上传、下载、metadata 和删除严格串行。

账号根 manifest 的初始化不持有整个上传任务的账号独占锁，而由 `CloudRuntime` 内部短临界区 single-flight 完成。否则所有上传仍会退化为全局串行。

manifest single-flight 是可重试状态机，不是 `OnceLock<Result<_>>`：

```text
Idle
  -> Running(account_id, credential_generation, waiters)
       -> Ready(account_id, credential_generation, manifest)
       -> Idle + 本轮错误通知全部 waiters
```

- 同一账号和 generation 只有首个调用者读取或创建 manifest，其余调用者等待同一轮结果。
- 成功结果可以在当前 Runtime 内缓存，但缓存键必须包含 `account_id + credential_generation`；授权失效、账号切换或 generation 递增立即丢弃。
- 网络、限流、授权和服务端瞬时失败只结束当前一轮，向所有等待者返回同一个已分类错误，然后原子清空 `Running`。下一任务按正常退避策略重新进入，不能永久记住失败。
- 创建竞态仍使用 `CreateOnly`：一个调用者成功创建，另一个读回服务端对象；最终成功结果必须持久化实际读取到的 `remote_repository_id`。
- manifest 得到成功结果后必须先执行 9.3.2 的三态比较。只有 `remote_repository_id` 为空且原子绑定成功，或已绑定值与远端值相同，才能进入 `Ready`；不一致时本轮返回 `repository_mismatch`，不得缓存为成功结果。
- generation 在请求期间失效时，本轮以 `StaleCredentialGeneration` 结束，不写成功缓存，也不把旧 manifest 交给新账号等待者。

`AccountCatalog` 是公平调度屏障，不是把账号内全部快照塞进一条 FIFO：用户刷新达到可调度优先级后先关闭新的共享入口，等待已经运行的快照任务结束，再独占执行；后台刷新尚未达到调度优先级时不能拦住新来的用户快照任务。aging 最终会让后台刷新获得屏障，避免永久饥饿。普通任务的严格 FIFO 应用于同一 `Snapshot` 和同一 `Game` 的任务链，删除取代规则除外。

### 6.2 各任务占用资源

| 任务 | 资源 |
| --- | --- |
| `CatalogRefresh` | `AccountCatalog(account)` 独占 |
| `SnapshotUpload` | `AccountCatalog(account)` 共享 + `Game(account, game)` 独占 + `Snapshot(account, game, snapshot)` 独占 |
| `SnapshotReceive` | `AccountCatalog(account)` 共享 + `Snapshot(account, game, snapshot)` 独占 |
| `SnapshotMetadataSync` | `AccountCatalog(account)` 共享 + `Snapshot(account, game, snapshot)` 独占 |
| `SnapshotDelete` | `AccountCatalog(account)` 共享 + `Snapshot(account, game, snapshot)` 独占 |
| `SnapshotUploadReconcile` | `AccountCatalog(account)` 共享 + `Snapshot(account, game, snapshot)` 独占 |

目录刷新作为账号级屏障有两个作用：

1. 刷新看到的是一个不与本机写任务交叉的远端视图，不会把“上传尚未发布”误判为删除。
2. 当前发现流程会更新多条本机 `cloud_snapshot_sync` 记录；屏障避免其用旧结果覆盖刚完成的上传或 metadata 状态。

已经发出的刷新不会因新用户任务而中断。真实百度回归中约 12 个游戏、80 余条快照首次增量发现约 6.8 秒，这个等待窗口可接受；后续仍应记录 P95 用时。

## 七、任务键、合并与顺序

### 7.1 任务键

```text
CatalogRefresh(account_id)
SnapshotUpload(account_id, snapshot_id)
SnapshotReceive(account_id, snapshot_id)
SnapshotMetadataSync(account_id, snapshot_id)
SnapshotDelete(account_id, snapshot_id)
SnapshotUploadReconcile(account_id, snapshot_id)
MaintenanceSweep
```

任务键用于合并完全相同的工作；资源键用于判断不同工作能否并发，两者不能混用。

### 7.2 合并规则

- 同一账号重复刷新：共享同一个任务结果。后台刷新排队后若用户主动刷新，原任务提升为用户优先级。
- 同一快照重复上传或下载：只执行一次；后来的调用者挂接同一结果。
- metadata 同步在排队时重复提交：合并为一次，执行时读取最新本地 revision。
- metadata 同步正在运行时又发生本地修改：标记 `rerun_requested`；当前任务完成后根据 revision 再提交一次，不能丢掉新修改。
- 同一快照重复删除：共享同一删除结果，不重复调用云端删除。
- 启动时同一快照只保留一个上传恢复检查；恢复检查完成前不允许普通上传越过它。
- 十分钟维护重复触发：只保留一个 `MaintenanceSweep`，扫描结束后如收到新触发再补跑一轮。

### 7.3 同一快照顺序

同一 `Snapshot` 资源通常按任务首次受理时间 FIFO。优先级只决定不同资源的队首任务谁先得到执行槽，不能插到同一快照前面。

删除是唯一的终止性合并规则，不属于普通优先级插队。删除意图成功落库后，协调器在一个队列临界区内处理同快照任务：

| 已有任务状态 | 删除到达后的处理 |
| --- | --- |
| `Queued` | 以 `SupersededByDelete` 结束原任务，释放等待者，删除任务取代它 |
| `WaitingRetry` | 取消后续重试，删除任务取代它 |
| `WaitingAuth` | 取消原任务的授权后续，删除任务等待授权以完成自身的远端清理 |
| `Running` | 不强制中断；删除排在它之后，运行任务结束或失败后立即执行 |
| `Succeeded/Failed` | 已是终态，不阻挡删除 |

对运行任务允许做协作式提前结束：若它尚未发出任何远端写入，并在安全检查点看到删除意图，可以返回 `SupersededByDelete`；一旦已经发出 HTTP 写请求，则不假定取消成功，后续统一由删除任务幂等清理。

示例：

```text
自动上传已运行
  -> 用户点击删除
  -> 删除意图立即落库、界面立即隐藏
  -> 删除任务在上传任务之后等待
  -> 上传完成
  -> 删除任务撤销云端发布并清理本地
```

这是“运行中的上传结束后删除”和“尚未运行的上传由删除取代”的统一实现。不会为了删除而先上传一个尚未开始的大 ZIP，也不会让 `WaitingAuth/WaitingRetry` 永久挡住删除。

一旦删除意图落库，新的上传、下载、恢复、备注和锁定命令均由后端拒绝；界面隐藏只是用户体验，后端校验才是正确性边界。

删除意图落库前已经进入 `Running` 的同快照任务允许安全结束；其他前序任务全部按上表取代。应用退出后内存队列丢失而删除意图仍在，重启只恢复删除，不恢复被删除取代的上传、下载或 metadata 任务。

删除命令先持久化删除意图，再向协调器提交删除。所有任务在第一次远端写入前都重新读取 `snapshots.delete_requested_at` 和 tombstone，因此即使上传恰好在“落库”和“队列取代”之间获得工作线程，也会安全退出或被视为已经运行，不能绕过删除。

## 八、优先级与公平性

基础优先级从高到低如下：

| 等级 | 任务来源 |
| --- | --- |
| P0 | 用户手动上传、下载、刷新、删除 |
| P1 | 用户修改备注/锁定后产生的即时 metadata 同步；上传中断恢复检查；用户删除的失败重试 |
| P2 | 新自动快照的即时上传 |
| P3 | 十分钟周期恢复的上传、metadata、删除失败任务 |
| P4 | 后台目录发现、保留策略清理 |

调度规则：

1. 先排除资源冲突和并发槽位不足的任务。
2. 每个 `Snapshot` 和 `Game` 只暴露自己的队首任务，后续任务不能越过；账号目录资源按上一节的公平屏障规则处理。
3. 在可运行的队首任务中选择有效优先级最高者；同级按首次受理时间 FIFO。
4. 后台任务每等待 60 秒提升一个有效等级，等待 4 分钟后最高可提升到 P0；同级仍按首次受理时间排序，因此不会永久饥饿，也永远不越过同一资源的前序任务。
5. P0 任务不设置排队超时。界面持续显示等待状态，用户关闭窗口不取消任务。

## 九、并发与共享运行时

### 9.1 顶层任务并发

- 工作任务上限：3。
- 后台工作任务同时最多运行 2 个，至少为新用户任务保留 1 个顶层执行槽。
- 同一快照：1。
- 同一游戏上传：1。
- `CatalogRefresh`：同一账号 1，并与该账号快照任务互斥。

### 9.2 HTTP 请求并发

- 百度请求全局上限：8。
- 后台请求额外受一个上限为 6 的信号量约束。
- 用户请求只占全局信号量，因此即使后台已经跑满，仍至少保留 2 个请求槽。
- 限流器属于应用级共享 `CloudRuntime`，任务级 Store 发出的每个请求都必须经过它；发现内部的游戏级线程和所有其他任务不能各自创建信号量。

顶层任务上限和 HTTP 上限解决不同问题：一个上传任务会连续发出多个请求，一个目录发现任务内部也会并发读取多个游戏。只限制顶层任务不能保证真实 HTTP 总量。

### 9.3 共享 Token 与客户端

`AppState` 应持有可懒加载的共享 `CloudRuntime`：

```text
CloudRuntime
  -> reqwest::blocking::Client（clone 共享连接池）
  -> Arc<HttpRequestLimiter>
  -> Arc<CredentialSessionState>
       -> 当前 account_id / credential_generation
       -> generation 级 RefreshingBaiduTokenProvider
       -> 请求租约与授权切换门闩
  -> manifest 初始化 single-flight

每个 CloudTaskExecutor
  -> 创建时绑定 account_id + expected_credential_generation
  -> runtime.new_store_session()
  -> BaiduNetdiskStore（任务级 ensured_directories，无 directory_cache）
  -> CloudSyncService
```

每个任务新建轻量 `BaiduNetdiskStore` 和 `CloudSyncService`，复用同一 HTTP Client 和限流器；Token Provider 只在同一账号、同一 generation 内共享，不能从可被后来授权覆盖的 Token 文件动态取得另一个 generation 的凭据。`BaiduNetdiskStore` 不再保留当前无 TTL 的透明 `directory_cache`；同一业务函数需要复用一页结果时，把该结果作为局部变量显式传递，不能让缓存跨越后续远端正确性检查。

父目录创建优化 `ensured_directories` 只允许存在于单个 Store/任务中。同一上传任务可复用已经确认的目录，任务结束即丢弃；远端返回路径不存在时仍按幂等创建并重试，不能把本地集合当成永久存在证明。

#### 9.3.1 generation 是网络写入边界

只在任务结束落库前比较 generation 不足以隔离账号。当前 `RefreshingBaiduTokenProvider` 每次请求都会读取同一个 Token 文件；如果直接沿用，旧任务可能在上传 ZIP 后遇到账号切换，并使用新 Token 把 metadata、`.ok` 或删除请求发到新账号。因此实现必须同时满足以下契约：

1. 协调器受理任务时保存 `account_id + expected_credential_generation`；任务合并也必须要求两者一致，不能把旧 generation 的等待者挂到新任务上。
2. Runtime 为每个 generation 创建独立的凭据会话。Provider 在该会话内维护 Token 和 single-flight 刷新状态，不在每次请求时重新读取一个可被新授权覆盖的共享 Token 文件。
3. Store 发出的每一次百度 HTTP 请求都必须先通过 `begin_request(expected_generation, account_id)` 取得请求租约；目录分页的每一页、上传 ZIP、上传 metadata、发布 `.ok`、下载、删除、Token 刷新及回读校验都不能绕过。租约从取得 Token 前保持到 `send` 返回，Token 刷新还要保持到条件持久化结束。
4. 请求租约在取得 Token 前和真正发送请求前都验证当前 generation。授权切换先设置 `credential_switch_pending`，协调器立即停止发放旧 generation 新租约；再等待已经发出的请求租约释放，在独占门闩内递增 generation、清除或安装凭据并替换 Runtime，最后解除切换状态。已经进入 `send` 的旧请求不强制中断，但它只持有旧会话 Token，且必须在切换完成前结束；切换完成后旧任务无法取得下一张租约，因此不能继续修改新账号。界面在等待旧请求收尾时显示“正在切换百度账号”，不能提前报告连接完成。
5. Token 刷新结果只有在 generation 仍匹配时才能更新内存和持久化文件。旧 generation 的迟到刷新结果必须丢弃，不能覆盖新授权。
6. 401、Token 失效等响应只能调用 `invalidate_if_current(account_id, expected_generation)`；先比较 generation，仍是当前会话时才清除凭据并递增 generation。旧请求迟到的授权错误不得删除刚保存的新 Token。
7. OAuth 回调同样绑定发起授权时的 generation/state。用户取消、重新发起或已经完成另一轮授权后，迟到回调不得覆盖当前凭据。
8. 旧 generation 的任务以 `StaleCredentialGeneration` 安全结束，不提交 SQLite 成功或失败状态。自动上传、metadata 和删除等可靠义务继续由 SQLite 保留；只有重新授权确认仍是同一 `account_id` 时，`MaintenanceSweep` 才创建绑定新 generation 的新任务。切换到另一账号时，旧义务保持暂停，不能转交给新账号；普通用户下载由用户重新点击。

`credential_generation` 与远端变化的 `remote_change_generation` 是两个无关概念：前者隔离本机授权会话，后者驱动云端入口未读提示，字段、事件和日志名称不得混用。

#### 9.3.2 真实账号身份与旧记录绑定

当前代码把所有百度授权固定写入 `BAIDU_ACCOUNT_ID`，并令 `cloud_accounts.account_identity=None`。v0.6.0 开放并发和可靠任务恢复前必须收口这个历史占位：

1. OAuth 成功后先调用百度 `uinfo` 接口，以响应中的稳定 `uk` 作为提供方身份，再允许创建可写 Runtime；显示名称、头像和网盘昵称都只用于展示，不能参与身份判断。内部 `account_id` 由 provider 与 `uk` 稳定派生或映射，日志和界面不暴露完整 `uk`。
2. 同一时刻仍只激活一个百度账号，不建设多账号切换中心；但不同真实身份必须对应不同本机 `cloud_accounts`、绑定、远端缓存、tombstone 和可靠义务命名空间。
3. 远端 manifest 读取成功后，对其中实际的 `repository_id` 和当前 `cloud_accounts.remote_repository_id` 做三态比较：本机为空时用 compare-and-set 原子绑定；值相同则继续；值不同时保留本机原值并进入 `repository_mismatch`。并发绑定 compare-and-set 失败时重新读取，只有新值相同才继续，否则同样判为不一致。`repository_mismatch` 是由两个 ID 推导的持久化安全状态，不另加数据库枚举；除账号身份和 manifest 诊断外，禁止该账号的目录刷新、上传、接收、metadata、删除和恢复任务，防止新仓库污染旧缓存或接收旧义务。v0.6.0 不提供自动改绑，界面显示“云仓库与本机记录不一致”，保留全部本机数据等待后续人工处理。
4. `AppState.cloud_repository_id`/`app_settings.repository_id` 继续只是“远端尚无 manifest 时创建新仓库”的候选 ID；不同设备随机值不同是正常现象，不能拿它比较账号、设备或 legacy 归属。
5. 第一次 v0.6.0 升级时，现有固定账号记录只有在当前授权成功取得身份后才能尝试绑定。legacy 已有云缓存或可靠义务时，使用本机已知 `.ok` 与当前远端不可变内容做强交叉匹配：至少一个相同 `snapshot_id` 的 `content_hash`、文件统计和归档 SHA-256 全部一致才可自动认领；任何同 ID 内容冲突都拒绝绑定。若 legacy 已持久化过 `remote_repository_id`，当前远端 manifest ID 相同也可直接认领。
6. legacy 没有可交叉匹配的 `.ok`、远端为空或证据不足时，不把“本机候选 repository ID 不同”判为账号不匹配，也不冒险自动认领；为当前真实身份建立新账号命名空间，将未绑定 legacy 记录保持暂停，等待原账号重新连接或后续人工确认。当前账号仍可正常刷新、发现和接收自己的远端对象，暂停的只是归属证据不足的旧本机缓存与可靠义务。legacy 完全没有绑定、云缓存和可靠义务时可以直接绑定当前身份。
7. 后续取得强证据并确认某个 `legacy_unbound` 行属于已经存在的 verified 账号时，不创建第二个相同身份账号，也不直接覆盖主键；在单个 SQLite 事务中把 legacy 子记录、`snapshots.cloud_owner_account_id` 和可靠义务合并到 verified `account_id`，逐项检查主键及内容冲突，全部成功后删除空的 legacy 行。任何冲突或中途失败整笔回滚并继续暂停，等待人工处理。
8. 同一身份重新授权只递增 credential generation，可恢复该 `account_id` 的可靠义务。身份不同则视为账号切换：旧账号业务状态完整保留但不调度，新账号只扫描和执行自己的记录；切回原身份后旧义务才恢复。
9. 身份核验必须分类处理：`uinfo` 明确返回 401 或 Token 已确认无效时，使当前凭据失效但保留账号行、`active_cloud_account_id` 和可靠义务，任务进入 `WaitingAuth`；网络不可用、超时、429 或百度 5xx 时保留活跃账号和 Token，任务进入 `WaitingRetry`，稍后重新核验；只有 `uinfo` 成功返回 `uk` 且与 Token 绑定摘要或 `cloud_accounts.account_identity` 不同，才视为安全异常，先使 generation 失效，再清空活跃账号并将可疑 Token 移出可用路径，禁止云写入并提示重新授权。显示名称变化永远不构成不一致。
10. 身份接口没有返回可验证 `uk` 或响应格式错误时不创建可写 Runtime，不得以固定 `BAIDU_ACCOUNT_ID` 或显示名称猜测同一账号；保留账号和凭据并报告身份响应异常，等待重试或用户重新授权。

该要求是账号安全边界，不等于 v0.6.0 要提供完整的多账号产品功能。

#### 9.3.3 分账号凭据与崩溃恢复

单一 `credentials/baidu-oauth.json` 不能与 SQLite 的活跃账号安全配对。v0.6.0 改为：

```text
credentials/baidu/{account_id}/oauth.json
credentials/baidu/pending/{oauth_state}.json.tmp
app_settings.active_cloud_account_id
```

安装新授权按以下顺序执行，前一步失败不得继续：

1. 授权码换得 Token 后先只保存在内存或 pending 临时文件，调用只读 `uinfo` 接口取得稳定 `uk` 并派生内部 `account_id`；失败按 9.3.2 分类，不改变旧活跃账号。
2. 使用该 Token 只读获取远端 manifest；存在则按 9.3.2 与该账号已经保存的 `remote_repository_id` 比较，只有首次为空或相同才能继续，不一致立即中止安装且绝不覆盖旧值；不存在则暂不创建，避免身份确认阶段产生远端写入。
3. 将 Token 写入该账号专属路径的临时文件，`flush/sync_all` 后在同目录原子替换 `oauth.json`；Token 文件内同时保存 provider、`account_identity` 和内部 `account_id` 绑定摘要，启动时可独立核对。
4. 在一个 SQLite 事务中 upsert 已验证账号、按首次绑定/相同规则保存 `remote_repository_id` 和 token 引用，最后更新 `active_cloud_account_id`。事务失败或仓库 ID compare-and-set 冲突时账号专属 Token 可以成为未激活孤儿，但不能影响旧活跃账号；后续启动清理或重新认领。
5. 事务提交后才创建新 Runtime、递增/发布新 generation 并向界面报告连接成功。

清除授权采用反向安全顺序：先阻止并排空请求租约，在 SQLite 原子清空 `active_cloud_account_id`，再删除对应账号 Token 文件并销毁 Runtime。删除 Token 失败时仍保持未激活，报告需要人工清理，不能重新启用旧 Runtime。

每次进程启动创建可写 Runtime 前都必须重新读取活跃账号专属 Token，调用只读 `uinfo`，并同时核对稳定 `uk`、Token 内绑定摘要、`cloud_accounts.account_identity` 和 `active_cloud_account_id`。核验结果严格按 9.3.2 分类：临时失败保留账号和 Token 并等待重试，401/Token 无效等待授权，只有成功取得不同 `uk` 才清空活跃状态；不能把“暂时无法核验”误当成账号切换。身份通过后还必须读取 manifest 并执行 `remote_repository_id` 三态比较，只有首次绑定或相同才能发布可写 Runtime。内存 generation 从零重新开始不构成信任依据。

旧版 `credentials/baidu-oauth.json` 作为一次性迁移候选处理：先保持原文件不动，用其中 Token 只读取得真实身份并按 9.3.2 判断 legacy 归属；随后执行上述账号专属 Token 安装和 SQLite 事务，确认新活跃账号可在重启后通过身份核对，最后才删除旧文件。任一步失败或崩溃都允许旧、新文件同时暂存，但启动只相信 SQLite 指向且身份复核通过的账号专属文件，绝不能按“哪个文件存在”自动选择账号。

### 9.4 远端新鲜度契约

v0.6.0 不新增一套容易漏用的 `*_fresh` 旁路 API，而是收紧现有 `CloudObjectStore` 语义：

- `list_directory()` 每次调用都向后端执行新鲜分页枚举；不返回上一次任务或上一次调用的目录结果。
- `stat_file()` 必须基于新鲜远端查询，不能通过旧父目录结果回答存在或不存在。
- `get_file()` 查找下载对象时使用新鲜远端条目，避免远端覆盖或新建后仍使用旧 `fs_id`/旧缺失结果。
- `delete_file()` 查找目标时使用新鲜远端状态；只有后端本次明确返回不存在，才按幂等删除成功，不能因本机旧缓存缺失而提前返回成功。
- `put_file(CreateOnly)` 以服务端创建结果为权威；若保留预检查，它也必须是新鲜查询，不能因旧缓存产生假冲突或漏冲突。

`CatalogRefresh` 每次运行都执行新鲜目录枚举。SQLite 的 `.ok`、metadata 和 tombstone 缓存只用于决定是否需要继续下载对象内容，不能替代本轮远端目录事实。上传流程中三次 tombstone 检查是三个独立的新鲜查询，即使前一次结果为空也不得复用。

新鲜查询的正确性优先于缓存，但不能忽略请求放大：当前百度 `stat_file()` 通过分页枚举父目录定位单个对象，10,000 条平铺 tombstone 下，上传的三次检查理论上可能产生至少 30 次分页请求。该成本必须按 19.3 的单对象专项门槛测量；不达标时先优化点查或目录布局，不能以“对象很小”放行。当前云端窗口约 543 毫秒恢复列表依赖前端会话缓存，仍可先展示旧结果并接回运行中的刷新；它不能被当作本轮远端校验结果。

### 9.5 第一版执行模型

当前 `savelink-core` 使用 `reqwest::blocking`，第一版不应为了队列同时重写成全异步 HTTP。推荐实现为：

```text
一个中央队列状态 + Condvar
  -> 一个调度循环
  -> 三个固定工作线程
  -> 每个工作线程执行一个阻塞式 CloudTaskExecutor
```

任务触发方不再像当前 `trigger_sync` 一样每次 `spawn` 新线程，只向协调器提交任务。任务条目至少保存：

```text
task_id / task_key / kind / origin
base_priority / submitted_sequence / submitted_at
resource_claims / state / attempt
rerun_requested / waiters / result / error_code
```

重复调用者通过共享完成对象等待同一个结果；Tauri async 命令如需等待，不得阻塞 UI 线程，可在 Tauri blocking pool 中等待完成条件。协调器通过与 Tauri 解耦的 Observer 回调发送状态事件，以便纯 Rust 单元测试不需要启动窗口。

这种模型与现有阻塞式百度客户端兼容，也能明确限制线程数量。未来若 Cloud Store 改为 async，只替换执行器和等待实现，不改变任务键、资源规则和业务状态。

## 十、状态流转

队列状态与业务状态必须分开。任务成功或从内存消失，不代表跨重启义务已经完成；反过来，SQLite 中的 `pending` 也不代表当前一定有一个运行中的任务。

### 10.1 内存任务状态

```text
Queued
  -> Running
  -> SupersededByDelete

Running
  -> Succeeded
  -> Failed
  -> WaitingRetry -> Queued（到达重试时间后）
  -> WaitingAuth  -> Queued（重新授权后）
  -> SupersededByDelete（只允许在尚未发出远端写入的安全检查点）
  -> StoppedByShutdown（显式退出且请求未完成）
```

- `WaitingRetry` 和 `WaitingAuth` 不占工作线程、资源锁或 HTTP 槽位。
- 用户手动上传/下载失败后立即把失败结果返回界面，不在后台无限重试。只有自动快照上传、metadata 和删除属于可靠业务义务，由十分钟维护重建任务。
- `SupersededByDelete` 是任务终态，不属于失败，也不进行重试。
- 已经发出的 HTTP 请求不做强制取消。显式退出时允许当前进程自然结束，下一次启动依靠 SQLite 状态恢复。

### 10.2 内容同步状态

`sync_status` 只描述 ZIP 和 `.ok` 的内容生命周期：

```text
尚无云记录 / error
  -> uploading
  -> uploaded

remote_only
  -> downloading
  -> downloaded

任一方向发现同 ID 不同不可变内容
  -> conflict
```

v0.6.0 第一次数据库迁移必须重建 `cloud_snapshot_sync` 的 SQLite CHECK，正式允许 `conflict` 并把旧删除值迁出 `sync_status`。新代码不得再向 `sync_status` 写删除状态。`conflict` 是不可自动重试的硬冲突；网络、授权和限流仍属于 `error` 或任务等待状态，不能显示为内容冲突。

`ignored` 继续保留原语义：旧版已在本机删除、但有意保留在云端的快照保持隐藏，不自动下载、不自动删除云端。只有用户明确“重新下载”时才清除 `ignored`。

### 10.3 metadata 状态

```text
synced
  -> pending（本地修改立即落库）
  -> syncing
  -> synced
  -> error -> syncing
```

`syncing` 可以只存在于内存，SQLite 继续保存 `pending/error/synced`，避免应用在网络中途退出后误认为已经完成。这些状态只适用于云端仍存在且没有合法 tombstone 的快照。

每条本机快照新增单调递增的 `metadata_local_revision`：

1. 用户修改备注或锁定时，在同一 SQLite 事务中更新字段、时间戳并递增 revision，同时查询该账号和快照的合法已发布 tombstone。
2. 不存在 tombstone 且仍有云快照记录时，把 metadata 标为 `pending`，事务提交后提交同步任务。
3. 已存在合法 tombstone 时，本次修改只落本地；不写 `pending`，不提交云任务，并清理可能残留的 `cloud_snapshot_sync`。
4. 同步任务开始时读取 revision、当前值和 tombstone 缓存。
5. 网络合并完成后，只有数据库 revision 仍等于开始值且仍无合法 tombstone，才允许把合并结果写回本机并标记 `synced`。
6. revision 已变化，说明用户在同步期间又编辑过；旧任务不得覆盖新值，也不得标记完成，只设置 `rerun_requested` 或保留 `pending`。
7. 下一条 metadata 任务重新读取最新值并同步。

合法远端 tombstone 被发现并持久化时，必须在同一 SQLite 事务中删除对应的陈旧 `cloud_snapshot_sync`，从而终止已有 `pending/error` 跨重启义务；事务提交后取消同快照尚未运行或正在等待的 metadata 任务。已经运行的任务依靠写入前后检查安全结束，发现 tombstone 的结果属于“远端已删除”，不是同步失败，不能再次写回 `error`。

仅靠“同一快照云任务串行”不能解决这个问题，因为本地编辑命令必须立即响应，不能为了等待网络而长期持锁。

### 10.4 删除状态

删除由本机快照意图和专用 tombstone 记录共同表达：

```text
snapshots.delete_requested_at: null -> 时间戳（界面立即隐藏）

cloud_snapshot_tombstones.status:
publish_pending
  -> cleaning_remote
  -> remote_clean

publish_pending
  -> failed(published=0)
  -> publish_pending

cleaning_remote
  -> failed(published=1)
  -> cleaning_remote

remote_clean + 本机有删除意图
  -> snapshots.status=deleting
  -> 删除本机文件、snapshots 和 cloud_snapshot_sync

合法远端 tombstone 已缓存 + 本机无删除意图
  -> 立即删除已过期的 cloud_snapshot_sync 缓存
  -> 终止 pending/error metadata 义务及等待任务
  -> 保留本机快照和 tombstone 缓存
  -> 远端残留清理继续由 tombstone 状态独立收尾
```

关键规则：

- 用户点击确认时先在 `snapshots.delete_requested_at` 持久化删除意图，正常快照列表从这一刻起隐藏该记录；专用 tombstone 表负责远端删除发布与残留清理。
- 纯本地快照且没有云记录、没有运行过的上传任务：取消尚未运行的上传后直接执行本地安全删除，不创建 tombstone。
- 只要已有云记录或上传曾进入远端写入阶段，就按“可能存在远端对象”处理，必须走 tombstone 和远端幂等清理。
- 上传必须在第一次远端写入前持久化 `sync_status=uploading`。因此应用崩溃后，有云记录表示可能存在远端对象；没有云记录且没有运行任务才允许判定为纯本地。
- 物理快照在远端删除成功前保持不动，保证失败可重试。
- 云端顺序固定为 `创建/确认 tombstone -> snapshot-meta -> .ok -> .zip`；tombstone 不删除。
- `.ok` 是发布标记；它删除后即使 ZIP 暂时成为孤儿，其他设备也不会把该快照视为可下载快照。
- 删除不存在的远端对象视为幂等成功。
- 云端全部删除后先持久化 tombstone 的 `remote_clean`，再删除本机文件。
- 本机文件删除成功后，最后删除 `snapshots` 和 `cloud_snapshot_sync` 记录。
- 如果进程在本机删除阶段退出，启动自检继续收尾。

### 10.5 远端存在状态

为了区分“本机尚未上传”和“曾经上云、后来被另一设备删除”，云缓存增加独立的远端存在状态：

```text
unknown / present / missing_confirmed
```

只有一次完整目录读取成功，并且对已知但缺失的 `.ok` 再做一次目标文件确认后，才能写 `missing_confirmed`。它表示旧客户端或人工操作造成的“已确认缺失”，没有跨设备强删除保证。网络失败、分页未完成或刷新被中断时不得据此判定远端删除。

读取并验证合法 tombstone 后写入专用 `cloud_snapshot_tombstones` 表，它表示 v0.6 强删除。`missing_confirmed` 和已缓存 tombstone 都保留本机已有快照，并在当前设备禁止自动上传和 metadata 同步；区别是只有远端 tombstone 能被所有 v0.6+ 设备在上传前主动识别。用户之后仍可修改本机名称和锁定状态，这些修改不再产生云端义务。

未来若提供“重新上传”，不能删除 tombstone 或复用原 snapshot ID，而应把本机内容复制成一个新的快照 ID。第一版只显示“云端副本已删除”。

### 10.6 tombstone 协议

tombstone 是 v1 目录下的向后兼容扩展对象。实现前初始候选采用平铺路径，最终布局必须在第一批正式对象写入前按本节规模门槛冻结：

```text
savelink/v1/games/{cloud_game_id}/snapshot-tombstones/{snapshot_id}.json
```

建议格式：

```json
{
  "schema_version": 1,
  "object_type": "snapshot_tombstone",
  "repository_id": "repo_xxx",
  "cloud_game_id": "game_xxx",
  "snapshot_id": "snapshot_xxx",
  "deleted_at": "2026-09-03T01:02:03Z",
  "deleted_by_device_id": "device_xxx",
  "reason": "user"
}
```

`reason` 允许 `user` 或 `retention`，只用于诊断，不影响优先级。tombstone 使用 `CreateOnly` 发布并保持不可变：

- `repository_id` 必须取自已经读取并验证的远端 manifest，不能直接使用当前设备新生成但尚未与远端确认的值。
- 不存在：创建后重新读取并验证。
- 幂等身份字段明确限定为 `schema_version/object_type/repository_id/cloud_game_id/snapshot_id`，只有这五项参与一致性比较。
- 已存在且五项身份字段一致：视为幂等成功，以远端第一份 tombstone 为准。
- `deleted_at/deleted_by_device_id/reason` 是审计字段，不参与并发删除的一致性比较；本机后创建失败时必须使用重新读取到的远端先创建版本覆盖本机未发布候选值。
- 已存在但五项身份字段任一不一致：报告协议冲突，不能继续删除。不得通过比较整个 JSON 把正常并发删除误判为冲突。
- v0.6.0 不自动清理或压缩 tombstone；它很小且必须长期存在。当前协议先保留平铺候选路径，但在写第一批正式 tombstone 前必须完成 19.3 点查规模测试。如果百度没有可靠的完整路径点查接口且平铺目录未达到门槛，则最终实现改为 `snapshot-tombstones/{bucket}/{snapshot_id}.json`；`bucket` 使用 `sha256(snapshot_id)` 的第一个小写十六进制字符形成 16 个稳定桶，不能直接取当前 `snap_...` ID 的共同前缀。由于 tombstone 尚未随正式版本发布，布局应在 v0.6.0 首次写入前一次定稿，不同时支持两种新布局制造迁移负担。

如果启用 16 桶布局，`CatalogRefresh` 先新鲜枚举 `snapshot-tombstones/` 根目录，只接受单字符小写十六进制桶名，再新鲜分页枚举每个实际存在的桶；不存在的桶无需预创建或逐个查询。目标 snapshot 的点查只进入其确定桶。根目录不存在仍等价于空集合，任一已列出桶读取失败则整轮刷新失败，不能把部分桶视图当作完整删除事实。

`CatalogRefresh` 读取每个游戏的 tombstone 目录时遵守以下兼容规则：

- 只有目标 `snapshot-tombstones/` 目录自身返回 `CloudStoreError::NotFound` 时，才等价于“当前没有 tombstone”，继续按 v0.5.0 仓库处理；第一次发布 tombstone 时由 Store 创建该目录。
- 目录存在但任一分页请求失败、响应不完整、路径非法或对象内容/身份校验失败时，本轮刷新失败并保留上一次完整缓存，不写 `missing_confirmed`，也不在未验证删除视图下继续接受对应 `.ok`。
- 其他目录的 `NotFound` 不得套用该兼容规则；它只适用于这个新增的可选扩展目录。
- 上述 `NotFound`、分页和修改时间都必须来自本轮新鲜远端枚举，不能由任务前或进程内的目录缓存推断。

所有 v0.6+ 上传执行以下检查：

1. 任何远端写入前检查 tombstone；存在则禁止上传。
2. 上传 ZIP 和初始 metadata。
3. 发布 `.ok` 前再次检查 tombstone；存在则清理自己可能写入的对象并停止。
4. 发布 `.ok` 后再次检查 tombstone；存在则缓存 tombstone，删除 metadata、`.ok`、ZIP，并停止上传。
5. 如果 tombstone 在第 4 步之后才由删除设备创建，删除设备会继续清理全部对象。

任一强制 tombstone 检查因网络错误无法完成时，上传都不能提交为 `uploaded`，而应进入错误/恢复检查。只有明确读到“不存在”才允许继续发布。

接收和 metadata 同样遵守删除优先：

- `SnapshotReceive` 在下载 ZIP 前检查 tombstone；ZIP 校验完成后、写入本机快照仓库和 SQLite 前再次检查。任一次发现都停止接收、缓存删除事实并刷新云端列表，不产生新的本机快照。
- 如果 tombstone 恰好在接收的最终检查之后才发布，本次本机提交可以完成；该副本等同于“删除发生前已经下载到本机”的快照，继续保留，但缓存 tombstone 后不得以原 ID 自动重新上传。
- `SnapshotMetadataSync` 在覆盖远端 metadata 前检查 tombstone，写入并回读验证后再次检查。任一次发现都不得标记 `synced`；若已经写出 metadata，则提交 `SnapshotDelete` 清理该孤儿对象，同时保留本机快照。
- 如果 tombstone 在 metadata 最终检查后才发布，由创建 tombstone 的删除任务按 `metadata -> .ok -> ZIP` 顺序幂等清理，不要求已经结束的 metadata 请求具备跨请求原子性。
- 上述检查读取失败时不能假定 tombstone 不存在：接收不得落地，metadata 不得标记完成，可靠 metadata 义务进入等待或重试状态。

本节所有 tombstone 检查均执行独立的新鲜远端查询；同一任务前面读到“不存在”不能成为后续检查的缓存答案。

目录发现先读取 tombstone，再处理 `.ok`。同一 ID 同时存在两者时 tombstone 永远获胜：该 `.ok` 不进入可下载列表，在 `cloud_snapshot_tombstones` 持久化该删除事实，并提交一个只清理远端残留、不删除本机副本的 `SnapshotDelete`。该清理即使失败也不影响 tombstone 的强删除语义，后续维护会根据 tombstone 表中的 `failed/cleaning_remote` 再次提交。

永久 tombstone 的发现必须利用本机缓存控制对象读取成本：

- 目录分页仍需枚举 tombstone 条目。只有本机缓存的 `remote_modified_at=Some(x)`、远端 `CloudEntry.modified_at=Some(x)` 且身份键相同时，才允许命中缓存并跳过 JSON 下载。
- 本地或远端修改时间任一为空、两者不相等、首次见到或缓存不完整时，都必须下载并验证 JSON。由于对象按协议不可变，修改时间变化属于需要重新校验并记录的协议异常，不能静默覆盖缓存身份；时间缺失则选择正确性优先，不能把两个 `None` 当成相等。
- 暖刷新必须记录分页请求数、实际下载 JSON 数、缓存命中数和总耗时；是否在 v0.6.0 引入上述 16 桶布局由 19.3 发布门槛决定，不能无限期后置已经测出的线性放大。

单条快照的安全检查不得借“新鲜度”掩盖线性点查成本：上传三次、接收两次、metadata 两次和上传恢复两次 tombstone 检查都纳入 19.3。若平铺路径不达标，先完成上述 16 桶布局并重新测试，再冻结 v0.6.0 的 tombstone 路径。

非法或身份不匹配的 tombstone 不能取得删除优先级：目录发现报告协议错误并停止处理该 ID，既不清理 `.ok`，也不删除本机数据。

强保证的版本边界必须如实表述：所有操作同一仓库的客户端都升级到 v0.6.0 后，tombstone 可以阻止自动复活；v0.5.0 及更早客户端完全不认识该对象，仍可能重新上传。v0.6.0 首次连接应提示或记录仓库启用 tombstone 扩展，但无法从云端强制旧二进制遵守。

### 10.7 上传失败与启动恢复

上传是否自动重试由本机快照原因决定，而不是只看 `sync_status=error`：

| 场景 | 当前会话 | 重启或十分钟维护 |
| --- | --- | --- |
| 自动快照上传失败 | 按退避规则重试 | 自动重新入队 |
| 手动快照上传失败 | 返回失败，等待用户 | 不自动上传 |
| 用户手动上传一条 `reason=auto` 快照失败 | 它仍是自动快照 | 可自动重试 |
| 下载失败 | 返回失败，等待用户 | 不自动下载 |

应用启动发现遗留 `sync_status=uploading` 时，不允许直接把它当成普通失败或永久保持“上传中”，而是提交 `SnapshotUploadReconcile`：

1. 先检查 tombstone；存在则写入 tombstone 缓存并提交残留清理。
2. 读取 `.ok`；若合法且身份、内容摘要与本机一致，再检查 ZIP 大小和 SHA-256。
3. `.ok` 和 ZIP 完整一致：在写入 `uploaded` 前再次检查 tombstone；仍明确不存在才补记成功，不重复上传。
4. 最终检查发现 tombstone：缓存删除事实，提交只清理远端残留的 `SnapshotDelete`，不得写入 `uploaded`。
5. `.ok` 缺失或对象不完整：标记 `error/interrupted_upload`；孤儿 ZIP 保持未发布，不在这里冒险删除。
6. 本机快照为 `reason=auto` 时重新提交上传；手动快照等待用户点击。

恢复检查需要网络或授权时进入 `WaitingRetry/WaitingAuth`。它在同一快照的新上传之前执行，避免用户重试时重复发布一个实际已经完成的对象。若此时已有删除意图，删除取代恢复检查并负责清理远端残留。

### 10.8 本机快照的单账号归属

v0.6.0 不实现一个本机快照同时扇出到多个百度账号。`snapshots.cloud_owner_account_id` 是唯一归属：

- 纯本地、从未上传的快照为 `NULL`；第一次向当前活跃账号上传时，在提交任务前用 SQLite 事务认领该账号。任务失败不撤销归属，避免切换账号后把同一 ID 再发往另一仓库。
- 从云端接收的新本机快照在最终落地事务中直接归属来源账号。
- 已有归属与当前活跃账号不同，上传命令立即拒绝并显示“此快照属于另一个百度账号”；不能新增第二条账号同步记录。
- 没有合法 tombstone 时，本地备注和锁定仍可立即修改，metadata `pending` 永远写入 owner 账号的记录；owner 未激活时不创建网络任务，切回该账号后再恢复。已有合法 tombstone 时以 11.3 的本地-only 规则为准，不保留或新建任何账号的 metadata 待办。
- 用户显式删除 owner 未激活的快照时，仍可写 `delete_requested_at` 并从列表隐藏，但 tombstone 和物理删除保持 `WaitingAuth`，直到原 owner 账号重新连接。不能在当前其他账号下执行删除，也不能先删本机后留下不可追踪的远端副本。
- 自动保留策略只处理 `cloud_owner_account_id IS NULL` 的纯本地快照，以及 owner 正是当前活跃账号的快照。属于未激活账号的快照不进入本轮淘汰候选，即使因此暂时超过保留数量；界面不把它误报成清理失败。
- 同一 `snapshot_id` 可以作为不同账号的纯远端目录记录存在，但一旦落地为同一条本机 `snapshots` 记录，只能绑定其中一个账号。来源账号等于 owner 时，同 ID、同内容才按幂等接收成功；owner 属于另一账号时，无论内容是否相同，都在下载 ZIP 前返回 `snapshot_owned_by_other_account`，不落地、不改变 owner、不新增该账号的 `cloud_snapshot_sync`，只让远端记录继续显示在当前账号的云端列表。owner 为空时，本次接收只能在最终短事务中以 compare-and-set 认领当前账号；若并发认领失败且新 owner 不是当前账号，本次接收失败并清理临时数据。

该模型保留未来“复制为新 snapshot ID 后上传到另一账号”的扩展空间，但 v0.6.0 不提供该操作。

## 十一、主要业务流程

### 11.1 用户手动上传

```text
前端点击上传
  -> 后端校验快照完整、未待删除
  -> 提交 P0 SnapshotUpload
  -> 返回/广播 Queued
  -> 获得资源和槽位后广播 Running
  -> 检查 tombstone
  -> 打包、校验、上传 zip/metadata
  -> 再查 tombstone、发布 .ok、最终复查 tombstone
  -> 提交 SQLite 内容和 metadata 状态
  -> 广播 Succeeded 或具体失败
```

相同快照重复点击只挂接已有任务。排队时显示“等待云端任务”，真正开始打包后才显示“正在打包并上传”。

### 11.2 新自动快照上传

本地自动快照创建和校验完成后，立即提交 P2 `SnapshotUpload`，不再启动一整轮 `sync_pending_and_prune`。失败状态落库，由内存退避或十分钟维护重试。普通手动快照失败后不由维护任务自动上传。云端失败不能回滚本机完整快照。

### 11.3 备注和锁定同步

本地命令只完成短事务：修改本地字段并递增 revision；若没有合法 tombstone 且存在云记录，同时标记 `pending`，事务提交后提交 P1 `SnapshotMetadataSync`。已有同键任务则合并。

若已有合法 tombstone，本机副本仍允许改名和锁定，但命令只提交本地字段，不标记 `pending`、不创建云任务。发现 tombstone 时已有的 `pending/error` 必须通过删除陈旧 `cloud_snapshot_sync` 终止；维护扫描和任务执行器也必须排除 tombstone 对应 ID，防止旧内存任务或重启扫描将其重新入队。

任务执行时按现有字段时间规则与远端合并；`device_id` 不参与决胜。同一时间戳下名称保留云端值，锁定取 `true`。revision 校验保护同步期间的新编辑。

执行器必须按 10.6 的规则在远端写入前后检查 tombstone。发现删除事实后停止该 ID 的 metadata 同步并清理可能已经写出的孤儿 metadata，不能仅依赖同一设备内的资源锁，因为另一设备不受本机协调器约束。

### 11.4 用户删除快照

确认无云记录、无正在运行的上传且没有遗留 `uploading` 状态的纯本地快照：继续使用本地安全删除流程，成功后立即消失，不创建 tombstone。

已上云快照：

```text
用户确认删除
  -> 短事务校验未锁定，读取 owner、云记录和上传执行状态
     -> 无云记录、无 Running/遗留 uploading：按纯本地流程（即使曾认领 owner）
     -> 可能存在远端对象：要求唯一 owner，写 snapshots.delete_requested_at
        -> 同事务只为该 owner upsert tombstone.status=publish_pending
  -> 前端刷新后立即隐藏
  -> owner 是当前活跃账号：提交 P0 SnapshotDelete
  -> owner 未激活：保持 WaitingAuth，不向当前账号提交任务
  -> 取代同快照未运行/等待中的任务
  -> 如有同快照 Running 任务则等待其结束
  -> 重新读取 cloud_snapshot_sync
     -> 无云记录：直接执行本地安全删除并结束
     -> 可能有远端对象：确保 tombstone.status=publish_pending 已持久化
        -> 创建或确认 tombstone
        -> 删除 metadata/.ok/.zip
        -> 标记 tombstone.status=remote_clean
        -> 删除本机文件
        -> 删除 snapshots 和 cloud_snapshot_sync，保留 tombstone 缓存
```

若可能存在远端对象却缺少唯一 owner，属于迁移或数据完整性错误：保持本机可见且拒绝删除，不能猜测当前账号。若立即尝试失败，记录 `failed` 和稳定错误码；十分钟维护重新提交。待删除记录不出现在正常页面，但后端的恢复、上传、下载、改名和锁定命令仍必须检查并拒绝它。远端不存在目标对象也必须先成功创建或确认 tombstone，随后各项删除按幂等成功处理。

### 11.5 保留策略删除

十分钟维护先完成当前活跃账号必要的远端发现和 metadata 合并，再计算未锁定超额快照。候选只包括纯本地快照和 owner 为当前活跃账号的快照；未激活账号拥有的快照本轮跳过。每个候选先写入本机删除意图并提交 P4 `SnapshotDelete`，随后使用 11.4 的相同规则分类：确认纯本地则直接安全删除；已有云记录或上传曾进入远端写阶段时才为唯一 owner 创建 `publish_pending` tombstone 并清理云端。不再在一个全局锁内逐条同步删除。

如果远端发现或 metadata 拉取失败，本轮不产生新的保留删除意图；tombstone 表中已有的 `publish_pending/cleaning_remote/failed` 删除义务仍可继续重试。

所有已上云快照在创建 tombstone 前最后读取一次远端 metadata；如果已经被其他设备锁定，则撤销本机删除意图、重新显示快照并提示用户先解锁。百度网盘没有跨对象 CAS，检查后若另一设备恰好并发锁定，tombstone 一旦创建则删除获胜；另一设备仍保留其本机快照，可以未来用新 snapshot ID 明确重新上传。保留策略和显式用户删除都遵守该规则。

### 11.6 云端目录刷新和未读提醒

刷新每个游戏时先新鲜读取 `snapshot-tombstones/`。旧 v0.5.0 仓库没有该目录时按空集合继续；目录存在但分页、响应或对象校验失败时，本轮不算成功，不处理依赖该删除视图的 `.ok`，也不推进 generation 或远端缺失状态。随后 `.ok`、ZIP 条目和 metadata 目录也必须来自本轮新鲜枚举，才能发现其他设备刚创建或修改的对象。

`CatalogRefresh` 返回目录结果与本轮 generation。刷新只在以下语义变化发生时增加 `remote_change_generation`，每个成功批次最多增加一次：

- 发现另一设备新增的有效远端快照。
- 发现远端 metadata 的值或字段版本发生有效变化。
- 发现此前已知快照对应的合法 tombstone，或确认旧式远端快照缺失。

以下情况不增加：

- 本机自己刚上传且本机已有相同记录的快照。
- 远端 metadata 与本机刚成功发布的字段值和时间戳完全一致。
- 本机自己刚完成删除所创建的 tombstone。
- 只有百度 `server_mtime` 变化，但语义内容没有变化。
- 刷新失败或只读到了不完整分页。

`cloud_accounts` 保存：

```text
remote_change_generation
last_viewed_remote_change_generation
```

当两者不相等时，首页“云端存档”入口显示小红点。云端窗口成功渲染 generation N 后调用确认命令，只把 `last_viewed` 推进到 N。若渲染期间后台又产生 N+1，N 的确认不会清除新红点。

第一版只显示红点，不显示精确数量。关闭窗口、刷新失败或错误页都不能标记已读。

### 11.7 接收云端快照

用户点击下载后，先读取本机同 ID 快照和 `cloud_owner_account_id`：owner 属于另一账号时直接返回 `snapshot_owned_by_other_account`，不提交下载任务；owner 等于来源账号时才允许同 ID、同内容作为幂等成功；owner 为空或本机不存在时可以提交 P0 `SnapshotReceive`。下载前检查 tombstone；下载、大小校验和 SHA-256 校验在临时目录完成；写入本机仓库和 SQLite 前再次检查 tombstone，并只在最后短提交阶段获取本地快照操作锁，以 compare-and-set 认领 owner 并建立唯一账号的同步记录。并发期间被另一账号认领时清理临时数据且不改变本机快照。失败不得产生可见半成品。最终检查之后才发生的远端删除不反向删除已提交的本机副本，但该副本不得以原 ID 自动重新上传。

### 11.8 移除游戏

“移除游戏”继续只清理本机管理记录和本机快照，不删除云端游戏目录，这是已确认的产品语义。

执行移除前应取消该游戏尚未开始的自动上传和 metadata 任务；若已有任务正在读取该游戏的本机快照，则短暂等待其本地读取阶段结束。移除游戏不是云端删除，不创建 tombstone；云端已发布对象保持不变，后续重新发现时仍可显示为远端内容。

## 十二、错误分类和重试

| 类别 | 示例 | 自动处理 | 用户显示 |
| --- | --- | --- | --- |
| 排队 | 资源忙、执行槽已满 | 保持 `Queued` | 等待云端任务，不算错误 |
| 授权 | `auth_required`、401、Token 已确认无效 | 使当前凭据失效但保留账号、active ID 和可靠义务，任务进入 `WaitingAuth` | 需要重新连接百度网盘 |
| 网络 | `network_unavailable`、超时 | 可靠任务退避，十分钟兜底 | 网络不可用，可稍后重试 |
| 限流 | `rate_limited` | 优先遵守 `Retry-After`，否则至少等待 60 秒 | 请求频繁，正在等待 |
| 可重试服务错误 | 百度 5xx | 30 秒、2 分钟、10 分钟退避 | 云服务暂时不可用 |
| 身份安全异常 | `uinfo` 成功但稳定 `uk` 与本机绑定不同 | generation 失效，清空 active ID，可疑 Token 移出可用路径；不调度云任务 | 百度账号身份与本机记录不一致，请重新授权 |
| 仓库不一致 | `remote_repository_id` 与远端 manifest ID 不同 | 保留原 ID 和全部业务状态；停止该账号仓库同步，不自动改绑 | 云仓库与本机记录不一致 |
| 本地失败 | 打包、磁盘写入、仓库删除失败 | 保留业务状态；按动作决定重试 | 显示本地文件错误，不称云冲突 |
| 硬冲突 | 同 snapshot ID 不同不可变内容 | 不自动重试、不覆盖 | 明确显示内容冲突并等待用户决策 |
| 远端损坏 | `.ok` 有效但 ZIP 缺失或校验失败 | 不自动覆盖 | 云端快照不完整/已损坏 |

内存退避只改善当前会话体验，不是可靠性来源。应用重启后，`MaintenanceSweep` 只从自动快照的上传错误、没有合法 tombstone 的 metadata `pending/error`、`snapshots.delete_requested_at` 以及 tombstone 的 `publish_pending/cleaning_remote/failed` 重建任务；手动快照上传错误不自动重试。遗留 `uploading` 先走恢复检查，不能直接重新上传。

## 十三、SQLite 调整

v0.6.0 需要迁移现有表，但不需要新增通用任务表。

### 13.0 `cloud_accounts` 账号身份收口

`cloud_accounts.account_identity` 已经存在，但当前不能区分历史占位和已验证身份，也没有保存真正读到的远端仓库 ID。本轮明确新增：

```sql
identity_state TEXT NOT NULL DEFAULT 'legacy_unbound'
  CHECK (identity_state IN ('legacy_unbound', 'verified')),
remote_repository_id TEXT,
CHECK (
  (identity_state = 'legacy_unbound' AND account_identity IS NULL) OR
  (identity_state = 'verified' AND account_identity IS NOT NULL)
)
```

`app_settings.active_cloud_account_id` 只保存当前激活的内部账号 ID；清除授权时清空该设置，但不删除账号行和可靠义务。升级后的首次授权按 9.3.2 取得真实身份并绑定：

- legacy 账号没有任何绑定、云快照缓存或可靠义务时，可直接补写当前身份。
- legacy 已有云状态时，按 9.3.2 使用已持久化的 `remote_repository_id` 或已知 `.ok` 强匹配认领；本机随机候选 `app_settings.repository_id` 不参与判断。证据不足时保留 legacy 行及其子表记录为未绑定/暂停，并为当前真实身份创建新的内部 `account_id`，但不阻止该账号正常发现和使用远端仓库。后续证据充分时按 9.3.2 的事务合并规则认领，不能产生两个相同 verified 身份。
- 旧表迁移时，`account_identity IS NULL` 的行写为 `legacy_unbound`，非空行写为 `verified`；新建真实账号只能直接写 `verified`。未授权由 Token/活跃账号设置表达，不复用 `identity_state`。
- 所有 `cloud_game_bindings`、`cloud_snapshot_sync` 和 `cloud_snapshot_tombstones` 查询都必须带内部 `account_id`；切换活跃账号不迁移这些子记录。
- `token_ref` 必须指向 `credentials/baidu/{account_id}/oauth.json`，不能继续让不同账号引用同一个固定文件；`remote_repository_id` 只保存验证通过的远端 manifest 值。

跨字段 CHECK 和现有 `UNIQUE(provider, account_identity)` 必须通过单事务重建表实现，不能只 `ALTER TABLE ADD COLUMN`：创建新表及完整约束 -> 映射复制旧行 -> 校验行数、identity 组合和唯一性 -> 替换旧表 -> 重建索引/唯一约束。任一步失败整笔回滚。由于子表以 `account_id` 关联，本次迁移不改已有账号主键；后续真实身份认领或建立新账号使用独立事务。

### 13.1 `snapshots`

新增：

```sql
metadata_local_revision INTEGER NOT NULL DEFAULT 0
delete_requested_at TEXT
cloud_owner_account_id TEXT
```

本地用户修改备注或锁定时递增；远端合并写回使用 compare-and-set，不把旧网络结果覆盖到更高 revision。

`delete_requested_at` 是本机删除意图的权威标记，解决上传仍在本地打包、尚未建立 `cloud_snapshot_sync` 记录时的删除竞态。列表隐藏、启动恢复和后端操作拒绝都以它为准：

- 用户删除已有云记录的快照：在同一 SQLite 事务中写 `delete_requested_at` 和一条 `cloud_snapshot_tombstones.status=publish_pending` 记录。
- 用户删除尚无云记录但已有内存上传任务的快照：先写 `delete_requested_at`，协调器再取消或等待任务。任务结束后重新读取云记录；仍不存在则按纯本地删除，已经存在则创建 `publish_pending` tombstone。
- 上传任务第一次远端写入前重新读取 `delete_requested_at`；存在则不发出写请求。
- 确认为纯本地后执行现有安全删除；本地文件删除失败时清除 `delete_requested_at` 并重新显示，向用户报告失败。
- 有过远端写入可能性的快照保持隐藏，直到 tombstone 和远端清理完成；不能因为断网清除删除意图。

`cloud_owner_account_id` 按 10.8 执行单账号归属：

- 旧库中本机快照没有任何 `cloud_snapshot_sync` 时保持 `NULL`；只关联一个 distinct `account_id` 时回填该账号。
- 同一本机快照若意外关联多个 distinct 账号，迁移必须报错并回滚，不得随机选择 owner。当前正式代码只使用固定百度账号，正常旧库不会命中该异常。
- 第一次上传认领 owner、云端接收落地写 owner，都必须与对应云记录在同一 SQLite 事务中完成。
- owner 一旦非空，普通上传、metadata、删除和保留流程都不得改成另一账号；只有删除整条本机快照时一起消失。

### 13.2 `cloud_snapshot_sync`

新增：

```sql
remote_presence TEXT NOT NULL DEFAULT 'unknown'
remote_presence_checked_at TEXT
```

`remote_presence` 允许：

```text
unknown / present / missing_confirmed
```

迁移策略：

- 第一次 v0.6.0 迁移就在同一事务中重建 `cloud_snapshot_sync`，不能等到阶段 6 才修改 CHECK。
- 新 `sync_status` CHECK 允许 `uploading / uploaded / remote_only / downloading / downloaded / ignored / error / conflict`；不再允许删除状态。
- 旧 `delete_pending/deleting/delete_failed` 的内容状态统一迁移为 `uploaded`，同时按下一节创建 tombstone 业务记录；存在对应本机快照时补 `snapshots.delete_requested_at`，保持升级前的隐藏和收尾语义。
- 旧 `remote_deleted` 不能直接当作强删除完成：旧版没有 tombstone。内容状态迁移为 `uploaded`、`remote_presence=missing_confirmed`，同时创建待发布 tombstone；存在本机快照时补 `delete_requested_at`。即使本机快照已经不存在，tombstone 记录也保留到远端发布和残留清理成功。
- 旧 `ignored` 原值保留，`remote_presence=present`；不显示、不自动下载、不自动删除云文件。
- 非删除记录根据 `uploaded/downloaded/remote_only/ignored` 初始化为 `remote_presence=present`，其余初始化为 `unknown`。
- `conflict` 只表示同 ID 不同不可变内容，不用于网络或排队失败。
- 旧的 `last_error_code` 继续只表达内容上传/下载错误。

迁移必须采用“建新表 -> 映射复制 -> 校验行数和状态值 -> 替换旧表 -> 重建索引”的现有模式，并包在单个 SQLite 事务中。任何一步失败都回滚，不能留下半张新表。

### 13.3 `cloud_snapshot_tombstones`

新增专用业务表：

```sql
CREATE TABLE cloud_snapshot_tombstones (
  account_id TEXT NOT NULL,
  repository_id TEXT,
  cloud_game_id TEXT NOT NULL,
  snapshot_id TEXT NOT NULL,
  deleted_at TEXT NOT NULL,
  deleted_by_device_id TEXT,
  reason TEXT NOT NULL CHECK (reason IN ('user', 'retention')),
  published INTEGER NOT NULL DEFAULT 0 CHECK (published IN (0, 1)),
  status TEXT NOT NULL CHECK (
    status IN ('publish_pending', 'cleaning_remote', 'remote_clean', 'failed')
  ),
  remote_modified_at INTEGER,
  last_attempt_at TEXT,
  last_error_code TEXT,
  CHECK (
    published = 0 OR
    (repository_id IS NOT NULL AND deleted_by_device_id IS NOT NULL)
  ),
  CHECK (status NOT IN ('cleaning_remote', 'remote_clean') OR published = 1),
  PRIMARY KEY (account_id, snapshot_id)
);

CREATE INDEX idx_cloud_snapshot_tombstones_account_game
  ON cloud_snapshot_tombstones(account_id, cloud_game_id);

CREATE INDEX idx_cloud_snapshot_tombstones_maintenance
  ON cloud_snapshot_tombstones(account_id, status, published);
```

这张表既保存本机尚未发布的删除义务，也缓存其他设备发布的 tombstone。它必须能独立于 `cloud_snapshot_sync` 存在，因为一台新设备可能只看到 tombstone，而 `.ok` 已删除，无法获得快照大小、哈希等非空字段。

- 本机用户/保留策略发起：插入 `published=0, status=publish_pending`；正常运行时应立即写入当前设备 ID，但旧状态迁移发生在设备 ID 初始化之前，允许暂时为空。远端创建并回读成功后原子更新为 `published=1, status=cleaning_remote`，残留对象全部清理后进入 `remote_clean`。
- 本机刚写 `publish_pending` 时 `repository_id` 和 `deleted_by_device_id` 可以为空；执行器读取并验证远端 manifest 后，必须在发布前分别补齐远端 repository ID 和当前设备 ID。所有已发布状态和远端发现记录中两者均不得为空。
- 其他设备发现：验证 JSON 后 upsert 为 `cleaning_remote`；清理残留对象后进入 `remote_clean`。
- 任一远端步骤失败：保存为 `failed` 和稳定错误码，维护周期重试；`published` 明确区分失败发生在 tombstone 发布前还是残留清理阶段。
- `failed, published=0` 重试发布 tombstone；`failed, published=1` 重试清理远端残留，不能混成同一恢复入口。
- `remote_clean` 记录不删除，作为本机长期抑制缓存；远端 tombstone 仍是跨设备权威事实。
- 若已存在同 ID tombstone，本机数据库中的不可变身份字段必须一致，不能用后来的对象覆盖。
- 合法远端 tombstone 首次缓存或确认时，同一事务删除对应 `cloud_snapshot_sync`；其 `pending/error` metadata 状态随记录一起终止，远端残留清理只依赖 tombstone 表继续执行。
- `account_game` 索引用于按游戏发现和清理，`maintenance` 索引用于启动及十分钟维护恢复未完成义务；索引随表迁移在同一事务中创建。

旧状态映射：

| 旧 `sync_status` | 新 tombstone 状态 | 说明 |
| --- | --- | --- |
| `delete_pending` | `publish_pending` | 尚未证明远端存在 tombstone |
| `deleting` | `publish_pending` | 旧版本可能已删除部分对象，但必须先补 tombstone |
| `delete_failed` | `publish_pending` | 从 tombstone 发布重新开始幂等收尾 |
| `remote_deleted` | `publish_pending` | 旧版远端对象虽已删，仍需补发强删除事实 |

现有版本只有保留策略会产生这四种旧删除状态，因此迁移出的 `reason=retention`；`deleted_at` 和对应本机快照的 `delete_requested_at` 使用本次迁移 UTC 时间，不能误用旧 `last_synced_at`（它可能只是更早的上传时间）。由于 `SqliteRepo::open()` 内部迁移早于 Tauri 读取或创建设备 ID，迁移记录的 `deleted_by_device_id` 保持 `NULL`；执行器真正发布前再用当前设备 ID 补齐。新 tombstone 上传前仍必须读取远端 manifest，并用其 `repository_id` 构造最终不可变文档。

### 13.4 `cloud_accounts`

新增：

```sql
remote_change_generation INTEGER NOT NULL DEFAULT 0
last_viewed_remote_change_generation INTEGER NOT NULL DEFAULT 0
```

generation 按账号持久化，避免应用重启或窗口关闭后丢失未读提醒。

### 13.5 为什么不建 `cloud_tasks`

通用任务表会引入任务参数版本、已运行请求恢复、重复消费、租约和崩溃接管等额外问题。当前真正需要跨重启的是业务义务，现有业务表能更准确表达：

- 自动快照的内容上传失败或未发布。
- 没有合法 tombstone 的 metadata `pending/error`。
- `snapshots.delete_requested_at` 表示的本机删除意图。
- `cloud_snapshot_tombstones` 中未完成的发布和远端残留清理。

启动时由这些状态重新生成幂等任务，比尝试恢复一条执行到一半的通用任务更可靠。

## 十四、前后端接口与界面状态

### 14.1 后端事件

新增统一事件：

```text
cloud-task-state-changed
cloud-unread-changed
```

任务事件至少包含：

```text
task_id
kind
snapshot_id（可空）
origin: user / immediate / background
state: queued / running / waiting_retry / waiting_auth / succeeded / failed / superseded_by_delete
queue_position（可空，仅提示用途）
error_code（可空）
```

不要把完整 Token、远端路径或真实存档路径写入事件和普通日志。

### 14.2 命令行为

- 上传、下载和用户刷新命令可以继续异步等待协调器结果，以减少前端接口改动；任务事件负责展示“排队/运行”。
- 删除命令在删除意图成功落库并入队后即可返回，不等待远端删除，保证页面立即响应。
- metadata 修改命令在本地事务和入队完成后返回，不等待网络。
- 增加轻量任务摘要查询，窗口重开后可以恢复正在排队/运行的显示。
- 云端列表结果携带当前 generation；前端成功渲染后调用按 generation 确认已读的命令。

### 14.3 用户可见状态

- `等待云端任务`：已经入队但尚未开始。
- `正在上传/下载/刷新/删除`：任务已进入 `Running`。
- `等待重新授权`：可靠任务保留，用户需重新连接。
- `正在切换百度账号`：停止新请求并等待当前账号已发请求结束。
- `此快照属于另一个百度账号`：当前账号禁止上传或云端修改该本机快照，不称为冲突。
- `稍后重试`：网络或限流，不能写成“云冲突”。
- `内容冲突`：只用于同 ID 不同不可变内容。
- 云端入口小红点：后台发现用户尚未查看的远端变化。

第一版不增加独立任务中心。首页或云端入口可显示一个简短的全局状态，例如“2 个云任务”，具体操作按钮显示自己的状态。

## 十五、冲突分类与处理边界

### 15.1 操作冲突

上传、下载、metadata、删除和目录刷新争用同一资源属于操作冲突。协调器通过排队消除，用户不应再看到错误弹窗。

### 15.2 metadata 冲突

备注和锁定继续按字段时间自动合并，不影响 ZIP 内容。极少数完全相同时间戳按现有简单规则处理：名称保留云端，锁定取 `true`，不引入 `device_id` 决胜。

### 15.3 不可变内容硬冲突

同一 `snapshot_id` 对应不同 `content_hash`、文件统计或归档摘要时：

- 禁止上传覆盖。
- 禁止下载覆盖。
- 标记 `conflict`，停止十分钟自动重试。
- 保留本机和云端双方。
- 界面明确告诉用户这是内容冲突，不是“任务正在进行”。

v0.6.0 第一阶段先做到可靠识别、停止覆盖和准确展示。后续“采用本机、采用云端、两边都保留”需要安全地重建一个新 snapshot ID，应在独立实现阶段完成，不能把覆盖按钮直接接到现有上传 API。

### 15.4 远端删除

合法 tombstone 是 v0.6+ 设备间的权威删除事实；完整发现加目标确认得到的 `.ok` 缺失只属于 `missing_confirmed`，不能冒充强删除。其他设备已经下载到本机的快照不跟随云端删除，继续作为本机副本保留。v0.6+ 客户端不得使用原 snapshot ID 自动复活 tombstone；旧客户端仍可能忽略该扩展，必须在发布说明中要求同仓库设备一起升级。

### 15.5 故障不是冲突

授权失效、网络断开、限流、百度服务错误、远端损坏和本地磁盘错误都使用自己的错误分类。准确分类是 v0.6.0 用户体验的一部分。

## 十六、应用生命周期

- 关闭主窗口进入托盘：队列继续运行。
- 重新打开窗口：通过任务摘要恢复排队/运行状态，不重复提交相同任务。
- 显式退出应用：不再受理新任务；未开始的内存任务停止，已发出的 HTTP 不强制中断。
- 下次启动：先运行现有快照自检，再扫描可靠业务状态并重建云任务。
- Token 无效或用户清除授权：先标记凭据切换并停止发放旧 generation 的请求租约，等待已发请求收尾后在独占门闩内递增 `credential_generation`、清除旧 Provider 并销毁 Runtime；可靠业务义务按原 `account_id` 保留在 SQLite，不占资源和线程。已经进入 `send` 的旧请求不强杀且只使用旧会话 Token，结束后旧任务不能取得下一张请求租约，也不能提交当前账号状态。
- 重新授权成功：授权回调先确认其发起 generation/state 仍有效，并读取真实账号身份。随后为该 `account_id` 安装新凭据并重建 Runtime；只有同账号义务可由新 generation 的 `MaintenanceSweep` 恢复。`directory_cache` 不存在，旧任务的 Provider、刷新结果和 `ensured_directories` 都不能跨授权继承；若身份发生变化，旧账号义务继续暂停。
- 旧 generation 返回 401 或刷新失败时只做 compare-and-invalidate；若当前 generation 已变化则忽略该迟到结果，不能清除新账号 Token。
- 退出期间未完成的用户普通下载不自动续传；临时目录由启动清理。用户重新点击后从头下载并重新校验。

## 十七、必须保持的正确性约束

1. 同一快照任何时刻最多有一个云任务处于 `Running`。
2. 已落库的删除意图不能被上传或 metadata 完成状态覆盖。
3. 删除远端成功前不能删除本机物理快照。
4. `.ok` 永远在 ZIP 和初始 metadata 成功上传后发布；删除先持久化 tombstone，再在 ZIP 前撤销 `.ok`。
5. 目录刷新失败不能把远端对象误判为删除。
6. metadata 旧任务不能覆盖更高 `metadata_local_revision` 的本地值。
7. 自动任务不能弹 OAuth 浏览器；只进入等待授权并提示。
8. 后台任务不能占满全部 HTTP 请求槽。
9. 断网、限流或应用退出不能删除本机完整快照。
10. 待删除记录即使前端仍持有旧对象，后端也必须拒绝恢复、上传和修改。
11. 本机自己的上传和 metadata 发布不能制造云端未读红点。
12. 相同 ID 不同内容永远不能自动覆盖任意一方。
13. v0.6+ 上传在远端写入前、发布 `.ok` 前和发布后都必须检查 tombstone。
14. `ignored` 迁移后继续保持隐藏和云端保留，不能转成自动删除或自动下载。
15. 接收在下载前和本机提交前、metadata 在远端写入前后、上传恢复在补记成功前都必须检查 tombstone。
16. 迁移产生的未发布 tombstone 可以暂缺设备 ID，但任何已发布 tombstone 的 repository ID 和设备 ID 都不能为空。
17. 远端存在性判断不得来自无 TTL 的跨任务目录缓存；三次 tombstone 检查必须分别访问远端。
18. 每个任务绑定创建时的账号和授权 generation；每一次 HTTP 请求都必须取得匹配租约。generation 变化后，旧任务既不得取得新 Token 或发起后续请求，也不得提交当前账号状态。
19. 旧 generation 的刷新结果、OAuth 回调和授权错误不得覆盖或清除新 generation 的 Token。
20. 可靠云义务必须属于已验证的真实账号身份；换账号不能恢复、删除或上传旧账号的对象，切回原账号后才能继续。
21. 一个本机快照最多有一个 `cloud_owner_account_id`；任何入口都不能在第二个账号为同一快照建立上传、metadata 或删除义务。
22. 进程重启后只有账号专属 Token 的真实身份与 SQLite 活跃账号一致才能建立可写 Runtime；内存 generation 重置不能绕过持久化核对。
23. manifest single-flight 的失败不能永久缓存，成功缓存不能跨账号或 credential generation。

## 十八、实施顺序

### 阶段 1：调度器骨架和共享网络运行时

- 一次性增加 metadata revision、本机删除意图、快照单账号 owner、账号 identity 状态/远端仓库 ID、专用 tombstone 表、远端存在状态和 `conflict` CHECK；完成 cloud_accounts 重建、owner 回填、旧删除状态及 `ignored` 迁移测试。
- 增加 tombstone 协议对象、路径、序列化、校验、可选目录兼容、可空修改时间和并发创建契约测试，但暂不切换用户删除入口。
- 新增纯调度状态机、任务键、资源锁、优先级、aging、合并和 Fake 执行器测试。
- 在 `AppState` 中接入单例协调器。
- 把 HTTP Client、HTTP 信号量和 manifest 初始化改为应用级共享；Token Provider 改为 generation 级凭据会话并支持 single-flight，Store 与 `ensured_directories` 改为任务级，移除透明的跨调用 `directory_cache`。
- 收紧 `CloudObjectStore` 新鲜度契约，让 list/stat/get/delete 和 CreateOnly 预检查不再使用旧目录结果；在 Store 的每个 HTTP 出口接入账号/generation 请求租约及 compare-and-invalidate，不能只在任务落库时检查。
- 接入真实百度账号身份读取、分账号 Token 路径、安全安装/启动核对和 legacy `BAIDU_ACCOUNT_ID/account_identity=NULL` 的强证据认领；持久化实际 `remote_repository_id`，业务查询、可靠恢复和任务资源键全部改用验证后的内部 `account_id`，仍只激活一个在线账号。
- 把 manifest 初始化实现为按账号/generation 隔离的可重试 single-flight；成功可缓存，失败清空本轮状态，generation 变化丢弃旧结果。
- 用 10,000 条 tombstone 先验证平铺路径的单对象点查成本；若不满足 19.3，在发布任何 tombstone 前切换到 16 桶哈希前缀布局并重跑协议、刷新和点查测试。
- 暂时保留现有命令外观，不改产品语义。

### 阶段 2：现有云入口全部进队列

- 接入用户上传、目录刷新、下载。
- 拆解 `sync_pending_and_prune`：维护扫描只提交 metadata、自动上传和删除任务。
- 移除 `baidu_sync_in_progress` 的拒绝式全局互斥。
- 接入任务事件和排队文案。
- 接入 `SnapshotUploadReconcile`，明确自动快照继续重试、手动快照等待用户。

### 阶段 3：metadata revision 防覆盖

- 接通 SQLite revision 和 compare-and-set 仓储方法。
- 覆盖同步中再次编辑、任务合并、失败重试和跨重启测试。
- 接通 tombstone 后本机 metadata-only 分支；合法删除事实必须终止旧 `pending/error`，后续本地编辑不得提交云任务。
- 缩短 `snapshot_operation_lock`，不在网络期间持有全局本地锁。

### 阶段 4：用户删除云端闭环

- 接通本机删除意图和 tombstone 生命周期。
- 用户删除已上云快照改为“落删除意图 -> 立即隐藏 -> 取代未运行任务 -> 创建 tombstone -> 清理远端 -> 本地收尾”。
- 所有旧入口增加待删除后端校验。
- 更新 `SaveLink云端快照协议V1.md` 中当前“本机删除不删云端”的旧规则。

### 阶段 5：远端变化与未读提醒

- 接通远端存在状态和 generation，目录发现先处理 tombstone。
- 后台增量发现产生语义变化事件。
- 首页云端入口小红点和按 generation 已读确认。
- 确保自上传、自 metadata 同步和失败刷新不误报。

### 阶段 6：硬冲突体验和完整回归

- 增加 `conflict` 状态和准确页面。
- 第一阶段只阻止覆盖并保留双方；安全解决动作单独实现和验收。
- 使用 Fake 云端、故障注入和真实百度完成回归，再发布 v0.6.0。

阶段 1 和阶段 2 必须先完成，其中 `conflict` CHECK、旧删除状态和 `ignored` 在阶段 1 一次迁移，避免后续再次重建同一张表。阶段 3、4 是本轮已确认产品行为的正确性基础，不应后置。阶段 5 是用户明确提出的云端变化提醒。阶段 6 的“识别和阻止覆盖”属于 v0.6.0，复杂的人工合并动作可以拆到后续小版本。

## 十九、测试计划

### 19.1 调度器单元测试

- 同一快照普通上传、下载和 metadata 严格 FIFO，任何时刻最多一个任务运行。
- 删除取代同快照 `Queued/WaitingRetry/WaitingAuth` 任务，不发出被取代上传的 HTTP 请求。
- 删除等待同快照 `Running` 上传结束或失败，不强制中断已发出的 HTTP。
- 删除意图落库与上传获得工作线程发生竞态时，上传在首次远端写入前安全退出。
- 不同游戏快照可并发，顶层运行数不超过 3。
- 同一游戏上传串行。
- 目录刷新与账号内所有快照任务形成屏障。
- 重复上传、下载、刷新、删除只执行一次并共享结果。
- 运行中 metadata 再次提交会安排下一轮。
- 后台 aging 生效但不破坏资源 FIFO。
- `WaitingRetry/WaitingAuth` 不占资源。
- 后台 HTTP 最多占 6，总请求最多 8。

### 19.2 SQLite 和核心服务测试

- SQLite 新 CHECK 接受 `conflict` 并拒绝删除状态；旧删除值正确迁移到 `snapshots.delete_requested_at` 和专用 tombstone 表。
- `cloud_accounts.identity_state` 迁移把空身份标为 `legacy_unbound`、非空身份标为 `verified`，约束拒绝两类非法组合；清除 `active_cloud_account_id` 不删除旧账号义务。
- `cloud_accounts` 使用新表映射复制迁移并保留 `UNIQUE(provider, account_identity)`；覆盖空/非空旧身份、重复身份、行数校验、约束失败和中途故障整笔回滚。
- 本机随机候选 repository ID 不参与 legacy 认领；设备 A 创建远端仓库、设备 B 使用不同本机候选 ID 时，B 仍能通过匹配 `.ok` 或已知 `remote_repository_id` 正确认领同一账号。证据不足时只暂停旧本机义务，B 仍能以 verified 新命名空间正常刷新和接收当前账号的远端对象。
- legacy 后续认领已存在的 verified 账号时，子记录、快照 owner 和可靠义务在单事务中无损合并；主键或内容冲突时整笔回滚，不留下双 verified 身份或半迁移记录。
- manifest 实际读取/创建成功后持久化远端 ID；瞬时失败唤醒本轮所有等待者并恢复 `Idle`，下一任务可以成功重试；成功缓存不跨账号或 credential generation。
- `remote_repository_id` 为空时只能 compare-and-set 首次绑定，相同值继续使用，不同值保留原值并返回 `repository_mismatch`；不一致时目录刷新、上传、接收、metadata、删除和恢复均不得发出仓库请求，重新授权也不能静默改绑。
- `snapshots.cloud_owner_account_id` 从零/单个 distinct 云账号正确回填，多账号异常整笔回滚；上传到第二账号、从第二账号覆盖接收都被拒绝。
- 本机已有账号 A owner 的快照时，账号 B 对同 ID、同内容的接收也返回 `snapshot_owned_by_other_account`，不下载 ZIP、不新增账号 B 同步记录、不改变 owner；owner 为空时两个账号并发接收只有 compare-and-set 胜者可以落地。
- 没有合法 tombstone 且 owner 未激活时，本地备注/锁定只为 owner 保留 pending；已有 tombstone 时只改本地并终止待办。显式删除进入 owner 的 WaitingAuth，自动保留策略跳过且不创建当前账号云任务；切回 owner 后才收尾。
- 旧 `ignored` 保持隐藏、云端保留且不产生自动任务。
- tombstone 表允许“只有删除事实、没有 `.ok`/`cloud_snapshot_sync`”的记录；约束拒绝未发布却进入远端清理状态的数据，两个检索索引均存在。
- `failed, published=0` 从发布步骤恢复，`failed, published=1` 从残留清理步骤恢复。
- 用户删除意图和上传完成并发时不会丢失。
- 纯本地快照的用户删除与保留策略清理都不创建 tombstone；存在云记录或遗留 `uploading` 时必须创建。
- metadata revision 变化时旧同步结果 compare-and-set 失败且保持 pending。
- 删除 metadata、`.ok`、ZIP 任一步失败都保留本机快照并可重试。
- 远端删除成功、本机删除失败或进程中断后可以收尾。
- 完整目录读取失败时不写 `missing_confirmed`。
- 从未创建 `snapshot-tombstones/` 的 v0.5.0 仓库返回 `NotFound` 时按空集合正常刷新；目录存在后的分页、响应或对象校验失败仍使刷新失败。
- tombstone JSON 和路径拒绝错误 repository、游戏及 snapshot 身份。
- 两台设备并发删除同一快照时，只比较五项身份字段；不同审计字段不产生冲突，本机最终缓存远端先创建成功的审计值。
- 发现合法 tombstone 后删除陈旧 `cloud_snapshot_sync`，原有 `pending/error` 不再被维护扫描恢复；随后修改本机备注或锁定不产生 Store 请求。
- 上传在 ZIP 前、`.ok` 前和 `.ok` 后三类删除竞态中都不会留下可发布快照。
- 模拟设备 B 分别在设备 A 下载前、下载完成但本机提交前发布 tombstone：A 不产生本机快照；若 B 在最终检查后才发布，则 A 保留已提交副本并禁止原 ID 自动重传。
- 模拟设备 B 分别在设备 A metadata 写入前、写入后发布 tombstone：写入前不产生对象，写入后提交孤儿清理且不标记 `synced`。
- `missing_confirmed` 只提供本机抑制；合法 tombstone 及其专用表缓存为 v0.6+ 设备提供强抑制。
- 遗留 `uploading` 在完整远端对象存在且最终 tombstone 检查仍不存在时补记成功；发现 tombstone 时缓存并清理，不完整时自动快照重试、手动快照等待用户。
- 同 ID 不同内容进入 conflict 且不自动重试。
- 同一共享 Runtime 下，任务 A 已读到空 tombstone 目录后，模拟设备 B 创建 tombstone；A 后续上传、接收、metadata 和上传恢复检查都必须立即读到，不能复用第一次空结果。
- 设备 A 已完成一次 `CatalogRefresh` 后，设备 B 新增 `.ok`、覆盖 metadata 或创建 tombstone；A 在同一进程的下一次刷新必须看到变化。
- `stat_file/get_file/delete_file` 在前一次父目录结果缺失后仍能看到设备 B 新对象；删除不得因旧缺失缓存假成功，下载不得因旧条目使用错误对象。
- `put_file(CreateOnly)` 的服务端结果覆盖任何预检查判断；并发创建只能得到一个成功和一个明确 `AlreadyExists`。
- 清除授权和重新授权会递增 generation、重建 Runtime；旧任务结果不能落库，新任务没有旧 `directory_cache/ensured_directories` 状态。
- 用可暂停的 Fake HTTP/Store 让旧账号任务完成 ZIP 上传后停在 metadata/`.ok` 之前，再清除授权并连接新账号；恢复旧任务后，新账号收到的 metadata、`.ok`、删除及其他 Store 请求数必须全部为 0，旧任务也不得提交成功状态。
- 旧 generation 的 Token 刷新响应和 401 在新授权安装后迟到时，不得覆盖或清除新 Token；排队中的旧 generation 任务开始执行时不得发出任何 HTTP 请求。
- OAuth 回调迟到、用户连续发起两轮授权时，只有当前 generation/state 对应的回调可以安装凭据。
- 同一账号重新授权后只恢复该 `account_id` 的可靠义务；切换到另一个测试账号后，旧账号的上传、metadata 和删除义务保持暂停，新账号任务列表中不得出现。切回原账号后可以继续恢复。
- 旧版固定 `BAIDU_ACCOUNT_ID + account_identity=NULL` 只在首次取得真实身份后绑定一次；身份读取失败不得建立可写 Runtime，已绑定身份不得被另一账号静默覆盖。
- `uinfo.uk` 是唯一百度身份来源；显示名称变化不影响账号。启动核验的 401/Token 无效进入 `WaitingAuth`，网络/超时/429/5xx 保留账号和 Token 并进入 `WaitingRetry`，只有成功取得不同 `uk` 才清空 active ID；三类分支均不得建立未经核验的可写 Runtime。
- legacy 账号已有可靠义务时，只有已持久化 `remote_repository_id` 匹配，或至少一条已知 `.ok` 的 ID、内容哈希、文件统计和归档 SHA-256 强匹配，才能自动绑定当前身份；证据不足进入新命名空间，legacy 义务保持暂停且不产生 Store 写请求，但当前 verified 账号仍可正常刷新和接收。后续强证据认领覆盖向已有 verified 账号的无损合并与冲突回滚。
- 在账号专属 Token 原子替换前后、账号 upsert 前后、`active_cloud_account_id` 提交前后和 Runtime 发布前后分别注入崩溃并重启；启动身份核对必须得到“旧账号仍安全激活”“新账号完整激活”或“无账号激活”之一，绝不能出现账号 A active ID 搭配账号 B Token 的可写 Runtime。
- 清除授权在清空 active 设置后、删除 Token 前崩溃时，重启不得因残留 Token 自动恢复可写 Runtime；孤儿 pending/账号 Token 可清理但不能误删其他账号凭据。
- 旧版单一 Token 迁移在身份读取、新路径原子写、SQLite active 提交和旧文件删除各阶段崩溃后均可重试；不得丢失唯一有效 Token，也不得因旧、新文件同时存在随机选择账号。

### 19.3 tombstone 规模测试

- 构造每个游戏 1,000 和 10,000 条 tombstone，分别验证冷刷新与暖刷新；结果必须完整、无重复、分页结束条件正确。
- 暖刷新只有在本地与远端 `modified_at` 都为 `Some` 且值相等时命中缓存，此时 tombstone JSON 内容下载请求必须为 0；任一时间为空、值变化或新增条目都必须下载验证。
- 在发布目标机器上，排除 Fake Store 人工延迟后，10,000 条暖刷新本地处理时间不得超过 5 秒，额外峰值内存不得超过 64 MiB；超过任一门槛则 v0.6.0 发布前必须增加远端索引/分片或进一步缩减处理集。
- 在相同 10,000 条数据上分别执行一条快照的上传（三次检查）、接收（两次）、metadata（两次）和上传恢复（两次），记录每个动作的 tombstone 列目录页数、对象读取数和检查总耗时。测试适配器必须真实模拟百度每页最多 1,000 条，不能用 O(1) Fake `stat` 掩盖正式适配器的分页实现。
- 点查发布门槛：单条上传的三次 tombstone 检查合计不得超过 6 个目录分页请求；其他单条动作每次检查平均不得超过 2 页。请求数必须相对 tombstone 总量保持有界，1,000 增长到 10,000 条时不得按 10 倍线性增长。
- 若百度存在经过契约测试的完整路径直接查询能力，可以用它满足门槛；否则在第一批正式 tombstone 写入前启用 10.6 定义的 16 桶哈希布局。启用分桶后必须同时重跑无目录兼容、全量刷新、并发创建和跨设备竞态测试。
- 性能测试记录布局、条目数、页数、Store 调用数、JSON 下载数、缓存命中数、总耗时和峰值内存，不能只断言功能结果。

### 19.4 Tauri 和前端测试

- 后台维护中点击上传、下载或刷新进入等待，不再报冲突。
- 排队和真正运行使用不同文案。
- 删除后列表立即隐藏，重启后仍隐藏，失败重试成功后记录最终消失。
- 旧页面对待删除快照发起恢复/改名/锁定会被后端拒绝。
- 关闭并重开云端窗口共享刷新任务和结果。
- generation N 已读不能清除同时产生的 N+1。
- 自己上传不亮红点，另一设备新增/改名/删除会亮红点。
- 远端快照已被另一设备删除后，本机副本改名和锁定仍正常，但不显示待同步、不产生 metadata 云任务。
- 授权失效显示等待授权，不显示内容冲突。
- 清除或重新授权期间的旧任务不得让界面显示错误账号的成功结果；重新连接后的刷新必须访问新账号远端。
- 当前账号不是快照 owner 时上传按钮不可执行并显示明确归属提示；未激活 owner 的自动淘汰不显示为错误，显式删除显示等待原账号连接。

### 19.5 真实百度验收

使用隔离测试游戏和假存档，不恢复到真实游戏目录：

1. 后台刷新时连续发起两个不同快照上传，确认排队而非报错。
2. 上传已运行时删除同一快照，确认运行任务结束后创建 tombstone，最终云端 metadata、`.ok`、ZIP 和本机记录都消失，tombstone 保留。
3. 让上传分别停在 `Queued`、`WaitingAuth` 和 `WaitingRetry` 后删除，确认上传被取代且不会为了删除先上传 ZIP。
4. 上传不同游戏的快照，确认存在受控并发且未触发 Token 重复刷新。
5. 断网修改备注和锁定，联网后确认 revision 最新值上云。
6. 断网删除已上云快照，确认界面立即隐藏、联网后自动创建 tombstone 并收尾。
7. 设备 B 新增快照或修改 metadata，设备 A 后台发现后显示红点；打开并成功展示后清除。
8. 设备 B 删除远端快照，设备 A 保留本机副本并缓存 tombstone；模拟设备 A 的旧自动上传义务，确认同 ID 发布被拒绝。
9. 人工制造同 ID 不同内容，确认双方保留且界面只报告硬冲突。
10. 用 v0.5.0 兼容性测试证明旧客户端会忽略 tombstone，并把“同仓库设备必须全部升级”写入发布说明，不伪造跨旧版本强保证。
11. 暖刷新已缓存 tombstone，记录百度分页次数、远端 `modified_at` 是否存在、tombstone JSON 下载次数和连续三轮刷新时间。本地与远端修改时间都存在且相等时 JSON 下载必须为 0；任一为空时必须重新下载验证。三轮中的最大值不得超过 15 秒，否则阻止发布并先优化发现协议。后续长期样本再单独统计 P95。
12. 使用从未创建 tombstone 目录的真实 v0.5.0 测试仓库刷新，确认目录 `NotFound` 不影响原有 `.ok`、metadata 和云端列表。
13. 保持设备 A 进程和共享 Runtime 不退出：先刷新并缓存空结果，再由设备 B 分别新增快照、修改 metadata、创建 tombstone；A 的下一次刷新及上传/下载安全检查必须立即看到变化。
14. 设备 A 的上传完成 ZIP 后暂停，在浏览器清除授权并连接设备 B 使用的另一个测试账号，再恢复旧任务；确认旧任务不再发出 metadata、`.ok` 或删除请求，新账号目录保持不变，旧账号结果不落入新账号 SQLite 状态。
15. 制造旧账号请求迟到的 401，再完成新账号授权；确认迟到错误不会清除新 Token，新账号刷新仍成功。确认旧账号可靠任务没有在新账号执行，切回旧账号后才恢复；记录账号切换等待时间和所有跨 generation 请求日志。
16. 在真实百度可承受的数据规模下测量单条上传三次 tombstone 检查的请求数与耗时；结合 10,000 条合成 HTTP 契约结果确认满足 19.3。不得为了测试向用户正式目录批量写入 10,000 个垃圾对象。
17. 使用两个隔离本机数据目录连接同一百度账号，确认两边随机生成的本机 repository 候选 ID 不同仍属正常；设备 B 有强证据时通过已知 `.ok` 或已持久化的 `remote_repository_id` 正确认领 legacy 状态。没有强证据时只暂停旧义务，新 verified 命名空间仍能刷新和接收远端对象；随后补足证据可事务合并 legacy 记录。
18. 快照在账号 A 首次上传后切换账号 B：B 不能再次上传该本机快照，十分钟保留策略跳过它；显式删除只进入账号 A 的等待状态且 B 远端无删除请求。切回 A 后完成 tombstone、远端清理和本机删除。
19. 账号 A、B 的远端都存在同 ID、同内容快照，先从 A 接收落地，再切换 B 点击接收；确认 B 不下载 ZIP、不建立第二条同步记录，A owner 不变且 B 的远端条目仍可见。
20. 分别模拟启动 `uinfo` 返回 401、网络错误、429、5xx、相同 `uk`、不同 `uk` 和缺失 `uk`；核对 WaitingAuth、WaitingRetry、正常启动、安全异常及响应异常分支，确认显示名称变化不会触发切号。
21. 先绑定 manifest ID A，再让远端返回 ID B；确认进入 `repository_mismatch`、数据库仍保存 A、全部仓库任务不发请求。恢复远端 A 后可以继续；仅重新授权不能把本机值覆盖成 B。

全过程记录百度远端对象、SQLite 状态、任务事件、重试次数和最终文件哈希。任何真实存档目录都不得用于删除或恢复故障注入。

## 二十、完成标准

只有同时满足以下条件，v0.6.0 云冲突治理才算完成：

- 所有现有云入口都经过统一协调器，不再使用全局忙碌状态直接拒绝用户操作。
- 同快照串行、不同快照有限并发、账号刷新屏障和共享 Token 刷新均有自动测试。
- 手动删除已上云快照具备立即隐藏、可靠重试和云后本地的完整闭环。
- v0.6+ 上传不能复活合法 tombstone 对应的 snapshot ID；旧版本边界和全设备升级要求已进入发布说明。
- `conflict` CHECK、旧删除状态、遗留 `uploading` 和 `ignored` 的迁移/恢复测试全部通过。
- 下载、metadata 和上传中断恢复的跨设备 tombstone 竞态测试全部通过。
- v0.5.0 无 tombstone 目录兼容、本地 metadata-only、可空修改时间缓存和并发删除审计字段测试全部通过。
- 同进程跨设备新鲜度、Store 各读取/删除入口、真实账号身份和重新授权 generation 测试全部通过，不存在跨任务目录缓存陈旧；旧任务不能读取新 Token、向新账号发请求、由迟到错误清除新凭据，旧账号可靠义务也不能在新账号恢复。
- 分账号 Token 安装/清除的逐步崩溃测试全部通过；启动时不存在 active ID 与 Token 身份错配的可写 Runtime，legacy 设备 B 不因本机随机 repository 候选不同被误判。
- `uinfo.uk` 身份核验分类和 `remote_repository_id` 首次绑定/相同/不一致测试全部通过；临时网络失败不清空账号，仓库不一致不覆盖旧值也不产生仓库请求。
- 单快照单账号归属、第二账号上传与同内容接收拒绝、未激活 owner 保留策略跳过和切回 owner 后删除收尾测试全部通过。
- manifest single-flight 瞬时失败可由下一任务重试，成功与失败结果均不跨账号/generation 泄漏。
- 1,000/10,000 条 tombstone 的全量刷新和单对象点查测试及真实百度暖刷新满足 19.3、19.5 的性能门槛；两端修改时间都存在且相等时不重复下载 JSON，任一为空时不错误命中缓存；最终平铺或 16 桶布局在第一批正式对象写入前冻结。
- metadata 同步期间再次编辑不会丢失用户新值。
- 后台远端变化有持久化小红点，自身操作不误报。
- 排队、授权、网络、限流、远端损坏和硬冲突在界面上能够区分。
- core、Tauri、前端构建、严格 Clippy、隔离端到端和真实百度专项全部通过。

## 二十一、最终结论

本方案选择“内存协调器 + SQLite 业务状态”而不是“全量持久化任务队列”。它既能消除当前用户可见的云操作互斥，又保留上传、metadata 和删除的跨重启可靠性，复杂度也保持在 v0.6.0 可以控制的范围内。

开发时应先完成队列和共享运行时，再接删除与未读提醒。不要在现有 `AtomicU8` 全局互斥上继续增加特殊分支，也不要先实现页面小红点再补底层状态；那会让同一批冲突以新的形式再次出现。

本文至此冻结为 v0.6.0 实施基线。后续不再通过整份文档循环评审扩大范围；只接受实现或测试证明会造成数据丢失、跨账号写入或可靠义务无法恢复的阻塞修正，其余建议进入后续版本清单。每个实施阶段完成后审查对应代码、迁移和测试，而不是重新推演整套系统。
