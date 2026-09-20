## [2026-04-22 16:58] | Task: bump v0.1.30 and document release notes rule

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `local macOS shell`

### User Query
> bump 一个版本；确认 v0.1.29 的 What's Changed / New Contributors 是否自动生成，并把后续 release notes 规范写进仓库。

### Changes Overview
**Scope:** release version sources, release guide, feature release notes

**Key Actions:**
- **[Version bump]**: 将 Open Computer Use 版本源从 `0.1.29` 同步 bump 到 `0.1.30`。
- **[Release notes rule]**: 在 `RELEASE_GUIDE.md` 里明确 GitHub Release 会用 `--generate-notes` 自动生成 notes；如果自动正文只有 `Full Changelog`，release agent 必须手动补 `What's Changed`。
- **[User notes]**: 在 `feature-release-notes.md` 记录 `0.1.30` 的 Windows runtime 预览和 release notes 规范。

### Design Intent
`v0.1.29` 出现 `What's Changed` / `New Contributors` 是 GitHub 自动 release notes 对 merged PR 的归类结果；direct commit release 可能只生成 `Full Changelog`。后续 AI 做版本 bump 时必须检查并补齐 release body，避免用户可见 release 页面缺少变更摘要。

### Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/RELEASE_GUIDE.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260422-1658-bump-v0-1-30-and-release-notes-rule.md`
