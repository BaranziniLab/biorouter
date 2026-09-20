## [2026-05-08 10:45] | Task: Headless smoke fixture

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### User Query
> `./scripts/run-tool-smoke-tests.sh` 每次运行都会出现橙色弹窗，去掉这个不必要的窗口。

### Changes Overview
**Scope:** OpenComputerUseFixture, OpenComputerUseSmokeSuite, OpenComputerUseKit, docs

**Key Actions:**
- **Headless fixture mode**: 为 smoke fixture 增加 `OPEN_COMPUTER_USE_FIXTURE_HEADLESS` 开关，headless 时不激活、不置前窗口。
- **Smoke runner default**: smoke suite 启动 fixture 时默认注入 headless 环境变量。
- **Discovery compatibility**: 允许内部 fixture 即使以 accessory activation policy 运行也能被测试发现。
- **Architecture docs**: 记录 smoke 脚本默认使用 headless fixture，避免在用户桌面弹出测试窗口。

### Design Intent (Why)
smoke 测试需要真实 fixture 承载 AX 控件和测试命令，但不需要把可见测试窗口弹到用户桌面。将 headless 作为 fixture 的显式环境开关，可以让脚本默认安静运行，同时保留手动调试时打开可见窗口的能力。

### Files Modified
- `apps/OpenComputerUseFixture/Sources/OpenComputerUseFixture/main.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AppDiscovery.swift`
- `docs/ARCHITECTURE.md`
