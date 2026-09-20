# Security policy

## Reporting a vulnerability

**Report a suspected vulnerability privately to [wanjun.gu@ucsf.edu](mailto:wanjun.gu@ucsf.edu). Do not open a public issue.** Issues on this repository are public, and Biorouter is used with clinical and other sensitive research data — a vulnerability disclosed in an issue is disclosed to everyone at once.

If [private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability#privately-reporting-a-security-vulnerability) is enabled on the repository, the Security tab → **Report a vulnerability** is an equivalent private channel. It is not always enabled; the email address above always works.

Please include the Biorouter version, your platform, and the smallest set of steps that reproduces the problem. Do not include patient data, credentials, or other sensitive material in a report.

**Acknowledgement.** We aim to acknowledge a report within **5 business days**, and to follow up with our assessment and an expected remediation timeline within **10 business days**. If you have not heard back in that window, send a reminder to the same address before considering any further disclosure.

The Baranzini Lab recognizes the important contributions our open source community makes. Part of keeping Biorouter and its users safe is finding and fixing security issues in our open source projects, and we are grateful for reports.

## Supported versions

**Only the latest released version is supported.** Fixes ship in a new release; there are no backports and no patch releases for older versions.

Updating is not uniform across platforms, so an old install can stay old indefinitely: macOS updates in place via `electron-updater` ("Restart & Update"), while Windows and Linux use an assisted-download fallback that requires you to reinstall the new package. Please update to the newest release before reporting, and confirm the issue still reproduces there.

## Scope

**In scope:**

- The `biorouterd` HTTP and WebSocket API and its `BIOROUTER_SERVER__SECRET_KEY` authentication, whether bound to loopback (the default) or to a LAN-reachable address via `biorouter serve --host` / `biorouter web --host`, together with the browser token and its session cookie that gate the served interface.
- Secret storage — including the plaintext `secrets.yaml` fallback, which is selected by `BIOROUTER_DISABLE_KEYRING=true` and automatically on headless Linux. See [secret storage](docs/security/secret-storage.md).
- The secret guard, the always-on refusal that keeps credential files (`~/.aws/credentials`, SSH private keys, `secrets.yaml`, `.env`) out of tool arguments and tool output in every chat, mode and tier. See [secret guard](docs/security/secret-guard.md).
- The Biorouter Copilot consent gate, the per-task approval that must be granted before any of the ten desktop observation and control tools (`list_apps`, `get_app_state`, `click`, `perform_secondary_action`, `scroll`, `drag`, `type_text`, `press_key`, `set_value`, `screen_capture`) runs, and the native helper it drives.
- MCP extension execution and the `.brxt` extension install path.
- The `serve.mjs` server shipped with an exported Agent Drafter app.
- The `biorouter://` URL scheme handler.
- Permission modes and managed policy failing to constrain the agent as documented — see [permission modes](docs/security/permission-modes.md) and [managed enterprise policy](docs/security/managed-policy.md).

**Out of scope:**

- A model producing a wrong, misleading, or unsafe answer. Model output quality is not a Biorouter vulnerability; raise it with the model provider, and see [ACCEPTABLE_USAGE.md](ACCEPTABLE_USAGE.md) for what Biorouter may not be used for.
- Vulnerabilities in third-party MCP extensions or models, which should go to their own maintainers.

## Known limits: safety, not a security boundary

**Biorouter's in-app controls — permission modes, tool permissions, `.biorouterignore` — are safety measures against mistakes, not security boundaries against a determined or injected path.** They act inside Biorouter, above the operating system. A control failing to constrain the agent *as documented* is a vulnerability and is in scope above. The secret guard and the Biorouter Copilot consent gate are a different matter: both are listed in scope, and a bypass of either is a reportable vulnerability. The limits below are not defects; they are the shape of the product, and a deployment handling regulated data has to plan around them.

- **The agent runs with your privileges.** `shell` and `text_editor` can run any command and read or modify any file your user account can reach. `.biorouterignore` and permission modes filter what the agent is offered and when it must ask; they are not an OS sandbox, and an approved command is not confined by them. See [the Developer extension's access controls](docs/extensions/built-in/developer.md).
- **Session history is not encrypted.** Conversations are kept in a local SQLite database at `~/.config/biorouter/sessions/sessions.db`. Whatever a session contained — including regulated data — stays readable on disk by anything running as your user account. See [managing sessions](docs/getting-started/managing-sessions.md).
- **Biorouter Copilot sees and acts beyond Biorouter.** Once you grant a Biorouter Copilot task, `screen_capture` can read whatever is on the display, including windows belonging to other applications, and the input tools can act in them. The consent gate is per task and revocable with Stop; it is not a restriction on what the granted task can reach.

These two limits are also what makes prompt injection consequential rather than merely annoying; read the autonomy caution below with them in mind. The practical consequence for patient data is that the boundary which matters is chosen **before** the session starts — the provider you pick and the machine you run on — not a setting applied afterwards.

## Suspected exposure of patient data

**A suspected exposure of PHI or other patient data is an institutional privacy incident, not a GitHub security advisory.** Report it first to your institution's privacy office and information security team and follow their instructions — at UCSF, the UCSF Privacy Office and UCSF IT Security. Do not file an issue or an advisory, and do not include patient data in any report.

If a Biorouter defect appears to be involved, email [wanjun.gu@ucsf.edu](mailto:wanjun.gu@ucsf.edu) as well, describing the defect only — never the data.

## Agent autonomy: understand this before you run Biorouter

> [!CAUTION]
> Biorouter is a biomedical research agent with access to a variety of systems that perform actions on behalf of the user on their local machine. Please be aware that since agents like Biorouter have the ability to run code and take actions on your computer, they pose a unique risk compared to chat based LLM interactions. While most foundational models include baseline protections against prompt injection, there is still inherent risk when using Biorouter to interact with the internet or through other untrusted data sources. To minimize these risks, consider taking the following precautions:
>
> - Use a dedicated virtual machine or container (Docker/Kubernetes) with limited privileged capabilities. This will minimize the risk of local system attacks or unintended access to critical system resources.
> - Always review the code and tests generated by Biorouter for accuracy.
> - Avoid providing Biorouter with sensitive or confidential information to prevent information leakage.
> - For any systems and actions that may result in significant changes, always require human confirmation.
> - If possible, break down complex Biorouter instructions into smaller, isolated operations. This reduces the risk of an errant command affecting multiple parts of the system at once and makes it easier to detect abnormal behaviour.
> - Only connect Biorouter with MCP extensions that you have reviewed
>
> In some circumstances, Biorouter may follow commands found embedded in content even if those commands conflict with the task given to Biorouter. We suggest taking the precautions above to limit risks from prompt injection. By taking these steps, you can reduce the potential security risks associated with code-executing agents and better protect your systems and users.

The control that decides how freely the agent may act is the **permission mode** — see [permission modes](docs/security/permission-modes.md) — and an administrator can impose a [managed enterprise policy](docs/security/managed-policy.md) that a user cannot override. `.biorouterignore` keeps chosen files out of the agent's reach.

## Related documentation

- [Acceptable Usage Policy](ACCEPTABLE_USAGE.md) — what Biorouter may not be used for.
- [Security documentation index](docs/security/README.md) — permission modes, managed policy, secret storage, and data privacy.
- [Data privacy and patient data](docs/security/data-privacy-and-phi.md) — which providers are acceptable for PHI.
