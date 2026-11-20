## [2026-10-19 23:36] | Task: 将官方参考 ZIP 从 Git LFS 迁移到 Releases

### Execution Context

- Agent ID: `/root`
- Base Model: GPT-6
- Runtime: Codex desktop

### User Query

> 将两份官方参考 ZIP 移到 GitHub Releases，提供固定下载地址，解决普通 clone 自动下载参考包导致 LFS 流量消耗的问题。

### Changes Overview

- 使用独立的 `reference-bundles` 归档 tag / pre-release，附件使用固定的 `releases/download/reference-bundles/<filename>` 地址。
- 从当前 Git tree 移除两份 LFS 指针，保留本地 ZIP 为忽略文件，并保留 SHA-256 校验清单。
- 同步归档目录说明、原始制品提交约定、归档发布指南及英文 Release 说明。
- 不改变正式应用或 npm 版本，不重写历史；旧提交的 LFS 引用和已消耗的月度流量仍然存在。

### Design Intent

ZIP 只用于逆向研究与版本比较，不参与默认构建。按需下载 Release 附件可以避免普通 clone 自动消耗账号的 LFS 流量；固定 tag 与文件名维持可引用的下载入口。

### Files Modified

- `.gitattributes`
- `.gitignore`
- `docs/references/codex-computer-use-reverse-engineering/assets/README.md`
- `docs/references/codex-computer-use-reverse-engineering/assets/official-bundles/computer-use/`
- `docs/references/codex-computer-use-reverse-engineering/background-click-free-tooling.md`
- `docs/releases/RELEASE_GUIDE.md`
- `docs/releases/reference-bundles.md`
- `docs/exec-plans/completed/20261019-reference-bundles-release.md`

### Validation

- 本地原始 ZIP 和独立备份均通过原有 SHA-256 清单校验；两个 ZIP 均通过完整性检查。
- 文档骨架检查和 diff 空白检查通过；ZIP 从 index 移除，保留的本地 ZIP 命中忽略规则。
- `reference-bundles` Release 的三个附件均上传完成，服务端 SHA-256 与本地文件一致；归档为 pre-release，正式 latest 仍是 `v1.1.2`。
- 无认证访问三个公开固定下载 URL 成功；两个 ZIP 下载后通过原有 SHA-256 清单与完整性检查，Release 中的校验清单与仓库文件字节一致。
- 迁移已推送到 `main`；从 GitHub 公开 HTTPS 地址完整 clone，在启用 Git LFS、未跳过 smudge 的条件下正常检出，工作区干净，当前 tree 没有 LFS 文件或原始参考 ZIP。
- 新克隆内的文档骨架检查通过。此任务只修改制品分发与文档，没有修改应用实现。
