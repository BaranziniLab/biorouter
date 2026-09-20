## [2026-04-17 21:26] | Task: 打通 npm 分发并发布

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 帮我把这个打包成可以发到 npm 的，一键安装 `open-computer-use-mcp`、`open-codex-computer-use-mcp` 或 `open-computer-use` 都可以；现在的打包门槛太高。并直接发到用户的 npmjs，后续改成 GitHub Actions 发。

### 🛠 Changes Overview
**Scope:** `scripts/`、`.github/workflows/`、`docs/`、`README.md`、`package.json`

**Key Actions:**
- **实现 npm 分发链路**：新增 `scripts/npm/build-packages.mjs` 与 `scripts/npm/publish-packages.mjs`，可 stage 和发布三个包名对应的 npm 包。
- **升级 app 打包**：扩展 `scripts/build-open-computer-use-app.sh`，支持构建 universal `Open Computer Use.app`，用于 npm 预编译分发。
- **让 npm 包可独立安装到 Codex**：npm 包内携带插件目录、marketplace 配置和 `install-codex-plugin.sh`，安装后可直接执行 `open-computer-use install-codex-plugin`。
- **替换 release 占位产物**：把 `scripts/release-package.sh` 从仓库元数据打包改成真实 npm tgz 产物输出，并生成 release manifest。
- **补 GitHub Actions 骨架**：新增 `.github/workflows/release.yml`，支持手动打包和用 `NPM_TOKEN` 发布到 npm。
- **实际发布到 npm**：发布 `open-computer-use@0.1.2`、`open-computer-use-mcp@0.1.2`、`open-codex-computer-use-mcp@0.1.2`。

### 🧠 Design Intent (Why)
这次改动的核心是把“用户安装成本”从终端用户侧转移到发布侧。与其要求每个用户都有 Swift/Xcode 构建环境，不如在发布时直接产出 universal `.app` 并塞进 npm 包，让安装后就能跑、就能装进 Codex 插件系统。这样既保留仓库内真实构建链路，也让后续 GitHub Actions 能复用同一套脚本。

### 📁 Files Modified
- `.github/workflows/release.yml`
- `Makefile`
- `README.md`
- `docs/ARCHITECTURE.md`
- `docs/CICD.md`
- `docs/exec-plans/completed/20260417-2115-npm-distribution.md`
- `package.json`
- `scripts/build-open-computer-use-app.sh`
- `scripts/ci.sh`
- `scripts/install-codex-plugin.sh`
- `scripts/npm/build-packages.mjs`
- `scripts/npm/publish-packages.mjs`
- `scripts/release-package.sh`
