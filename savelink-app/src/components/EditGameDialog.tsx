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
  onRebindDesmume: () => void;
}

export function EditGameDialog({ game, onClose, onSaved, onDeleted, onRebindDesmume }: Props) {
  const toast = useToast();
  const isDesmume = game.emulator === "desmume";
  const [name, setName] = useState(game.name);
  const [paths, setPaths] = useState(game.save_paths.length > 0 ? game.save_paths : [""]);
  const [scan, setScan] = useState<{ index: number; state: "idle" | "loading" | "done" | "err"; text: string }>({
    index: -1,
    state: "idle",
    text: "目录未检测。",
  });
  const [saving, setSaving] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  async function pickDir(index: number) {
    const picked = await open({ directory: true, multiple: false, title: "选择新的存档目录" });
    if (typeof picked === "string") {
      setPaths((current) => current.map((value, currentIndex) => currentIndex === index ? picked : value));
      setScan({ index, state: "idle", text: "目录未检测。" });
    }
  }

  async function testRead(index: number) {
    const path = paths[index]?.trim() ?? "";
    if (!path) {
      setScan({ index, state: "err", text: "请先选择一个存档目录。" });
      return;
    }
    setScan({ index, state: "loading", text: "正在读取目录…" });
    try {
      const r = await api.scanPath(path);
      setScan({ index, state: "done", text: `已检测到：${r.file_count} 个文件，${formatSize(r.total_size)}` });
    } catch {
      setScan({ index, state: "err", text: "无法访问该目录，请重新选择。" });
    }
  }

  async function save() {
    if (!name.trim()) return toast("请填写游戏名称", "err");
    const savePaths = paths.map((value) => value.trim()).filter(Boolean);
    if (!isDesmume && savePaths.length === 0) return toast("请至少选择一个存档目录", "err");
    setSaving(true);
    try {
      const updated = await api.updateGame(game.id, name.trim(), isDesmume ? game.save_paths : savePaths);
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
          {isDesmume ? (
            <div className="field">
              <label>DeSmuME 存档</label>
              <div className="target-box">
                {game.save_paths.length > 0
                  ? <div className="path-stack">{game.save_paths.map((item, index) => <span className="path-mono" key={index}>{item}</span>)}</div>
                  : <span className="path-mono">尚未绑定本机 DeSmuME ROM</span>}
              </div>
              <div className="field-actions">
                <button className="btn sm" onClick={onRebindDesmume} disabled={saving}>
                  <Icon.Gamepad /> 重新绑定 DeSmuME
                </button>
              </div>
            </div>
          ) : (
            <div className="field">
              <label>存档目录</label>
              <div className="save-path-editor">
                {paths.map((path, index) => (
                  <div className="save-path-edit" key={index}>
                    <div className="save-path-edit-label">目录 {index + 1}</div>
                    <div className="save-path-edit-row">
                      <input className="input path-mono" value={path} onChange={(event) => {
                        setPaths((current) => current.map((value, currentIndex) => currentIndex === index ? event.target.value : value));
                        setScan({ index, state: "idle", text: "目录未检测。" });
                      }} />
                      <button className="iconbtn" title="选择目录" onClick={() => pickDir(index)}><Icon.Folder /></button>
                      <button className="iconbtn" title="测试读取" onClick={() => testRead(index)}
                        disabled={scan.index === index && scan.state === "loading"}>
                        {scan.index === index && scan.state === "loading"
                          ? <span className="spin"><Icon.RotateCcw /></span>
                          : <Icon.Camera />}
                      </button>
                      <button className="iconbtn danger-text" title="移除目录" disabled={paths.length === 1}
                        onClick={() => setPaths((current) => current.filter((_, currentIndex) => currentIndex !== index))}>
                        <Icon.Trash />
                      </button>
                    </div>
                    {scan.index === index && scan.state !== "idle" && (
                      <div className={`hint ${scan.state === "done" ? "ok" : scan.state === "err" ? "err" : "muted"}`}>
                        {scan.state === "done" && <Icon.CheckCircle />}
                        {scan.state === "err" && <Icon.Alert />}
                        <span>{scan.text}</span>
                      </div>
                    )}
                  </div>
                ))}
              </div>
              <button className="btn sm add-save-path" onClick={() => setPaths((current) => [...current, ""])}>
                <Icon.Plus /> 添加目录
              </button>
            </div>
          )}
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
                {game.save_paths.length > 0
                  ? <div className="path-stack">{game.save_paths.map((item, index) => <span className="path-mono" key={index}>{item}</span>)}</div>
                  : <span className="path-mono">未设置存档目录</span>}
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
