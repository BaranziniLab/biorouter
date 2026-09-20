# Dev 权限引导刷新

## 用户诉求

Dev 版在系统设置里已经授权后，权限引导窗口仍停留在 `Allow`，看起来像授权没有生效。

## 主要改动

- 权限引导标题和说明改为使用当前 bundle 名称，Dev 版显示 `Open Computer Use (Dev)`。
- 用户点击 `Allow` 后，如果当前进程仍未看到权限刷新，短暂等待后将对应卡片切换为 `Restart`。
- 点击 `Restart` 会重新拉起当前 app bundle，让 macOS 对新进程刷新 Accessibility / Screen Recording 权限状态。

## 设计动机

macOS 对已经运行的 app agent 不一定会即时刷新新授予的 TCC 权限。系统设置里已经授权时，继续显示 `Allow` 会误导用户反复操作。显式切换到 `Restart` 更符合实际状态，也避免需要手动找进程并重启。

## 验证

- `swift test`
- `./scripts/build-open-computer-use-app.sh debug`
- `./scripts/run-tool-smoke-tests.sh`
- `./scripts/check-docs.sh`
- `git diff --check`

## 受影响文件

- `apps/OpenComputerUse/Sources/OpenComputerUse/PermissionOnboardingApp.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/Permissions.swift`
