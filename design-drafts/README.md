# design-drafts 目录说明

## 版本历史

| 版本 | 修改人 | 时间 | 备注 |
| --- | --- | --- | --- |
| 1.0 | 代强 | 2026-07-29 | 第一版：说明 SaveLink 图标设计稿、生成脚本和正式图标之间的关系 |

## 目录用途

本目录保存 SaveLink 视觉设计过程中的草图、中间稿、对比稿、局部检查图和生成脚本。目前主要内容是 SaveLink 云链图标从早期概念到 V9 定稿的完整设计过程。

本目录不参与 SaveLink 运行或打包。它的作用是保留设计依据，方便将来追溯图标为什么这样设计，或者在 V9 母版基础上继续调整。

## 当前结构

| 路径 | 用途 |
| --- | --- |
| `icon/savelink-icon-blue-tile-approved-v9.svg` | SaveLink 云链 V9 的定稿矢量母版 |
| `icon/*.svg` | 云朵、链条、间距和构图调整过程中的矢量版本 |
| `icon/*.py` | 生成或调整各版图标的 Python 脚本 |
| `icon/gpt-image/*.png` | GPT Image 生成稿、人工修改稿、缩略预览和版本对比图 |
| `icon/gpt-image/_inspection-*.png` | 为检查链条连接、留白、居中和局部形状而生成的放大图 |
| `icon/__pycache__/` | Python 运行产生的缓存，不是设计源文件 |

## 正式应用图标位置

定稿图标已经复制并转换到应用实际使用的位置：

- Web favicon：`savelink-app/public/savelink.svg`
- Tauri/Windows 图标：`savelink-app/src-tauri/icons/`
- 安装包图标配置：`savelink-app/src-tauri/tauri.conf.json`

应用构建只使用上述正式位置，不会从 `design-drafts/` 动态读取图标。因此修改本目录中的草稿不会自动改变 SaveLink，需要重新生成并替换正式图标资源。

## 维护约定

- 保留 V9 矢量母版和对应 PNG，后续设计应从定稿母版继续演进。
- 新方案使用清晰的版本号命名，不要直接覆盖已确认的 V9 文件。
- GPT 生成稿和 `_inspection-*` 文件属于设计过程证据，不应作为程序资源直接引用。
- Python 缓存可以重新生成，不应被当作图标源文件。
- 发布前以 `savelink-app/public/`、`savelink-app/src-tauri/icons/` 和 `tauri.conf.json` 为准，不以本目录中的预览图为准。

