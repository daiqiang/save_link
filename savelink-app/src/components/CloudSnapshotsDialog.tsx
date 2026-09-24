import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../lib/icons";
import { formatSize, formatTimestamp, REASON_LABEL } from "../lib/format";
import * as api from "../lib/api";
import type { CloudSnapshot, Game } from "../lib/types";
import { AssociateCloudGameDialog } from "./AssociateCloudGameDialog";
import { useToast } from "./Toast";

interface Props {
  games: Game[];
  onClose: () => void;
  onReceived: (gameId: string) => Promise<void>;
}

interface CloudGameGroup {
  id: string;
  name: string;
  localGameId: string | null;
  isPrimary: boolean;
  historicalSourceCount: number;
  snapshots: CloudSnapshot[];
}

interface AssociationRequest {
  game: CloudGameGroup;
  pendingSnapshot: CloudSnapshot | null;
}

export function CloudSnapshotsDialog({ games: localGames, onClose, onReceived }: Props) {
  const toast = useToast();
  const [connected, setConnected] = useState<boolean | null>(null);
  const [snapshots, setSnapshots] = useState<CloudSnapshot[]>(() => api.getCachedBaiduSnapshots());
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [receivingId, setReceivingId] = useState<string | null>(null);
  const [association, setAssociation] = useState<AssociationRequest | null>(null);
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const status = await api.getBaiduConnectionStatus();
      if (!mounted.current) return;
      setConnected(status.connected);
      if (!status.connected) {
        setSnapshots([]);
        return;
      }
      const discovered = await api.discoverBaiduSnapshots();
      if (mounted.current) setSnapshots(discovered);
    } catch (error) {
      const status = await api.getBaiduConnectionStatus().catch(() => null);
      if (mounted.current) {
        if (status) setConnected(status.connected);
        setLoadError(String(error));
        toast(String(error), "err");
      }
    } finally {
      if (mounted.current) setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    mounted.current = true;
    refresh();
    return () => { mounted.current = false; };
  }, [refresh]);

  const games = useMemo(() => {
    const grouped = new Map<string, CloudGameGroup>();
    for (const snapshot of snapshots) {
      const game = grouped.get(snapshot.cloud_game_id) ?? {
        id: snapshot.cloud_game_id,
        name: snapshot.game_name,
        localGameId: snapshot.local_game_id,
        isPrimary: snapshot.is_primary,
        historicalSourceCount: Math.max(1, snapshot.source_count),
        snapshots: [],
      };
      if (snapshot.local_game_id) game.localGameId = snapshot.local_game_id;
      if (snapshot.is_primary) game.isPrimary = true;
      game.historicalSourceCount = Math.max(game.historicalSourceCount, snapshot.source_count);
      game.snapshots.push(snapshot);
      grouped.set(snapshot.cloud_game_id, game);
    }
    return Array.from(grouped.values());
  }, [snapshots]);

  async function connect() {
    setConnecting(true);
    try {
      toast("请在浏览器中完成百度网盘授权", "warn");
      const status = await api.connectBaidu();
      if (!status.connected) throw new Error("百度网盘授权未完成");
      await refresh();
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setConnecting(false);
    }
  }

  async function receiveDirect(snapshot: CloudSnapshot) {
    if (receivingId) return;
    setReceivingId(snapshot.snapshot_id);
    try {
      const result = await api.receiveBaiduSnapshot(snapshot.snapshot_id);
      setSnapshots((current) => current.map((item) => item.snapshot_id === snapshot.snapshot_id
        ? { ...item, cloud_status: "downloaded", last_error_code: null }
        : item));
      await onReceived(result.game_id);
      toast(result.outcome === "already_present" ? "这条快照已在本机" : "云端快照已下载到本机仓库", "ok");
    } catch (error) {
      toast(String(error), "err");
      await refresh();
    } finally {
      setReceivingId(null);
    }
  }

  function receive(snapshot: CloudSnapshot, game: CloudGameGroup) {
    const sameNameCandidate = localGames.some((localGame) =>
      localGame.emulator === null
      && (localGame.save_paths.length > 0 || localGame.launch_kind !== null)
      && localGame.name.trim().toLocaleLowerCase() === game.name.trim().toLocaleLowerCase()
    );
    if (!game.localGameId && sameNameCandidate) {
      setAssociation({ game, pendingSnapshot: snapshot });
      return;
    }
    void receiveDirect(snapshot);
  }

  async function associated(gameId: string) {
    const pending = association?.pendingSnapshot ?? null;
    setAssociation(null);
    await refresh();
    if (pending) {
      await receiveDirect(pending);
    } else {
      await onReceived(gameId);
    }
  }

  return (
    <div className="overlay" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="modal cloud-modal">
        <div className="modal-head">
          <h3>云端存档</h3>
          <div className="cloud-head-actions">
            {connected && <button className="iconbtn" title="刷新" onClick={refresh} disabled={loading || receivingId !== null}>
              <Icon.RotateCcw />
            </button>}
            <button className="iconbtn" title="关闭" onClick={onClose}><Icon.Close /></button>
          </div>
        </div>
        <div className="modal-body cloud-body">
          {loading && games.length === 0 && <div className="cloud-empty"><span className="spin"><Icon.RotateCcw size={22} /></span><span>正在读取百度网盘</span></div>}
          {!loading && connected === false && (
            <div className="cloud-empty">
              <Icon.CloudUpload size={34} />
              <strong>百度网盘未连接</strong>
              <button className="btn primary" onClick={connect} disabled={connecting}>
                {connecting ? <><span className="spin"><Icon.RotateCcw /></span> 等待授权</> : "连接百度网盘"}
              </button>
            </div>
          )}
          {!loading && connected && loadError && games.length === 0 && (
            <div className="cloud-empty">
              <strong>读取云端存档失败</strong>
              <span>{loadError}</span>
              <button className="btn" onClick={refresh}>重试</button>
            </div>
          )}
          {!loading && connected && !loadError && games.length === 0 && <div className="cloud-empty"><span>云端还没有 SaveLink 快照</span></div>}
          {connected && games.map((game) => (
            <section className="cloud-game" key={game.id}>
              <div className="cloud-game-head">
                <span className="game-cover">{game.name[0] ?? "游"}</span>
                <div className="cloud-game-title">
                  <strong>{game.name}</strong>
                  <span>{game.snapshots.length} 个快照{game.localGameId
                    ? ` · 已关联至 ${localGames.find((item) => item.id === game.localGameId)?.name ?? "本机游戏"} · ${game.isPrimary ? "主云分组" : "历史云分组"}`
                    : ""}</span>
                </div>
                {localGames.some((localGame) => localGame.emulator === null
                  && (localGame.save_paths.length > 0 || localGame.launch_kind !== null))
                  && (!game.localGameId || !game.isPrimary || localGames.some((localGame) =>
                    localGame.id === game.localGameId
                    && localGame.save_paths.length === 0
                    && localGame.launch_kind === null)) && (
                  <button className="btn sm cloud-associate" onClick={() => setAssociation({ game, pendingSnapshot: null })}
                    disabled={receivingId !== null || loading}><Icon.Gamepad /> {game.localGameId && !game.isPrimary
                      ? "设为主云分组"
                      : "关联到已有游戏"}</button>
                )}
              </div>
              <div className="cloud-snapshot-list">
                {game.snapshots.map((snapshot) => {
                  const available = snapshot.cloud_status === "uploaded" || snapshot.cloud_status === "downloaded";
                  const receiving = receivingId === snapshot.snapshot_id;
                  return (
                    <div className="cloud-snapshot-row" key={snapshot.snapshot_id}>
                      <div className="cloud-snapshot-main">
                        <strong>{snapshot.note || "未命名快照"}</strong>
                        <span>{formatTimestamp(snapshot.created_at)} · {snapshot.file_count} 个文件 · {formatSize(snapshot.total_size)}
                          {snapshot.source_count > 1 ? ` · ${snapshot.source_count} 个存档目录` : ""} · {REASON_LABEL[snapshot.reason]}</span>
                      </div>
                      {available ? (
                        <span className="cloud-state ok"><Icon.Check /> 已在本机</span>
                      ) : (
                        <button className="btn sm" onClick={() => receive(snapshot, game)} disabled={receivingId !== null}>
                          {receiving ? <><span className="spin"><Icon.RotateCcw /></span> 下载中</> : <><Icon.Download /> 下载</>}
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            </section>
          ))}
        </div>
      </div>
      {association && (
        <AssociateCloudGameDialog
          cloudGameId={association.game.id}
          cloudGameName={association.game.name}
          historicalSourceCount={association.game.historicalSourceCount}
          games={localGames}
          allowDownloadAsNew={association.pendingSnapshot !== null && association.game.localGameId === null}
          onClose={() => setAssociation(null)}
          onAssociated={associated}
          onDownloadAsNew={association.pendingSnapshot ? async () => {
            const pending = association.pendingSnapshot;
            if (!pending) return;
            setAssociation(null);
            await receiveDirect(pending);
          } : undefined}
        />
      )}
    </div>
  );
}
