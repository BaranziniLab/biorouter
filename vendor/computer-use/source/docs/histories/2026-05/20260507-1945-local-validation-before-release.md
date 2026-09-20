# 本地验证优先于公开发版

## 用户诉求

用户指出构建后不应该持续公开发版，建议通过 Codex TOML 配置指向本地 `open-computer-use` 来验证，避免外部用户看到过多 patch release。

## 主要改动

- 更新 `docs/releases/RELEASE_GUIDE.md`，明确日常对齐验证默认走本地构建和本地 MCP 配置。
- 补充公开 release 的触发条件：只有用户明确要求发版，或修复已经稳定到需要对外交付时，才进入 release checklist。

## 设计动机

公开 release 应该表达对外可消费的稳定交付，而不是每次本地验证的副产物。后续 agent 应先用本地构建验证 `open-computer-use` 与官方 `computer-use` 的行为差异，减少无意义的版本噪音。

## 受影响文件

- `docs/releases/RELEASE_GUIDE.md`
