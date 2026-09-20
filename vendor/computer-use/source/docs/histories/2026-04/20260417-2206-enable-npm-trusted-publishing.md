## [2026-04-17 22:06] | Task: 启用 npm Trusted Publishing

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 已经在 npmjs 上保存了 Trusted Publishing，处理一下 `.github/workflows/release.yml`。

### 🛠 Changes Overview
**Scope:** `.github/workflows/`、`scripts/npm/`、`README.md`、`docs/`

**Key Actions:**
- **更新 workflow 权限**：给 `release.yml` 增加 `id-token: write`，满足 npm Trusted Publishing 的 OIDC 要求。
- **移除 token 依赖**：删掉 workflow 中对 `NPM_TOKEN` secret 的依赖。
- **放宽发布脚本校验**：让 `scripts/npm/publish-packages.mjs` 在 GitHub Actions OIDC 场景下不再强制要求 `NODE_AUTH_TOKEN`。
- **同步文档**：把 README 和 CI/CD 文档中的发布说明改成 Trusted Publishing 路径。

### 🧠 Design Intent (Why)
Trusted Publishing 的核心是短时 OIDC 凭证，不应该再让 workflow 依赖长期 npm token。只改 workflow 还不够，因为仓库内的发布脚本之前会在没有 `NODE_AUTH_TOKEN` 时直接失败，所以需要把脚本和文档一起收敛到同一条发布模型。

### 📁 Files Modified
- `.github/workflows/release.yml`
- `README.md`
- `docs/CICD.md`
- `scripts/npm/publish-packages.mjs`
