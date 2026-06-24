# savelink-app — SaveLink 桌面应用（Tauri + React）

SaveLink 的桌面外壳。前端 React 在 `src/`，Rust 命令层在 `src-tauri/`，
核心逻辑在隔壁 `../savelink-core`（路径依赖，保持纯净、可独立测试）。

## 结构

```
savelink-app/
├── src/                     React 前端（TS）
│   ├── App.tsx              主壳：顶栏 + 左栏游戏列表 + 右栏时间线
│   ├── App.css              设计令牌 + 全部样式（工具气质，参照视觉规范）
│   ├── lib/
│   │   ├── types.ts         与 Rust DTO 对齐的类型
│   │   ├── api.ts           ★ 数据访问层：现已调真 invoke。换后端只改这一处
│   │   ├── format.ts        大小/标签格式化
│   │   └── icons.tsx        内联 SVG 图标（零依赖）
│   └── components/          Toast / AddGameDialog / RestoreDialog / SnapshotDrawer
└── src-tauri/
    ├── src/
    │   ├── lib.rs           Tauri 入口：注册插件、初始化 AppState、注册命令
    │   └── commands.rs      ★ 命令层（薄壳）：DTO + AppState + 8 个 #[tauri::command]
    ├── Cargo.toml           依赖 savelink-core、tauri-plugin-dialog、chrono、rusqlite
    ├── tauri.conf.json      productName=SaveLink，identifier=com.daiq.savelink
    └── capabilities/        权限（含 dialog:default 用于目录选择器）
```

## 开发 / 构建

```bash
npm install
npm run tauri dev      # 真桌面窗口（开发）。⚠️ 纯 npm run dev 没有 invoke，命令会报错
npm run build          # 仅前端 tsc+vite 构建（验证前端编译）
npm run tauri build    # 打包安装包（MSI + NSIS + 独立 exe）
```

产物：`src-tauri/target/release/`（独立 exe）与 `.../bundle/`（msi、nsis 安装包）。
运行时数据：`%APPDATA%/com.daiq.savelink/`（savelink.db + repository/）。

## 给后续开发者 / Codex 的要点

- **前端只通过 `lib/api.ts` 访问后端，不在组件里直接 `invoke`。** 加新功能时，
  先在 api.ts 加函数（对应一个 Rust 命令），组件调它。这是当初让「假数据→真命令」一次替换的设计。
- **新增命令的三件套**：① `savelink-core` 加/改逻辑并补测试 → ② `commands.rs` 包一层 DTO 命令
  → ③ `lib.rs` 的 `generate_handler!` 注册 → ④ 前端 `api.ts` 加调用。
- **DTO 必须与前端 `types.ts` 对齐**。Tauri v2 参数名用 snake_case 传递（如 `{ gameId }` 对应 `game_id`）。
- 详细交接见 `../HANDOFF-codex.md`。
