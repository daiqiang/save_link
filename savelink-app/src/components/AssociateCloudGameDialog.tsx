import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Icon } from "../lib/icons";
import { formatSize } from "../lib/format";
import * as api from "../lib/api";
import type { Game, ScanResult } from "../lib/types";
import { useToast } from "./Toast";

interface Props {
  cloudGameId: string;
  cloudGameName: string;
  historicalSourceCount: number;
  games: Game[];
  allowDownloadAsNew: boolean;
  onClose: () => void;
  onAssociated: (gameId: string) => Promise<void>;
  onDownloadAsNew?: () => Promise<void>;
}

type ScanState =
  | { status: "idle" | "loading"; path: string }
  | { status: "done"; path: string; result: ScanResult }
  | { status: "error"; path: string; message: string; missing: boolean };

function readableError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isMissingDirectory(message: string): boolean {
  return /目录不存在|找不到|not found|cannot find/i.test(message);
}

export function AssociateCloudGameDialog({
  cloudGameId,
  cloudGameName,
  historicalSourceCount,
  games,
  allowDownloadAsNew,
  onClose,
  onAssociated,
  onDownloadAsNew,
}: Props) {
  const toast = useToast();
  const candidates = useMemo(() => games.filter((game) =>
    game.emulator === null
    && (game.save_paths.length > 0 || game.launch_kind !== null)
  ), [games]);
  const initialGameId = candidates.find((game) =>
    game.name.trim().toLocaleLowerCase() === cloudGameName.trim().toLocaleLowerCase()
  )?.id ?? candidates[0]?.id ?? "";
  const [selectedGameId, setSelectedGameId] = useState(initialGameId);
  const [paths, setPaths] = useState<string[]>([]);
  const [scans, setScans] = useState<ScanState[]>([]);
  const [saving, setSaving] = useState(false);

  const selectedGame = candidates.find((game) => game.id === selectedGameId) ?? null;
  const pathCount = Math.max(1, selectedGame?.save_paths.length ?? 0);
  const normalizedPaths = paths.map((path) => path.trim());
  const allScansCurrent = paths.length === pathCount
    && scans.length === pathCount
    && scans.every((scan, index) =>
    scan.status === "done" && scan.path === normalizedPaths[index]
  );
  const totalFiles = scans.reduce((total, scan) =>
    total + (scan.status === "done" ? scan.result.file_count : 0), 0);

  useEffect(() => {
    let cancelled = false;
    if (!selectedGame) {
      setPaths([]);
      setScans([]);
      return () => { cancelled = true; };
    }
    const count = Math.max(1, selectedGame.save_paths.length);
    const nextPaths = Array.from({ length: count }, (_, index) => selectedGame.save_paths[index] ?? "");
    setPaths(nextPaths);
    setScans(nextPaths.map((path) => path
      ? { status: "loading", path } as ScanState
      : { status: "idle", path: "" } as ScanState));
    nextPaths.forEach((path, index) => {
      if (!path) return;
      void api.scanPath(path).then(
        (result) => {
          if (!cancelled) setScans((current) => current.map((scan, currentIndex) =>
            currentIndex === index && scan.path === path ? { status: "done", path, result } : scan));
        },
        (error) => {
          const message = readableError(error);
          if (!cancelled) setScans((current) => current.map((scan, currentIndex) =>
            currentIndex === index && scan.path === path
              ? { status: "error", path, message, missing: isMissingDirectory(message) }
              : scan));
        },
      );
    });
    return () => { cancelled = true; };
  }, [pathCount, selectedGame]);

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
        ? { status: "error", path: value, message: "存档目录不能为空", missing: false }
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
    } catch (error) {
      const message = readableError(error);
      setScans((current) => current.map((scan, currentIndex) => currentIndex === index
        ? { status: "error", path: value, message, missing: isMissingDirectory(message) }
        : scan));
    }
  }

  async function pickDir(index: number) {
    const picked = await open({
      directory: true,
      multiple: false,
      title: `选择 ${cloudGameName} 的存档目录 ${index + 1}`,
    });
    if (typeof picked !== "string") return;
    updatePath(index, picked);
    await inspectPath(index, picked);
  }

  async function createDir(index: number) {
    const path = normalizedPaths[index];
    if (!path) return;
    try {
      await api.createSaveDirectory(path);
      await inspectPath(index, path);
      toast("存档目录已创建", "ok");
    } catch (error) {
      toast(readableError(error), "err");
    }
  }

  async function associate() {
    if (!selectedGame || !allScansCurrent) {
      toast("请先确认全部存档目录都可以正常读取", "warn");
      return;
    }
    setSaving(true);
    try {
      const result = await api.associateBaiduCloudGame(
        cloudGameId,
        selectedGame.id,
        normalizedPaths,
      );
      await onAssociated(result.game.id);
      const moved = result.moved_snapshot_count > 0
        ? `，已合并 ${result.moved_snapshot_count} 个本机快照`
        : "";
      toast(`已关联到“${result.game.name}”${moved}`, "ok");
    } catch (error) {
      toast(readableError(error), "err");
    } finally {
      setSaving(false);
    }
  }

  async function downloadAsNew() {
    if (!onDownloadAsNew) return;
    setSaving(true);
    try {
      await onDownloadAsNew();
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="overlay association-overlay" onMouseDown={(event) => event.target === event.currentTarget && !saving && onClose()}>
      <div className="modal associate-cloud-modal">
        <div className="modal-head">
          <h3>关联到已有游戏</h3>
          <button className="iconbtn" title="关闭" onClick={onClose} disabled={saving}><Icon.Close /></button>
        </div>
        <div className="modal-body bind-path-body">
          <div className="field">
            <label>云端游戏</label>
            <div className="target-box"><strong>{cloudGameName}</strong></div>
          </div>
          <div className="field">
            <label>本机游戏</label>
            {candidates.length > 0 ? (
              <select className="input" value={selectedGameId} onChange={(event) => setSelectedGameId(event.target.value)} disabled={saving}>
                {candidates.map((game) => <option value={game.id} key={game.id}>{game.name}</option>)}
              </select>
            ) : (
              <div className="hint err"><Icon.Alert /><span>本机没有可以关联的已管理游戏。</span></div>
            )}
          </div>
          {selectedGame && (
            <div className="field">
              <label>{pathCount > 1 ? `本机存档目录（${pathCount} 个）` : "本机存档目录"}</label>
              <div className="save-path-editor">
                {paths.map((path, index) => {
                  const scan = scans[index];
                  return (
                    <div className="save-path-edit" key={index}>
                      {paths.length > 1 && <div className="save-path-edit-label">目录 {index + 1}</div>}
                      <div className="save-path-edit-row">
                        <input className="input path-mono" value={path} placeholder="选择或输入这台电脑上的存档目录"
                          onChange={(event) => updatePath(index, event.target.value)}
                          onBlur={() => scan?.status === "idle" && normalizedPaths[index] && void inspectPath(index)}
                          disabled={saving} />
                        <button className="iconbtn" title="选择目录" onClick={() => pickDir(index)}
                          disabled={saving || scan?.status === "loading"}><Icon.Folder /></button>
                        <button className="iconbtn" title="测试读取" onClick={() => inspectPath(index)}
                          disabled={saving || scan?.status === "loading" || !normalizedPaths[index]}>
                          {scan?.status === "loading" ? <span className="spin"><Icon.RotateCcw /></span> : <Icon.Camera />}
                        </button>
                      </div>
                      {scan?.status === "error" && (
                        <div className="path-error-row">
                          <div className="hint err"><Icon.Alert /><span>{scan.message}</span></div>
                          {scan.missing && <button className="btn sm" onClick={() => createDir(index)} disabled={saving}>创建目录</button>}
                        </div>
                      )}
                      {scan?.status === "done" && (
                        <div className="hint ok"><Icon.CheckCircle /><span>{scan.result.file_count === 0
                          ? "目录可读取，当前为空"
                          : `目录可读取：${scan.result.file_count} 个文件，${formatSize(scan.result.total_size)}`}</span></div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}
          {selectedGame && historicalSourceCount > pathCount && (
            <div className="callout warn">
              <span className="ic"><Icon.Alert /></span>
              <div>云端仍有曾使用 {historicalSourceCount} 个目录的历史快照。本次按本机当前的 {pathCount} 个目录关联，不会改动旧快照；恢复这些旧快照前，仍需补齐对应目录。</div>
            </div>
          )}
          <div className="callout info">
            <span className="ic"><Icon.Shield /></span>
            <div>关联只整理本机游戏和快照归属，不会立即恢复存档，也不会删除原来的云端分组。</div>
          </div>
          {allScansCurrent && totalFiles > 0 && (
            <div className="callout warn">
              <span className="ic"><Icon.Alert /></span>
              <div>当前目录已有文件。以后恢复云端快照前，请先确认需要保留的本机进度已经创建快照。</div>
            </div>
          )}
        </div>
        <div className="modal-foot association-actions">
          {allowDownloadAsNew && onDownloadAsNew && (
            <button className="btn" onClick={downloadAsNew} disabled={saving}>作为独立游戏下载</button>
          )}
          <span className="association-action-spacer" />
          <button className="btn" onClick={onClose} disabled={saving}>取消</button>
          <button className="btn primary" onClick={associate} disabled={saving || !selectedGame || !allScansCurrent}>
            {saving ? <><span className="spin"><Icon.RotateCcw /></span> 处理中</> : <><Icon.Gamepad /> 确认关联</>}
          </button>
        </div>
      </div>
    </div>
  );
}
