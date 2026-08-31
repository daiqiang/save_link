// 数据访问层（第 5 步：真实现，调用 Tauri 命令）。
//
// 函数签名与第 4 步的 mock 版完全一致 —— 组件代码一行未改，
// 只是这里的实现从"操作内存数组"换成了"await invoke(Rust 命令)"。
// 这就是当初让组件只依赖本层、不直接碰 invoke 的回报。

import { invoke } from "@tauri-apps/api/core";
import type {
  AppInfo,
  AutoBackupSettings,
  BaiduConnection,
  CloudUploadResult,
  CloudReceiveResult,
  CloudSnapshot,
  ConfirmSaveDiscoveryPathsResult,
  DesmumeDiscoveryReport,
  Game,
  ProgramDiscoveryReport,
  RestoreResult,
  ScanResult,
  SaveDiscoveryStatus,
  Snapshot,
  SteamDiscoveryReport,
} from "./types";

export async function listGames(): Promise<Game[]> {
  return invoke<Game[]>("list_games");
}

export async function getRepositoryPath(): Promise<string> {
  return invoke<string>("get_repository_path");
}

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("get_app_info");
}

export async function getAutoBackupSettings(): Promise<AutoBackupSettings> {
  return invoke<AutoBackupSettings>("get_auto_backup_settings");
}

export async function setAutoBackupEnabled(enabled: boolean): Promise<AutoBackupSettings> {
  return invoke<AutoBackupSettings>("set_auto_backup_enabled", { enabled });
}

export async function setAutoBackupRetention(limit: number): Promise<AutoBackupSettings> {
  return invoke<AutoBackupSettings>("set_auto_backup_retention", { limit });
}

export async function getBaiduConnectionStatus(): Promise<BaiduConnection> {
  return invoke<BaiduConnection>("get_baidu_connection_status");
}

export async function connectBaidu(): Promise<BaiduConnection> {
  return invoke<BaiduConnection>("connect_baidu");
}

export async function uploadSnapshotToBaidu(
  gameId: string,
  snapshotId: string,
): Promise<CloudUploadResult> {
  return invoke<CloudUploadResult>("upload_snapshot_to_baidu", { gameId, snapshotId });
}

export async function discoverBaiduSnapshots(): Promise<CloudSnapshot[]> {
  return invoke<CloudSnapshot[]>("discover_baidu_snapshots");
}

export async function receiveBaiduSnapshot(snapshotId: string): Promise<CloudReceiveResult> {
  return invoke<CloudReceiveResult>("receive_baidu_snapshot", { snapshotId });
}

export async function listSnapshots(gameId: string): Promise<Snapshot[]> {
  return invoke<Snapshot[]>("list_snapshots", { gameId });
}

export async function compareSnapshotWithLocal(snapshotId: string): Promise<boolean> {
  return invoke<boolean>("compare_snapshot_with_local", { snapshotId });
}

export async function scanPath(path: string): Promise<ScanResult> {
  // Rust scan_path 返回 SnapshotDto，取其中 file_count / total_size。
  const r = await invoke<{ file_count: number; total_size: number }>("scan_path", { path });
  return { file_count: r.file_count, total_size: r.total_size };
}

export async function scanSteamGames(steamRoot?: string): Promise<SteamDiscoveryReport> {
  return invoke<SteamDiscoveryReport>("scan_steam_games", {
    steamRoot: steamRoot?.trim() || null,
  });
}

export async function scanProgramGame(selectedPath: string): Promise<ProgramDiscoveryReport> {
  return invoke<ProgramDiscoveryReport>("scan_program_game", { selectedPath });
}

export async function scanDesmumeGames(
  emulatorRoot: string,
  romRoot?: string,
): Promise<DesmumeDiscoveryReport> {
  return invoke<DesmumeDiscoveryReport>("scan_desmume_games", {
    emulatorRoot,
    romRoot: romRoot?.trim() || null,
  });
}

export async function registerDesmumeGame(
  emulatorRoot: string,
  romRoot: string | null,
  romPath: string,
  bindGameId: string | null,
): Promise<Game> {
  return invoke<Game>("register_desmume_game", {
    emulatorRoot,
    romRoot,
    romPath,
    bindGameId,
  });
}

export async function addGame(
  name: string,
  savePaths: string[],
  executablePath: string,
  installDir: string,
): Promise<Game> {
  return invoke<Game>("add_game", { name, savePaths, executablePath, installDir });
}

export async function getSaveDiscoveryStatus(): Promise<SaveDiscoveryStatus> {
  return invoke<SaveDiscoveryStatus>("get_save_discovery_status");
}

export async function startSaveDiscovery(gameId: string): Promise<SaveDiscoveryStatus> {
  return invoke<SaveDiscoveryStatus>("start_save_discovery", { gameId });
}

export async function launchGame(gameId: string): Promise<{ pid: number }> {
  return invoke<{ pid: number }>("launch_game", { gameId });
}

export async function stopSaveDiscovery(): Promise<SaveDiscoveryStatus> {
  return invoke<SaveDiscoveryStatus>("stop_save_discovery");
}

export async function cancelSaveDiscovery(): Promise<SaveDiscoveryStatus> {
  return invoke<SaveDiscoveryStatus>("cancel_save_discovery");
}

export async function confirmSaveDiscoveryPaths(
  gameId: string,
  savePaths: string[],
): Promise<ConfirmSaveDiscoveryPathsResult> {
  return invoke<ConfirmSaveDiscoveryPathsResult>("confirm_save_discovery_paths", {
    gameId,
    savePaths,
  });
}

export async function addProgramGame(
  name: string,
  savePaths: string[],
  executablePath: string,
  installDir: string,
): Promise<Game> {
  return invoke<Game>("add_program_game", { name, savePaths, executablePath, installDir });
}

export async function registerSteamGame(
  name: string,
  savePaths: string[],
  steamRoot: string,
  installDir: string,
  appId: number,
): Promise<Game> {
  return invoke<Game>("register_steam_game", { name, savePaths, steamRoot, installDir, appId });
}

export async function bindProgramToGame(
  gameId: string,
  executablePath: string,
  installDir: string | null,
  replaceExisting: boolean,
): Promise<Game> {
  return invoke<Game>("bind_program_to_game", {
    gameId,
    executablePath,
    installDir,
    replaceExisting,
  });
}

export async function updateGame(gameId: string, name: string, savePaths: string[]): Promise<Game> {
  return invoke<Game>("update_game", { gameId, name, savePaths });
}

// 返回 null 表示"存档未变化"（Rust 端 NoChange 映射为 None → JS null）。
export async function createSnapshot(gameId: string, note: string | null): Promise<Snapshot | null> {
  return invoke<Snapshot | null>("create_snapshot", { gameId, note });
}

export async function updateSnapshotMeta(
  snapshotId: string, note: string | null, locked: boolean | null,
): Promise<void> {
  return invoke("update_snapshot_meta", { snapshotId, note, locked });
}

export async function deleteSnapshot(snapshotId: string): Promise<void> {
  return invoke("delete_snapshot", { snapshotId });
}

export async function deleteGame(gameId: string): Promise<void> {
  return invoke("delete_game", { gameId });
}

export async function restoreSnapshot(gameId: string, snapshotId: string): Promise<RestoreResult> {
  return invoke<RestoreResult>("restore_snapshot", { gameId, snapshotId });
}

// 存档目录缺失时的“用户已决策”续走：choice = "create" | "reselect" | "cancel"。
export async function restoreSnapshotWithChoice(
  gameId: string,
  snapshotId: string,
  choice: "create" | "reselect" | "cancel",
): Promise<RestoreResult> {
  return invoke<RestoreResult>("restore_snapshot_with_choice", { gameId, snapshotId, choice });
}
