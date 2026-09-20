# [2026-04-22 14:59] | Task: 明确英文输入时的默认回复语言

## 📥 User Query
> add in Agent.md, if user input in english, reply in english

## 🛠 Changes Overview
- 在 `AGENTS.md` 的工作规则里补充明确约束：如果用户这一轮输入是英文，则直接用英文回复。
- 保留原有“默认跟随用户提问语言、切换语言则切换回复语言”的仓库级协作规则，并把英文输入场景写成更直接的显式规则，减少后续执行歧义。

## 🧠 Design Intent
- 这类协作约束应直接落在仓库入口文档里，而不是依赖会话记忆。
- 把英文输入场景显式化后，后续 Agent 不需要再从泛化规则里推断，执行会更稳定。

## 📁 Files Touched
- `AGENTS.md`
