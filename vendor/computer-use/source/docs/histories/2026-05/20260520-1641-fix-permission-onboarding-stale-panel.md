# 修复权限浮层完成后不消失

## 用户诉求

用户执行 `open-computer-use doctor` 已显示 `accessibility=granted, screenRecording=granted`，但 System Settings 上方的 `Drag Open Computer Use...` 授权辅助浮层仍然持续显示。

## 主要改动

- `PermissionDiagnostics.current()` 现在合并 TCC 持久授权记录与当前 app 进程的 runtime preflight 结果。
- TCC 中任一匹配 client 已授权时仍视为 granted，保留 app-agent / CLI 之间的稳定判断。
- 如果当前运行中的 `.app` 进程已经通过 `AXIsProcessTrusted()` 或 `CGPreflightScreenCaptureAccess()`，也立即视为 granted，避免 stale 或不匹配的 TCC path 查询结果覆盖真实运行态权限。
- 增加单测覆盖 persisted/runtime 权限合并规则。
- 新增 `scripts/run-permission-onboarding-e2e.sh`，在真实本机授权环境中先检查 `doctor` 已 granted，再断言无参数 onboarding 启动会快速退出，不会继续挂住浮层；脚本默认禁用 app-agent proxy，避免本地 ad-hoc `.app` 授权身份干扰当前 CLI 运行态回归。
- 更新架构文档里的权限判断说明。

## 设计动机

onboarding 浮层是否消失取决于 `PermissionDiagnostics.current()` 的轮询结果。此前代码优先采用 TCC 数据库查询结果；当 TCC 查询因为 path/client 不匹配或 stale 记录返回 `false` 时，即使当前 app 进程实际已经拥有权限，也不会再 fallback 到 runtime preflight。这样就可能出现 CLI `doctor` 已经显示 granted，但旧的 onboarding UI 仍认为权限缺失并继续悬浮的状态。

新的合并规则把 TCC grant 和当前进程 runtime grant 都当成可信正信号，避免授权完成后的 UI 卡住。

## 受影响文件

- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/Permissions.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/run-permission-onboarding-e2e.sh`
- `docs/ARCHITECTURE.md`

## 验证

- `swift test --filter Permission`
- `swift test`
- `./scripts/check-docs.sh`
- `git diff --check`
- `.build/debug/OpenComputerUse doctor`
- `./scripts/run-permission-onboarding-e2e.sh`
