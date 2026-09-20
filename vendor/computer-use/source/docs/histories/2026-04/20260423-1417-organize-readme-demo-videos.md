## [2026-04-23 14:17] | Task: Organize README demo videos

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `codex-cli`

### User Query
> Add the new Linux demo video to both READMEs and split the three top demo videos into independent second-level sections.

### Changes Overview
**Scope:** root README documentation

**Key Actions:**
- **Organized demo area**: Added a dedicated demos section at the top of both English and Chinese READMEs.
- **Separated demos**: Split the Codex App / Codex CLI, Gemini CLI, and Linux runtime videos into their own third-level subsections with captions.
- **Added Linux demo**: Embedded the new GitHub user-attachment video URL in both README variants.
- **Aligned English copy**: Reworked the English README to follow the shorter Chinese README structure and removed redundant runtime detail sections.
- **Added README badges**: Replaced the top language links in both READMEs with repository-specific language, release, DeepWiki, and LLMAPIS badges.

### Design Intent (Why)
The top of the README now makes the three usage contexts scan-friendly and avoids mixing unrelated demo videos without labels.

### Files Modified
- `README.md`
- `README.zh-CN.md`
