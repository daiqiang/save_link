# SaveLink 打包说明（写给 Java 开发者）

> 目标读者：熟悉 Java/Maven，但不熟悉 Rust / 前端 / Tauri 这套技术栈的人。
> 下面用 Java 世界的概念来类比，帮你快速建立心智模型。

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
| `../savelink-core/` | 纯业务逻辑（Rust，被 33 个测试焊死） | 相当于一个独立的 `core` 模块/jar |

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

## 产物：两个安装包的区别

都在 `src-tauri/target/release/bundle/` 下：

| 文件 | 在哪 | 特点 | 什么时候用 |
|---|---|---|---|
| `SaveLink_x.y.z_x64-setup.exe` | `bundle/nsis/` | 向导式安装，可免管理员按用户安装，体积小 | **给普通用户分发，首选** |
| `SaveLink_x.y.z_x64_en-US.msi` | `bundle/msi/` | 标准 MSI，可 `msiexec` 静默安装、组策略批量部署 | 企业/批量部署 |

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

## 以后怎么重新打包

不需要重装任何东西，二选一：

- **双击** `build-installer.bat`（推荐）。它会：自动定位 MSVC 环境 → 构建前自动关闭正在运行的 SaveLink（否则链接器无法覆盖 exe，会报 `LNK1104` 文件占用）→ 对"网络掉线/文件占用"等**瞬时**失败智能重试，但遇到真正的 TypeScript/Rust 编译错误会立即停下并提示 → 打完自动打开产物文件夹
- 或在能找到 MSVC 环境的终端里手动执行：
  ```
  npm run tauri build
  ```
  （若直接跑报链接器找不到，先执行一次
  `"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"`
  再跑构建——`build-installer.bat` 已经帮你自动做了这一步。）

## 开发时怎么跑（不打包，热重载）

```
npm run tauri dev
```
相当于"开发模式"：改前端代码会热刷新，改 Rust 代码会自动重编重启。≈ Spring Boot DevTools 的体验。
