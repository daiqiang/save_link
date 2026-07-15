// 与 Rust DTO 对齐的前端类型（对应 src-tauri/commands.rs 的 GameDto / SnapshotDto）。
// 第 5 步接线时，invoke 的返回值就是这些类型。

export type SnapshotReason = "manual" | "before_restore" | "auto";

export interface Game {
  id: string;
  name: string;
  icon: string | null;
  save_paths: string[];
  snapshot_count: number;
  last_snapshot_at: string | null;
}

export interface Snapshot {
  id: string;
  game_id: string;
  created_at: string;
  note: string | null;
  reason: SnapshotReason;
  locked: boolean;
  file_count: number;
  total_size: number; // 字节
}

export interface ScanResult {
  file_count: number;
  total_size: number;
}

export interface RestoreResult {
  target_id: string;
  backup_id: string;
}

export interface AppInfo {
  version: string;
  data_dir: string;
  repository_dir: string;
  database_path: string;
}

export interface BaiduConnection {
  connected: boolean;
  provider: string;
  display_name: string | null;
  expires_at: string | null;
}

// 恢复进度步骤（对应核心 RestoreStep）。
export type RestoreStep = "backup_current" | "restore_target" | "verify";
