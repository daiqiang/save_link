import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Icon } from "../lib/icons";
import { formatSize } from "../lib/format";
import * as api from "../lib/api";
import type { Game, ScanResult } from "../lib/types";
import { useToast } from "./Toast";

interface Props {
  game: Game;
  sourceCount: number;
  onClose: () => void;
  onBound: (game: Game) => void;
}

type ScanState =
  | { status: "idle" | "loading"; path: string }
  | { status: "done"; path: string; result: ScanResult }
  | { status: "error"; path: string };

export function BindSavePathDialog({ game, sourceCount, onClose, onBound }: Props) {
  const toast = useToast();
  const requiredCount = Math.max(1, sourceCount);
  const [paths, setPaths] = useState(() => Array.from({ length: requiredCount }, () => ""));
  const [scans, setScans] = useState<ScanState[]>(() =>
    Array.from({ length: requiredCount }, () => ({ status: "idle", path: "" })),
  );
  const [saving, setSaving] = useState(false);

  const normalizedPaths = paths.map((path) => path.trim());
  const allScansCurrent = scans.every((scan, index) =>
    scan.status === "done" && scan.path === normalizedPaths[index],
  );
  const totalFiles = scans.reduce((total, scan) => total + (scan.status === "done" ? scan.result.file_count : 0), 0);

  function updatePath(index: number, value: string) {
    setPaths((current) => current.map((path, currentIndex) => currentIndex === index ? value : path));
    setScans((current) => current.map((scan, currentIndex) => currentIndex === index
      ? { status: "idle", path: "" }
      : scan));
  }

  async function inspectPath(index: number, candidate = normalizedPaths[index]) {
    const value = candidate.trim();
    if (!value) {
      setScans((current) => current.map((scan, currentIndex) => currentIndex === index
        ? { status: "error", path: value }
        : scan));
      return;
    }
    setScans((current) => current.map((scan, currentIndex) => currentIndex === index
      ? { status: "loading", path: value }
      : scan));
    try {
      const result = await api.scanPath(value);
      setScans((current) => current.map((scan, currentIndex) => currentIndex === index
        ? { status: "done", path: value, result }
        : scan));
    } catch {
      setScans((current) => current.map((scan, currentIndex) => currentIndex === index
        ? { status: "error", path: value }
        : scan));
    }
  }

  async function pickDir(index: number) {
    const picked = await open({
      directory: true,
      multiple: false,
      title: `选择 ${game.name} 的存档目录 ${index + 1}`,
    });
    if (typeof picked !== "string") return;
    updatePath(index, picked);
    await inspectPath(index, picked);
  }

  async function bind() {
    if (!allScansCurrent) return toast("请先确认全部目录都可以正常读取", "warn");
    setSaving(true);
    try {
      const updated = await api.updateGame(game.id, game.name, normalizedPaths);
      toast(requiredCount > 1 ? `${requiredCount} 个本机存档目录已绑定` : "本机存档目录已绑定", "ok");
      onBound(updated);
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="overlay" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="modal bind-path-modal">
        <div className="modal-head">
          <h3>绑定存档目录</h3>
          <button className="iconbtn" title="关闭" onClick={onClose}><Icon.Close /></button>
        </div>
        <div className="modal-body bind-path-body">
          <div className="field">
            <label>游戏</label>
            <div className="target-box"><strong>{game.name}</strong></div>
          </div>
          <div className="field">
            <label>{requiredCount > 1 ? `本机存档目录（${requiredCount} 个）` : "本机存档目录"}</label>
            <div className="save-path-editor">
              {paths.map((path, index) => {
                const scan = scans[index];
                return (
                  <div className="save-path-edit" key={index}>
                    {requiredCount > 1 && <div className="save-path-edit-label">目录 {index + 1}</div>}
                    <div className="save-path-edit-row">
                      <input className="input path-mono" value={path} autoFocus={index === 0}
                        placeholder="选择或输入这台电脑上的存档目录"
                        onChange={(event) => updatePath(index, event.target.value)} />
                      <button className="iconbtn" title="选择目录" onClick={() => pickDir(index)}
                        disabled={saving || scan.status === "loading"}><Icon.Folder /></button>
                      <button className="iconbtn" title="测试读取" onClick={() => inspectPath(index)}
                        disabled={saving || scan.status === "loading" || !normalizedPaths[index]}>
                        {scan.status === "loading" ? <span className="spin"><Icon.RotateCcw /></span> : <Icon.Camera />}
                      </button>
                    </div>
                    {scan.status === "error" && (
                      <div className="hint err"><Icon.Alert /><span>无法读取该目录，请重新选择。</span></div>
                    )}
                    {scan.status === "done" && (
                      <div className="hint ok"><Icon.CheckCircle /><span>{scan.result.file_count === 0
                        ? "目录可读取，当前为空"
                        : `目录可读取：${scan.result.file_count} 个文件，${formatSize(scan.result.total_size)}`}</span></div>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
          {allScansCurrent && (
            <div className={`callout ${totalFiles > 0 ? "warn" : "info"}`}>
              <span className="ic">{totalFiles > 0 ? <Icon.Shield /> : <Icon.Folder />}</span>
              <div>{totalFiles > 0
                ? "绑定不会修改目录中的文件。恢复云端快照前，如需保留当前进度，请先手动创建快照。"
                : "绑定后不会自动恢复。你可以回到时间线，另行选择需要恢复的云端快照。"}</div>
            </div>
          )}
        </div>
        <div className="modal-foot">
          <button className="btn" onClick={onClose} disabled={saving}>取消</button>
          <button className="btn primary" onClick={bind} disabled={saving || !allScansCurrent}>
            {saving ? "绑定中…" : <><Icon.Folder /> 确认绑定</>}
          </button>
        </div>
      </div>
    </div>
  );
}
