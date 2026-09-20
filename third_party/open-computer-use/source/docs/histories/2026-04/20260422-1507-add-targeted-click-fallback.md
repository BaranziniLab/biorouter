## [2026-04-22 15:07] | Task: add targeted click fallback

### 🤖 Execution Context
* **Agent ID**: `019db3f1-0538-7a70-bdd4-19395299085c`
* **Base Model**: `GPT-5 Codex`
* **Runtime**: `Codex CLI`

### 📥 User Query
> okie, try to optimize it, the key thing is that I don't wanna to grab user mouse, offical computer use can do it, so as we can

### 🛠 Changes Overview
**Scope:** `OpenComputerUseKit`, `docs/ARCHITECTURE.md`, `docs/histories/`

**Key Actions:**
- **[Targeted click fallback]**: 给 `click` 增加 `CGEvent.postToPid` 定向鼠标事件兜底，避免 AX 失败后默认直接落到会移动真实鼠标的全局 HID 路径。
- **[Semantic click ordering]**: 调整 element-targeted / coordinate `click` 的 AX 顺序，先试直接语义动作与子孙 `AXOpen` 候选，再把 `AXRaise` / focus 类激活放到后面，避免 Finder 侧边栏这类“只聚焦、不导航”的假成功。
- **[Behavior docs]**: 更新架构文档，明确 `click` 现在的顺序是 `AX -> pid-targeted mouse event -> optional global pointer fallback`。
- **[Live validation]**: 用 Finder 侧边栏 `Applications` 做真实样本回归，验证默认路径无需抓用户鼠标也能点通。

### 🧠 Design Intent (Why)
官方 Computer Use 在 Finder 这类 AX 不完整的目标上仍能点击成功，而不会默认抢用户硬件鼠标。本地实现也需要先走更窄的定向事件路径，把全局物理指针 fallback 保持为显式 opt-in 的最后逃生口。

后续回归里又发现一个更细的行为问题：Finder sidebar row 本身可能允许 `focus/main` 之类激活成功，但这不等于真正执行了导航。如果把这一步放在 `AXOpen` 之前，就会把“没切页的聚焦”误报成成功点击，所以顺序必须继续收敛到“语义点击优先，聚焦激活兜底”。

### 📁 Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/InputSimulation.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-04/20260422-1507-add-targeted-click-fallback.md`
