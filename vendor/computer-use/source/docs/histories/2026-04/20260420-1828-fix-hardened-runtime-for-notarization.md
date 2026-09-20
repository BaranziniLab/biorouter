## [2026-04-20 18:28] | Task: 修复 Cursor Motion notarization 缺少 hardened runtime

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> 可以，bump 小版本 tag 推，然后看看结果

### 🛠 Changes Overview
**Scope:** `docs/`、`scripts/`

**Key Actions:**
- **[Failure Triage]**: 拉取 `v0.1.17` 的 Apple notary log，确认 `.dmg` 被判 `Invalid` 的直接原因是 `Cursor Motion.app/Contents/MacOS/CursorMotion` 在 `arm64` / `x86_64` 两个架构上都没有启用 hardened runtime。
- **[Runtime Signing Fix]**: `build-open-computer-use-app.sh` 与 `build-cursor-motion-dmg.sh` 在使用非 ad-hoc identity 签名时，现在会显式传 `codesign --options runtime`。
- **[Next Release Prep]**: 为后续新的 patch release 做准备，避免继续复用已经成功 publish npm、但未通过 notarization 的 `0.1.17` 发布结果。

### 🧠 Design Intent (Why)
Apple notary service 对 Developer ID 分发的可执行文件要求 hardened runtime。此前仓库虽然已经改成了 `Developer ID Application` 签名，但没有同步打开 `--options runtime`，因此会在公证阶段被 Apple 直接拒绝。把这个签名参数补上后，才算真正满足 notarization 的最低门槛。

### 📁 Files Modified
- `scripts/build-open-computer-use-app.sh`
- `scripts/build-cursor-motion-dmg.sh`
- `docs/histories/2026-04/20260420-1828-fix-hardened-runtime-for-notarization.md`
