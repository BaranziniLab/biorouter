## [2026-04-20 20:28] | Task: 调整 Cursor Motion 的 Dock 图标尺寸

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI / zsh / macOS`

### 📥 User Query
> “我们调整一下吧，和其他APP一样的宽高”

### 🛠 Changes Overview
**Scope:** `scripts/`、`apps/`、`docs/`

**Key Actions:**
- **[收缩 icon 有效画布]**: `scripts/render-open-computer-use-icon.swift` 现在会给 `1024x1024` app icon 母版留出 `92px` 的透明安全边，不再让背景 tile 直接顶满整张画布。
- **[同步 app 内 branding 几何]**: `apps/OpenComputerUse/Sources/OpenComputerUse/PermissionOnboardingApp.swift` 里的 `Branding.makeAppIconImage` 同步采用相同 inset，避免打包 icon 和 app 内品牌图形几何继续分叉。
- **[按系统 icon 量级校准]**: 额外对比了本机 `Terminal` / `Notes` / `QuickTime Player` 的 `.icns` 内容边界，确认标准 Apple icon 的水平安全边大约在 `7.8%`，因此把 inset 从初版 `6%` 再调整到更接近系统量级的 `8%`。
- **[落 checked-in 1024 母版]**: 新增仓库内的 `1024x1024` master PNG，并让打包脚本改走 `master PNG -> .iconset -> .icns` 的 CLI 链路，后续如果还要微调 Dock 观感，只需要修改这一张母版。
- **[微调 2px optical inset]**: 根据后续肉眼检查，把 `1024x1024` 母版的有效内容边界从约 `81...942` 进一步收敛到 `83...940`，等价于上下左右各再减少约 `2px`。
- **[微调 1px optical inset]**: 根据 Dock 复查反馈，把母版有效内容边界继续从 `83...940` 收敛到 `84...939`，等价于上下左右各再减少 `1px`。
- **[改用可见步长继续缩小]**: 因为 `1024` 母版里的 `1px` 映射到 Dock 后肉眼几乎不可见，这次把有效内容边界从 `84...939` 直接收敛到 `92...931`，让 Dock 里能看到明确缩小效果。
- **[补变更留档]**: 新增这份 history，记录这次针对 Dock 图标有效尺寸的收口。

### 🧠 Design Intent (Why)
Dock 会统一缩放整个 icon 画布，但不会替不同应用做额外的光学校正。之前我们的 icon 背景直接铺满画布，导致在同一排 Dock 图标里看起来明显更高。把有效图形统一内缩一圈，并对齐到本机 Apple app icon 的常见安全边量级，比继续依赖系统显示结果更稳定；同时把 icon 资产链收口到一张 checked-in `1024x1024` 母版，也能避免后续继续在临时几何脚本里追着 Dock 效果做不可追踪的微调。

### 📁 Files Modified
- `assets/app-icons/open-computer-use-1024.png`
- `scripts/build-apple-iconset.sh`
- `scripts/render-open-computer-use-icon.swift`
- `scripts/build-open-computer-use-app.sh`
- `apps/OpenComputerUse/Sources/OpenComputerUse/PermissionOnboardingApp.swift`
- `docs/histories/2026-04/20260420-2028-shrink-app-icon-safe-area.md`
