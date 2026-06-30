import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Icon } from "../lib/icons";
import { formatSize } from "../lib/format";
import * as api from "../lib/api";
import { useToast } from "./Toast";
import type { Game } from "../lib/types";

interface Props {
  game: Game;
  onClose: () => void;
  onSaved: (game: Game) => void;
  onDeleted: (game: Game) => void;
}

export function EditGameDialog({ game, onClose, onSaved, onDeleted }: Props) {
  const toast = useToast();
  const [name, setName] = useState(game.name);
  const [path, setPath] = useState(game.save_paths[0] ?? "");
  const [scan, setScan] = useState<{ state: "idle" | "loading" | "done" | "err"; text: string }>({
    state: "idle",
    text: "目录未检测。",
  });
  const [saving, setSaving] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  async function pickDir() {
    const picked = await open({ directory: true, multiple: false, title: "选择新的存档目录" });
    if (typeof picked === "string") {
      setPath(picked);
      setScan({ state: "idle", text: "目录未检测。" });
    }
  }

  async function testRead() {
    if (!path.trim()) {
      setScan({ state: "err", text: "请先选择一个存档目录。" });
      return;
    }
    setScan({ state: "loading", text: "正在读取目录…" });
    try {
      const r = await api.scanPath(path.trim());
      setScan({ state: "done", text: `已检测到：${r.file_count} 个文件，${formatSize(r.total_size)}` });
    } catch {
      setScan({ state: "err", text: "无法访问该目录，请重新选择。" });
    }
  }

  async function save() {
    if (!name.trim()) return toast("请填写游戏名称", "err");
    if (!path.trim()) return toast("请至少选择一个存档目录", "err");
    setSaving(true);
    try {
      const updated = await api.updateGame(game.id, name.trim(), [path.trim()]);
      toast("游戏信息已保存", "ok");
      onSaved(updated);
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setSaving(false);
    }
  }

  async function deleteGame() {
    setDeleting(true);
    try {
      await api.deleteGame(game.id);
      toast("游戏已移除", "ok");
      onDeleted(game);
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setDeleting(false);
    }
  }

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal">
        <div className="modal-head">
          <h3>编辑游戏</h3>
          <button className="iconbtn" onClick={onClose}><Icon.Close /></button>
        </div>
        <div className="modal-body">
          <div className="field">
            <label>游戏名称</label>
            <input className="input" value={name} autoFocus
              onChange={(e) => setName(e.target.value)} placeholder="例如：艾尔登法环" />
          </div>
          <div className="field">
            <label>存档目录</label>
            <input className="input path-mono" value={path} onChange={(e) => {
              setPath(e.target.value);
              setScan({ state: "idle", text: "目录未检测。" });
            }} />
            <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
              <button className="btn sm" onClick={pickDir}><Icon.Folder /> 选择目录</button>
              <button className="btn sm" onClick={testRead}><Icon.Camera /> 测试读取</button>
            </div>
            <div className={`hint ${scan.state === "done" ? "ok" : scan.state === "err" ? "err" : "muted"}`}>
              {scan.state === "loading" && <span className="spin"><Icon.RotateCcw /></span>}
              {scan.state === "done" && <Icon.CheckCircle />}
              {scan.state === "err" && <Icon.Alert />}
              <span>{scan.text}</span>
            </div>
          </div>
        </div>
        <div className="modal-foot">
          <button className="btn danger" onClick={() => setConfirmingDelete(true)} disabled={saving}>
            <Icon.Trash /> 移除游戏
          </button>
          <div className="spacer" />
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn primary" onClick={save} disabled={saving}>
            {saving ? "保存中…" : "保存修改"}
          </button>
        </div>
      </div>
      {confirmingDelete && (
        <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && setConfirmingDelete(false)}>
          <div className="modal narrow">
            <div className="modal-head">
              <h3>移除游戏？</h3>
              <button className="iconbtn" onClick={() => setConfirmingDelete(false)}><Icon.Close /></button>
            </div>
            <div className="modal-body">
              <div className="callout warn">
                <span className="ic"><Icon.Alert /></span>
                <div>
                  将从 SaveLink 中移除「{game.name}」及其全部快照记录和备份文件。<br />
                  不会删除真实存档目录。
                </div>
              </div>
              <div className="target-box">
                <span className="path-mono">{game.save_paths[0] || "未设置存档目录"}</span>
              </div>
            </div>
            <div className="modal-foot">
              <button className="btn" onClick={() => setConfirmingDelete(false)} disabled={deleting}>取消</button>
              <button className="btn danger" onClick={deleteGame} disabled={deleting}>
                {deleting ? "移除中…" : <><Icon.Trash /> 确认移除</>}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
