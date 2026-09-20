## [2026-04-22 15:51] | Task: 发布 0.1.29

### 背景

- 用户要求提交 PR，并在当前已完成改动基础上 bump 版本。
- 前置提交已完成 `click` 的定向鼠标事件 fallback，以及一条仓库级英文回复规则文档补充；其中本次 patch release 只收录用户可感知的 `click` 行为改进。

### 变更

- **[Version Bump]**: 将插件 manifest、Swift/Go 版本常量、smoke suite 初始化版本、测试 MCP client version 与 CLI 文档路径统一提升到 `0.1.29`。
- **[Release Notes]**: 在用户可见发布记录中增加 `0.1.29`，说明本次 patch release 聚焦 `click` 的 AX 失败后定向鼠标事件 fallback，以及更稳的语义点击顺序。
- **[Release Trigger]**: 为后续 `v0.1.29` tag / GitHub Release / npm publish 准备一致的版本源。
- **[Guide Fix]**: 修正 `docs/releases/RELEASE_GUIDE.md` 里的 npm staging 验证命令，默认改为不带 `--skip-build`，避免干净 checkout 因缺少 `dist/Open Computer Use.app` 而直接失败。
- **[Rebase Adjustment]**: 因为 `origin/main` 在 PR 期间已经发布了 `0.1.28`，本分支在解冲突时把 release bump 顺延到 `0.1.29`，并保留主分支已有的 `0.1.28` release note。

### 验证

- 通过：`swift test`
- 通过：`make check-docs`
- 通过：`node ./scripts/npm/build-packages.mjs --out-dir dist/release/npm-staging-check`，staging package version 为 `0.1.29`
- 通过：`./scripts/build-cursor-motion-dmg.sh --configuration release --arch universal --version 0.1.29`

### 影响文件

- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/feature-release-notes.md`
- `docs/releases/RELEASE_GUIDE.md`
- `docs/histories/2026-04/20260422-1551-bump-open-computer-use-to-0.1.29.md`
