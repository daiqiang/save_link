import { useState } from "react";
import { Icon } from "../lib/icons";
import { formatSize, formatTimestamp, REASON_LABEL } from "../lib/format";
import * as api from "../lib/api";
import { useToast } from "./Toast";
import type { Game, Snapshot } from "../lib/types";

type PendingAction = "close" | "restore" | "delete";

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
  const [savedNote, setSavedNote] = useState(snapshot.note ?? "");
  const [savingNote, setSavingNote] = useState(false);
  const [noteSaveFailed, setNoteSaveFailed] = useState(false);
  const [pendingAction, setPendingAction] = useState<PendingAction | null>(null);
  // 用本地 locked 状态：toggle 后立刻同步抽屉内的按钮/状态/删除禁用，
  // 否则抽屉读的是打开时的 snapshot prop（refresh 不会更新它），按钮文案会和时间线不一致。
  const [locked, setLocked] = useState(snapshot.locked);
  const [pendingReorganization, setPendingReorganization] = useState(snapshot.pending_reorganization);
  const noteDirty = note !== savedNote;

  async function saveNote() {
    if (!noteDirty || savingNote) return;
    const normalized = note.trim();
    setSavingNote(true);
    setNoteSaveFailed(false);
    try {
      await api.updateSnapshotMeta(snapshot.id, normalized, null);
      setNote(normalized);
      setSavedNote(normalized);
      onChanged();
      toast("备注已保存", "ok");
    } catch (error) {
      setNoteSaveFailed(true);
      toast(`备注保存失败：${String(error)}`, "err");
    } finally {
      setSavingNote(false);
    }
  }
  async function toggleLock() {
    const next = !locked;
    await api.updateSnapshotMeta(snapshot.id, null, next);
    setLocked(next);
    setPendingReorganization(snapshot.display_zone !== (next ? "locked" : "normal"));
    onChanged();
    toast(next ? "快照已锁定，不会被自动清理" : "已取消锁定", "ok");
  }

  function executeAction(action: PendingAction) {
    if (action === "restore") onRestore(snapshot);
    else if (action === "delete") onDelete(snapshot);
    else onClose();
  }

  function requestAction(action: PendingAction) {
    if (noteDirty) setPendingAction(action);
    else executeAction(action);
  }

  return (
    <>
      <div className="drawer-mask" onClick={() => requestAction("close")} />
      <div className="drawer">
        <div className="drawer-head">
          <h3>快照详情</h3>
          <button className="iconbtn" title="关闭" onClick={() => requestAction("close")}><Icon.Close /></button>
        </div>
        <div className="drawer-body">
          <div className="drawer-title">{game.name}</div>
          <div className="drawer-time">{formatTimestamp(snapshot.created_at)}</div>

          <div className="field">
            <div className="note-label-row">
              <label>备注</label>
              <span className={`note-state ${noteSaveFailed ? "error" : noteDirty ? "dirty" : "saved"}`} aria-live="polite">
                {noteSaveFailed
                  ? <><Icon.Alert size={13} /> 保存失败，请重试</>
                  : noteDirty
                    ? <><span className="note-state-dot" /> 有未保存修改</>
                    : <><Icon.Check size={13} /> 已保存</>}
              </span>
            </div>
            <textarea className="input" value={note} onChange={(event) => {
              setNote(event.target.value);
              setNoteSaveFailed(false);
            }} />
            <div className="note-actions">
              <button className="btn sm primary note-save" onClick={saveNote} disabled={!noteDirty || savingNote}>
                <Icon.Save size={14} /> 保存备注
              </button>
            </div>
          </div>

          <div className="meta-list">
            <div className="mrow"><span className="mk">文件数量</span><span>{snapshot.file_count}</span></div>
            <div className="mrow"><span className="mk">总大小</span><span>{formatSize(snapshot.total_size)}</span></div>
            <div className="mrow"><span className="mk">创建原因</span><span>{REASON_LABEL[snapshot.reason]}</span></div>
            <div className="mrow"><span className="mk">状态</span><span>{pendingReorganization
              ? (locked ? "已锁定，待整理" : "已解锁，待整理")
              : locked ? "已锁定" : "正常"}</span></div>
          </div>

          <div style={{ marginTop: 16 }}>
            <label style={{ display: "block", fontSize: 12.5, fontWeight: 600, color: "var(--color-text-2)", marginBottom: 6 }}>存档目录</label>
            <div className="target-box">
              <div className="path-stack">
                {game.save_paths.map((path, index) => <span className="path-mono" key={index}>{path}</span>)}
              </div>
            </div>
          </div>
        </div>
        <div className="drawer-foot">
          <button className="btn primary" disabled={game.configuration_state !== "configured"}
            title={game.configuration_state !== "configured" ? "请先设置本机存档目录" : "恢复这个版本"}
            onClick={() => requestAction("restore")}>
            <Icon.RotateCcw /> 恢复这个版本
          </button>
          <div style={{ display: "flex", gap: 10 }}>
            <button className="btn" style={{ flex: 1 }} onClick={toggleLock}>
              {locked ? <><Icon.Unlock /> 取消锁定</> : <><Icon.Lock /> 锁定</>}
            </button>
            <button className="btn danger" style={{ flex: 1 }} disabled={locked}
              onClick={() => locked ? toast("锁定快照不能删除，请先取消锁定", "warn") : requestAction("delete")}>
              <Icon.Trash /> 删除
            </button>
          </div>
        </div>
      </div>
      {pendingAction && (
        <div className="overlay" onMouseDown={(event) => event.target === event.currentTarget && setPendingAction(null)}>
          <div className="modal narrow">
            <div className="modal-head">
              <h3>放弃备注修改？</h3>
              <button className="iconbtn" title="关闭" onClick={() => setPendingAction(null)}><Icon.Close /></button>
            </div>
            <div className="modal-body">
              <div className="callout warn">
                <span className="ic"><Icon.Alert /></span>
                <div>当前备注尚未保存。放弃后，本次修改将不会保留。</div>
              </div>
            </div>
            <div className="modal-foot">
              <button className="btn" onClick={() => setPendingAction(null)}>继续编辑</button>
              <button className="btn danger" onClick={() => executeAction(pendingAction)}>放弃修改</button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
