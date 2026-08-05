import { useEffect, useState } from "react";
import { Icon } from "../lib/icons";
import * as api from "../lib/api";
import { useToast } from "./Toast";
import type { AutoBackupSettings } from "../lib/types";

interface Props {
  onClose: () => void;
}

export function SettingsDialog({ onClose }: Props) {
  const toast = useToast();
  const [settings, setSettings] = useState<AutoBackupSettings | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let alive = true;
    api.getAutoBackupSettings()
      .then((value) => {
        if (alive) setSettings(value);
      })
      .catch((error) => toast(String(error), "err"));
    return () => {
      alive = false;
    };
  }, [toast]);

  async function toggleAutoBackup() {
    if (!settings || saving) return;
    const next = !settings.enabled;
    setSaving(true);
    try {
      const updated = await api.setAutoBackupEnabled(next);
      setSettings(updated);
      toast(next ? "自动备份已开启" : "自动备份已关闭", "ok");
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="overlay" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="modal narrow">
        <div className="modal-head">
          <h3>设置</h3>
          <button className="iconbtn" title="关闭" onClick={onClose}><Icon.Close /></button>
        </div>
        <div className="modal-body settings-body">
          {!settings ? (
            <div className="settings-loading"><span className="spin"><Icon.RotateCcw /></span></div>
          ) : (
            <div className="setting-control-row">
              <div className="setting-control-copy">
                <strong>自动备份</strong>
                <span>每 {settings.interval_minutes} 分钟检查一次</span>
              </div>
              <button
                className={`switch ${settings.enabled ? "on" : ""}`}
                type="button"
                role="switch"
                aria-checked={settings.enabled}
                aria-label="自动备份"
                title={settings.enabled ? "关闭自动备份" : "开启自动备份"}
                disabled={saving}
                onClick={toggleAutoBackup}
              >
                <span />
              </button>
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
