import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Icon } from "../lib/icons";
import { formatSize } from "../lib/format";
import * as api from "../lib/api";
import type { Game, ScanResult } from "../lib/types";
import { useToast } from "./Toast";

interface Props {
  game: Game;
  onClose: () => void;
  onBound: (game: Game) => void;
}

type ScanState =
  | { status: "idle" | "loading"; path: string }
  | { status: "done"; path: string; result: ScanResult }
  | { status: "error"; path: string };

export function BindSavePathDialog({ game, onClose, onBound }: Props) {
  const toast = useToast();
  const [path, setPath] = useState("");
  const [scan, setScan] = useState<ScanState>({ status: "idle", path: "" });
  const [saving, setSaving] = useState(false);

  const normalizedPath = path.trim();
  const scanIsCurrent = scan.status === "done" && scan.path === normalizedPath;

  async function inspectPath(candidate = normalizedPath) {
    const value = candidate.trim();
    if (!value) {
      setScan({ status: "error", path: value });
      return;
    }
    setScan({ status: "loading", path: value });
    try {
      const result = await api.scanPath(value);
      setScan({ status: "done", path: value, result });
    } catch {
      setScan({ status: "error", path: value });
    }
  }

  async function pickDir() {
    const picked = await open({
      directory: true,
      multiple: false,
      title: `选择 ${game.name} 的存档目录`,
    });
    if (typeof picked !== "string") return;
    setPath(picked);
    await inspectPath(picked);
  }

  async function bind() {
    if (!scanIsCurrent) return toast("请先确认当前目录可以正常读取", "warn");
    setSaving(true);
    try {
      const updated = await api.updateGame(game.id, game.name, [normalizedPath]);
      toast("本机存档目录已绑定", "ok");
      onBound(updated);
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="overlay" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="modal">
        <div className="modal-head">
          <h3>绑定存档目录</h3>
          <button className="iconbtn" title="关闭" onClick={onClose}><Icon.Close /></button>
        </div>
        <div className="modal-body">
          <div className="field">
            <label>游戏</label>
            <div className="target-box"><strong>{game.name}</strong></div>
          </div>
          <div className="field">
            <label>本机存档目录</label>
            <input
              className="input path-mono"
              value={path}
              autoFocus
              placeholder="选择或输入这台电脑上的存档目录"
              onChange={(event) => {
                setPath(event.target.value);
                setScan({ status: "idle", path: "" });
              }}
            />
            <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
              <button className="btn sm" onClick={pickDir} disabled={saving || scan.status === "loading"}>
                <Icon.Folder /> 选择目录
              </button>
              <button className="btn sm" onClick={() => inspectPath()} disabled={saving || scan.status === "loading" || !normalizedPath}>
                {scan.status === "loading"
                  ? <><span className="spin"><Icon.RotateCcw /></span> 正在读取</>
                  : <><Icon.Camera /> 测试读取</>}
              </button>
            </div>
            {scan.status === "idle" && (
              <div className="hint muted"><span>选择目录后将自动检查文件数量和大小。</span></div>
            )}
            {scan.status === "error" && (
              <div className="hint err"><Icon.Alert /><span>无法读取该目录，请重新选择。</span></div>
            )}
            {scan.status === "done" && (
              <div className="hint ok">
                <Icon.CheckCircle />
                <span>{scan.result.file_count === 0
                  ? "目录可读取，当前为空"
                  : `目录可读取：${scan.result.file_count} 个文件，${formatSize(scan.result.total_size)}`}</span>
              </div>
            )}
          </div>
          {scanIsCurrent && (
            <div className={`callout ${scan.result.file_count > 0 ? "warn" : "info"}`}>
              <span className="ic">{scan.result.file_count > 0 ? <Icon.Shield /> : <Icon.Folder />}</span>
              <div>{scan.result.file_count > 0
                ? "绑定不会修改目录中的文件。以后恢复云端快照前，SaveLink 会先自动备份当前存档。"
                : "绑定后不会自动恢复。你可以回到时间线，另行选择需要恢复的云端快照。"}</div>
            </div>
          )}
        </div>
        <div className="modal-foot">
          <button className="btn" onClick={onClose} disabled={saving}>取消</button>
          <button className="btn primary" onClick={bind} disabled={saving || !scanIsCurrent}>
            {saving ? "绑定中…" : <><Icon.Folder /> 确认绑定</>}
          </button>
        </div>
      </div>
    </div>
  );
}
