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
  ProgramDiscoveredGame,
  ProgramDiscoveryReport,
  SteamDiscoveredGame,
  SteamDiscoveryReport,
} from "../lib/types";

interface Props {
  onClose: () => void;
  onCreated: (game: Game) => void;
  initialMode?: AddGameMode;
}

export type AddGameMode = "steam" | "program" | "desmume" | "manual";
type SteamState =
  | { status: "loading" }
  | { status: "done"; report: SteamDiscoveryReport }
  | { status: "error"; message: string };
type DesmumeState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "done"; report: DesmumeDiscoveryReport }
  | { status: "error"; message: string };
type ProgramState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "done"; report: ProgramDiscoveryReport }
  | { status: "error"; message: string };

export function AddGameDialog({ onClose, onCreated, initialMode = "steam" }: Props) {
  const toast = useToast();
  const [mode, setMode] = useState<AddGameMode>(initialMode);
  const [steam, setSteam] = useState<SteamState>({ status: "loading" });
  const [program, setProgram] = useState<ProgramState>({ status: "idle" });
  const [desmume, setDesmume] = useState<DesmumeState>({ status: "idle" });
  const [selectedAppId, setSelectedAppId] = useState<number | null>(null);
  const [selectedProgramKey, setSelectedProgramKey] = useState<string | null>(null);
  const [programName, setProgramName] = useState("");
  const [useDiscoveredPaths, setUseDiscoveredPaths] = useState(true);
  const [selectedDesmumeRom, setSelectedDesmumeRom] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [manualExecutablePath, setManualExecutablePath] = useState("");
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
  const selectedProgram = useMemo(() => {
    if (program.status !== "done") return null;
    return program.report.games.find((game) => programKey(game) === selectedProgramKey) ?? null;
  }, [program, selectedProgramKey]);

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

  async function scanProgram(selectedPath: string) {
    setProgram({ status: "loading" });
    setSelectedProgramKey(null);
    try {
      const report = await api.scanProgramGame(selectedPath);
      setProgram({ status: "done", report });
      const first = firstAvailableProgram(report) ?? report.games[0];
      setSelectedProgramKey(first ? programKey(first) : null);
      setProgramName(first?.name ?? report.suggested_name);
      setUseDiscoveredPaths(Boolean(first?.can_add_directly));
    } catch (error) {
      setProgram({ status: "error", message: String(error) });
    }
  }

  async function pickProgramFile() {
    const picked = await open({
      directory: false,
      multiple: false,
      title: "选择游戏快捷方式或 EXE",
      filters: [{ name: "游戏程序或快捷方式", extensions: ["exe", "lnk"] }],
    });
    if (typeof picked === "string") await scanProgram(picked);
  }

  function selectProgram(key: string) {
    setSelectedProgramKey(key);
    if (program.status !== "done") return;
    const game = program.report.games.find((candidate) => programKey(candidate) === key);
    if (!game) return;
    setProgramName(game.name);
    setUseDiscoveredPaths(game.can_add_directly);
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
    if (!game.can_add_directly || saving) return;
    if (game.already_added && !game.can_bind_existing_launch) return;
    if (game.already_added && game.existing_game_name
      && !window.confirm(`将把 Steam 启动方式绑定到“${game.existing_game_name}”，不创建新游戏。继续吗？`)) return;
    setSaving(true);
    try {
      const created = await api.registerSteamGame(
        game.name,
        game.save_paths,
        steam.status === "done" ? steam.report.steam_root : "",
        game.install_dir,
        game.app_id,
      );
      toast(game.already_added ? "已为现有游戏补充 Steam 启动方式" : "游戏已添加，去创建第一个快照吧", "ok");
      onCreated(created);
    } catch (error) {
      toast(String(error), "err");
    } finally {
      setSaving(false);
    }
  }

  async function addSelectedProgram() {
    if (program.status !== "done" || saving) return;
    const executablePath = program.report.resolved_program_path;
    if (!programName.trim()) return toast("请填写游戏名称", "err");
    if (program.report.program_already_added) {
      if (!program.report.existing_game_id || !program.report.can_bind_existing_launch) return;
      if (!window.confirm(`将把这个 EXE 绑定到“${program.report.existing_game_name ?? "现有游戏"}”，不创建新游戏。继续吗？`)) return;
      setSaving(true);
      try {
        const bound = await api.bindProgramToGame(
          program.report.existing_game_id,
          executablePath ?? "",
          program.report.install_dir || null,
          false,
        );
        toast("已为现有游戏补充启动方式", "ok");
        onCreated(bound);
      } catch (error) {
        toast(String(error), "err");
      } finally {
        setSaving(false);
      }
      return;
    }
    if (selectedProgram?.already_added) return;
    const savePaths = selectedProgram && useDiscoveredPaths ? selectedProgram.save_paths : [];
    if (!executablePath) return toast("请选择实际的游戏 EXE，所有游戏都需要启动方式", "warn");
    setSaving(true);
    try {
      const created = await api.addProgramGame(
          programName.trim(),
          savePaths,
          executablePath,
          program.report.install_dir,
        );
      toast(savePaths.length > 0
        ? "游戏及存档目录已添加"
        : "游戏已添加，首次游玩后再设置存档目录", "ok");
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

  async function pickManualProgram() {
    const picked = await open({
      directory: false,
      multiple: false,
      title: "选择游戏 EXE",
      filters: [{ name: "游戏程序", extensions: ["exe"] }],
    });
    if (typeof picked === "string") setManualExecutablePath(picked);
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
    if (!manualExecutablePath.trim()) return toast("请选择游戏 EXE，所有游戏都需要启动方式", "err");
    setSaving(true);
    try {
      const created = await api.addGame(
        name.trim(),
        [path.trim()],
        manualExecutablePath.trim(),
        manualExecutablePath.trim().replace(/[\\/][^\\/]+$/, ""),
      );
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
          <button className={mode === "program" ? "active" : ""} onClick={() => setMode("program")}>
            <Icon.FileSearch /> 游戏程序
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
        ) : mode === "program" ? (
          <ProgramDiscoveryBody
            state={program}
            selectedKey={selectedProgramKey}
            onSelect={selectProgram}
            name={programName}
            onNameChange={setProgramName}
            useDiscoveredPaths={useDiscoveredPaths}
            onUseDiscoveredPathsChange={setUseDiscoveredPaths}
            onPickFile={pickProgramFile}
            onRefresh={() => program.status === "done" && scanProgram(program.report.selected_path)}
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
              <label>游戏程序</label>
              <input className="input path-mono" value={manualExecutablePath} placeholder="请选择游戏 EXE"
                onChange={(event) => setManualExecutablePath(event.target.value)} />
              <div className="field-actions">
                <button className="btn sm" onClick={pickManualProgram}><Icon.FileSearch /> 选择 EXE</button>
              </div>
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
              : mode === "program"
                ? addSelectedProgram()
              : mode === "desmume"
                ? selectedDesmume && addDesmumeGame(selectedDesmume)
                : addManualGame()}
            disabled={saving
              || (mode === "steam" && (!selected?.can_add_directly
                || (selected.already_added && !selected.can_bind_existing_launch)))
              || (mode === "program" && (program.status !== "done"
                || !programName.trim()
                || (program.report.program_already_added && !program.report.can_bind_existing_launch)
                || (Boolean(selectedProgram?.already_added) && !program.report.can_bind_existing_launch)
                || !program.report.resolved_program_path))
              || (mode === "manual" && (!name.trim() || !path.trim() || !manualExecutablePath.trim()))
              || (mode === "desmume" && (!selectedDesmume
                || !selectedDesmume.has_save
                || selectedDesmume.matches[0]?.already_bound_here))}>
            {saving ? "添加中…"
              : mode === "steam" && selected?.already_added && selected.can_bind_existing_launch ? "绑定启动程序"
                : mode === "steam" && selected?.already_added ? "已经添加"
                : mode === "program" && program.status === "done"
                  && program.report.program_already_added && program.report.can_bind_existing_launch ? "绑定启动程序"
                : mode === "program" && program.status === "done"
                  && (program.report.program_already_added || selectedProgram?.already_added) ? "已经添加"
                  : mode === "program" && (!selectedProgram || !useDiscoveredPaths || !selectedProgram.can_add_directly)
                    ? "添加并等待查找"
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

function ProgramDiscoveryBody({
  state,
  selectedKey,
  onSelect,
  onPickFile,
  onRefresh,
  name,
  onNameChange,
  useDiscoveredPaths,
  onUseDiscoveredPathsChange,
}: {
  state: ProgramState;
  selectedKey: string | null;
  onSelect: (key: string) => void;
  onPickFile: () => void;
  onRefresh: () => void;
  name: string;
  onNameChange: (value: string) => void;
  useDiscoveredPaths: boolean;
  onUseDiscoveredPathsChange: (value: boolean) => void;
}) {
  if (state.status === "idle") {
    return (
      <div className="steam-discovery-state">
        <Icon.FileSearch size={28} />
        <strong>从游戏程序识别存档</strong>
        <div className="program-pick-actions">
          <button className="btn primary" onClick={onPickFile}><Icon.FileSearch /> 快捷方式或 EXE</button>
        </div>
      </div>
    );
  }
  if (state.status === "loading") {
    return <div className="steam-discovery-state"><span className="spin"><Icon.RotateCcw size={22} /></span><span>正在识别游戏与存档</span></div>;
  }
  if (state.status === "error") {
    return (
      <div className="steam-discovery-state">
        <Icon.Alert size={26} />
        <strong>未能识别游戏程序</strong>
        <span className="steam-error">{state.message}</span>
        <div className="program-pick-actions">
          <button className="btn primary" onClick={onPickFile}><Icon.FileSearch /> 重新选择程序</button>
        </div>
      </div>
    );
  }

  const selected = state.report.games.find((game) => programKey(game) === selectedKey) ?? null;
  const sourcePath = state.report.resolved_program_path ?? state.report.selected_path;
  return (
    <div className="steam-discovery-body">
      <div className="steam-scan-head">
        <div>
          <strong>{state.report.games.length} 个游戏匹配</strong>
          <span className="path-mono" title={state.report.selected_path}>{state.report.selected_path}</span>
        </div>
        <button className="iconbtn" title="重新识别" onClick={onRefresh}><Icon.RotateCcw /></button>
        <button className="btn sm" onClick={onPickFile}><Icon.FileSearch /> 程序</button>
      </div>
      {state.report.ignored_app_id_game_names.length > 0 && (
        <div className="hint muted"><Icon.Alert /><span>
          检测到配置 AppID {state.report.detected_app_id ?? "未知"}（规则库对应
          {state.report.ignored_app_id_game_names.join("、")}），但与程序或目录名称不一致，
          已忽略该身份和存档规则。
        </span></div>
      )}
      {state.report.games.length === 0 ? (
        <div className="program-discovery-empty">
          <Icon.Search size={26} />
          <strong>没有找到对应的存档规则</strong>
          {state.report.resolved_program_path ? <>
            <div className="program-name-field">
              <label>游戏名称</label>
              <input className="input" value={name} onChange={(event) => onNameChange(event.target.value)} />
            </div>
            <PathGroup title="游戏程序" paths={[state.report.resolved_program_path]} muted />
            {state.report.program_already_added ? (
              <div className="hint ok"><Icon.CheckCircle /><span>
                {state.report.can_bind_existing_launch
                  ? `已找到现有游戏“${state.report.existing_game_name ?? "现有游戏"}”，确认后补充启动程序`
                  : `已作为“${state.report.existing_game_name ?? "现有游戏"}”添加，请编辑现有游戏`}
              </span></div>
            ) : (
              <div className="hint muted"><Icon.Alert /><span>添加后标记为待设置存档目录，当前不会创建备份</span></div>
            )}
          </> : (
            <div className="hint err"><Icon.Alert /><span>请选择实际的游戏 EXE，游戏目录不能用于动态发现</span></div>
          )}
        </div>
      ) : (
        <div className="steam-discovery-layout">
          <div className="steam-game-list">
            {state.report.games.map((game) => (
              <button key={`${game.app_id}-${game.name}`}
                className={`steam-game-row ${selectedKey === programKey(game) ? "active" : ""}`}
                onClick={() => onSelect(programKey(game))}>
                <span className="game-cover">{game.name[0] ?? "游"}</span>
                <span className="steam-game-copy">
                  <strong>{game.name}</strong>
                  <span>{game.match_kind === "app_id" ? "AppID 精确识别" : "程序名称匹配"}</span>
                </span>
                {game.already_added && <span className="steam-added"><Icon.Check size={13} /> 已添加</span>}
              </button>
            ))}
          </div>
          <div className="steam-game-detail">
            {selected && <>
              <div className="steam-detail-title">
                <input className="input program-name-input" value={name}
                  onChange={(event) => onNameChange(event.target.value)} aria-label="游戏名称" />
                <span>Steam AppID {selected.app_id}</span>
              </div>
              <PathGroup title="选择的游戏程序" paths={[sourcePath]} muted />
              {state.report.install_dir !== sourcePath && (
                <PathGroup title="识别的游戏目录" paths={[state.report.install_dir]} muted />
              )}
              {selected.save_paths.length > 0 && (
                <PathGroup title="识别到的存档目录" paths={selected.save_paths} />
              )}
              {selected.config_paths.length > 0 && (
                <PathGroup title="不纳入快照的纯配置路径" paths={selected.config_paths} muted />
              )}
              {(state.report.program_already_added || selected.already_added) && (
                <div className="hint ok"><Icon.CheckCircle /><span>
                  {state.report.program_already_added && state.report.can_bind_existing_launch && state.report.existing_game_name
                    ? `已找到现有游戏“${state.report.existing_game_name}”，确认后补充启动程序`
                    : state.report.existing_game_name
                    ? `已作为“${state.report.existing_game_name}”添加，请编辑现有游戏`
                    : "这个游戏已经在 SaveLink 中管理"}
                </span></div>
              )}
              {selected.match_kind === "app_id" ? (
                <div className="hint ok"><Icon.CheckCircle /><span>已通过游戏 AppID 精确匹配</span></div>
              ) : (
                <div className="hint muted"><Icon.Alert /><span>根据程序或目录名称匹配，请确认游戏名称</span></div>
              )}
              {!selected.can_add_directly && (
                <div className="hint muted"><Icon.Alert /><span>当前没有可读取的存档目录，将在首次游玩后设置</span></div>
              )}
              {selected.can_add_directly && (
                <label className="program-path-choice">
                  <input type="checkbox" checked={useDiscoveredPaths}
                    onChange={(event) => onUseDiscoveredPathsChange(event.target.checked)} />
                  <span>直接使用本次识别到的存档目录</span>
                </label>
              )}
              {selected.can_add_directly && !useDiscoveredPaths && (
                <div className="hint muted"><Icon.Alert /><span>将忽略本次匹配，添加为待设置存档目录</span></div>
              )}
              {!state.report.resolved_program_path && useDiscoveredPaths && (
                <div className="hint muted"><Icon.Alert /><span>本次只添加识别到的存档目录，不保存游戏启动程序</span></div>
              )}
              {!state.report.resolved_program_path && !useDiscoveredPaths && (
                <div className="hint err"><Icon.Alert /><span>请选择实际的游戏 EXE 后再添加为待设置状态</span></div>
              )}
            </>}
          </div>
        </div>
      )}
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
              {selected.already_added && selected.can_bind_existing_launch && (
                <div className="hint ok"><Icon.CheckCircle /><span>
                  已找到现有游戏“{selected.existing_game_name ?? "现有游戏"}”，确认后补充 Steam 启动方式
                </span></div>
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

function firstAvailableProgram(report: ProgramDiscoveryReport): ProgramDiscoveredGame | undefined {
  return report.games.find((game) => !game.already_added);
}

function programKey(game: ProgramDiscoveredGame): string {
  return `${game.app_id}:${game.name}`;
}
