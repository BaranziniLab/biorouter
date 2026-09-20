## [2026-04-20 18:18] | Task: 修复 CI 在导入 Developer ID 证书后未能解析 signing identity 的回退逻辑

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> 可以，bump 小版本 tag 推，然后看看结果

### 🛠 Changes Overview
**Scope:** `.github/workflows/`、`docs/`

**Key Actions:**
- **[Failure Triage]**: 检查 `v0.1.17` 的 GitHub Actions 失败日志，确认 `package-npm` 与 `release-cursor-motion-dmg` 都在 “Prepare ... signing config” 阶段退出，原因是 runner 上的 `security find-identity` 输出没有被当前解析逻辑识别到。
- **[Identity Fallback Fix]**: release workflow 现在会优先使用已配置的 `OPEN_COMPUTER_USE_CODESIGN_IDENTITY` secret 作为签名 identity，只有在该 secret 缺失时才尝试从导入后的 keychain 自动解析。
- **[Retry Preparation]**: 为同版本重跑 release 做好修复，避免 `.p12` 已正确导入但因输出格式差异导致 workflow 误判 “no usable codesigning identity”。

### 🧠 Design Intent (Why)
这次失败不是证书本身不可用，而是 CI 对 `security find-identity` 输出的假设过于脆弱。既然 repo secret 里已经明确保存了目标 `Developer ID Application` CN，最稳的做法就是优先信任这份配置，而不是把整个 release 成败绑定在 runner 的工具输出格式上。

### 📁 Files Modified
- `.github/workflows/release.yml`
- `docs/histories/2026-04/20260420-1818-fix-ci-signing-identity-fallback.md`
