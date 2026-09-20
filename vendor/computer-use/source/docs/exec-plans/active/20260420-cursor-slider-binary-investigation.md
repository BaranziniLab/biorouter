# Cursor Slider Binary Investigation

## 目标

继续围绕官方 `Codex Computer Use.app` 的 cursor motion 做一轮更聚焦的逆向分析，回答 5 个 slider (`start handle`、`end handle`、`arc size`、`arc flow`、`spring`) 在 shipping binary 中是否存在直接证据；如果没有，则把它们和当前已经 binary-confirmed 的路径几何 / spring 链路做一层明确标注的映射分析，并产出可重复运行的敏感性分析输出。

## 范围

- 包含：
  - 检查官方 shipping bundle 中是否存在 slider 文案、独立参数类型或明显的调试 UI 证据。
  - 复用 `scripts/cursor-motion-re/official_cursor_motion.py` 中已经确认的候选几何与 spring 常量，补一版参数敏感性分析。
  - 把“binary-confirmed 证据”和“根据路径几何做的 slider 映射推断”清晰分开写回文档。
- 不包含：
  - 不把新的 slider 映射直接宣称为官方一一字段对照。
  - 不修改主 MCP runtime 的 `SoftwareCursorOverlay` 行为。
  - 不要求在这轮里恢复内部 debug UI 的完整实现来源。

## 背景

- 相关文档：
  - `docs/references/codex-computer-use-reverse-engineering/software-cursor-motion-model.md`
  - `docs/references/codex-computer-use-reverse-engineering/software-cursor-motion-reconstruction.md`
  - `docs/exec-plans/active/20260419-official-cursor-motion-reconstruction.md`
- 相关代码路径：
  - `scripts/cursor-motion-re/`
  - `experiments/CursorMotion/`
- 已知约束：
  - 当前本机可用的官方 binary 为 `~/.codex/plugins/cache/openai-bundled/computer-use/1.0.750/Codex Computer Use.app/Contents/MacOS/SkyComputerUseService`。
  - 用户看到的 slider 来自视频 / 调试构建，不代表 shipping bundle 一定保留相同 UI 文案。
  - 当前仓库已经明确要求区分 `confirmed_from_binary` 与 `reconstructed`。

## 风险

- 风险：把 shipping bundle 中不存在的 slider 文案误写成“官方 release 里已确认存在”。
  - 缓解方式：先做全 bundle 文本扫描；若未命中，只记为“debug-build evidence from video, not string-confirmed in release bundle”。
- 风险：把参数敏感性分析误写成“已经定位到真实调参接口”。
  - 缓解方式：分析输出与文档都显式区分“直接命中 binary 的字段/常量”和“基于这些量做的 slider 映射推断”。
- 风险：分析脚本只对某个起终点样本成立，结论过拟合。
  - 缓解方式：让脚本支持传入任意 `start/end/bounds`，输出具体 measurement / chosen candidate 变化。

## 里程碑

1. 收敛 shipping binary 中与 slider 相关的直接证据。
2. 实现可重复运行的 slider sensitivity / parameter mapping analysis。
3. 文档沉淀、history 与验证。

## 验证方式

- 命令：
  - `python3 scripts/cursor-motion-re/reconstruct_cursor_motion.py inspect --pretty`
  - `python3 scripts/cursor-motion-re/reconstruct_cursor_motion.py slider-study --start 220 440 --end 860 260 --bounds 0 0 1120 760 --pretty`
- 手工检查：
  - 确认 shipping bundle 中的字符串扫描没有把 slider 文案误报为已存在。
  - 确认每个 slider 的分析输出都能落到具体几何字段、measurement 或 spring timeline 变化。
- 观测检查：
  - 文档里明确标出“release bundle string evidence”和“geometry/timing inference”两层边界。

## 进度记录

- [x] 里程碑 1
- [x] 里程碑 2
- [x] 里程碑 3

## 决策记录

- 2026-04-20：这轮不继续追“视频里的 slider UI 本身怎么渲染”，而是优先回答 shipping binary 中是否保留对应证据，以及这些 knob 更接近哪些已确认几何 / spring 量。
- 2026-04-20：整包 phrase scan 没命中 `START HANDLE`、`END HANDLE`、`ARC SIZE`、`ARC FLOW`，所以仓库文档改成“视频证据仍成立，但更像内部调试构建而非 release bundle 直接可见 UI”。
- 2026-04-20：新增 `slider-study` CLI，而不是把参数敏感性分析继续塞进 `inspect` / `demo`；这样可以把“shipping phrase evidence”“binary-confirmed motion terms”“slider mapping inference”三层输出保持分离。
