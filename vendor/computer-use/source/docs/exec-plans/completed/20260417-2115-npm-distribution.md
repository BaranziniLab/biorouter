# npm 分发与发布执行计划

## 目标

让 `open-computer-use` 这套本地 macOS `computer-use` MCP server 能以 npm 形式直接分发，用户可以通过 `open-computer-use`、`open-computer-use-mcp` 或 `open-codex-computer-use-mcp` 任一包名完成安装，并拿到可直接运行的预编译产物与 Codex 插件安装入口。

## 范围

- 包含：
  - 为仓库补一套真实可发布的 npm 打包与发布脚本。
  - 产出可直接安装的预编译 `.app` 分发物，而不是只发源码。
  - 支持三个 npm 包名发布到同一套内容。
  - 更新 README、release 打包脚本、history 等仓库文档。
- 不包含：
  - 完整的 code signing / notarization。
  - 自动修改用户系统权限设置。
  - Windows / Linux 支持。

## 背景

- 相关文档：
  - `docs/ARCHITECTURE.md`
  - `docs/CICD.md`
  - `docs/SECURITY.md`
  - `docs/SUPPLY_CHAIN_SECURITY.md`
- 相关代码路径：
  - `scripts/build-open-computer-use-app.sh`
  - `scripts/release-package.sh`
  - `scripts/install-codex-plugin.sh`
  - `plugins/open-computer-use/`
- 已知约束：
  - 当前仓库主体是 Swift 可执行程序，不是 Node 项目。
  - 现有 release 打包脚本还是占位实现，不代表真实构建产物。
  - npm 分发要尽量降低门槛，不能要求用户本地再装一套 Swift 构建链。

## 风险

- 风险：只发布源码 wrapper，会把安装门槛继续留给用户。
- 缓解方式：npm 包里直接携带预编译 `.app` 和 CLI wrapper。

- 风险：如果只发单架构二进制，会让一部分 macOS 用户安装后不可用。
- 缓解方式：优先尝试产出 universal macOS binary；如果做不到，至少在 npm 元数据中显式限制平台。

- 风险：Codex 插件安装仍依赖源码仓库路径，会削弱 npm 包价值。
- 缓解方式：让 npm 包自身包含插件目录和安装脚本，能独立完成安装。

## 里程碑

1. 调研与方案收敛。
2. 分阶段实现。
3. 验证、交付与收尾。

## 验证方式

- 命令：
  - `./scripts/build-open-computer-use-app.sh release --arch universal`
  - `node ./scripts/npm/build-packages.mjs`
  - `npm pack --dry-run`
  - `swift test`
- 手工检查：
  - 解包后确认任一 npm 包都包含 `dist/Open Computer Use.app`、CLI alias 和 Codex 插件目录。
  - 本地通过包内命令执行 `doctor` / `mcp`。
- 观测检查：
  - npm publish 前确认每个包名已 stage 为独立目录，版本一致。

## 进度记录

- [x] 里程碑 1
- [x] 里程碑 2
- [x] 里程碑 3

## 决策记录

- 2026-04-17：npm 分发不走“安装时本地编译”，而是优先走“发布时预编译、安装时直接可用”的方案，目的是把 Swift/Xcode 门槛从最终用户侧移到发布链路。
- 2026-04-17：`.app` 分发物改成 universal binary，同时覆盖 `arm64` 与 `x86_64`，避免 npm 包把 Intel Mac 用户挡在门外。
- 2026-04-17：npm 包内部保留一份最小仓库镜像，包括 `.agents/plugins/marketplace.json`、`plugins/open-computer-use/`、`dist/Open Computer Use.app` 与 `scripts/install-codex-plugin.sh`，这样包本身就能独立完成 Codex 插件安装。
- 2026-04-17：新增 `.github/workflows/release.yml`，后续 GitHub Actions 沿用仓库内 `scripts/release-package.sh` 和 `scripts/npm/publish-packages.mjs` 这条真实构建链路，而不是另起一套发布脚本。
