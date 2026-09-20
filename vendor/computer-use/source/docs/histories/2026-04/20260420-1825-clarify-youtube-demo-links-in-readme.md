## [2026-04-20 18:25] | Task: clarify README YouTube demo links

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5 family (Codex)`
* **Runtime**: `Codex CLI`

### 📥 User Query
> README 里的 YouTube 链接渲染后看起来像普通图片，不容易让人意识到它其实是视频；希望说明原因并把展示改得更明确。

> 后续又要求把显式说明收敛成更简洁的样式，改为在视频缩略图下方放一个居中的 caption。

> 之后又提出已经做了一张 base64 图片，希望给 README 留一个可替换的占位，并说明该怎么接入。

> 最后提供了两张本地 PNG，希望直接移到仓库里作为正式封面图使用。

### 🛠 Changes Overview
**Scope:** `repository docs`

**Key Actions:**
- **[Centered captions]**: 在中英文 README 的两个演示区保留原有缩略图链接，并把说明文字改成图片下方居中的 caption。
- **[Less visual noise]**: 去掉图片上方额外的说明行，避免 README 顶部和章节之间被重复文案打断。
- **[Custom cover placeholder]**: 在中英文 README 顶部 demo 区增加注释式占位，约定把 base64 解码到仓库内固定路径后再替换图片引用。
- **[Repo-local covers]**: 把用户提供的两张 `1280x720` PNG 移到 `docs/generated/readme-assets/`，并把中英文 README 的两处视频封面都切到仓库内相对路径。
- **[History sync]**: 新增本次文档改动 history，保持 README 展示策略的变更可追踪。

### 🧠 Design Intent (Why)
GitHub README 不会把普通外链视频缩略图自动渲染成带播放键或 YouTube 角标的嵌入式卡片；在不引入 iframe 的前提下，用自定义封面图加下方 caption 是更直接的表达。最终封面资源落到仓库内固定目录，并通过相对路径引用，既符合 GitHub README 的稳定渲染方式，也比继续保留 base64 占位或外链缩略图更可维护。

### 📁 Files Modified
- `README.md`
- `README.zh-CN.md`
- `docs/generated/readme-assets/open-computer-use-demo-cover.png`
- `docs/generated/readme-assets/cursor-motion-demo-cover.png`
- `docs/histories/2026-04/20260420-1825-clarify-youtube-demo-links-in-readme.md`
