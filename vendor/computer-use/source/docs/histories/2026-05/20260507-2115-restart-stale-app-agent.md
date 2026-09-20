# 自动替换过期 app-agent

## 用户诉求

继续优化 `open-computer-use` 与官方工具的一致性，并减少每次构建后需要手动重启隐藏 app-agent 的情况。

## 主要改动

- app-agent socket 新增 `agentInfo` 请求，返回当前 bundle、executable 和 agent 进程启动时间。
- CLI proxy 连接已有 socket 时，会校验 agent 是否来自当前 bundle，并且启动时间是否晚于当前 executable 修改时间。
- 如果已有 agent 过期或不支持 `agentInfo`，proxy 会丢弃旧 socket 并拉起新的 Dev app-agent。
- 新 agent 支持 `terminate` 请求，后续替换时可优雅退出。

## 设计动机

本地 debug 构建会原地替换 `dist/Open Computer Use (Dev).app`，但旧的隐藏 app-agent 可能继续持有旧代码，导致 Codex 重启后仍拿到过期 tool 行为。用 agent 自报信息和 executable mtime 做一次轻量握手，可以让代理在连接前自动发现并替换 stale agent。

## 验证

- `swift test`
- `./scripts/build-open-computer-use-app.sh debug`
- `./scripts/run-tool-smoke-tests.sh`
- `./scripts/check-docs.sh`
- `git diff --check`
- 普通代理路径 `call list_apps` 验证可自动拉起新 agent，并输出 `frontmost`

## 受影响文件

- `apps/OpenComputerUse/Sources/OpenComputerUse/MacOSAppAgentProxy.swift`
