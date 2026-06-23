import { useState } from "react";
import { Icon } from "../lib/icons";
import * as api from "../lib/api";
import type { Game, Snapshot } from "../lib/types";

interface Props {
  game: Game;
  target: Snapshot;
  onClose: () => void;
  onDone: () => void; // 恢复成功后刷新时间线
}

type Phase = "confirm" | "running" | "done";
const STEPS = [
  { key: "backup", label: "备份当前存档" },
  { key: "restore", label: "恢复目标版本" },
  { key: "verify", label: "校验恢复结果" },
];

export function RestoreDialog({ game, target, onClose, onDone }: Props) {
  const [phase, setPhase] = useState<Phase>("confirm");
  const [step, setStep] = useState(0);
  const [backupStamp, setBackupStamp] = useState("");

  async function run() {
    setPhase("running");
    // 逐步推进进度（演示节奏）；真实进度第 5 步接 Tauri event。
    for (let i = 0; i < STEPS.length; i++) {
      setStep(i);
      await new Promise((r) => setTimeout(r, 700));
    }
    const res = await api.restoreSnapshot(game.id, target.id);
    // 取回备份时间用于"完成"提示
    const snaps = await api.listSnapshots(game.id);
    const backup = snaps.find((s) => s.id === res.backup_id);
    setBackupStamp(backup?.created_at ?? "");
    setStep(STEPS.length);
    setPhase("done");
    onDone();
  }

  return (
    <div className="overlay" onMouseDown={(e) => phase === "confirm" && e.target === e.currentTarget && onClose()}>
      <div className="modal narrow">
        {phase === "confirm" && (
          <>
            <div className="modal-head">
              <h3>恢复到这个存档版本？</h3>
              <button className="iconbtn" onClick={onClose}><Icon.Close /></button>
            </div>
            <div className="modal-body">
              <div className="kv"><span className="k">游戏</span><span className="v">{game.name}</span></div>
              <div className="kv"><span className="k">目标版本</span>
                <span className="v">{target.created_at} {target.note}</span></div>
              <div className="callout warn">
                <span className="ic"><Icon.Shield /></span>
                <div>
                  <div style={{ fontWeight: 600, marginBottom: 2 }}>SaveLink 将执行以下操作</div>
                  <ol>
                    <li>先自动备份当前真实存档。</li>
                    <li>再把目标版本恢复到游戏存档目录（会覆盖当前文件）。</li>
                    <li>恢复完成后，你仍然可以恢复「恢复前自动备份」。</li>
                  </ol>
                </div>
              </div>
              <div className="target-box">
                <div style={{ fontSize: 12, color: "var(--color-text-3)", marginBottom: 4 }}>当前存档目录</div>
                <div className="path-mono">{game.save_paths[0]}</div>
              </div>
            </div>
            <div className="modal-foot">
              <button className="btn" onClick={onClose}>取消</button>
              <button className="btn primary" onClick={run}><Icon.Shield /> 备份当前存档并恢复</button>
            </div>
          </>
        )}

        {phase === "running" && (
          <>
            <div className="modal-head"><h3>正在恢复</h3></div>
            <div className="modal-body">
              <div className="steps">
                {STEPS.map((s, i) => {
                  const cls = i < step ? "done" : i === step ? "active" : "";
                  return (
                    <div className={`step ${cls}`} key={s.key}>
                      <span className="si">
                        {i < step ? <Icon.Check /> : i === step ? <span className="spin"><Icon.RotateCcw /></span> : null}
                      </span>
                      <span>{i < step ? `已${s.label}` : i === step ? `正在${s.label}` : s.label}</span>
                    </div>
                  );
                })}
              </div>
              <div className="callout info" style={{ marginTop: 16 }}>
                <span className="ic"><Icon.Alert /></span>
                <div>请不要在恢复过程中启动游戏。</div>
              </div>
            </div>
          </>
        )}

        {phase === "done" && (
          <>
            <div className="modal-head"><h3>恢复完成</h3></div>
            <div className="modal-body">
              <div className="callout ok">
                <span className="ic"><Icon.CheckCircle /></span>
                <div>已恢复到：<strong>{target.created_at} {target.note}</strong></div>
              </div>
              <div style={{ marginTop: 14, fontSize: 13, color: "var(--color-text-2)" }}>
                恢复前的当前存档已自动保存为：
                <div className="target-box" style={{ marginTop: 6, fontWeight: 600, color: "var(--color-text)" }}>
                  <Icon.Shield size={14} /> {backupStamp} 恢复前自动备份
                </div>
              </div>
            </div>
            <div className="modal-foot">
              <button className="btn primary" onClick={onClose}>回到时间线</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
