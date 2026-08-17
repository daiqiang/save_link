// 与 Rust DTO 对齐的前端类型（对应 src-tauri/commands.rs 的 GameDto / SnapshotDto）。
// 第 5 步接线时，invoke 的返回值就是这些类型。

export type SnapshotReason = "manual" | "before_restore" | "auto";
export type CloudSyncStatus =
  | "uploading"
  | "uploaded"
  | "remote_only"
  | "downloading"
  | "downloaded"
  | "ignored"
  | "error"
  | "delete_pending"
  | "deleting"
  | "delete_failed"
  | "remote_deleted";

export interface Game {
  id: string;
  name: string;
  icon: string | null;
  save_paths: string[];
  emulator: string | null;
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
  source_count: number;
  cloud_status: CloudSyncStatus | null;
  cloud_error_code: string | null;
}

export interface ScanResult {
  file_count: number;
  total_size: number;
}

export interface SteamDiscoveredGame {
  name: string;
  steam_name: string;
  app_id: number;
  install_dir: string;
  save_paths: string[];
  config_paths: string[];
  current_system_unresolved_rules: number;
  other_environment_rules: number;
  already_added: boolean;
  can_add_directly: boolean;
}

export interface SteamDiscoveryReport {
  steam_root: string;
  library_count: number;
  registered_app_count: number;
  manifest_match_count: number;
  games: SteamDiscoveredGame[];
}

export interface ProgramDiscoveredGame {
  name: string;
  app_id: number;
  match_kind: "app_id" | "name";
  save_paths: string[];
  config_paths: string[];
  current_system_unresolved_rules: number;
  other_environment_rules: number;
  already_added: boolean;
  can_add_directly: boolean;
}

export interface ProgramDiscoveryReport {
  selected_path: string;
  selection_kind: "directory" | "executable" | "shortcut";
  resolved_program_path: string | null;
  install_dir: string;
  detected_app_id: number | null;
  app_id_source: string | null;
  identity_hints: string[];
  games: ProgramDiscoveredGame[];
}

export interface DesmumeGameMatch {
  game_id: string;
  game_name: string;
  match_kind: "exact" | "possible";
  already_bound_here: boolean;
}

export interface DesmumeDiscoveredGame {
  name: string;
  rom_path: string;
  save_path: string;
  has_save: boolean;
  rom_sha256: string;
  rom_header_title: string;
  rom_game_code: string;
  matches: DesmumeGameMatch[];
}

export interface DesmumeDiscoveryReport {
  emulator_root: string;
  configured_rom_root: string | null;
  rom_root: string | null;
  configured_rom_root_missing: boolean;
  battery_dir: string;
  games: DesmumeDiscoveredGame[];
}

export interface RestoreResult {
  target_id: string;
  restored: boolean;
}

export interface AppInfo {
  version: string;
  data_dir: string;
  repository_dir: string;
  database_path: string;
  profile_label: string | null;
}

export interface AutoBackupSettings {
  enabled: boolean;
  interval_minutes: number;
}

export interface BaiduConnection {
  connected: boolean;
  provider: string;
  display_name: string | null;
  expires_at: string | null;
}

export interface CloudUploadResult {
  snapshot_id: string;
  outcome: "uploaded" | "already_present";
  cloud_status: "uploaded";
}

export interface CloudSnapshot {
  cloud_game_id: string;
  game_name: string;
  snapshot_id: string;
  created_at: string;
  note: string | null;
  reason: SnapshotReason;
  locked: boolean;
  file_count: number;
  total_size: number;
  source_count: number;
  cloud_status: CloudSyncStatus;
  last_error_code: string | null;
}

export interface CloudReceiveResult {
  snapshot_id: string;
  game_id: string;
  outcome: "downloaded" | "already_present";
}

// 恢复进度步骤（对应核心 RestoreStep）。
export type RestoreStep = "backup_current" | "restore_target" | "verify";
