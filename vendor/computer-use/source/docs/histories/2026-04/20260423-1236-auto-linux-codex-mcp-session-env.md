## [2026-04-23 12:36] | Task: Linux runtime should auto-detect desktop session env

## 用户诉求

希望 Linux 上也能直接按 `npm i -g open-computer-use`、`open-computer-use install-codex-mcp`、`codex` 的路径使用，不需要手动把 `XDG_RUNTIME_DIR`、`DBUS_SESSION_BUS_ADDRESS` 等桌面 session 环境变量写进 Codex 配置。

## 主要改动

- **[Linux Runtime]**: Go runtime 在调用 Python AT-SPI bridge 前会补齐缺失的 Linux 桌面 session env，从当前用户的 `/proc` 桌面进程和 `/run/user/<uid>` 自动发现 session bus、Wayland / X11 display、X authority 和 AT-SPI 相关环境。
- **[Codex Install]**: `install-codex-mcp` 继续写入简单的 `open-computer-use mcp`，不把 session 相关变量固化到 `~/.codex/config.toml`。
- **[Docs]**: README、中文 README、架构、可靠性文档和 release notes 同步说明 Linux runtime 的动态 session env 发现行为。
- **[Version Bump]**: 将 Open Computer Use bump 到 `0.1.36`，用于发布包含 installer 修复的新 npm 版本。

## 设计动机

Linux AT-SPI 不是无环境的后台系统服务，它挂在已登录桌面用户的 D-Bus session 下。`tools/list` 可以只暴露 schema，但真实 `list_apps` / `get_app_state` 需要 bridge 进程带着桌面 session 环境启动。把探测逻辑放进 runtime，而不是写死到 Codex config，更适合 session 重启、不同 shell / terminal 和不同主流发行版的默认体验。

## 受影响文件

- `apps/OpenComputerUseLinux/main.go`
- `apps/OpenComputerUseLinux/main_test.go`
- `README.md`
- `README.zh-CN.md`
- `docs/ARCHITECTURE.md`
- `docs/RELIABILITY.md`
- `docs/releases/RELEASE_GUIDE.md`
- `docs/releases/feature-release-notes.md`
- 版本源相关文件

## 验证

- 通过：`bash -n scripts/install-codex-mcp.sh`
- 通过：`node --check scripts/install-config-helper.mjs`
- 通过：`node --check scripts/npm/build-packages.mjs`
- 通过：`(cd apps/OpenComputerUseLinux && go test ./...)`
- 通过：`(cd apps/OpenComputerUseWindows && go test ./...)`
- 通过：`swift test`
- 通过：`./scripts/build-open-computer-use-linux.sh --arch arm64`
- 通过：`./scripts/build-open-computer-use-linux.sh --arch amd64`
- 通过：`node ./scripts/npm/build-packages.mjs --out-dir dist/release/npm-staging-check`
- 通过：`./scripts/release-package.sh`
- 通过：临时 `CODEX_HOME` 安装配置检查，确认 Codex config 仍为 `command = "open-computer-use"` / `args = ["mcp"]`，没有写入 session env。
- 通过：Linux VM 中按 `leo` 用户用 `env -i` 清空桌面环境后执行 `/tmp/open-computer-use-0.1.36-test call list_apps`，成功列出 `gnome-shell`、`gnome-text-editor` 和 `ptyxis`。
- 通过：Linux VM 中按 `leo` 用户用 `env -i` 启动 MCP，`initialize`、`tools/list` 和 `tools/call(list_apps)` 均成功。
- 通过：Linux VM 中按 `leo` 用户用 `env -i` 对 9 个工具执行 sequence smoke：`list_apps`、`get_app_state`、`click`、`set_value`、`type_text`、`press_key`、`scroll`、`drag`、`perform_secondary_action` 均返回 `isError=false`。
- 通过：GitHub Actions release workflow `24817430041`，`package-npm` 与 `release-cursor-motion-dmg` 均成功。
- 通过：`npm view open-computer-use@0.1.36`、`open-computer-use-mcp@0.1.36`、`open-codex-computer-use-mcp@0.1.36` 均可见。
- 通过：GitHub Release `v0.1.36` 已补充 `What's Changed`，并保留 `Full Changelog`。
- 通过：Ubuntu aarch64 VM 中 `leo` 用户执行 `npm i -g open-computer-use@0.1.36` 成功；`open-computer-use -v` 和 package.json 均为 `0.1.36`，安装包内 `dist/linux/arm64/open-computer-use` 是 aarch64 ELF。
- 通过：Ubuntu aarch64 VM 中 `open-computer-use install-codex-mcp` 后，`~/.codex/config.toml` 仍为 `command = "open-computer-use"` / `args = ["mcp"]`，没有写入 session env。
- 通过：Ubuntu aarch64 VM 中 `codex mcp list` / `codex mcp get open-computer-use` 显示 server enabled，command 为 `open-computer-use mcp`。
- 通过：Ubuntu aarch64 VM 中按 `leo` 用户用 npm 安装版 `open-computer-use` 在 `env -i` 下执行 `call list_apps` 成功；raw MCP `tools/list` 返回 9 个 tools。
