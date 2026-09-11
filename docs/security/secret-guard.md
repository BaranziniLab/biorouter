# Secret guard

> **What this is.** How BioRouter keeps credential files and credential values away from the
> model: the always-on deny floor, how a tool call's arguments are resolved before they are
> judged, and the redaction of credential material in what a tool returns.
> **Status:** Current. The argument scan was rebuilt and the output redaction added on
> 2026-09-11, fixing QA-C finding H1; the sections below describe that code.
> **Audience:** developers working on tool dispatch, the Developer and Computer Controller
> extensions or the guardrails, and anyone reviewing what BioRouter promises about secrets.

A model with a shell can read any file the user can. The secret guard is the part of BioRouter
that refuses to let it read the few files whose whole purpose is to hold a credential — an AWS
credentials file, an SSH private key, the store BioRouter keeps provider API keys in — and, when
something gets past that refusal anyway, withholds the credential itself from the tool result.
It applies to every chat, in every permission mode, on every model: it is not a privacy-tier
feature, and a private model has no more business with the user's AWS key than a public one.

## What is protected

The floor is `DEFAULT_SECRET_PATTERNS` in
[`crates/biorouter-mcp/src/secret_guard.rs`](../../crates/biorouter-mcp/src/secret_guard.rs):
`.env`, `.env.*`, `secrets.*` (which covers `~/.config/biorouter/secrets.yaml`, the plaintext
provider-key store), `*.pem`, `id_rsa`, `id_dsa`, `id_ecdsa`, `id_ed25519`, `*.p12`, `*.pfx`,
`.aws/credentials`, `.codex/auth.json` and `.claude/.credentials.json`, at any depth.

A `.biorouterignore` adds patterns: the project's own file for paths inside the project, and the
global `<config>/.biorouterignore` for every path. Either can reopen one specific file with a
gitignore negation (`!path`), because user patterns are layered after the floor.

## Where it runs

| Place | What it checks |
|---|---|
| `ExtensionManager::dispatch_tool_call` (`secret_guard_denial`) | Every tool call's arguments, for every extension. The one choke point every call passes. |
| Developer server, `validate_shell_command` | `developer__shell`'s command, from the directory it will actually run in. |
| Developer server, `is_ignored` | `text_editor` and `image_processor` paths, after symlinks are resolved. |
| Computer Controller, `refuse_secret_access` | `automation_script` and `computer_control` bodies, from the server's own working directory. |
| `call_tool_withholding_secrets`, inside the dispatched future | Every tool **result** and error, before any model sees it. |

The three argument checks share one resolver, so they cannot disagree about what a command means.

## How a command is judged

### What was wrong (H1)

The scan used to split a command on whitespace, match each token against the patterns as a
literal path, and let a match through unless that literal existed on disk. Measured on
2026-09-10 from a public-model chat in Auto mode: `cat /Users/…/.aws/credentials` was refused,
while `cat ~/.aws/credentials`, `cat $HOME/.aws/credentials`, `cat …/.aws/cred*` and
`cd …/.aws && head credentials` all returned the AWS key, as did `~/.ssh/id_ed25519`,
`~/.ssh/*.pem` and `~/.config/biorouter/secrets.yaml`. The pattern matched `~/.aws/credentials`
— the matcher treats `~` as an ordinary directory name — and the `exists()` check then asked
about a directory literally named `~`, found none, and let the command run.

### What happens now

A command is read the way the shell will read it, by the resolver under
[`crates/biorouter-mcp/src/secret_guard/`](../../crates/biorouter-mcp/src/secret_guard/):

- **Quoting** (`lex.rs`). Quotes are removed and pieces concatenated (`cred""entials`,
  `cred\entials`, `$'cred\x65ntials'`), and each character keeps whether the shell may still
  treat it as glob syntax.
- **Expansion** (`expand.rs`). `~`, `~user`, `~+`, `$VAR`, `${VAR}`, `${VAR:-default}`, brace
  expansion (`{credentials,config}`), zsh alternation and bash extglobs (`(credentials|config)`),
  and values assigned earlier in the same command (`H=$HOME; cat $H/.aws/credentials`). A value
  that cannot be known yet — a command substitution, a variable nothing set — is kept as an
  explicit hole rather than guessed.
- **Where the command is** (`resolve.rs`). The set of directories the command may be in is
  tracked through `cd`, `cd` with no argument, `pushd`, `CDPATH`, and a sibling
  `working_directory` argument. The set only grows, because the resolver does not follow control
  flow: after `cd ~/.aws; cat credentials`, `credentials` is judged in both the old directory and
  `~/.aws`.
- **Globs** are matched against what the directory really holds (`cred*` finds `credentials`),
  honouring the shell's rule that `*` does not reach a dot-file unless `shopt -s dotglob`,
  `GLOBIGNORE`, `setopt globdots` or a zsh `(D)` qualifier turned it on. `**/` is expanded by a
  bounded walk that, like zsh, does not enter dot-directories or follow symlinked ones.
- **Symlinks** are resolved before the final match, so a link into `~/.aws` is judged as
  `~/.aws`. On macOS `realpath` also returns the on-disk case, and every match is made with
  case folded, because on APFS and NTFS `~/.AWS/Credentials` opens the real file.
- **Nested scripts** are followed: `sh -c` / `bash -c` anywhere in a command (including after
  `find -exec`, `xargs` and `sudo`), `eval`, here-documents, command substitutions, and the
  strings code hands to a shell (`os.system("…")`, backquotes, AppleScript's
  `do shell script "…"`). Path literals inside code (`python -c "open('…')"`) are judged too.
- **`find -name`** patterns are judged as the file names they will find, so
  `find ~ -name credentials -exec cat {} \;` is refused.

Each resolved path is matched component by component against the deny set.

### Fail closed

**A match is a refusal, whether or not the file exists.** Existence is consulted in exactly two
places, and both can only add a refusal: glob expansion, and symlink resolution. Consequences
worth knowing:

- Creating a file whose name is in the deny set (`echo X=1 > .env`) is refused. `text_editor`
  always refused that; the shell now agrees with it.
- A command that merely *names* a protected file — a commit message, an `echo` into
  `.gitignore`, `ls -l ~/.ssh/id_ed25519` — is refused, because the literal pass sees the name.
- A project `.biorouterignore` rule applies to files that do not exist yet: with `*.log` in it,
  `cmd > build.log` is refused.

In each case a negation in `.biorouterignore` reopens one specific file on purpose.

### Paths that cannot be known before the command runs

- When part of a path is a hole, the path is refused if the directory it would be read from
  directly holds a protected file (`cat ~/.ssh/$(ls ~/.ssh | head -1)`), or if its known end could
  complete a deny pattern (`cd "$(…)" && cat credentials`).
- A glob that cannot be expanded — too many entries, or a walk past its budget — is refused if
  the directory it would expand in holds a protected file.
- A word that is *entirely* unknown (`cat "$(ls *.csv)"`) is not refused: refusing it would
  refuse every command substitution. That residue is what the output redaction is for.

### Cost

The scan runs on every tool call, so it is bounded: at most 512 symlink resolutions, 64
directory listings of at most 4,096 entries, a 256-directory / 8,192-entry walk for `**`, four
levels of nested shells, and 2,048 path literals pulled out of code. A certain path that cannot
be verified inside those budgets fails closed; a path-looking string pulled out of code does not.
`h1_scan_cost_stays_bounded` holds a 2,000-command line, a 2,000-line here-document and a
10,000-token content field to a few seconds in a debug build.

## Output redaction

[`crates/biorouter/src/guardrails/secret_output.rs`](../../crates/biorouter/src/guardrails/secret_output.rs)
recognises credential material in a tool result and replaces the value with a marker such as
`[REDACTED:aws-secret-access-key]`:

- PEM, OpenSSH and PGP **private-key blocks** — the BEGIN and END lines are kept, the body is
  replaced, including a block cut off by `head` and one embedded in JSON;
- **AWS** access key ids (`AKIA…`, `ASIA…`), the 40-character secret next to one or under an
  `aws_secret_access_key`-style name, and session tokens;
- values of the **provider-key store's own key names** and of names that say they hold a secret
  (`*_API_KEY`, `*_TOKEN`, `*_SECRET`, `*_PASSWORD`, …) in the YAML, env and JSON shapes that
  `secrets.yaml`, the keyring blob and `.env` files use — a bare value only when it looks like a
  literal rather than code, so `API_KEY = os.environ["API_KEY"]` is left alone;
- provider tokens with unambiguous prefixes (`sk-ant-`, `sk-proj-`, `AIza`, `ghp_`,
  `github_pat_`, `xox?-`, `hf_`).

It runs inside the future `dispatch_tool_call` returns, so the agent loop, the Claude Code and
Codex tool bridge (whose results never pass the agent loop's own output guardrail),
`POST /agent/call_tool` and code execution's sub-calls all receive the redacted result. Errors are
redacted as well: a failing `automation_script` puts the script's output in its error. A redacted
result carries `_meta.biorouterSecretRedaction` (`{count, kinds}`); the agent loop turns that into
a `[BIOROUTER GUARDRAIL]` line above the untrusted-data frame, and the daemon logs a warning that
names the tool and the kinds — never a value. There is no switch.

## Known gaps

This is a safety net against mistakes and against a cooperative model being steered, not a
boundary against a determined adversary — the same ruling that governs
[privacy tiers](privacy-tiers.md).

- **A path a program assembles while it runs** — `python -c` joining `".aws"` and
  `"credentials"`, base64, a script written a turn earlier — is invisible to a text scan. Only
  the output redaction stands behind it, and only for the formats above.
- **Recursive readers that name an ancestor directory** — `grep -r … ~/.aws`, `tar c ~/.ssh`,
  `cp -r ~/.ssh /tmp/x`, `find ~/.aws -type f -exec cat {} +` — are not refused by the argument
  scan; refusing every recursive command in a directory holding a `.env` would refuse ordinary
  work.
- **Formats the redactor does not know** — a password in a file with no recognisable shape —
  pass through.
- **Live shell output** streamed to the desktop as progress notifications is not redacted. It
  reaches only the user's own screen; the model receives the final, redacted result.
- **The deny set is editable by the agent**: a negation written into `.biorouterignore` reopens a
  file, as it always could.
- **Windows shells** are judged by the literal pass and by the POSIX reading of the command;
  PowerShell's own `$env:` expansion and `Set-Location` are not modelled.

## Tests

```bash
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter-mcp --lib -- secret_guard h1_
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --lib -- secret_output extension_manager guardrails::tool_output
```

The H1 tables run every measured spelling — and the families around them — against a throwaway
HOME holding made-up credentials, through the dispatch scan, through the resolver alone, through
`developer__shell`'s own check and through `automation_script`'s. The fake key material is
assembled at run time so no key-shaped literal sits in the source. None of the tests reads the
real `~/.aws`, `~/.ssh` or `~/.config/biorouter`.

## Related documentation

- [Privacy tiers](privacy-tiers.md) — the tier barrier this floor sits beside, and the
  safety-not-security ruling both follow
- [Secret storage](secret-storage.md) — where the provider keys the redaction recognises are kept
- [Permission modes](permission-modes.md) — why no mode, Autonomous included, opens this floor
