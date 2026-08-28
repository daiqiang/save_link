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
  const [retentionDraft, setRetentionDraft] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let alive = true;
    api.getAutoBackupSettings()
      .then((value) => {
        if (alive) {
          setSettings(value);
          setRetentionDraft(String(value.retention_limit));
        }
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

  async function saveRetention() {
    if (!settings || saving) return;
    const next = Number(retentionDraft);
    if (!Number.isInteger(next) || next < 1 || next > 100) {
      toast("快照保留数量必须是 1 到 100 之间的整数", "warn");
      setRetentionDraft(String(settings.retention_limit));
      return;
    }
    if (next === settings.retention_limit && settings.retention_policy_confirmed) return;
    if ((next < settings.retention_limit || !settings.retention_policy_confirmed) && !window.confirm(
      `将每个游戏的未锁定快照保留数量设为 ${next} 个，并清理超出的旧快照。锁定快照不受影响。继续吗？`,
    )) {
      setRetentionDraft(String(settings.retention_limit));
      return;
    }

    setSaving(true);
    try {
      const updated = await api.setAutoBackupRetention(next);
      setSettings(updated);
      setRetentionDraft(String(updated.retention_limit));
      toast(
        next < settings.retention_limit
          ? `已改为保留 ${next} 个，旧快照正在按新规则清理`
          : `已改为保留 ${next} 个快照`,
        "ok",
      );
    } catch (error) {
      setRetentionDraft(String(settings.retention_limit));
      toast(String(error), "err");
    } finally {
      setSaving(false);
    }
  }

  function closeSettings() {
    if (
      settings
      && retentionDraft !== String(settings.retention_limit)
      && !saving
      && !window.confirm("快照保留数量尚未保存，确定放弃修改吗？")
    ) {
      return;
    }
    onClose();
  }

  return (
    <div className="overlay" onMouseDown={(event) => event.target === event.currentTarget && closeSettings()}>
      <div className="modal narrow">
        <div className="modal-head">
          <h3>设置</h3>
          <button className="iconbtn" title="关闭" onClick={closeSettings}><Icon.Close /></button>
        </div>
        <div className="modal-body settings-body">
          {!settings ? (
            <div className="settings-loading"><span className="spin"><Icon.RotateCcw /></span></div>
          ) : (
            <>
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
              <div className="setting-control-row setting-retention-row">
                <div className="setting-control-copy">
                  <strong>快照保留数量</strong>
                  <span>
                    每个游戏保留最新的未锁定快照{settings.retention_policy_confirmed ? "" : "；保存后应用新规则"}
                  </span>
                </div>
                <div className="setting-number-control">
                  <input
                    className="input"
                    type="number"
                    min="1"
                    max="100"
                    step="1"
                    value={retentionDraft}
                    aria-label="快照保留数量"
                    disabled={saving}
                    onChange={(event) => setRetentionDraft(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        void saveRetention();
                      }
                    }}
                  />
                  <span className="setting-number-unit">个</span>
                  <button
                    className="iconbtn"
                    type="button"
                    title="保存保留数量"
                    aria-label="保存保留数量"
                    disabled={saving || (
                      retentionDraft === String(settings.retention_limit)
                      && settings.retention_policy_confirmed
                    )}
                    onClick={() => void saveRetention()}
                  >
                    <Icon.Save />
                  </button>
                </div>
              </div>
            </>
          )}
        </div>
        <div className="modal-foot">
          <button className="btn primary" onClick={closeSettings}>完成</button>
        </div>
      </div>
    </div>
  );
}
