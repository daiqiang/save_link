import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Icon } from "../lib/icons";
import { formatSize } from "../lib/format";
import * as api from "../lib/api";
import { useToast } from "./Toast";
import type { Game } from "../lib/types";

interface Props {
  onClose: () => void;
  onCreated: (game: Game) => void;
}

export function AddGameDialog({ onClose, onCreated }: Props) {
  const toast = useToast();
  const [name, setName] = useState("");
  const [path, setPath] = useState("C:\\Users\\daiq\\AppData\\Roaming\\EldenRing\\76561198...");
  const [scan, setScan] = useState<{ state: "idle" | "loading" | "done"; text: string }>({
    state: "idle",
    text: "点击「测试读取」检测该目录中的存档文件。",
  });
  const [saving, setSaving] = useState(false);

  async function pickDir() {
    const picked = await open({ directory: true, multiple: false, title: "选择存档目录" });
    if (typeof picked === "string") {
      setPath(picked);
      setScan({ state: "idle", text: "点击「测试读取」检测该目录中的存档文件。" });
    }
  }

  async function testRead() {
    setScan({ state: "loading", text: "正在读取目录…" });
    try {
      const r = await api.scanPath(path);
      setScan({ state: "done", text: `已检测到：${r.file_count} 个文件，${formatSize(r.total_size)}` });
    } catch {
      setScan({ state: "idle", text: "无法访问该目录，请重新选择。" });
    }
  }

  async function save() {
    if (!name.trim()) return toast("请填写游戏名称", "err");
    if (!path.trim()) return toast("请至少选择一个存档目录", "err");
    setSaving(true);
    try {
      const g = await api.addGame(name.trim(), [path.trim()]);
      toast("游戏已添加，去创建第一个快照吧", "ok");
      onCreated(g);
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
          <h3>添加游戏</h3>
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
            <input className="input path-mono" value={path} onChange={(e) => setPath(e.target.value)} />
            <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
              <button className="btn sm" onClick={pickDir}><Icon.Folder /> 选择目录</button>
              <button className="btn sm" onClick={testRead}><Icon.Camera /> 测试读取</button>
            </div>
            <div className={`hint ${scan.state === "done" ? "ok" : "muted"}`}>
              {scan.state === "loading" && <span className="spin"><Icon.RotateCcw /></span>}
              {scan.state === "done" && <Icon.CheckCircle />}
              <span>{scan.text}</span>
            </div>
          </div>
          <div className="field">
            <label>快照仓库</label>
            <div className="target-box"><span className="path-mono">使用默认位置：D:\SaveLink\Repository</span></div>
          </div>
        </div>
        <div className="modal-foot">
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn primary" onClick={save} disabled={saving}>
            {saving ? "保存中…" : "保存并创建"}
          </button>
        </div>
      </div>
    </div>
  );
}
