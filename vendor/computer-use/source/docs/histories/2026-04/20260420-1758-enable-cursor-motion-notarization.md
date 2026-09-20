## [2026-04-20 17:58] | Task: 接通 Cursor Motion 的 Developer ID 签名与 notarization

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> gh 已经有权限的，你可以配置，还需要我提供什么 secret 么？还是你都可以拿到

### 🛠 Changes Overview
**Scope:** `.github/workflows/`、`docs/`、`scripts/`

**Key Actions:**
- **[Repo Secrets Installed]**: 将 `Developer ID Application` 证书 `.p12`、密码、identity，以及 notarization 需要的 Team API key、Key ID、Issuer ID、Team ID 写入 GitHub repo secrets。
- **[Cursor Motion Signing]**: `scripts/build-cursor-motion-dmg.sh` 新增 `CURSOR_MOTION_CODESIGN_*` 环境变量支持，允许在构建 `Cursor Motion.app` 时用 `Developer ID Application` 证书签名，而不是固定 ad-hoc。
- **[Cursor Motion Notarization]**: release workflow 的 `release-cursor-motion-dmg` job 现在会在检测到 `APPLE_NOTARY_*` secrets 后，使用 `xcrun notarytool submit --wait` 对生成的 `.dmg` 执行 notarization，并在成功后 `stapler staple`。
- **[Fallback Safety]**: 缺失 signing 或 notary secrets 时，workflow 会明确打印降级原因，但不会阻塞 release。

### 🧠 Design Intent (Why)
在 `Developer ID Application` 证书与 App Store Connect Team API key 都已齐备的情况下，仅仅把 secret 存进 GitHub 还不够；真正影响用户体验的是 `Cursor Motion` 下载物本身是否已经过 `Developer ID` 签名和 Apple notarization。把这条链路放进 workflow 后，tag release 才能稳定产出更接近标准 macOS 分发体验的 `.dmg`。

### 📁 Files Modified
- `scripts/build-cursor-motion-dmg.sh`
- `.github/workflows/release.yml`
- `docs/CICD.md`
- `docs/releases/RELEASE_GUIDE.md`
- `docs/histories/2026-04/20260420-1758-enable-cursor-motion-notarization.md`
