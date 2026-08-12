# SaveLink 打包说明（写给 Java 开发者）

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | 代强 | 2026-07-14 | 补齐版本历史；同步 core 50 个测试、Fake 云同步闭环和当前 rustfmt 状态 |
| 1.1 | 代强 | 2026-08-05 | 补充绿色版 ZIP 发布流程、SHA-256 产物和标签一致性检查 |
| 1.2 | 代强 | 2026-08-06 | 同步 v0.2.0 发布产物、当前构建状态和发布标签示例 |
| 1.3 | 代强 | 2026-08-12 | 同步 v0.3.0 绿色版、安装包产物和发布标签示例 |

> 目标读者：熟悉 Java/Maven，但不熟悉 Rust / 前端 / Tauri 这套技术栈的人。
> 下面用 Java 世界的概念来类比，帮你快速建立心智模型。

## 当前状态（2026-08-12）

面向 GitHub/Gitee Release 发布绿色版时，运行：

```bat
build-portable.bat
```

需要生成安装器时，运行：

```bat
build-installer.bat
```

两个脚本已经验证可以生成以下产物：

```text
src-tauri/target/release/savelink-app.exe
src-tauri/target/release/bundle/portable/SaveLink_0.3.0_windows_x64_portable.zip
src-tauri/target/release/bundle/portable/SaveLink_0.3.0_windows_x64_portable.zip.sha256.txt
src-tauri/target/release/bundle/nsis/SaveLink_0.3.0_x64-setup.exe
src-tauri/target/release/bundle/msi/SaveLink_0.3.0_x64_en-US.msi
```

绿色版 `savelink-app.exe` 和安装版使用同一个 Tauri identifier：`com.daiq.savelink`，所以共用同一个用户数据目录：

```text
%APPDATA%\com.daiq.savelink\
├── savelink.db
└── repository\
```

## 一句话：这是什么

SaveLink 是一个 **Tauri** 桌面应用。可以把 Tauri 理解成"轻量版 Electron"：

- **界面**：用网页技术写（React + TypeScript），运行时由 **系统自带的浏览器内核**（WebView2，即 Edge 内核）来渲染。
  - 类比：有点像 JavaFX 的 `WebView`，界面其实是 HTML/CSS/JS。
- **后端/业务逻辑**：用 **Rust** 写，编译成原生 `.exe`，体积小、启动快、不需要装 JVM。
  - 对比 Electron：Electron 把整个 Node + Chromium 打进去（动辄 100MB+）；Tauri 复用系统浏览器内核，所以安装包只有几 MB。

整个项目分三块：

| 目录 | 是什么 | Java 类比 |
|---|---|---|
| `src/` | 前端界面（React + TypeScript） | 相当于"展示层" |
| `src-tauri/` | Rust 写的桌面外壳，调用业务逻辑 | 相当于 `main()` + 胶水层 |
| `../savelink-core/` | 纯业务逻辑（Rust，被 50 个测试保护） | 相当于一个独立的 `core` 模块/jar |

## 工具链对照表（重点）

| 这套技术栈 | 作用 | Java 里的对应物 |
|---|---|---|
| **Node.js / npm** | 跑前端构建、管理前端依赖 | JDK + Maven/Gradle |
| `package.json` | 前端依赖与脚本清单 | `pom.xml` |
| `node_modules/` | 下载下来的前端依赖 | 本地 `~/.m2` 仓库（但放在项目内） |
| **Rust / Cargo** | 编译 Rust、管理 Rust 依赖 | javac + Maven/Gradle |
| `Cargo.toml` | Rust 依赖清单 | `pom.xml` |
| `Cargo.lock` | 锁定依赖精确版本 | `gradle.lockfile` |
| crates.io | Rust 的公共依赖仓库 | Maven Central |
| **MSVC 生成工具 + Windows SDK** | 把 Rust 编译产物链接成本地 `.exe` | （Java 没有对应物，见下） |
| **WebView2** | 渲染界面的浏览器内核 | JavaFX WebView |

### 为什么 Java 没有"MSVC / 链接器"这一步？

Java 编译成 **字节码**，靠 JVM 跨平台，运行时不碰底层系统库。
Rust 编译成 **本地机器码**（直接是 Windows 的 `.exe`），所以最后必须有一个"**链接器**"把编译出来的目标文件和 Windows 系统库拼到一起——在 Windows 上这个链接器就是微软的 **MSVC**（`link.exe`）。
这就是为什么本机必须装 **Visual Studio 生成工具**，否则 `cargo build` 到最后一步会失败。
（类比：只有当你在 Java 里写 JNI、要编译本地 `.dll/.so` 时，才会需要 gcc/cl 这种本地工具链——Tauri 等于全程都在做这件事。）

## 打包时实际发生了什么

运行 `build-installer.bat`（或 `npm run tauri build`）时，按顺序发生：

1. **前端构建**（`npm run build` → `tsc && vite build`）
   - `tsc`：TypeScript 编译检查（≈ `javac` 的类型检查）
   - `vite build`：把 React 代码打包压缩成静态网页，输出到 `dist/`
   - 产物：`dist/index.html` + 一个 JS、一个 CSS（≈ 前端的"可部署制品"）

2. **Rust 编译**（`cargo build --release`）
   - 编译 Tauri 外壳 + `savelink-core` 业务逻辑 + 所有依赖
   - 把第 1 步的 `dist/` 网页资源**嵌进** exe
   - 产物：`src-tauri/target/release/savelink-app.exe`（绿色可执行版，约 11MB）
   - ⚠️ 首次很慢：要从零编译几百个 Rust 依赖（类比第一次 `mvn install` 拉满整个依赖树并全部编译）

3. **打包成安装器**（Tauri bundler）
   - 用 **WiX** 工具生成 `.msi`
   - 用 **NSIS** 工具生成向导式 `setup.exe`
   - 这两个工具是 Tauri 第一次构建时从 GitHub 自动下载的（所以首次打包网络要通）

## 产物：绿色版与两个安装包的区别

| 文件 | 在哪 | 特点 | 什么时候用 |
|---|---|---|---|
| `savelink-app.exe` | `src-tauri/target/release/` | 绿色版，免安装，直接双击运行 | 本机快速验收、临时试用 |
| `SaveLink_x.y.z_windows_x64_portable.zip` | `bundle/portable/` | 包含 `SaveLink.exe` 和用户 README，并生成 SHA-256 | **GitHub/Gitee Release 面向普通用户分发，首选** |
| `SaveLink_x.y.z_x64-setup.exe` | `bundle/nsis/` | 向导式安装，可免管理员按用户安装，体积小 | 需要安装向导时使用 |
| `SaveLink_x.y.z_x64_en-US.msi` | `bundle/msi/` | 标准 MSI，可 `msiexec` 静默安装、组策略批量部署 | 企业/批量部署 |

注意：绿色版不是“独立数据沙箱”。它和安装版读写同一份 `%APPDATA%\com.daiq.savelink\`。

## 本机环境要求（已经装好了）

- Node.js LTS（`C:\Program Files\nodejs`）
- Rust 工具链（`~/.cargo`，MSVC 变体）
- Visual Studio 2022 生成工具 + Windows 11 SDK（MSVC 链接器）
- WebView2 运行时（Win10/11 一般自带）

## 两个曾经踩到的坑（已解决，记录备查）

1. **直连 crates.io 不稳，`curl failed`**
   - 已在 `C:\Users\daiqiang\.cargo\config.toml` 配置 **rsproxy 国内镜像**，下载走镜像即可。

2. **智能应用控制（Smart App Control）拦截编译**
   - Windows 11 的智能应用控制会拦截 Rust 编译过程中临时生成的、无签名的小程序（`build-script-build.exe`），报"应用程序控制策略已阻止此文件 (os error 4551)"。
   - 已**手动关闭**智能应用控制。注意：该开关一旦关闭，需重置/重装 Windows 才能重新开启。

3. **exe 被正在运行的 SaveLink 占用**
   - 现象：打包时无法覆盖 `src-tauri/target/release/savelink-app.exe`，可能出现 `os error 5` 或类似文件占用错误。
   - 当前 `build-installer.bat` 已在构建前关闭运行中的 SaveLink，并对锁文件类瞬时失败做重试。

4. **cargo fmt / rustfmt**
   - 当前 `rustfmt` 已可用，本轮新增 Rust 文件已单独格式化。
   - 全量 `cargo fmt --check` 会报告旧文件历史格式差异；建议未来单独做一次格式化提交，不要与业务改动混在一起。

## 以后怎么重新打包

不需要重装任何东西，根据目标选择：

- **公开发布绿色版**：双击 `build-portable.bat`，生成带 README 的 ZIP 和 SHA-256 文件。
- **需要安装器**：双击 `build-installer.bat`。它会自动定位 MSVC 环境 → 构建前自动关闭正在运行的 SaveLink（否则链接器无法覆盖 exe）→ 对"网络掉线/文件占用"等**瞬时**失败智能重试，但遇到真正的 TypeScript/Rust 编译错误会立即停下并提示 → 打完自动打开产物文件夹。
- 或在能找到 MSVC 环境的终端里手动执行：
  ```
  npm run tauri build
  ```
  （若直接跑报链接器找不到，先执行一次
  `"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"`
  再跑构建——`build-installer.bat` 已经帮你自动做了这一步。）

## 正式发布前的一致性检查

1. `git status` 必须干净。
2. 发布标签必须指向准备打包的提交，例如 `git rev-list -n 1 v0.3.0` 与当前 `HEAD` 一致。
3. 从该提交执行 `build-portable.bat`，不要复用更早生成的 ZIP。
4. 打开绿色版做启动、托盘、创建快照、恢复和百度授权冒烟。
5. 上传 ZIP 与 `.sha256.txt` 到同一个 Release，并核对 SHA-256。

## 开发时怎么跑（不打包，热重载）

```
npm run tauri dev
```
相当于"开发模式"：改前端代码会热刷新，改 Rust 代码会自动重编重启。≈ Spring Boot DevTools 的体验。
