## [2026-04-17 22:18] | Task: 修正 npm 发布认证模式选择

### 📥 User Request

> GitHub Action 要能通过 git tag 真正把包发出去；如果中间发现问题，就继续修到发布成功为止。

### 🔧 What Changed

- **修正包仓库元数据**：把 npm staging 包里生成的 `repository.url` 从 `git+https://...git` 改成和 GitHub 仓库精确一致的 `https://github.com/iFurySt/open-codex-computer-use`。
- **强制 token fallback 真正生效**：在 `publish-packages.mjs` 里，当 `NODE_AUTH_TOKEN` 已经存在时，主动清掉 GitHub Actions 暴露给 npm CLI 的 OIDC 环境变量，避免 npm 继续优先走 Trusted Publishing。
- **绕开 setup-node 生成的 npmrc**：workflow 不再依赖 `actions/setup-node` 的 `registry-url` 配置，而是在存在 `NPM_TOKEN` 时显式写一份临时 `.npmrc` 并通过 `NPM_CONFIG_USERCONFIG` 注入，保证 CI 与本地成功路径一致。
- **补一轮最小外部验证**：本地用同一个 `NPM_TOKEN` 对 `open-codex-computer-use-mcp` 成功发布了一个 `debug` dist-tag 的临时 prerelease，确认 token 本身具备真实 publish 能力。
- **修正 workflow YAML 细节**：把 `.npmrc` 生成逻辑从 heredoc 改成 `printf`，避免 GitHub Actions 把 workflow 解析成无效 YAML 后出现“run 存在但没有 job”的假失败。

### 🧠 Design Intent (Why)

这次联调暴露出两条认证路径会互相干扰：

1. npm CLI 在 GitHub Actions 中检测到 OIDC 环境后，会优先尝试 Trusted Publishing。
2. 仓库里生成的包 `repository.url` 不够“精确匹配”，可能导致 Trusted Publishing 侧校验失败。
3. 即便 workflow 已经注入 `NODE_AUTH_TOKEN`，如果不显式屏蔽 OIDC 环境，npm 也不一定会真的走 token 路径。

所以这里不是单纯“再补一个 secret”就够，而是要明确把“包元数据”和“认证模式选择”两层都收紧，避免发布过程中继续落到错误的认证分支。

### 📌 Key Files

- `scripts/npm/build-packages.mjs`
- `scripts/npm/publish-packages.mjs`
