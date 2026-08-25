import { useEffect, useState } from "react";
import { Icon } from "../lib/icons";
import type {
  FileActivityKind,
  SaveCandidateConfidence,
  SaveDiscoveryPhase,
  SaveDiscoveryStatus,
} from "../lib/types";

const PHASE_LABEL: Record<SaveDiscoveryPhase, string> = {
  idle: "尚未开始监测",
  starting_watchers: "正在准备监测目录",
  launching_game: "监测已就绪，正在启动游戏",
  monitoring: "正在监测游戏期间的文件变化",
  exit_grace_period: "游戏已经退出，正在等待最后一批文件写入",
  analyzing: "正在整理存档目录候选",
  awaiting_confirmation: "存档活动分析完成",
  confirming: "正在保存存档目录",
  failed: "存档活动监测失败",
  cancelled: "存档活动监测已取消",
};

const CONFIDENCE_LABEL: Record<SaveCandidateConfidence, string> = {
  high: "高可信",
  medium: "中可信",
  low: "低可信",
};

const EVENT_LABEL: Record<FileActivityKind, string> = {
  create: "创建",
  modify: "修改",
  delete: "删除",
  rename_from: "重命名前",
  rename_to: "重命名后",
  observed: "目录变化后存在",
};

interface SaveDiscoveryPanelProps {
  status: SaveDiscoveryStatus;
  busy: boolean;
  onConfirm: (savePaths: string[]) => void;
}

export function SaveDiscoveryPanel({ status, busy, onConfirm }: SaveDiscoveryPanelProps) {
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const finished = status.phase === "awaiting_confirmation";
  const showingCandidates = finished || status.phase === "confirming";
  const active = [
    "starting_watchers",
    "launching_game",
    "monitoring",
    "exit_grace_period",
    "analyzing",
    "confirming",
  ].includes(status.phase);

  useEffect(() => {
    setSelectedPaths(new Set());
  }, [status.started_at_unix_ms]);

  function togglePath(path: string, checked: boolean) {
    setSelectedPaths((current) => {
      const next = new Set(current);
      if (checked) next.add(path);
      else next.delete(path);
      return next;
    });
  }

  const recommendedCandidates = status.candidates.filter((candidate) => candidate.confidence !== "low");
  const lowConfidenceCandidates = status.candidates.filter((candidate) => candidate.confidence === "low");

  function renderCandidate(candidate: SaveDiscoveryStatus["candidates"][number]) {
    const selectable = candidate.confirmable && !candidate.unsafe_reason && !status.incomplete;
    const selected = selectedPaths.has(candidate.directory);
    const details = (
      <>
        <div className="discovery-signals">
          {candidate.positive_signals.map((signal) => <span className="positive" key={signal}>{signal}</span>)}
          {candidate.downgrade_reasons.map((reason) => <span className="negative" key={reason}>{reason}</span>)}
        </div>
        <div className="discovery-files">
          {candidate.files.map((file) => (
            <div className="discovery-file" key={file.path} title={file.path}>
              <span className="discovery-file-path">{file.path}</span>
              <span>{file.kinds.map((kind) => EVENT_LABEL[kind]).join(" / ")}</span>
            </div>
          ))}
        </div>
      </>
    );

    return <article className={`discovery-candidate${selected ? " selected" : ""}`} key={candidate.directory}>
      <label className={`discovery-candidate-head${selectable ? " selectable" : " disabled"}`}>
        <input
          type="checkbox"
          checked={selected}
          disabled={!selectable || busy || status.phase === "confirming"}
          onChange={(event) => togglePath(candidate.directory, event.target.checked)}
          aria-label={`选择存档目录 ${candidate.directory}`}
        />
        <div className="discovery-candidate-path" title={candidate.directory}>
          <Icon.Folder size={15} />
          <span>{candidate.directory}</span>
        </div>
        <span className={`discovery-confidence ${candidate.confidence}`}>
          {CONFIDENCE_LABEL[candidate.confidence]}
        </span>
      </label>
      <div className="discovery-candidate-meta">
        {candidate.distinct_file_count} 个文件 · {candidate.event_count} 次有效变化 · 评分 {candidate.score}
      </div>
      {candidate.unsafe_reason && (
        <div className="discovery-candidate-risk"><Icon.Alert size={14} />{candidate.unsafe_reason}</div>
      )}
      {candidate.confidence === "low" ? (
        <details className="discovery-candidate-details">
          <summary>展开详情</summary>
          {details}
        </details>
      ) : details}
    </article>;
  }

  return (
    <section className="discovery-panel" aria-live="polite">
      <div className={`discovery-state ${status.phase === "failed" ? "failed" : ""}`}>
        <span className={active ? "spin" : "discovery-state-icon"}>
          {active ? <Icon.RotateCcw /> : finished ? <Icon.CheckCircle /> : <Icon.Alert />}
        </span>
        <div className="discovery-state-copy">
          <strong>{PHASE_LABEL[status.phase]}</strong>
          <span>
            {status.launcher_fallback
              ? "启动进程已退出，可能已转交给启动器；请在游戏真正退出后停止并分析。"
              : status.phase === "monitoring"
                ? `已记录 ${status.event_count} 条文件变化${status.pid ? ` · PID ${status.pid}` : ""}`
                : status.phase === "awaiting_confirmation"
                  ? `共记录 ${status.event_count} 条文件变化，得到 ${status.candidates.length} 个候选目录`
                  : status.phase === "cancelled"
                    ? "本次监测没有保留候选结果。"
                    : ""}
          </span>
        </div>
      </div>

      {status.incomplete && (
        <div className="callout warn discovery-warning">
          <span className="ic"><Icon.Alert /></span>
          <div>本次监测结果不完整，候选目录仅供排查，请重新监测后再确认。</div>
        </div>
      )}

      {status.errors.length > 0 && (
        <div className="discovery-errors">
          {status.errors.map((error, index) => <div key={index}>{error}</div>)}
        </div>
      )}

      {showingCandidates && status.candidates.length === 0 && !status.incomplete && (
        <div className="discovery-empty-result">本次游玩期间没有捕获到可分析的文件变化。</div>
      )}

      {showingCandidates && status.candidates.length > 0 && (
        <div className="discovery-candidates">
          {recommendedCandidates.map(renderCandidate)}
          {lowConfidenceCandidates.length > 0 && (
            <details className="discovery-low-confidence-group">
              <summary>其他低可信候选（{lowConfidenceCandidates.length}）</summary>
              <div className="discovery-low-confidence-list">
                {lowConfidenceCandidates.map(renderCandidate)}
              </div>
            </details>
          )}
          <div className="discovery-confirm-actions">
            <span>{selectedPaths.size > 0 ? `已选择 ${selectedPaths.size} 个目录` : "尚未选择目录"}</span>
            <button
              className="btn primary"
              disabled={selectedPaths.size === 0 || status.incomplete || busy || status.phase === "confirming"}
              onClick={() => onConfirm(Array.from(selectedPaths))}
            >
              {busy || status.phase === "confirming"
                ? <><span className="spin"><Icon.RotateCcw /></span> 正在保存…</>
                : <><Icon.Shield /> 确认并开始保护</>}
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
