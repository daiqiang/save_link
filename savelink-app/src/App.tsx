import { useEffect, useState, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import "./App.css";
import { Icon } from "./lib/icons";
import { formatSize, formatTimestamp, formatTimestampTime, REASON_LABEL } from "./lib/format";
import * as api from "./lib/api";
import type { Game, SaveDiscoveryStatus, Snapshot } from "./lib/types";
import { ToastProvider, useToast } from "./components/Toast";
import { AddGameDialog } from "./components/AddGameDialog";
import type { AddGameMode } from "./components/AddGameDialog";
import { EditGameDialog } from "./components/EditGameDialog";
import { RestoreDialog } from "./components/RestoreDialog";
import { SnapshotDrawer } from "./components/SnapshotDrawer";
import { CloudSnapshotsDialog } from "./components/CloudSnapshotsDialog";
import { BindSavePathDialog } from "./components/BindSavePathDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { SaveDiscoveryPanel } from "./components/SaveDiscoveryPanel";

const ACTIVE_DISCOVERY_PHASES = new Set([
  "starting_watchers",
  "launching_game",
  "monitoring",
  "exit_grace_period",
  "analyzing",
  "confirming",
]);

function SaveLink() {
  const toast = useToast();
  const [games, setGames] = useState<Game[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [creating, setCreating] = useState(false);
  const [cloudUploadingId, setCloudUploadingId] = useState<string | null>(null);
  const [launchingGame, setLaunchingGame] = useState(false);
  const [profileLabel, setProfileLabel] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [discovery, setDiscovery] = useState<SaveDiscoveryStatus | null>(null);
  const [discoveryAction, setDiscoveryAction] = useState(false);

  // 弹窗 / 抽屉 / 菜单状态
  const [addMode, setAddMode] = useState<AddGameMode | null>(null);
  const [showCloud, setShowCloud] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [editingGame, setEditingGame] = useState<Game | null>(null);
  const [bindingGame, setBindingGame] = useState<Game | null>(null);
  const [drawerSnap, setDrawerSnap] = useState<Snapshot | null>(null);
  const [restoreSnap, setRestoreSnap] = useState<Snapshot | null>(null);
  const [deleteSnap, setDeleteSnap] = useState<Snapshot | null>(null);
  const [menu, setMenu] = useState<{ snap: Snapshot; x: number; y: number } | null>(null);

  const selected = games.find((g) => g.id === selectedId) ?? null;
  // 只渲染/操作属于当前所选游戏的快照，杜绝“看到或误操作到别的游戏快照”的串档风险。
  const shown = selected ? snapshots.filter((s) => s.game_id === selected.id) : [];
  // 后端按创建时间返回快照；待整理项仍属于原显示区域，直到维护周期统一移动。
  const lockedShown = shown.filter((snapshot) => snapshot.display_zone === "locked");
  const recentShown = shown.filter((snapshot) => snapshot.display_zone === "normal");
  const snapshotGroups = [
    ...(lockedShown.length > 0
      ? [{ key: "locked", title: "锁定存档", snapshots: lockedShown }]
      : []),
    { key: "recent", title: "最近存档", snapshots: recentShown },
  ];
  const requiredBindingSourceCount = Math.max(1, ...shown.map((snapshot) => snapshot.source_count));
  const discoveryActive = discovery ? ACTIVE_DISCOVERY_PHASES.has(discovery.phase) : false;
  const selectedDiscovery = selected && discovery?.game_id === selected.id ? discovery : null;
  const selectedDiscoveryActive = Boolean(selectedDiscovery && discoveryActive);

  const loadGames = useCallback(async () => {
    const gs = await api.listGames();
    setGames(gs);
    setSelectedId((cur) => cur && gs.some((g) => g.id === cur) ? cur : gs[0]?.id ?? null);
  }, []);

  const loadSnapshots = useCallback(async (gameId: string) => {
    setSnapshots(await api.listSnapshots(gameId));
  }, []);

  useEffect(() => { loadGames(); }, [loadGames]);
  useEffect(() => {
    api.getAppInfo()
      .then((info) => {
        setProfileLabel(info.profile_label);
        setAppVersion(info.version);
      })
      .catch(() => undefined);
  }, []);
  // 切换游戏：先立即清空上一个游戏的残留，再按 selectedId 加载；
  // cancelled 守卫避免“切得快时旧请求后到、把别的游戏快照回填进来”。
  useEffect(() => {
    if (!selectedId) { setSnapshots([]); return; }
    let cancelled = false;
    setSnapshots([]);
    api.listSnapshots(selectedId).then((s) => { if (!cancelled) setSnapshots(s); });
    return () => { cancelled = true; };
  }, [selectedId]);

  // 刷新当前游戏的时间线 + 列表元数据
  const refresh = useCallback(async () => {
    await loadGames();
    if (selectedId) await loadSnapshots(selectedId);
  }, [loadGames, loadSnapshots, selectedId]);

  useEffect(() => {
    const unlisten = listen("auto-backup-changed", () => {
      void refresh();
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, [refresh]);

  useEffect(() => {
    let disposed = false;
    const unlisten = listen<SaveDiscoveryStatus>("save-discovery-status-changed", (event) => {
      if (!disposed) setDiscovery(event.payload);
    });
    void api.getSaveDiscoveryStatus()
      .then((status) => { if (!disposed) setDiscovery(status); })
      .catch(() => undefined);
    return () => {
      disposed = true;
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  async function startDiscovery() {
    if (!selected || discoveryAction) return;
    setDiscoveryAction(true);
    try {
      setDiscovery(await api.startSaveDiscovery(selected.id));
    } catch (error) {
      toast(String(error), "err");
      await api.getSaveDiscoveryStatus().then(setDiscovery).catch(() => undefined);
    } finally {
      setDiscoveryAction(false);
    }
  }

  async function launchConfiguredGame() {
    if (!selected || launchingGame || discoveryActive) return;
    setLaunchingGame(true);
    try {
      const result = await api.launchGame(selected.id);
      toast(`游戏已启动（PID ${result.pid}），已配置游戏不进行存档目录监测`, "ok");
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setLaunchingGame(false);
    }
  }

  async function stopDiscovery() {
    if (discoveryAction) return;
    setDiscoveryAction(true);
    try {
      setDiscovery(await api.stopSaveDiscovery());
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setDiscoveryAction(false);
    }
  }

  async function cancelDiscovery() {
    if (discoveryAction) return;
    setDiscoveryAction(true);
    try {
      setDiscovery(await api.cancelSaveDiscovery());
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setDiscoveryAction(false);
    }
  }

  async function confirmDiscovery(savePaths: string[]) {
    if (!selected || discoveryAction || savePaths.length === 0) return;
    setDiscoveryAction(true);
    try {
      const result = await api.confirmSaveDiscoveryPaths(selected.id, savePaths);
      if (result.first_backup === "created") {
        toast("存档目录已设置，并已创建第一个自动快照", "ok");
      } else if (result.first_backup === "no_change") {
        toast("存档目录已设置；已有相同内容的快照，未重复创建", "ok");
      } else if (result.first_backup === "disabled") {
        toast("存档目录已设置；自动备份已关闭，未创建快照", "warn");
      } else {
        toast(`存档目录已设置，但首次自动备份失败：${result.backup_error ?? "未知错误"}`, "warn");
      }
      await refresh();
      await api.getSaveDiscoveryStatus().then(setDiscovery).catch(() => undefined);
    } catch (error) {
      toast(String(error), "err");
      await api.getSaveDiscoveryStatus().then(setDiscovery).catch(() => undefined);
    } finally {
      setDiscoveryAction(false);
    }
  }

  async function createSnapshot() {
    if (!selected) return;
    if (selected.configuration_state !== "configured") return toast("请先设置本机存档目录", "warn");
    setCreating(true);
    try {
      const s = await api.createSnapshot(selected.id, null);
      if (s === null) toast("存档未变化，未创建新快照", "warn");
      else toast("快照已创建", "ok");
      await refresh();
    } catch (e) {
      // 后端会返回中文错误（如“存档目录不存在”），之前没 catch 会被吞掉、用户毫无反馈。
      toast(String(e), "err");
    } finally {
      setCreating(false);
    }
  }

  async function toggleLock(s: Snapshot) {
    await api.updateSnapshotMeta(s.id, null, !s.locked);
    await refresh();
    toast(s.locked ? "已取消锁定" : "快照已锁定，不会被自动清理", "ok");
  }

  async function uploadToCloud(s: Snapshot) {
    if (selected?.configuration_state !== "configured") return toast("请先设置本机存档目录", "warn");
    if (cloudUploadingId || s.cloud_status === "uploaded" || s.cloud_status === "downloaded") return;
    setCloudUploadingId(s.id);
    try {
      const current = await api.getBaiduConnectionStatus();
      if (!current.connected) {
        toast("请在浏览器中完成百度网盘授权，授权后将继续上传", "warn");
        const connected = await api.connectBaidu();
        if (!connected.connected) throw new Error("百度网盘授权未完成");
      }
      toast("正在打包并上传这条快照", "warn");
      const result = await api.uploadSnapshotToBaidu(s.game_id, s.id);
      await loadSnapshots(s.game_id);
      toast(
        result.outcome === "already_present" ? "这条快照已经保存在百度网盘" : "快照已保存到百度网盘",
        "ok",
      );
    } catch (error) {
      toast(String(error), "err");
      await loadSnapshots(s.game_id).catch(() => undefined);
    } finally {
      setCloudUploadingId(null);
    }
  }

  async function confirmDelete() {
    if (!deleteSnap) return;
    await api.deleteSnapshot(deleteSnap.id);
    setDeleteSnap(null);
    await refresh();
    toast("快照已删除", "ok");
  }

  return (
    <div className="app" onClick={() => menu && setMenu(null)}>
      <div className="topbar">
        <div className="brand">
          <span className="logo"><Icon.History size={20} /></span>
          SaveLink
          {appVersion && <span className="app-version">v{appVersion}</span>}
          <span className={`sub ${profileLabel ? "test-profile" : ""}`}>{profileLabel ?? "本地存档管理"}</span>
        </div>
        <div className="spacer" />
        <button
          className="btn sm cloud-entry"
          title="云端存档"
          aria-label="云端存档"
          onClick={() => setShowCloud(true)}
        >
          <Icon.Download />
          <span>云端存档</span>
        </button>
        <button className="iconbtn" title="设置" onClick={() => setShowSettings(true)}><Icon.Settings /></button>
        <button className="iconbtn" title="帮助" onClick={() => toast("帮助文档：后续补充", "warn")}><Icon.Help /></button>
      </div>

      <aside className="sidebar">
        <div className="head">
          <span>游戏列表</span>
          <button className="iconbtn" title="添加游戏" onClick={() => setAddMode("steam")}><Icon.Plus /></button>
        </div>
        {games.map((g) => (
          <div key={g.id} className={`game-item ${g.id === selectedId ? "active" : ""}`}
            onClick={() => setSelectedId(g.id)}>
            <div className="game-cover">{g.name[0] ?? "游"}</div>
            <div className="game-meta">
              <div className="game-name"><span className={`status-dot ${g.configuration_state === "configured" ? "ok" : "warn"}`} />{g.name}</div>
              <div className="game-sub">
                {g.configuration_state !== "configured"
                  ? `${g.snapshot_count} 个快照 · ${discoveryActive && discovery?.game_id === g.id
                    ? "正在查找存档"
                    : g.configuration_state === "pending_discovery"
                    ? "待设置存档目录"
                    : g.emulator === "desmume" ? "尚未绑定 DeSmuME" : "尚未绑定存档目录"}`
                  : <>{g.snapshot_count} 个快照{g.last_snapshot_at ? ` · 最近 ${formatTimestampTime(g.last_snapshot_at)}` : ""}</>}
              </div>
            </div>
          </div>
        ))}
      </aside>

      <main className="detail">
        {!selected ? (
          <EmptyState
            onAdd={() => setAddMode("steam")}
            onCloud={() => setShowCloud(true)}
          />
        ) : (
          <div>
            <div className="ghead">
              <div className="cover">{selected.name[0] ?? "游"}</div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <h1>{selected.name}</h1>
                {selected.configuration_state === "configured" ? (
                  <div className="game-paths">
                    {selected.save_paths.map((path, index) => (
                      <div className="path" key={index}><Icon.Folder size={14} /><span className="mono">{path}</span></div>
                    ))}
                  </div>
                ) : (
                  <div className="game-paths">
                    <div className="path"><Icon.Folder size={14} /><span className="mono">
                      {selected.configuration_state === "pending_discovery"
                        ? "尚未设置存档目录，当前不会创建备份"
                        : selected.emulator === "desmume" ? "尚未绑定本机 DeSmuME ROM" : "尚未绑定本机存档目录"}
                    </span></div>
                    {selected.launch_executable_path && (
                      <div className="path" title={selected.launch_executable_path}><Icon.Gamepad size={14} />
                        <span className="mono">{selected.launch_executable_path}</span></div>
                    )}
                  </div>
                )}
              </div>
            </div>

            <div className="gstats">
              <div className="gstat"><div className="k">快照数量</div><div className="v">{shown.length} 个</div></div>
              <div className="gstat"><div className="k">最近快照</div><div className="v">{formatTimestamp(shown[0]?.created_at)}</div></div>
              <div className="gstat"><div className="k">仓库占用</div>
                <div className="v">{formatSize(shown.reduce((a, s) => a + s.total_size, 0))}</div></div>
            </div>

            <div className="toolbar">
              {selected.configuration_state === "pending_discovery" ? <>
                {selectedDiscoveryActive ? <>
                  <button className="btn primary" onClick={stopDiscovery} disabled={discoveryAction}>
                    <Icon.Search /> 停止并分析
                  </button>
                  <button className="btn" onClick={cancelDiscovery} disabled={discoveryAction}>
                    <Icon.Close /> 取消监测
                  </button>
                </> : (
                  <button className="btn primary" onClick={startDiscovery}
                    disabled={discoveryAction || discoveryActive}
                    title={discoveryActive ? `${discovery?.game_name ?? "另一款游戏"} 正在查找存档` : "启动游戏并查找存档"}>
                    <Icon.Search /> {selectedDiscovery?.phase === "awaiting_confirmation" ? "重新监测" : "启动游戏并查找存档"}
                  </button>
                )}
                <button className="btn" onClick={() => setBindingGame(selected)}>
                  <Icon.Folder /> 手动设置存档目录
                </button>
              </> : selected.configuration_state === "pending_binding" ? (
                <button className="btn primary" onClick={() => selected.emulator === "desmume"
                  ? setAddMode("desmume")
                  : setBindingGame(selected)}>
                  {selected.emulator === "desmume"
                    ? <><Icon.Gamepad /> 绑定 DeSmuME</>
                    : <><Icon.Folder /> 绑定存档目录</>}
                </button>
              ) : (
                <>
                  {(selected.launch_kind === "executable" || selected.launch_kind === "steam") && (
                    <button className="btn" onClick={launchConfiguredGame}
                      disabled={launchingGame || discoveryActive}
                      title={discoveryActive ? "请先结束另一个游戏的存档查找" : "启动游戏，不进行存档目录监测"}>
                      {launchingGame
                        ? <><span className="spin"><Icon.RotateCcw /></span> 正在启动…</>
                        : <><Icon.Gamepad /> 启动游戏</>}
                    </button>
                  )}
                  <button className="btn primary" onClick={createSnapshot} disabled={creating}>
                    {creating ? <><span className="spin"><Icon.RotateCcw /></span> 正在扫描…</> : <><Icon.Camera /> 创建快照</>}
                  </button>
                </>
              )}
              <button className="btn" onClick={() => setEditingGame(selected)} disabled={selectedDiscoveryActive}
                title={selectedDiscoveryActive ? "请先停止或取消监测" : "编辑游戏"}>
                <Icon.Edit /> 编辑游戏
              </button>
            </div>

            {selected.configuration_state === "pending_discovery" && selectedDiscovery && selectedDiscovery.phase !== "idle" && (
              <SaveDiscoveryPanel
                status={selectedDiscovery}
                busy={discoveryAction}
                onConfirm={confirmDiscovery}
              />
            )}

            <div className="snapshot-divider" aria-hidden="true" />
            <div className="timeline">
              {shown.length === 0 && <div className="empty-tl">{selected.configuration_state !== "configured"
                ? selected.configuration_state === "pending_discovery"
                  ? "尚未设置存档目录，当前不会创建备份。"
                  : selected.emulator === "desmume" ? "尚未绑定本机 DeSmuME ROM。" : "尚未绑定本机存档目录。"
                : "还没有快照。点击「创建快照」保存当前存档状态。"}</div>}
              {shown.length > 0 && snapshotGroups.map((group) => <section className="snapshot-group" key={group.key}
                aria-labelledby={`snapshot-group-${group.key}`}>
                <div className="snapshot-group-head" id={`snapshot-group-${group.key}`}>
                  <span>{group.title}</span>
                  <span>{group.snapshots.length} 个</span>
                </div>
                <div className="snapshot-group-list">
              {group.snapshots.length === 0 && <div className="snapshot-group-empty">暂无最近存档</div>}
              {group.snapshots.map((s) => {
                const cloudBusy = cloudUploadingId === s.id;
                const cloudUploaded = s.cloud_status === "uploaded" || s.cloud_status === "downloaded";
                const cloudFailed = s.cloud_status === "error";
                const restorePathsReady = selected.configuration_state === "configured"
                  && selected.save_paths.length >= s.source_count;
                const cloudTitle = cloudBusy
                  ? "正在上传到百度网盘"
                  : cloudUploaded
                    ? "已保存到百度网盘"
                    : cloudFailed
                      ? "上次上传失败，点击重试"
                      : "上传到百度网盘";
                const cloudLabel = cloudBusy
                  ? "上传中"
                  : cloudUploaded
                    ? "已上云"
                    : cloudFailed
                      ? "重试"
                      : "上传";
                return (
                <div key={s.id} className={`snap ${s.locked ? "is-locked" : ""} ${s.pending_reorganization ? "is-pending" : ""}`}
                  onClick={() => setDrawerSnap(s)}>
                  <div className="snap-main">
                    <div className="snap-time">{formatTimestamp(s.created_at)}</div>
                    <div className="snap-note">
                      {s.display_name}
                      {s.pending_reorganization
                        ? <span className="badge pending">{s.locked ? <Icon.Lock size={12} /> : <Icon.Unlock size={12} />} {s.locked ? "已锁定，待整理" : "已解锁，待整理"}</span>
                        : s.locked && <span className="badge lock"><Icon.Lock size={12} /> 已锁定</span>}
                    </div>
                    <div className="snap-info">
                      {s.file_count} 个文件<span className="sep">·</span>{formatSize(s.total_size)}
                      {s.source_count > 1 && <><span className="sep">·</span>{s.source_count} 个存档目录</>}
                      <span className="sep">·</span>{REASON_LABEL[s.reason]}
                    </div>
                  </div>
                  <div className="snap-actions" onClick={(e) => e.stopPropagation()}>
                    <button className="btn sm" title={restorePathsReady ? "恢复" : `请先绑定 ${s.source_count} 个本机存档目录`}
                      disabled={!restorePathsReady} onClick={() => setRestoreSnap(s)}><Icon.RotateCcw /> 恢复</button>
                    <button
                      className={`btn sm cloud-upload ${cloudUploaded ? "is-uploaded" : ""} ${cloudFailed ? "is-error" : ""}`}
                      title={cloudTitle}
                      aria-label={cloudTitle}
                      aria-busy={cloudBusy}
                      disabled={selected.configuration_state !== "configured" || cloudUploadingId !== null || cloudUploaded}
                      onClick={() => uploadToCloud(s)}
                    >
                      {cloudBusy
                        ? <span className="spin"><Icon.RotateCcw /></span>
                        : cloudUploaded
                          ? <Icon.Check />
                          : <Icon.CloudUpload />}
                      <span>{cloudLabel}</span>
                    </button>
                    <button className="iconbtn" title={s.locked ? "取消锁定" : "锁定"} onClick={() => toggleLock(s)}>
                      {s.locked ? <Icon.Unlock /> : <Icon.Lock />}
                    </button>
                    <button className="iconbtn" title="更多"
                      onClick={(e) => setMenu({ snap: s, x: e.clientX, y: e.clientY })}><Icon.More /></button>
                  </div>
                </div>
                );
              })}
                </div>
              </section>)}
            </div>
          </div>
        )}
      </main>

      {/* 上下文菜单 */}
      {menu && (
        <div className="ctx-menu" style={{ left: Math.min(menu.x, window.innerWidth - 190), top: menu.y + 4 }}
          onClick={(e) => e.stopPropagation()}>
          <button onClick={() => { setDrawerSnap(menu.snap); setMenu(null); }}><Icon.Edit /> 修改备注</button>
          <button onClick={() => { toggleLock(menu.snap); setMenu(null); }}>
            {menu.snap.locked ? <><Icon.Unlock /> 取消锁定</> : <><Icon.Lock /> 锁定快照</>}
          </button>
          <div className="divider" />
          <button className="danger" disabled={menu.snap.locked}
            onClick={() => { if (!menu.snap.locked) { setDeleteSnap(menu.snap); setMenu(null); } }}>
            <Icon.Trash /> {menu.snap.locked ? "锁定快照不能删除" : "删除快照"}
          </button>
        </div>
      )}

      {addMode && <AddGameDialog initialMode={addMode} onClose={() => setAddMode(null)}
        onCreated={(g) => { setAddMode(null); setSelectedId(g.id); loadGames(); }} />}


      {showCloud && <CloudSnapshotsDialog onClose={() => setShowCloud(false)}
        onReceived={async (gameId) => {
          await loadGames();
          setSelectedId(gameId);
          await loadSnapshots(gameId);
        }} />}

      {showSettings && <SettingsDialog onClose={() => setShowSettings(false)} />}

      {editingGame && <EditGameDialog game={editingGame}
        onClose={() => setEditingGame(null)}
        onRebindDesmume={() => {
          setEditingGame(null);
          setAddMode("desmume");
        }}
        onSaved={(g) => {
          setEditingGame(null);
          setSelectedId(g.id);
          loadGames();
        }}
        onDeleted={(g) => {
          setEditingGame(null);
          setDrawerSnap(null);
          setRestoreSnap(null);
          setDeleteSnap(null);
          setMenu(null);
          const remaining = games.filter((item) => item.id !== g.id);
          setGames(remaining);
          setSelectedId(remaining[0]?.id ?? null);
          setSnapshots([]);
          loadGames();
        }} />}

      {bindingGame && <BindSavePathDialog game={bindingGame} sourceCount={requiredBindingSourceCount}
        onClose={() => setBindingGame(null)}
        onBound={(game) => {
          setBindingGame(null);
          setGames((current) => current.map((item) => item.id === game.id ? game : item));
          setSelectedId(game.id);
        }} />}

      {drawerSnap && selected && (
        <SnapshotDrawer game={selected} snapshot={drawerSnap}
          onClose={() => setDrawerSnap(null)}
          onChanged={refresh}
          onRestore={(s) => { setDrawerSnap(null); setRestoreSnap(s); }}
          onDelete={(s) => { setDrawerSnap(null); setDeleteSnap(s); }} />
      )}

      {restoreSnap && selected && (
        <RestoreDialog game={selected} target={restoreSnap}
          onClose={() => setRestoreSnap(null)} onDone={refresh} />
      )}

      {deleteSnap && (
        <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && setDeleteSnap(null)}>
          <div className="modal narrow">
            <div className="modal-head"><h3>删除快照？</h3>
              <button className="iconbtn" onClick={() => setDeleteSnap(null)}><Icon.Close /></button></div>
            <div className="modal-body">
              <div className="callout warn"><span className="ic"><Icon.Alert /></span>
                <div>将删除快照「{deleteSnap.note || "未命名快照"}」（{formatTimestamp(deleteSnap.created_at)}）。<br />删除后该版本将无法恢复。</div>
              </div>
            </div>
            <div className="modal-foot">
              <button className="btn" onClick={() => setDeleteSnap(null)}>取消</button>
              <button className="btn danger" onClick={confirmDelete}><Icon.Trash /> 删除</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function EmptyState({ onAdd, onCloud }: { onAdd: () => void; onCloud: () => void }) {
  return (
    <div className="empty">
      <span className="ic"><Icon.History size={48} /></span>
      <h2>还没有添加游戏</h2>
      <p>添加一个游戏后，就可以为它创建存档快照和时间线。</p>
      <div className="empty-actions">
        <button className="btn primary" onClick={onAdd}><Icon.Plus /> 添加游戏</button>
        <button className="btn cloud-entry" onClick={onCloud}><Icon.Download /> 下载云存档</button>
      </div>
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <SaveLink />
    </ToastProvider>
  );
}
