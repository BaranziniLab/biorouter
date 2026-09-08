# Installing and signing in

> **What this is.** How to get **Claude Code** or **Codex** working in BioRouter: install the
> vendor CLI, sign in by running the vendor's own command yourself, and read the four states the
> settings card can report. Also covers the escape hatch for a binary BioRouter cannot find.
> **Status:** Current.
> **Audience:** end users setting up either provider, and maintainers diagnosing a card that will
> not go green.

These two providers have no API key to paste. They use a subscription you already pay for, through
a CLI you install once and sign in to once. BioRouter's part is small and deliberately limited: it
finds the binary, asks it whether it is signed in, and starts it. It never performs the sign-in
and never handles the credential — see [why](#why-biorouter-does-not-sign-you-in) below, and
[the compliance page](compliance.md) for the terms that make it a requirement rather than a
preference.

## Step 1 — install the CLI

Run whichever you want, in your own terminal:

```bash
# Claude Code — the `claude` CLI
curl -fsSL https://claude.ai/install.sh | bash

# Codex — the `codex` CLI (needs Node.js)
npm install -g @openai/codex@latest
```

BioRouter shows these commands rather than running them. Installing another vendor's toolchain is
your decision, and the Claude installer in particular is a piped shell script — not something an
application should execute on your behalf.

## Step 2 — sign in, yourself

```bash
claude auth login
codex login
```

Sign in with the **subscription** you want to use — a Claude Pro/Max plan, or a ChatGPT plan. Both
commands open the vendor's own browser flow. When it finishes, the credential is stored by the
vendor's CLI, where only that CLI reads it:

| | Claude Code | Codex |
| --- | --- | --- |
| macOS | The Keychain, service `Claude Code-credentials` | `~/.codex/auth.json` |
| Linux and Windows | `~/.claude/.credentials.json` | `~/.codex/auth.json` |

> **Note.** On macOS there is no `~/.claude/.credentials.json`, so its absence proves nothing
> about whether you are signed in. Check with `claude auth status` instead.

## Step 3 — pick the provider in BioRouter

Choose **Claude Code** or **Codex** in the model picker or in provider settings, exactly as you
would any other provider. There is no key to enter. The only setting either provider has is the
name or path of its executable, which defaults to `claude` and `codex` respectively.

> **Note — which models you can pick depends on your CLI version.** The newest model in each
> catalogue has a vendor-set version floor: `claude-fable-5-1` needs `claude` **2.1.251** or newer,
> and `gpt-6-astra` needs `codex-cli` **0.153.4** or newer. On an older CLI the turn fails with the
> vendor's own upgrade message, which BioRouter shows you verbatim. Each CLI has its own remedy:
> for Claude Code, `claude update` (or updating the Claude desktop app); for Codex, `codex update`,
> or re-running step 1's `npm install -g @openai/codex@latest`. Every other advertised model works
> on older CLIs too. Details, including the exact messages, are on
> [which models you can pick depends on the CLI version](how-it-works.md#which-models-you-can-pick-depends-on-the-cli-version).

## The four states a card can show

BioRouter asks each CLI what it thinks its own situation is, over
`GET /coding_agents/status`, which reports a row for **both** providers whether or not either is
installed. Each row carries the resolved path, the raw `--version` line, the credential state, and
the exact command to run when you need to act.

| State | What it means | What to do |
| --- | --- | --- |
| **Not installed** | The executable is not on any path BioRouter searches. | Run the install command in step 1 — or, if it is already installed somewhere unusual, [pin its path](#when-biorouter-cannot-find-the-binary). |
| **Installed, not signed in** | The binary is there; no credential is stored. | Run `claude auth login` or `codex login` yourself, in a terminal. |
| **Signed in with an API key** | The CLI works, but it is authenticated with a metered API key rather than a subscription, so every turn would be billed per token. **The provider refuses to run.** | Sign in with your subscription instead — or use the ordinary metered provider for that vendor (`anthropic`, `openai`), which is what an API key is for. |
| **Ready** | Signed in on a subscription. | Nothing. |

A fifth outcome exists and is not a state you can fix by guessing: **indeterminate**. It appears
when the probe ran but its output could not be understood — malformed JSON from
`claude auth status`, an `auth.json` with no `auth_mode`, an unrecognised mode, or a
`codex login status` that reported a problem (typically an expired refresh token behind a
still-well-formed file). The card carries the reason, and BioRouter says it does not know rather
than telling you to log in when you already are.

> **Why the API-key state is a refusal and not a fallback.** These providers exist specifically to
> use your own plan, and they remove API credentials from the environment they start. Falling back
> would bill an account you did not choose for this turn, quietly. If you *want* metered API
> billing, the `anthropic` and `openai` providers do that properly, with usage accounting that
> reconciles.

## Why BioRouter does not sign you in

Anthropic's terms are explicit that third-party developers may not offer Claude.ai login or route
requests through Free, Pro, or Max plan credentials on behalf of their users, and OpenAI's
position on third-party use of a ChatGPT plan could not be confirmed from any first-party source.
BioRouter therefore surfaces the vendor's own command for you to run, and the credential stays
between you and the vendor. It is never seen, stored, brokered, proxied or transmitted by
BioRouter. The signed-out message says so in as many words, and a test pins that sentence so it
cannot be edited away. The full reasoning, with the terms quoted, is on
[the compliance page](compliance.md).

## When BioRouter cannot find the binary

`biorouterd` is launched by the desktop app with a truncated `PATH`. On macOS a GUI app's
inherited `PATH` excludes `/opt/homebrew/bin`, `~/.local/bin` and every npm prefix, so a CLI your
terminal finds instantly can still be invisible. BioRouter compensates by searching its own
augmented path, including npm prefixes — but toolchain managers install outside all of it. If you
use **nvm, volta, bun or asdf**, pin the full path:

| Provider | Config key | Default |
| --- | --- | --- |
| Claude Code | `CLAUDE_CODE_COMMAND` | `claude` |
| Codex | `CODEX_COMMAND` | `codex` |

Set it in provider settings, or in
[`config.yaml`](../../configuration/config-file-reference.md). A value containing a path separator
is taken verbatim and is not looked up; a pinned path that does not exist is **not** silently
replaced by a search, so a typo reports "not installed" rather than running some other binary.
Find the real path with `which claude` or `which codex` in the terminal where it works.

> **Note.** Each provider declares exactly one required config key, with a default, and that shape
> is deliberate: a provider with no config keys at all would report `is_configured: false` forever
> and never appear in the model picker. `llamacpp` solves the same problem the same way with
> `LLAMACPP_PORT`.

## Verifying from the command line

```bash
# what BioRouter will see, per provider (no secret key needed on this route)
curl -s http://127.0.0.1:3000/coding_agents/status | jq

# what each vendor CLI says about itself
claude auth status
codex login status
```

The port is `3000` for a `biorouterd` you started yourself; the desktop app starts its own daemon
on an **ephemeral** port, so query that one only through the app.

`GET /coding_agents/status` spawns both CLIs, so it takes a moment — a cold `claude` start was
measured at about 3.5 seconds, and the probe's own ceiling is 20 seconds per CLI. This is exactly
why the sign-in check is *not* part of `GET /config/providers`, which builds every configured
provider under a three-second budget.

## Related documentation

- [How the coding-agent providers work](how-it-works.md) — what happens after the card goes green,
  including how the binary is located and how the run is kept on your subscription.
- [Compliance: vendor terms, BAA and PHI](compliance.md) — the terms behind the sign-in policy,
  and the hard rule that no protected health information may reach either provider.
- [What the child agent may not do](child-agent-isolation.md) — what the spawned CLI is and is not
  allowed to touch on your machine.
- [Performance, limits and known gaps](performance-and-limits.md) — why the first response feels
  slower than an API provider's.
- [Choosing a model provider](../../getting-started/choosing-a-model-provider.md) — the other
  providers, and what an API key buys you instead.
- [Environment variables](../../configuration/environment-variables.md) — the two command keys
  among all other configuration.
- [Common problems and fixes](../../troubleshooting/common-problems-and-fixes.md) — general
  troubleshooting for the app and the daemon.
