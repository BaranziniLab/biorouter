## [2026-05-03 17:31] | Task: 修复 macOS 终端权限归属

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `local macOS workspace`

### User Query
> `open-computer-use doctor` 能打开权限页面，但从终端执行时权限似乎要求的是 iTerm/Terminal，而不是 `Open Computer Use.app`；需要保证 iTerm 没有 Accessibility / Screen Recording 权限时，只给 Open Computer Use 授权也能工作。

### Changes Overview
**Scope:** `apps/OpenComputerUse`, `packages/OpenComputerUseKit`, `docs`

**Key Actions:**
- **[App agent proxy]**: 新增隐藏 app-agent 启动模式；终端 CLI 对 `mcp`、`doctor`、`call`、`snapshot` 和 `list-apps` 通过 Unix domain socket 转发到 LaunchServices 启动的 `.app` 进程。
- **[Onboarding reuse]**: 将权限 onboarding 拆出可复用的 `present()` 路径，让 doctor 可以在 app agent 已运行的 NSApplication 内显示授权窗口，而不是在终端子进程内直接跑 UI。
- **[Decision tests]**: 将 app-agent 代理选择规则下沉为 Kit 内纯函数，并补单测覆盖 automation 命令代理、非 automation 命令本地执行、LaunchServices 打开 app 不递归代理、禁用开关和缺少 bundle fallback。
- **[Launch mode guard]**: 对无参数启动区分 LaunchServices 打开的 `.app` 和终端直接执行的 bundle executable，避免双击 app 时留下多余后台 agent，同时让终端入口仍走 app 身份代理。
- **[Socket permissions]**: app-agent Unix socket 创建后收紧为当前用户读写，维持本地-only 权限边界。
- **[Permission relaunch]**: app-agent 内完成授权后仍会终止当前 app 进程，确保下一次命令用重新启动后的授权进程执行 ScreenCaptureKit / AX 路径。
- **[Permission wording]**: 更新权限错误和文档口径，明确 macOS 授权目标是 `Open Computer Use.app`，不是宿主终端。
- **[Swift 6.2 build fix]**: 适配 Swift 6.2 并发检查，将 cursor reference `NSImage` 静态缓存显式标记为 `nonisolated(unsafe)`，保持当前 AppKit 主线程绘制路径可编译。

### Design Intent (Why)
macOS TCC 对 Accessibility 和 Screen Recording 的责任归属取决于真正调用系统 API 的进程。终端直接启动 native runtime 时，系统可能要求 iTerm/Terminal 获得权限；把真实 automation 放到通过 LaunchServices 启动的 app bundle 进程内，可以让用户只授权 `Open Computer Use.app`，终端负责 stdio/命令代理。

### Files Modified
- `apps/OpenComputerUse/Sources/OpenComputerUse/MacOSAppAgentProxy.swift`
- `apps/OpenComputerUse/Sources/OpenComputerUse/OpenComputerUseMain.swift`
- `apps/OpenComputerUse/Sources/OpenComputerUse/PermissionOnboardingApp.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseCLI.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorGlyphRenderer.swift`
- `docs/ARCHITECTURE.md`
- `docs/SECURITY.md`
- `docs/RELIABILITY.md`

### Verification
- Passed: `git diff --check`
- Passed: `./scripts/check-docs.sh`
- Passed: `xcrun swiftc -parse apps/OpenComputerUse/Sources/OpenComputerUse/MacOSAppAgentProxy.swift`
- Passed: `xcrun swiftc -parse packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseCLI.swift`
- Passed: `xcrun swiftc -parse packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- Passed: `PATH="$HOME/.swiftly/bin:$PATH" swift build --product OpenComputerUse` with Swift `6.2.4`.
- Passed: `PATH="$HOME/.swiftly/bin:$PATH" ./scripts/build-open-computer-use-app.sh debug --arch native`.
- Passed: `.build/arm64-apple-macosx/debug/OpenComputerUse doctor` returned missing permission status while launching `dist/Open Computer Use (Dev).app/Contents/MacOS/OpenComputerUse __open-computer-use-app-agent ...`, confirming the terminal command hands off to the app bundle process.
- Passed: the app-agent socket was created as `srw-------`, current-user read/write only.
- Passed: after granting the dev app both macOS permissions, `OpenComputerUse doctor` reported `accessibility=granted, screenRecording=granted`.
- Passed: `OpenComputerUse call get_app_state --args '{"app":"Finder"}'` returned `isError=false` with both `text` and `image` content while iTerm did not hold the permissions.
- Passed: Codex MCP integration via `codex exec` with the bundled official `computer-use` plugin disabled called `server="open-computer-use"` for `list_apps` and `get_app_state`.
- Passed: Codex-driven action smoke covered `click`, `scroll`, `set_value`, `press_key`, `drag`, and `type_text`; Finder, Calendar, and the local fixture succeeded, while TextEdit `click` exposed an existing window-bounds edge case but TextEdit typing still succeeded.
- Blocked locally: `PATH="$HOME/.swiftly/bin:$PATH" swift test` cannot import `XCTest` from the Swift.org toolchain in this CLT-only environment.
