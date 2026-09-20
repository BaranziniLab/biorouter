## [2026-04-20 18:28] | Task: 修复 release 签名身份解析

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> 提交相关改动，然后加个版本号 git tag 推送触发一波看看。

### 🛠 Changes Overview
**Scope:** `.github/workflows/`、`docs/`

**Key Actions:**
- **[CI Signing Resolution]**: 把 release workflow 的签名配置从“直接信任 repo secret 里的 identity 名”改成“导入 `.p12` 后，直接从临时 keychain 解析第一条可用 codesigning identity”，避免 runner 上出现 `codesign: no identity found`。
- **[Runner Keychain Compatibility]**: 新增把临时 keychain 加入 user search list、并设成 default keychain 的步骤，再让 `security find-identity` 与后续 `codesign` 统一走默认搜索链，规避 GitHub macOS runner 上直接按 `.keychain-db` 路径查 identity 不稳定的问题。
- **[Failure Recording]**: 新增 history，记录 `v0.1.15` 首次 tag push 时 `package-npm` 在 `Build npm release artifacts` 阶段失败的原因和修复方式。

### 🧠 Design Intent (Why)
这次问题不是证书没有导入，而是 workflow 对 identity 名的解析太脆弱。既然 `.p12` 已经是签名真源，最稳的做法就是让 CI 在导入后自己发现可用 identity，而不是继续依赖手工维护的 CN 字符串。

### 📁 Files Modified
- `.github/workflows/release.yml`
- `docs/histories/2026-04/20260420-1828-fix-release-codesign-identity-resolution.md`
