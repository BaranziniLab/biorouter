## [2026-04-19 21:35] | Task: 收紧权限浮窗出场动画和返回交互

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 参考一下 permiso。我们提升一下权限浮窗相关的，主要是点击 allow 后的出现动画，还有浮窗里的返回按钮。

### 🛠 Changes Overview
**Scope:** `apps/OpenComputerUse`、`docs/`

**Key Actions:**
- **[Allow Source Tracking]**: 把主 onboarding 卡片里的 `Allow` 按钮屏幕坐标沿着 delegate 链路传到 accessory panel controller，让浮窗第一次出现时能拿到明确的 source frame。
- **[Launch Transition]**: 给权限浮窗补了一段参考 `permiso` 的 spring + curved frame 入场动画；当 `System Settings` 冷启动或窗口稍后才 ready 时，仍然会在第一次真正挂到目标窗口时执行这段 transition。
- **[Post-Launch Reanchor]**: 把 panel 定位从“冷启动重试几次后停止”改成 guidance 期间持续按 `.common` run loop timer 跟踪 `System Settings` 的窗口 frame，并在 launch 结束前后额外做几次 settle pass，修正动画结束后系统窗口再 settle 一次时 panel 会停在旧位置、必须手动点一下才归位的问题。
- **[Back Affordance]**: 给 accessory panel 加入 material 风格的返回按钮，并把点击动作接回 onboarding 主窗口：隐藏当前 guidance、恢复卡片列表、重新激活 app。
- **[Drag Polish]**: 拖拽 tile 现在在取消或失败时会回弹到初始位置，减少和新浮窗过渡割裂的感觉。
- **[Docs Sync]**: 更新架构文档，记录权限 panel 现在具备 source-to-target 入场和显式返回按钮。

### 🧠 Design Intent (Why)
这轮不是再扩权限功能边界，而是收紧交互质感。现有实现虽然已经能跳页、跟窗和拖拽，但 `Allow` 之后的 panel 还是“硬切”出来，和官方/`permiso` 那种从触发点飞入 `System Settings` 的引导感差一截；同时 panel 内缺少返回 affordance，用户一旦想中断当前步骤，只能自己切回主窗口。把 source-tracked transition 和返回按钮补齐后，权限引导的节奏会更连贯，也更容易在两项权限之间来回切换。

### 📁 Files Modified
- `apps/OpenComputerUse/Sources/OpenComputerUse/PermissionOnboardingApp.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-04/20260419-2135-refine-permission-panel-transition-and-back-button.md`
