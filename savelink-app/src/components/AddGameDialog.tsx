import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Icon } from "../lib/icons";
import { formatSize } from "../lib/format";
import * as api from "../lib/api";
import { useToast } from "./Toast";
import type {
  DesmumeDiscoveredGame,
  DesmumeDiscoveryReport,
  Game,
  SteamDiscoveredGame,
  SteamDiscoveryReport,
} from "../lib/types";

interface Props {
  onClose: () => void;
  onCreated: (game: Game) => void;
  initialMode?: AddGameMode;
}

export type AddGameMode = "steam" | "desmume" | "manual";
type SteamState =
  | { status: "loading" }
  | { status: "done"; report: SteamDiscoveryReport }
  | { status: "error"; message: string };
type DesmumeState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "done"; report: DesmumeDiscoveryReport }
  | { status: "error"; message: string };

export function AddGameDialog({ onClose, onCreated, initialMode = "steam" }: Props) {
  const toast = useToast();
  const [mode, setMode] = useState<AddGameMode>(initialMode);
  const [steam, setSteam] = useState<SteamState>({ status: "loading" });
  const [desmume, setDesmume] = useState<DesmumeState>({ status: "idle" });
  const [selectedAppId, setSelectedAppId] = useState<number | null>(null);
  const [selectedDesmumeRom, setSelectedDesmumeRom] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [scan, setScan] = useState<{ state: "idle" | "loading" | "done" | "error"; text: string }>({
    state: "idle",
    text: "目录未检测。",
  });
  const [saving, setSaving] = useState(false);
  const [repositoryPath, setRepositoryPath] = useState("");

  const selected = useMemo(() => {
    if (steam.status !== "done") return null;
    return steam.report.games.find((game) => game.app_id === selectedAppId) ?? null;
  }, [selectedAppId, steam]);
  const selectedDesmume = useMemo(() => {
    if (desmume.status !== "done") return null;
    return desmume.report.games.find((game) => game.rom_path === selectedDesmumeRom) ?? null;
  }, [desmume, selectedDesmumeRom]);

  useEffect(() => {
    let alive = true;
    api.getRepositoryPath()
      .then((value) => alive && setRepositoryPath(value))
      .catch(() => alive && setRepositoryPath("应用默认仓库位置"));
    api.scanSteamGames()
      .then((report) => {
        if (!alive) return;
        setSteam({ status: "done", report });
        setSelectedAppId(firstAvailableGame(report)?.app_id ?? report.games[0]?.app_id ?? null);
      })
      .catch((error) => alive && setSteam({ status: "error", message: String(error) }));
    return () => { alive = false; };
  }, []);

  async function scanSteam(steamRoot?: string) {
    setSteam({ status: "loading" });
    setSelectedAppId(null);
    try {
      const report = await api.scanSteamGames(steamRoot);
      setSteam({ status: "done", report });
      setSelectedAppId(firstAvailableGame(report)?.app_id ?? report.games[0]?.app_id ?? null);
    } catch (error) {
      setSteam({ status: "error", message: String(error) });
    }
  }

  async function pickSteamRoot() {
    const picked = await open({ directory: true, multiple: false, title: "选择 Steam 安装目录" });
    if (typeof picked === "string") await scanSteam(picked);
  }

  async function scanDesmume(emulatorRoot: string, romRoot?: string) {
    setDesmume({ status: "loading" });
    setSelectedDesmumeRom(null);
    try {
      const report = await api.scanDesmumeGames(emulatorRoot, romRoot);
      setDesmume({ status: "done", report });
      setSelectedDesmumeRom(report.games.find((game) => game.has_save)?.rom_path ?? report.games[0]?.rom_path ?? null);
    } catch (error) {
      setDesmume({ status: "error", message: String(error) });
    }
  }

  async function pickDesmumeRoot() {
    const picked = await open({ directory: true, multiple: false, title: "选择 DeSmuME 目录" });
    if (typeof picked === "string") await scanDesmume(picked);
  }

  async function pickDesmumeRomRoot() {
    if (desmume.status !== "done") return;
    const picked = await open({ directory: true, multiple: false, title: "选择 DeSmuME ROM 目录" });
    if (typeof picked === "string") await scanDesmume(desmume.report.emulator_root, picked);
  }

  async function addDesmumeGame(game: DesmumeDiscoveredGame) {
    if (desmume.status !== "done" || !game.has_save || saving) return;
    const candidate = game.matches[0];
    if (candidate?.already_bound_here) return;
    if (candidate?.match_kind === "possible") {
      const confirmed = window.confirm(
        `检测到可能对应“${candidate.game_name}”的 ROM，但 ROM 内容不同。\n\n不同版本的存档不一定兼容，确认绑定吗？`,
      );
      if (!confirmed) return;
    }
    setSaving(true);
    try {
      const created = await api.registerDesmumeGame(
        desmume.report.emulator_root,
        desmume.report.rom_root,
        game.rom_path,
        candidate?.game_id ?? null,
      );
      toast(candidate ? "已绑定 DeSmuME 游戏" : "DeSmuME 游戏已添加，去创建第一个快照吧", "ok");
      onCreated(created);
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setSaving(false);
    }
  }

  async function addDiscoveredGame(game: SteamDiscoveredGame) {
    if (!game.can_add_directly || game.already_added || saving) return;
    setSaving(true);
    try {
      const created = await api.addGame(game.name, game.save_paths);
      toast(game.save_paths.length > 1
        ? `游戏已添加，将共同保护 ${game.save_paths.length} 个存档目录`
        : "游戏已添加，去创建第一个快照吧", "ok");
      onCreated(created);
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setSaving(false);
    }
  }

  async function pickManualDir() {
    const picked = await open({ directory: true, multiple: false, title: "选择存档目录" });
    if (typeof picked === "string") {
      setPath(picked);
      setScan({ state: "idle", text: "目录未检测。" });
    }
  }

  async function testRead() {
    if (!path.trim()) return setScan({ state: "error", text: "请先选择存档目录。" });
    setScan({ state: "loading", text: "正在读取目录…" });
    try {
      const result = await api.scanPath(path.trim());
      setScan({ state: "done", text: `已检测到：${result.file_count} 个文件，${formatSize(result.total_size)}` });
    } catch {
      setScan({ state: "error", text: "无法访问该目录，请重新选择。" });
    }
  }

  async function addManualGame() {
    if (!name.trim()) return toast("请填写游戏名称", "err");
    if (!path.trim()) return toast("请至少选择一个存档目录", "err");
    setSaving(true);
    try {
      const created = await api.addGame(name.trim(), [path.trim()]);
      toast("游戏已添加，去创建第一个快照吧", "ok");
      onCreated(created);
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="overlay" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div className="modal add-game-modal">
        <div className="modal-head">
          <h3>添加游戏</h3>
          <button className="iconbtn" title="关闭" onClick={onClose}><Icon.Close /></button>
        </div>
        <div className="add-game-tabs" role="tablist" aria-label="添加方式">
          <button className={mode === "steam" ? "active" : ""} onClick={() => setMode("steam")}>
            <Icon.Search /> Steam 自动发现
          </button>
          <button className={mode === "desmume" ? "active" : ""} onClick={() => setMode("desmume")}>
            <Icon.Gamepad /> DeSmuME
          </button>
          <button className={mode === "manual" ? "active" : ""} onClick={() => setMode("manual")}>
            <Icon.Folder /> 手动添加
          </button>
        </div>

        {mode === "steam" ? (
          <SteamDiscoveryBody
            state={steam}
            selectedAppId={selectedAppId}
            onSelect={setSelectedAppId}
            onRefresh={() => scanSteam(steam.status === "done" ? steam.report.steam_root : undefined)}
            onPickRoot={pickSteamRoot}
          />
        ) : mode === "desmume" ? (
          <DesmumeDiscoveryBody
            state={desmume}
            selectedRomPath={selectedDesmumeRom}
            onSelect={setSelectedDesmumeRom}
            onPickRoot={pickDesmumeRoot}
            onPickRomRoot={pickDesmumeRomRoot}
            onRefresh={() => desmume.status === "done" && scanDesmume(desmume.report.emulator_root, desmume.report.rom_root ?? undefined)}
          />
        ) : (
          <div className="modal-body add-manual-body">
            <div className="field">
              <label>游戏名称</label>
              <input className="input" value={name} autoFocus
                onChange={(event) => setName(event.target.value)} placeholder="例如：艾尔登法环" />
            </div>
            <div className="field">
              <label>存档目录</label>
              <input className="input path-mono" value={path} placeholder="选择或输入存档目录"
                onChange={(event) => {
                  setPath(event.target.value);
                  setScan({ state: "idle", text: "目录未检测。" });
                }} />
              <div className="field-actions">
                <button className="btn sm" onClick={pickManualDir}><Icon.Folder /> 选择目录</button>
                <button className="btn sm" onClick={testRead} disabled={scan.state === "loading"}>
                  {scan.state === "loading"
                    ? <><span className="spin"><Icon.RotateCcw /></span> 正在读取</>
                    : <><Icon.Camera /> 测试读取</>}
                </button>
              </div>
              <div className={`hint ${scan.state === "done" ? "ok" : scan.state === "error" ? "err" : "muted"}`}>
                {scan.state === "done" && <Icon.CheckCircle />}
                {scan.state === "error" && <Icon.Alert />}
                <span>{scan.text}</span>
              </div>
            </div>
            <div className="field">
              <label>快照仓库</label>
              <div className="target-box"><span className="path-mono">
                使用默认位置：{repositoryPath || "正在获取默认位置…"}
              </span></div>
            </div>
          </div>
        )}

        <div className="modal-foot">
          <button className="btn" onClick={onClose} disabled={saving}>取消</button>
          <button className="btn primary"
            onClick={() => mode === "steam"
              ? selected && addDiscoveredGame(selected)
              : mode === "desmume"
                ? selectedDesmume && addDesmumeGame(selectedDesmume)
                : addManualGame()}
            disabled={saving
              || (mode === "steam" && (!selected?.can_add_directly || selected.already_added))
              || (mode === "desmume" && (!selectedDesmume
                || !selectedDesmume.has_save
                || selectedDesmume.matches[0]?.already_bound_here))}>
            {saving ? "添加中…"
              : mode === "steam" && selected?.already_added ? "已经添加"
                : mode === "desmume" && selectedDesmume?.matches[0]?.already_bound_here
                    ? "已经添加"
                    : mode === "desmume" && selectedDesmume?.matches[0]
                        ? "绑定游戏"
                        : "添加游戏"}
          </button>
        </div>
      </div>
    </div>
  );
}

function DesmumeDiscoveryBody({ state, selectedRomPath, onSelect, onPickRoot, onPickRomRoot, onRefresh }: {
  state: DesmumeState;
  selectedRomPath: string | null;
  onSelect: (romPath: string) => void;
  onPickRoot: () => void;
  onPickRomRoot: () => void;
  onRefresh: () => void;
}) {
  if (state.status === "idle") {
    return (
      <div className="steam-discovery-state">
        <Icon.Gamepad size={28} />
        <strong>选择 DeSmuME 目录</strong>
        <button className="btn primary" onClick={onPickRoot}><Icon.Folder /> 选择目录</button>
      </div>
    );
  }
  if (state.status === "loading") {
    return <div className="steam-discovery-state"><span className="spin"><Icon.RotateCcw size={22} /></span><span>正在读取 ROM 与存档</span></div>;
  }
  if (state.status === "error") {
    return (
      <div className="steam-discovery-state">
        <Icon.Alert size={26} />
        <strong>未能读取 DeSmuME</strong>
        <span className="steam-error">{state.message}</span>
        <button className="btn primary" onClick={onPickRoot}><Icon.Folder /> 重新选择</button>
      </div>
    );
  }

  if (!state.report.rom_root) {
    return (
      <div className="steam-discovery-state">
        <Icon.Alert size={26} />
        <strong>ROM 目录不可用</strong>
        {state.report.configured_rom_root && (
          <span className="path-mono steam-error">{state.report.configured_rom_root}</span>
        )}
        <button className="btn primary" onClick={onPickRomRoot}><Icon.Folder /> 选择 ROM 目录</button>
        <button className="btn sm" onClick={onPickRoot}>更换 DeSmuME 目录</button>
      </div>
    );
  }

  const selected = state.report.games.find((game) => game.rom_path === selectedRomPath) ?? null;
  return (
    <div className="steam-discovery-body">
      <div className="steam-scan-head">
        <div>
          <strong>{state.report.games.filter((game) => game.has_save).length} 个游戏已有存档</strong>
          <span className="path-mono" title={state.report.emulator_root}>{state.report.emulator_root}</span>
        </div>
        <button className="iconbtn" title="重新扫描" onClick={onRefresh}><Icon.RotateCcw /></button>
        <button className="btn sm" onClick={onPickRomRoot}><Icon.Folder /> ROM 目录</button>
        <button className="btn sm" onClick={onPickRoot}><Icon.Gamepad /> 模拟器目录</button>
      </div>
      {state.report.games.length === 0 ? (
        <div className="steam-discovery-state"><Icon.Search size={26} /><strong>没有发现 NDS ROM</strong></div>
      ) : (
        <div className="steam-discovery-layout">
          <div className="steam-game-list">
            {state.report.games.map((game) => {
              const candidate = game.matches[0];
              return (
                <button key={game.rom_path} className={`steam-game-row ${selectedRomPath === game.rom_path ? "active" : ""}`}
                  onClick={() => onSelect(game.rom_path)}>
                  <span className="game-cover">{game.name[0] ?? "游"}</span>
                  <span className="steam-game-copy">
                    <strong>{game.name}</strong>
                    <span>{game.has_save ? "已找到 .dsv 存档" : "尚无 .dsv 存档"}</span>
                  </span>
                  {candidate?.already_bound_here && <span className="steam-added"><Icon.Check size={13} /> 已添加</span>}
                </button>
              );
            })}
          </div>
          <div className="steam-game-detail">
            {selected && <>
              <div className="steam-detail-title">
                <strong>{selected.name}</strong>
                <span>{selected.rom_header_title || "无内部标题"} · {selected.rom_game_code}</span>
              </div>
              <PathGroup title="ROM" paths={[selected.rom_path]} />
              <PathGroup title="将保护的游戏内存档" paths={[selected.save_path]} />
              <div className="desmume-rom-id">
                <span>SHA-256</span>
                <code title={selected.rom_sha256}>{selected.rom_sha256}</code>
              </div>
              {!selected.has_save && (
                <div className="hint muted"><Icon.Alert /><span>该游戏尚未生成 .dsv 存档</span></div>
              )}
              {selected.matches[0]?.match_kind === "exact" && !selected.matches[0].already_bound_here && (
                <div className="hint ok"><Icon.CheckCircle /><span>与“{selected.matches[0].game_name}”的 ROM 完全一致</span></div>
              )}
              {selected.matches[0]?.match_kind === "possible" && (
                <div className="hint err"><Icon.Alert /><span>可能对应“{selected.matches[0].game_name}”，绑定时需要确认</span></div>
              )}
            </>}
          </div>
        </div>
      )}
    </div>
  );
}

function SteamDiscoveryBody({ state, selectedAppId, onSelect, onRefresh, onPickRoot }: {
  state: SteamState;
  selectedAppId: number | null;
  onSelect: (appId: number) => void;
  onRefresh: () => void;
  onPickRoot: () => void;
}) {
  if (state.status === "loading") {
    return <div className="steam-discovery-state"><span className="spin"><Icon.RotateCcw size={22} /></span><span>正在扫描 Steam 游戏</span></div>;
  }
  if (state.status === "error") {
    return (
      <div className="steam-discovery-state">
        <Icon.Alert size={26} />
        <strong>未能自动找到 Steam</strong>
        <span className="steam-error">{state.message}</span>
        <button className="btn primary" onClick={onPickRoot}><Icon.Folder /> 选择 Steam 目录</button>
      </div>
    );
  }

  const selected = state.report.games.find((game) => game.app_id === selectedAppId) ?? null;
  return (
    <div className="steam-discovery-body">
      <div className="steam-scan-head">
        <div>
          <strong>{state.report.games.length} 个游戏可识别</strong>
          <span className="path-mono" title={state.report.steam_root}>{state.report.steam_root}</span>
        </div>
        <button className="iconbtn" title="重新扫描" onClick={onRefresh}><Icon.RotateCcw /></button>
        <button className="btn sm" onClick={onPickRoot}><Icon.Folder /> 更换目录</button>
      </div>
      {state.report.games.length === 0 ? (
        <div className="steam-discovery-state"><Icon.Search size={26} /><strong>没有发现可添加的游戏</strong></div>
      ) : (
        <div className="steam-discovery-layout">
          <div className="steam-game-list">
            {state.report.games.map((game) => (
              <button key={game.app_id} className={`steam-game-row ${selectedAppId === game.app_id ? "active" : ""}`}
                onClick={() => onSelect(game.app_id)}>
                <span className="game-cover">{game.name[0] ?? "游"}</span>
                <span className="steam-game-copy">
                  <strong>{game.name}</strong>
                  <span>{game.save_paths.length} 个存档目录</span>
                </span>
                {game.already_added && <span className="steam-added"><Icon.Check size={13} /> 已添加</span>}
              </button>
            ))}
          </div>
          <div className="steam-game-detail">
            {selected && <>
              <div className="steam-detail-title">
                <strong>{selected.name}</strong>
                <span>Steam AppID {selected.app_id}</span>
              </div>
              <PathGroup title="将保护的存档目录" paths={selected.save_paths} />
              {selected.config_paths.length > 0 && (
                <PathGroup title="不纳入快照的纯配置路径" paths={selected.config_paths} muted />
              )}
              {!selected.can_add_directly && (
                <div className="hint err"><Icon.Alert /><span>没有找到当前可读取的存档目录</span></div>
              )}
            </>}
          </div>
        </div>
      )}
    </div>
  );
}

function PathGroup({ title, paths, muted = false }: { title: string; paths: string[]; muted?: boolean }) {
  return (
    <div className={`steam-path-group ${muted ? "muted" : ""}`}>
      <div className="steam-path-label">{title}<span>{paths.length}</span></div>
      {paths.map((item, index) => (
        <div className="steam-path" key={`${index}-${item}`} title={item}>
          <Icon.Folder size={14} /><span className="path-mono">{item}</span>
        </div>
      ))}
    </div>
  );
}

function firstAvailableGame(report: SteamDiscoveryReport): SteamDiscoveredGame | undefined {
  return report.games.find((game) => game.can_add_directly && !game.already_added);
}
