import { useEffect, useState, useCallback } from "react";
import "./App.css";
import { Icon } from "./lib/icons";
import { formatSize, REASON_LABEL } from "./lib/format";
import * as api from "./lib/api";
import type { Game, Snapshot } from "./lib/types";
import { ToastProvider, useToast } from "./components/Toast";
import { AddGameDialog } from "./components/AddGameDialog";
import { EditGameDialog } from "./components/EditGameDialog";
import { RestoreDialog } from "./components/RestoreDialog";
import { SnapshotDrawer } from "./components/SnapshotDrawer";
import { CloudSnapshotsDialog } from "./components/CloudSnapshotsDialog";
import { BindSavePathDialog } from "./components/BindSavePathDialog";

function SaveLink() {
  const toast = useToast();
  const [games, setGames] = useState<Game[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [creating, setCreating] = useState(false);
  const [cloudUploadingId, setCloudUploadingId] = useState<string | null>(null);
  const [profileLabel, setProfileLabel] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string | null>(null);

  // 弹窗 / 抽屉 / 菜单状态
  const [showAdd, setShowAdd] = useState(false);
  const [showCloud, setShowCloud] = useState(false);
  const [editingGame, setEditingGame] = useState<Game | null>(null);
  const [bindingGame, setBindingGame] = useState<Game | null>(null);
  const [drawerSnap, setDrawerSnap] = useState<Snapshot | null>(null);
  const [restoreSnap, setRestoreSnap] = useState<Snapshot | null>(null);
  const [deleteSnap, setDeleteSnap] = useState<Snapshot | null>(null);
  const [menu, setMenu] = useState<{ snap: Snapshot; x: number; y: number } | null>(null);

  const selected = games.find((g) => g.id === selectedId) ?? null;
  // 只渲染/操作属于当前所选游戏的快照，杜绝“看到或误操作到别的游戏快照”的串档风险。
  const shown = selected ? snapshots.filter((s) => s.game_id === selected.id) : [];

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

  async function createSnapshot() {
    if (!selected) return;
    if (selected.save_paths.length === 0) return toast("请先绑定本机存档目录", "warn");
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
          <span className={`sub ${profileLabel ? "test-profile" : ""}`}>{profileLabel ?? "本地存档时间线"}</span>
        </div>
        <div className="spacer" />
        <button className="iconbtn" title="云端存档" onClick={() => setShowCloud(true)}><Icon.CloudUpload /></button>
        <button className="iconbtn" title="设置" onClick={() => toast("设置功能：后续补充", "warn")}><Icon.Settings /></button>
        <button className="iconbtn" title="帮助" onClick={() => toast("帮助文档：后续补充", "warn")}><Icon.Help /></button>
      </div>

      <aside className="sidebar">
        <div className="head">
          <span>游戏列表</span>
          <button className="iconbtn" title="添加游戏" onClick={() => setShowAdd(true)}><Icon.Plus /></button>
        </div>
        {games.map((g) => (
          <div key={g.id} className={`game-item ${g.id === selectedId ? "active" : ""}`}
            onClick={() => setSelectedId(g.id)}>
            <div className="game-cover">{g.name[0] ?? "游"}</div>
            <div className="game-meta">
              <div className="game-name"><span className={`status-dot ${g.save_paths.length > 0 ? "ok" : "warn"}`} />{g.name}</div>
              <div className="game-sub">
                {g.save_paths.length === 0
                  ? `${g.snapshot_count} 个快照 · 尚未绑定存档目录`
                  : <>{g.snapshot_count} 个快照{g.last_snapshot_at ? ` · 最近 ${g.last_snapshot_at.slice(11) || g.last_snapshot_at}` : ""}</>}
              </div>
            </div>
          </div>
        ))}
      </aside>

      <main className="detail">
        {!selected ? (
          <EmptyState onAdd={() => setShowAdd(true)} />
        ) : (
          <div>
            <div className="ghead">
              <div className="cover">{selected.name[0] ?? "游"}</div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <h1>{selected.name}</h1>
                <div className="path"><Icon.Folder size={14} /><span className="mono">{selected.save_paths[0] ?? "尚未绑定本机存档目录"}</span></div>
              </div>
            </div>

            <div className="gstats">
              <div className="gstat"><div className="k">快照数量</div><div className="v">{shown.length} 个</div></div>
              <div className="gstat"><div className="k">最近快照</div><div className="v">{shown[0]?.created_at ?? "—"}</div></div>
              <div className="gstat"><div className="k">仓库占用</div>
                <div className="v">{formatSize(shown.reduce((a, s) => a + s.total_size, 0))}</div></div>
            </div>

            <div className="toolbar">
              {selected.save_paths.length === 0 ? (
                <button className="btn primary" onClick={() => setBindingGame(selected)}>
                  <Icon.Folder /> 绑定存档目录
                </button>
              ) : (
                <button className="btn primary" onClick={createSnapshot} disabled={creating}>
                  {creating ? <><span className="spin"><Icon.RotateCcw /></span> 正在扫描…</> : <><Icon.Camera /> 创建快照</>}
                </button>
              )}
              <button className="btn" onClick={() => setEditingGame(selected)}><Icon.Edit /> 编辑游戏</button>
            </div>

            <div className="section-label">时间线</div>
            <div className="timeline">
              {shown.length === 0 && <div className="empty-tl">{selected.save_paths.length === 0
                ? "尚未绑定本机存档目录。"
                : "还没有快照。点击「创建快照」保存当前存档状态。"}</div>}
              {shown.map((s) => {
                const cloudBusy = cloudUploadingId === s.id;
                const cloudUploaded = s.cloud_status === "uploaded" || s.cloud_status === "downloaded";
                const cloudFailed = s.cloud_status === "error";
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
                <div key={s.id} className="snap"
                  onClick={() => setDrawerSnap(s)}>
                  <div className="snap-main">
                    <div className="snap-time">{s.created_at}</div>
                    <div className="snap-note">
                      {s.note || "未命名快照"}
                      {s.locked && <span className="badge lock"><Icon.Lock size={12} /> 已锁定</span>}
                    </div>
                    <div className="snap-info">
                      {s.file_count} 个文件<span className="sep">·</span>{formatSize(s.total_size)}
                      <span className="sep">·</span>{REASON_LABEL[s.reason]}
                    </div>
                  </div>
                  <div className="snap-actions" onClick={(e) => e.stopPropagation()}>
                    <button className="btn sm" title={selected.save_paths.length === 0 ? "请先绑定本机存档目录" : "恢复"}
                      disabled={selected.save_paths.length === 0} onClick={() => setRestoreSnap(s)}><Icon.RotateCcw /> 恢复</button>
                    <button
                      className={`btn sm cloud-upload ${cloudUploaded ? "is-uploaded" : ""} ${cloudFailed ? "is-error" : ""}`}
                      title={cloudTitle}
                      aria-label={cloudTitle}
                      aria-busy={cloudBusy}
                      disabled={cloudUploadingId !== null || cloudUploaded}
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

      {showAdd && <AddGameDialog onClose={() => setShowAdd(false)}
        onCreated={(g) => { setShowAdd(false); setSelectedId(g.id); loadGames(); }} />}


      {showCloud && <CloudSnapshotsDialog onClose={() => setShowCloud(false)}
        onReceived={async (gameId) => {
          await loadGames();
          setSelectedId(gameId);
          await loadSnapshots(gameId);
        }} />}

      {editingGame && <EditGameDialog game={editingGame}
        onClose={() => setEditingGame(null)}
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

      {bindingGame && <BindSavePathDialog game={bindingGame}
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
                <div>将删除快照「{deleteSnap.note || "未命名快照"}」（{deleteSnap.created_at}）。<br />删除后该版本将无法恢复。</div>
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

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="empty">
      <span className="ic"><Icon.History size={48} /></span>
      <h2>还没有添加游戏</h2>
      <p>添加一个游戏后，就可以为它创建存档快照和时间线。</p>
      <button className="btn primary" onClick={onAdd}><Icon.Plus /> 添加游戏</button>
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
