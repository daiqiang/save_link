import { useState } from "react";
import { Icon } from "../lib/icons";
import { formatSize, REASON_LABEL } from "../lib/format";
import * as api from "../lib/api";
import { useToast } from "./Toast";
import type { Game, Snapshot } from "../lib/types";

interface Props {
  game: Game;
  snapshot: Snapshot;
  onClose: () => void;
  onChanged: () => void;
  onRestore: (s: Snapshot) => void;
  onDelete: (s: Snapshot) => void;
}

export function SnapshotDrawer({ game, snapshot, onClose, onChanged, onRestore, onDelete }: Props) {
  const toast = useToast();
  const [note, setNote] = useState(snapshot.note ?? "");

  async function saveNote() {
    await api.updateSnapshotMeta(snapshot.id, note.trim(), null);
    onChanged();
    toast("备注已保存", "ok");
  }
  async function toggleLock() {
    await api.updateSnapshotMeta(snapshot.id, null, !snapshot.locked);
    onChanged();
    toast(snapshot.locked ? "已取消锁定" : "快照已锁定，不会被自动清理", "ok");
  }

  return (
    <>
      <div className="drawer-mask" onClick={onClose} />
      <div className="drawer">
        <div className="drawer-head">
          <h3>快照详情</h3>
          <button className="iconbtn" onClick={onClose}><Icon.Close /></button>
        </div>
        <div className="drawer-body">
          <div className="drawer-title">{game.name}</div>
          <div className="drawer-time">{snapshot.created_at}</div>

          <div className="field">
            <label>备注</label>
            <textarea className="input" value={note} onChange={(e) => setNote(e.target.value)} />
            <div style={{ marginTop: 8 }}>
              <button className="btn sm" onClick={saveNote}>保存备注</button>
            </div>
          </div>

          <div className="meta-list">
            <div className="mrow"><span className="mk">文件数量</span><span>{snapshot.file_count}</span></div>
            <div className="mrow"><span className="mk">总大小</span><span>{formatSize(snapshot.total_size)}</span></div>
            <div className="mrow"><span className="mk">创建原因</span><span>{REASON_LABEL[snapshot.reason]}</span></div>
            <div className="mrow"><span className="mk">状态</span><span>{snapshot.locked ? "已锁定" : "正常"}</span></div>
          </div>

          <div style={{ marginTop: 16 }}>
            <label style={{ display: "block", fontSize: 12.5, fontWeight: 600, color: "var(--color-text-2)", marginBottom: 6 }}>存档目录</label>
            <div className="target-box"><span className="path-mono">{game.save_paths[0]}</span></div>
          </div>
        </div>
        <div className="drawer-foot">
          <button className="btn primary" onClick={() => onRestore(snapshot)}>
            <Icon.RotateCcw /> 恢复这个版本
          </button>
          <div style={{ display: "flex", gap: 10 }}>
            <button className="btn" style={{ flex: 1 }} onClick={toggleLock}>
              {snapshot.locked ? <><Icon.Unlock /> 取消锁定</> : <><Icon.Lock /> 锁定</>}
            </button>
            <button className="btn danger" style={{ flex: 1 }} disabled={snapshot.locked}
              onClick={() => snapshot.locked ? toast("锁定快照不能删除，请先取消锁定", "warn") : onDelete(snapshot)}>
              <Icon.Trash /> 删除
            </button>
          </div>
        </div>
      </div>
    </>
  );
}
