## [2026-04-18 00:15] | Task: 收口权限 bundle 身份并简化 onboarding 生命周期

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> 把 bundle identifier 改成 `com.ifuryst.opencomputeruse`，未来名字都按这个走；权限长期以 `npm install -g open-computer-use` 安装后的路径为准。另外已全部授权时不要再反复弹 onboarding，授权完成后窗口要自动关闭。

### 🛠 Changes Overview
**Scope:** `apps/OpenComputerUse`, `packages/OpenComputerUseKit`, `scripts/`, `docs/`

**Key Actions:**
- **[Bundle Identity]**: 把 app bundle identifier 从 `dev.opencodex.OpenComputerUse` 统一改成 `com.ifuryst.opencomputeruse`，并同步更新打包产物校验逻辑。
- **[Onboarding Lifecycle]**: 默认启动和 `doctor` 都先检查权限；如果两项已授权，则不再弹出 onboarding。窗口内两项权限都完成后，会自动关闭并退出 app。
- **[Stable Permission Target]**: 文档与权限识别逻辑统一强调 npm 全局安装后的 `Open Computer Use.app` 是长期授权对象，避免把源码仓库里的临时 `dist` 路径当成最终稳定身份。
- **[NPM Path Priority]**: 当源码启动时，权限识别和拖拽目标会优先寻找 npm 全局安装目录里的 `Open Computer Use.app`；只有没有全局安装产物时才回退到仓库 `dist/`，进一步减少开发态路径参与长期授权身份的概率。

### 🧠 Design Intent (Why)
权限体验要想接近“只授权一次，以后升级不重复折腾”，核心不是继续堆更多检测分支，而是尽量收口到一个稳定 bundle 身份和稳定安装路径。与此同时，onboarding 作为一次性修复流程，不应该在权限已经齐全时继续打扰用户，也不应该在最后一步还要求用户手动关窗。

### 📁 Files Modified
- `apps/OpenComputerUse/Sources/OpenComputerUse/OpenComputerUseMain.swift`
- `apps/OpenComputerUse/Sources/OpenComputerUse/PermissionOnboardingApp.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/Permissions.swift`
- `scripts/build-open-computer-use-app.sh`
- `scripts/npm/build-packages.mjs`
- `README.md`
- `docs/ARCHITECTURE.md`
- `docs/RELIABILITY.md`
- `docs/histories/2026-04/20260418-0015-stabilize-permission-bundle-identity-and-onboarding.md`
