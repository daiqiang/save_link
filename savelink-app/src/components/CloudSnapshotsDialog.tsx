import { useCallback, useEffect, useMemo, useState } from "react";
import { Icon } from "../lib/icons";
import { formatSize, formatTimestamp, REASON_LABEL } from "../lib/format";
import * as api from "../lib/api";
import type { CloudSnapshot } from "../lib/types";
import { useToast } from "./Toast";

interface Props {
  onClose: () => void;
  onReceived: (gameId: string) => Promise<void>;
}

export function CloudSnapshotsDialog({ onClose, onReceived }: Props) {
  const toast = useToast();
  const [connected, setConnected] = useState<boolean | null>(null);
  const [snapshots, setSnapshots] = useState<CloudSnapshot[]>([]);
  const [loading, setLoading] = useState(true);
  const [connecting, setConnecting] = useState(false);
  const [receivingId, setReceivingId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const status = await api.getBaiduConnectionStatus();
      setConnected(status.connected);
      if (!status.connected) {
        setSnapshots([]);
        return;
      }
      setSnapshots(await api.discoverBaiduSnapshots());
    } catch (error) {
      const status = await api.getBaiduConnectionStatus().catch(() => null);
      if (status) setConnected(status.connected);
      toast(String(error), "err");
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => { refresh(); }, [refresh]);

  const games = useMemo(() => {
    const grouped = new Map<string, { name: string; snapshots: CloudSnapshot[] }>();
    for (const snapshot of snapshots) {
      const game = grouped.get(snapshot.cloud_game_id) ?? { name: snapshot.game_name, snapshots: [] };
      game.snapshots.push(snapshot);
      grouped.set(snapshot.cloud_game_id, game);
    }
    return Array.from(grouped, ([id, value]) => ({ id, ...value }));
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

  async function receive(snapshot: CloudSnapshot) {
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
          {loading && <div className="cloud-empty"><span className="spin"><Icon.RotateCcw size={22} /></span><span>正在读取百度网盘</span></div>}
          {!loading && connected === false && (
            <div className="cloud-empty">
              <Icon.CloudUpload size={34} />
              <strong>百度网盘未连接</strong>
              <button className="btn primary" onClick={connect} disabled={connecting}>
                {connecting ? <><span className="spin"><Icon.RotateCcw /></span> 等待授权</> : "连接百度网盘"}
              </button>
            </div>
          )}
          {!loading && connected && games.length === 0 && <div className="cloud-empty"><span>云端还没有 SaveLink 快照</span></div>}
          {!loading && connected && games.map((game) => (
            <section className="cloud-game" key={game.id}>
              <div className="cloud-game-head">
                <span className="game-cover">{game.name[0] ?? "游"}</span>
                <div><strong>{game.name}</strong><span>{game.snapshots.length} 个快照</span></div>
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
                        <button className="btn sm" onClick={() => receive(snapshot)} disabled={receivingId !== null}>
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
    </div>
  );
}
