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
}

export function EditGameDialog({ game, onClose, onSaved }: Props) {
  const toast = useToast();
  const [name, setName] = useState(game.name);
  const [path, setPath] = useState(game.save_paths[0] ?? "");
  const [scan, setScan] = useState<{ state: "idle" | "loading" | "done" | "err"; text: string }>({
    state: "idle",
    text: "目录未检测。",
  });
  const [saving, setSaving] = useState(false);

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
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn primary" onClick={save} disabled={saving}>
            {saving ? "保存中…" : "保存修改"}
          </button>
        </div>
      </div>
    </div>
  );
}
