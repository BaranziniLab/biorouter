## [2026-04-17 22:14] | Task: 启用 tag 发布并 bump 到 0.1.5

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 通过 gh 设置 workflow，后续通过 git tag 来发；搞完就提交相关改动，打个 0.1.5 发，然后观察结果。

### 🛠 Changes Overview
**Scope:** `.github/workflows/`、`plugins/`、`packages/`、`apps/`、`scripts/`、`docs/`

**Key Actions:**
- **调整 release 触发方式**：让 `release.yml` 在 push `v*` 和 `*.*.*` tag 时自动发布，同时保留手动触发。
- **保留 Trusted Publishing**：发布步骤继续走 GitHub Actions OIDC，不依赖长期 npm token。
- **版本 bump**：将插件 manifest、MCP server、自测和 CLI 文档中的版本统一更新到 `0.1.5`。
- **同步文档**：把 README 和 CI/CD 文档明确改成“打 git tag 自动发 npm”的用法。
- **修正 GitHub runner**：把 release workflow 从 `macos-14` 调整到 `macos-26`，避免 GitHub Hosted Runner 默认 Xcode 15.4 / Swift 5.10 无法构建 `swift-tools-version: 6.2` 包。

### 🧠 Design Intent (Why)
既然 npm 侧已经配置了 Trusted Publishing，最自然的 release 路径就是“commit -> tag -> GitHub Actions 自动发”。这样发布动作能绑定到一个明确的 Git tag，也更符合 npm 包版本和 Git 版本一一对应的维护习惯。

### 📁 Files Modified
- `.github/workflows/release.yml`
- `README.md`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `docs/CICD.md`
- `docs/references/codex-computer-use-cli.md`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/MCPServer.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `scripts/computer-use-cli/README.md`
- `scripts/computer-use-cli/main.go`
