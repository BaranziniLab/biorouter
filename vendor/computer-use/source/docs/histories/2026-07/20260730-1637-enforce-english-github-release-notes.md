## [2026-07-30 16:37] | Task: Enforce reviewed English GitHub Release notes

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex desktop`

### User Query
> 将近期 GitHub Release 更新日志统一为英文，并让后续 workflow 从审核过的英文 notes 文件发布。

### Changes Overview
**Scope:** GitHub Release metadata, validation, workflow gating, and release documentation.

**Key Actions:**
- Added reviewed English GitHub Release notes for `v0.2.0`, `v0.2.1`, `v0.3.0`, and `v0.3.1`, plus a reusable template.
- Added a fail-closed Node validator for tag/version consistency, one to three user-visible changes, CJK exclusion, and the required Full Changelog link.
- Added a `release-metadata` workflow job that must pass before npm packaging or Cursor Motion DMG publication starts.
- Replaced generated notes with `--notes-file` for both release creation and idempotent release updates.

### Design Intent
GitHub generated notes copy merged pull request titles without translation, so Chinese PR titles produced mixed-language public release history. Checked-in English notes make release text reviewable and reproducible, while the pre-publication gate prevents npm or DMG publication from starting when release metadata is missing or inconsistent.

### Files Modified
- `.github/workflows/release.yml`
- `package.json`
- `scripts/validate-github-release-notes.mjs`
- `scripts/check-docs.sh`
- `docs/releases/README.md`
- `docs/releases/RELEASE_GUIDE.md`
- `docs/releases/github/TEMPLATE.md`
- `docs/releases/github/v0.2.0.md`
- `docs/releases/github/v0.2.1.md`
- `docs/releases/github/v0.3.0.md`
- `docs/releases/github/v0.3.1.md`
- `docs/histories/2026-07/20260730-1637-enforce-english-github-release-notes.md`
