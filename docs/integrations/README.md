# Integrations with other tools

This folder documents the packages that let **another application host a Biorouter agent** — the
opposite direction from everything in [`extensions/`](../extensions/README.md), which is about
adding capabilities *into* Biorouter. An integration here is a small adapter that lives outside the
Rust workspace, speaks a host's plugin contract, and delegates the whole turn to `biorouter` so the
user gets their configured model, extensions, skills, workflows, knowledge bases, and permissions
inside the host's own chat surface.

Come here when you want to use Biorouter from a tool you already work in, or when you are writing or
maintaining one of those adapters. The adapters themselves live in the repository's top-level
[`integrations/`](../../integrations/) folder, next to the manifests that install them; the pages
here carry the architecture, the version pins and the verification record, and link out to each
package's own install instructions.

Every adapter here reaches Biorouter through the Agent Client Protocol (ACP) by launching
`biorouter acp`. A new ACP session opens in the folder the host names as its `cwd`, so the agent's
shell and file tools work in the host's project; a `cwd` that is not an absolute path to an existing
folder falls back to the folder `biorouter acp` was started in. A prompt the host stops with
`session/cancel` answers with the `cancelled` stop reason, and `initialize` reports the agent's name
and version in `agentInfo`.

## Documents in this folder

| Document | What it covers |
|---|---|
| [Biorouter persona for Jupyter AI](jupyter-ai-persona.md) | The `@Biorouter` persona for JupyterLab's Jupyter AI chat: how it launches `biorouter acp` over stdio, the pinned JupyterLab 4.5.9 / Jupyter AI 3.0.1 stack and why it is pinned, the acceptance-test suite, and the dated records of the runs that verified it. |

## Related documentation

- [Extensions, skills, and MCP agents](../extensions/extensions-and-skills-guide.md) — the other direction: giving the Biorouter agent new tools rather than giving another host the agent
- [biorouter CLI command reference](../cli/command-reference.md) — `biorouter acp`, the subcommand every ACP integration launches
- [Application Drafter](../agent-drafter/README.md) — when you want Biorouter to *build* an interactive application rather than live inside someone else's
- [Headless Linux deployment](../deployment/headless-linux.md) — hosting the daemon for shared or remote use, rather than embedding the agent in a desktop tool
