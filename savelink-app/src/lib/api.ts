// 数据访问层（第 5 步：真实现，调用 Tauri 命令）。
//
// 函数签名与第 4 步的 mock 版完全一致 —— 组件代码一行未改，
// 只是这里的实现从"操作内存数组"换成了"await invoke(Rust 命令)"。
// 这就是当初让组件只依赖本层、不直接碰 invoke 的回报。

import { invoke } from "@tauri-apps/api/core";
import type { Game, Snapshot, ScanResult, RestoreResult, AppInfo } from "./types";

export async function listGames(): Promise<Game[]> {
  return invoke<Game[]>("list_games");
}

export async function getRepositoryPath(): Promise<string> {
  return invoke<string>("get_repository_path");
}

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("get_app_info");
}

export async function listSnapshots(gameId: string): Promise<Snapshot[]> {
  return invoke<Snapshot[]>("list_snapshots", { gameId });
}

export async function scanPath(path: string): Promise<ScanResult> {
  // Rust scan_path 返回 SnapshotDto，取其中 file_count / total_size。
  const r = await invoke<{ file_count: number; total_size: number }>("scan_path", { path });
  return { file_count: r.file_count, total_size: r.total_size };
}

export async function addGame(name: string, savePaths: string[]): Promise<Game> {
  return invoke<Game>("add_game", { name, savePaths });
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
