## [2026-04-22 14:52] | Task: 收口示例路径并脱敏样例应用名

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### User Query
> 清理 `computer-use-cli` 相关示例和文档引用，统一改成仓库根目录样例路径，并避免继续在 README、测试和参考日志里出现真实产品名。

### Changes Overview
**Scope:** 示例路径、helper 文档、测试样例、参考日志

**Key Actions:**
- **[Example path consolidation]**: 删除 `scripts/computer-use-cli/` 下重复的 sequence JSON，统一改为引用根目录 `examples/textedit-overlay-seq.json`。
- **[Sample name sanitization]**: 将 helper README、Go 测试、Swift 测试和参考日志里的具体应用名改成 `TextEdit` 或 `Sample Chat` 这类通用样例。
- **[History sync]**: 同步修正既有 history 中已经过时的样例路径描述，避免文档继续指向已删除位置。

### Design Intent (Why)
这些样例本来只是为了说明调用形状，不应该继续绑定到某个真实产品名，也不应该在仓库里维护两份内容重复的 sequence 文件。把路径和命名统一后，后续手工验证、README 示例和测试断言会更稳定，也更适合开源仓库继续演进。

### Files Modified
- `scripts/computer-use-cli/README.md`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/main_test.go`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/references/codex-local-runtime-logs.md`
- `docs/histories/2026-04/20260420-2033-integrate-set-value-visual-cursor.md`
- `docs/histories/2026-04/20260422-1452-sanitize-example-paths-and-sample-app-names.md`
