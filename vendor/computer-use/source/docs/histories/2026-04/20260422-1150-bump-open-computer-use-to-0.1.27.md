## [2026-04-22 11:50] | Task: 发布 0.1.27

### 背景

- 用户要求提交所有剩余改动并 bump version。
- 前置提交已完成 9 个 Computer Use tools 的官方对齐收口，主要覆盖 `perform_secondary_action` 错误语义、fixture `Raise` 非物理指针路径和 `press_key` key table alias。

### 变更

- **[Version Bump]**: 将插件 manifest、Swift/Go 版本常量、smoke suite 初始化版本、测试 MCP client version 与 CLI 文档路径统一提升到 `0.1.27`。
- **[Release Notes]**: 在用户可见发布记录中增加 `0.1.27`，说明本次 patch release 聚焦剩余 tools checklist 收口、secondary action 错误形态和 xdotool alias 补齐。
- **[Release Trigger]**: 准备用 `v0.1.27` tag 标记本次版本 bump，供后续发布链路使用。

### 验证

- 通过：`swift test`
- 通过：`node ./scripts/npm/build-packages.mjs --skip-build --out-dir dist/release/npm-staging-check`，staging package version 为 `0.1.27`
- 通过：`./scripts/build-cursor-motion-dmg.sh --configuration release --arch universal --version 0.1.27`

### 影响文件

- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260422-1150-bump-open-computer-use-to-0.1.27.md`
