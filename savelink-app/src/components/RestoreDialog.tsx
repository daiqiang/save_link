import { useState } from "react";
import { Icon } from "../lib/icons";
import * as api from "../lib/api";
import type { Game, RestoreResult, Snapshot } from "../lib/types";

interface Props {
  game: Game;
  target: Snapshot;
  onClose: () => void;
  onDone: () => void; // 恢复成功后刷新时间线
}

type Phase = "confirm" | "running" | "done" | "choose" | "error";

export function RestoreDialog({ game, target, onClose, onDone }: Props) {
  const [phase, setPhase] = useState<Phase>("confirm");
  const [result, setResult] = useState<RestoreResult | null>(null);
  const [errMsg, setErrMsg] = useState("");

  function finishDone(restoreResult: RestoreResult) {
    setResult(restoreResult);
    setPhase("done");
    onDone();
  }

  async function run() {
    setPhase("running");
    try {
      const res = await api.restoreSnapshot(game.id, target.id);
      finishDone(res);
    } catch (e) {
      const msg = String(e);
      setErrMsg(msg);
      // 存档目录不存在 → 进入“如何处理”选择页，而不是直接判失败。
      setPhase(msg.includes("请选择如何处理") ? "choose" : "error");
    }
  }

  // 缺失目录时：新建目录并把目标版本恢复进去。
  async function createAndRestore() {
    setPhase("running");
    try {
      const res = await api.restoreSnapshotWithChoice(game.id, target.id, "create");
      finishDone(res);
    } catch (e) {
      // 创建并恢复本身也可能失败；按普通失败页处理。
      setErrMsg(String(e));
      setPhase("error");
    }
  }

  // 仅“恢复失败（已回滚: false）”这一种才可能动过真实存档；其余失败都没碰存档。
  const archiveUntouched = !errMsg.includes("已回滚: false");

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
                <span className="ic"><Icon.Alert /></span>
                <div>
                  <div style={{ fontWeight: 600, marginBottom: 2 }}>当前存档将被覆盖</div>
                  <div style={{ fontSize: 13 }}>
                    当前存档不会自动备份。请确认当前进度已经创建快照，再恢复目标版本。
                  </div>
                </div>
              </div>
              <div className="target-box">
                <div style={{ fontSize: 12, color: "var(--color-text-3)", marginBottom: 4 }}>当前存档目录</div>
                <div className="path-mono">{game.save_paths[0]}</div>
              </div>
            </div>
            <div className="modal-foot">
              <button className="btn" onClick={onClose}>取消</button>
              <button className="btn primary" onClick={run}><Icon.RotateCcw /> 恢复这个版本</button>
            </div>
          </>
        )}

        {phase === "running" && (
          <>
            <div className="modal-head"><h3>正在恢复</h3></div>
            <div className="modal-body">
              {/* 后端进度回调当前为空，前端无法获知真实分步状态；这里只给诚实的整体进行中提示，
                  不再伪造“已备份/已恢复”的完成态。 */}
              <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "6px 0" }}>
                <span className="spin"><Icon.RotateCcw /></span>
                <span>正在恢复：准备目标文件 → 替换存档 → 校验结果…请稍候</span>
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
                <div>
                  {result?.restored ? "已恢复到：" : "当前已经是这个版本："}
                  <strong>{target.created_at} {target.note}</strong>
                </div>
              </div>
              {!result?.restored && (
                <div style={{ marginTop: 14, fontSize: 13, color: "var(--color-text-2)" }}>
                  没有覆盖任何文件。
                </div>
              )}
            </div>
            <div className="modal-foot">
              <button className="btn primary" onClick={onClose}>回到时间线</button>
            </div>
          </>
        )}

        {phase === "choose" && (
          <>
            <div className="modal-head">
              <h3>存档目录不存在</h3>
              <button className="iconbtn" onClick={onClose}><Icon.Close /></button>
            </div>
            <div className="modal-body">
              <div className="callout warn">
                <span className="ic"><Icon.Alert /></span>
                <div>
                  <div style={{ fontWeight: 600, marginBottom: 2 }}>没找到这个游戏的存档目录</div>
                  <div className="path-mono" style={{ fontSize: 12, margin: "4px 0" }}>{game.save_paths[0]}</div>
                  <div style={{ fontSize: 13 }}>可以让 SaveLink 新建这个目录，并把目标版本直接恢复进去。</div>
                  <div style={{ fontSize: 12, color: "var(--color-text-3)", marginTop: 6 }}>到目前为止没有写入任何文件。</div>
                </div>
              </div>
            </div>
            <div className="modal-foot">
              <button className="btn" onClick={onClose}>取消</button>
              <button className="btn primary" onClick={createAndRestore}><Icon.Folder /> 创建目录并恢复</button>
            </div>
          </>
        )}

        {phase === "error" && (
          <>
            <div className="modal-head">
              <h3>恢复失败</h3>
              <button className="iconbtn" onClick={onClose}><Icon.Close /></button>
            </div>
            <div className="modal-body">
              <div className="callout warn">
                <span className="ic"><Icon.Alert /></span>
                <div>
                  <div style={{ fontWeight: 600, marginBottom: 2 }}>未能完成恢复</div>
                  <div style={{ fontSize: 13 }}>{errMsg}</div>
                  <div style={{ fontSize: 12, color: "var(--color-text-3)", marginTop: 6 }}>
                    {archiveUntouched
                      ? "本次恢复未修改你的真实存档。"
                      : "真实存档可能已被改动，请核对存档目录。"}
                  </div>
                </div>
              </div>
            </div>
            <div className="modal-foot">
              <button className="btn primary" onClick={onClose}>关闭</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
