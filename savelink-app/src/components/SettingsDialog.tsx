import { useEffect, useState } from "react";
import { openPath } from "@tauri-apps/plugin-opener";
import { Icon } from "../lib/icons";
import * as api from "../lib/api";
import { useToast } from "./Toast";
import type { AppInfo } from "../lib/types";

interface Props {
  onClose: () => void;
}

export function SettingsDialog({ onClose }: Props) {
  const toast = useToast();
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    let alive = true;
    api.getAppInfo()
      .then((value) => {
        if (alive) setInfo(value);
      })
      .catch((e) => toast(String(e), "err"));
    return () => {
      alive = false;
    };
  }, [toast]);

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast("路径已复制", "ok");
    } catch {
      toast("复制失败", "err");
    }
  }

  async function reveal(path: string) {
    try {
      await openPath(path);
    } catch (e) {
      toast(String(e), "err");
    }
  }

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal settings-modal">
        <div className="modal-head">
          <h3>设置</h3>
          <button className="iconbtn" onClick={onClose}><Icon.Close /></button>
        </div>
        <div className="modal-body">
          {!info ? (
            <div className="hint muted"><span className="spin"><Icon.RotateCcw /></span> 正在读取应用信息…</div>
          ) : (
            <div className="settings-list">
              <InfoRow label="运行方式" value="绿色版和安装版共用同一个用户数据目录" />
              <PathRow label="数据目录" value={info.data_dir} onCopy={copy} onOpen={reveal} />
              <PathRow label="快照仓库" value={info.repository_dir} onCopy={copy} onOpen={reveal} />
              <PathRow label="数据库文件" value={info.database_path} onCopy={copy} />
            </div>
          )}
        </div>
        <div className="modal-foot">
          <button className="btn primary" onClick={onClose}>完成</button>
        </div>
      </div>
    </div>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-row">
      <div className="settings-label">{label}</div>
      <div className="settings-value">{value}</div>
    </div>
  );
}

function PathRow({
  label,
  value,
  onCopy,
  onOpen,
}: {
  label: string;
  value: string;
  onCopy: (value: string) => void;
  onOpen?: (value: string) => void;
}) {
  return (
    <div className="settings-row">
      <div className="settings-label">{label}</div>
      <div className="settings-path-line">
        <div className="target-box path-mono">{value}</div>
        <div className="settings-actions">
          {onOpen && <button className="btn sm" onClick={() => onOpen(value)}><Icon.Folder /> 打开</button>}
          <button className="btn sm" onClick={() => onCopy(value)}><Icon.Copy /> 复制</button>
        </div>
      </div>
    </div>
  );
}
