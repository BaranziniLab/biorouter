# document-bgclick-free-research-workflow

## 用户诉求

用 Ghidra 等免费工具替代 IDA Pro，对 bundled computer-use app 做后台点击方向的反编译研究；研究目录太大，不提交 `research/`，只把后续 Agent 可复跑的方法论沉淀到 docs。

## 本次改动

- 将 `research/` 加入仓库级 `.gitignore`，避免提交一次性 Ghidra project、反汇编大文件和临时 Swift 构建产物。
- 新增后台点击免费工具链研究方法论文档，记录 Ghidra/radare2/Apple CLI/Swift 原型的复跑步骤。
- 更新 reverse-engineering docs 入口，说明大体积分析产物应在本地 `research/` 下重新生成。
- 保留本轮已经验证过的关键结论：`NSEvent -> CGEvent`、window id fields、私有 `CGEventSetWindowLocation` 和 `postToPid` 是后台点击复现的核心路径。

## 设计动机

官方二进制是 stripped Swift/ObjC Mach-O，无法直接恢复原始源码；一次性反编译产物体积过大，长期价值也不如可复跑方法论。将方法、关键证据形状和验证矩阵写进 docs，可以让后续开发者按需让 AI 重新生成本地研究目录，同时保持仓库轻量。

## 影响文件

- `.gitignore`
- `docs/references/README.md`
- `docs/references/codex-computer-use-reverse-engineering/README.md`
- `docs/references/codex-computer-use-reverse-engineering/background-click-free-tooling.md`
