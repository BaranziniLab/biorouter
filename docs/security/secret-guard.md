# Secret guard

> **What this is.** How BioRouter keeps credential files and credential values away from the
> model: the always-on deny floor, how a tool call's arguments are resolved before they are
> judged, and the redaction of credential material in what a tool returns.
> **Status:** Current. The argument scan was rebuilt and the output redaction added on
> 2026-09-11, fixing QA-C finding H1; the sections below describe that code. Seven well-known
> password and token stores joined the floor on 2026-09-25 (Crew live QA round 4, Q4-56). On
> 2026-11-17 the symlink budget began to fail closed (SG-30) and the served page's two file routes
> began to apply the floor and the redaction (SG-27). Since #546 (1.94.6) a quoted here-document
> that only writes a file is read as data; see "Here-documents" below. On 2026-11-19 conventional
> `.env` templates were reopened, and the dispatch scan began to leave out the free text of the
> Todo tools and the source of `execute_code` (issue #522). Since 2026-11-20 a shell may not start,
> copy or link an app bundle in Biorouter's extensions folder, and a wrapper's options are read
> with their values; see "Extensions' helper apps" below.
> **Audience:** developers working on tool dispatch, the Developer and Biorouter Copilot
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

Since 2026-09-25 it also names `.netrc`, `_netrc` (its Windows name), `.pgpass`,
`.git-credentials`, `.docker/config.json`, `.kube/config` and `.config/gh/hosts.yml`. Crew's live
QA (round 4, Q4-56) shared `~/.netrc`, `~/.pgpass`, `~/.git-credentials`, `~/.docker/config.json`
and `~/.kube/config` from a renderer with a 200: none was on the floor, and what each holds — a
netrc `password` line, a pgpass `host:port:db:user:pass` row, a `https://user:token@host` URL,
docker's base64 `auth`, a lowercase kubeconfig `token:` — is a shape the output redaction below
does not recognise, so the name is the only thing that stops a read.
Their neighbours stay readable: `.git/config`, `.ssh/config`, `.docker/daemon.json`,
`.kube/cache/…` and `.config/gh/config.yml`.

Since 2026-09-29 it also names `daemon/user-action-key.json`. That file holds the key that tells
the shared background service a person is acting through the desktop app or `biorouter crew`.
It lives beside the service's `runtime.json`, at `~/.local/state/biorouter/daemon/` by default
(`$BIOROUTER_PATH_ROOT/state/daemon/` or `$XDG_STATE_HOME/biorouter/daemon/` when set). Whoever
starts the service writes it with mode 0600 in a 0700 directory, and the service removes it when
it exits. The name stops a model's casual read. The computer belongs to its user, so the design
does not try to stop an agent the user runs from acting as them. `runtime.json` is not on the floor.

Since 2026-11-20 it also names the cookie and password stores of Google Chrome and of the
Biorouter Browser (BB-25 in [the built-in browser](../desktop-ui/built-in-browser.md)): `Cookies`,
`Login Data`, `Login Data For Account` and `Web Data` under a `Google/Chrome/` folder (macOS
`~/Library/Application Support/Google/Chrome/`, Windows `…/Google/Chrome/User Data/`) or a
`google-chrome/` folder (Linux `~/.config/google-chrome/`), at any profile depth, and `Cookies`
under the browser's own partition, `Partitions/biorouter-embedded-browser/`. These hold a person's
signed in sessions, saved passwords and payment data, which is what the browser's Chrome import
takes care never to read (passwords) or to let leave the app (cookie values). Chrome's other
files (`Bookmarks`, `Preferences`, `Local State`) and any file merely called `Cookies` elsewhere
stay readable. Like the rest of the floor, this stops an accident, not a determined program.

Since 1.94.6 it also names gptr's session records, `serve-*` under a `gptr/mcp-serve/` folder
(R's per-user cache: macOS `~/Library/Caches/org.R-project.R/R/gptr/`, Linux `~/.cache/R/gptr/`,
Windows `…/AppData/Local/R/cache/R/gptr/`). gptr Agent's attach step writes one for each live R
session it serves, and it holds the bearer token of that session's gptr server, which runs R code
in the person's open session. The extension reads the record itself, so no chat's shell needs it.
gptr's other cache files stay readable.

### The Chrome key helper (BB-23)

One command is refused by what it runs rather than by a path: `biorouterd browser-key`, the
hidden helper that reads the key Chrome encrypts its cookies with, for the desktop app's
**Import from Chrome**. On macOS a person who once answers the Keychain prompt with **Always
Allow** has allowed `biorouterd` to read that key silently from then on, and an agent's shell
runs `biorouterd` as the same person. So the command scan refuses any command that names the
helper, in every spelling the resolver can read: a path or another case (`BIOROUTERD`, `.exe`),
quotes and backslashes, `env` and the other wrappers, variables and earlier assignments, `sh -c`,
`eval`, here-documents, an `alias` body, a command substitution, and a list or string handed to
`subprocess` or `system`. It judges words, not paths: the subcommand `browser-key` together with
either the program `biorouterd` or the `--source` option the helper cannot run without, anywhere
in one tool call, so a renamed copy (`/tmp/x browser-key --source chrome`) is refused too, while
`rg browser-key` (searching this code) is not. The refusal tells the model to ask the person to
import from Chrome themselves (`HelperSighting` in `secret_guard/resolve.rs`).

This is the first of two layers. A static scan cannot see a command assembled at run time
(`printf` pieces, base64), so the helper itself also refuses any caller but the desktop app's
main process, and answers that one only masked with a one-time request on its standard input
(`crates/biorouter/src/browser_key.rs`; [the built-in browser](../desktop-ui/built-in-browser.md),
"The macOS Keychain prompt (BB-23)").

### Extensions' helper apps

A second command is refused by what it starts. An extension installed into Biorouter's
extensions folder may ship a helper app that the person granted macOS Accessibility and Screen
Recording, and an agent's shell runs as that person, so a shell that started the helper would
look at and control the desktop with none of the extension's own consent. The command scan
therefore refuses a command that starts an app bundle (a folder whose name ends in `.app`) inside
that folder:

- the command's program is a path inside such a bundle: the real program behind `sudo`, `env`,
  `nice`, `timeout`, `exec`, `nohup` and the other wrappers, a program in the command line
  `env -S` splits, or the one `xargs`, `arch` or `find -exec` runs. Each wrapper's options are
  read as its `getopt` reads them (`wrapper_spec` in `secret_guard/resolve.rs`), so an option's
  value (`nice -n 10`, `sudo -u root`, `env -u FOO`, `timeout -s KILL 5`) is never taken for the
  program. When a wrapper's option is one its table does not know, the word after a bare name is
  read as the program too, since the option may have taken that name as its value;
- `open` starts something in such a bundle: the app `-a` names by path or name, or `-b` by bundle
  identifier, or an operand (a path, or a `file:` URL, also as `-u`'s value) that is an app
  bundle, the extension's own or one nested in it, or a file in one that can run, which Terminal
  would run. A folder in a bundle opens in the Finder and any other file in it opens in the app
  that reads it, so those stay allowed, as does every operand of `open -R` (reveal in the
  Finder), `-e` and `-t` (open in a text editor);
- `osascript -e` names it as an application (`application "Name"`, `application id "id"`,
  JavaScript's `Application("Name")`);
- `cp`, `mv`, `ln`, `link`, `ditto`, `rsync` or `install` copies, moves or links it: a source is
  such a bundle, a folder or a file that can run inside one, or a folder in the extensions folder
  that holds one. macOS keys the grants to the app's code signature, not its path, so a copy keeps
  them, and a link starts the bundle itself. The start rule cannot follow either, because the copy
  or the link does not exist yet while the command is read. Copying a file in a bundle that cannot
  run (its `Info.plist`, a resource) is reading, and a destination is never judged, so copying
  into the folder stays allowed.

The folder is `<config>/extensions` as Biorouter resolves it, `$BIOROUTER_PATH_ROOT/config/extensions`
when that is set, together with the platform folder (`~/.config/biorouter/extensions` on macOS
and Linux), which a run under a path root can still reach. A path is resolved as every other word
is (`~`, variables and earlier assignments, globs, `cd`, links, `sh -c`) and compared without
regard to case. A name or identifier is matched against the bundles found in the folder: each
bundle's file name and the `CFBundleName`, `CFBundleDisplayName`, `CFBundleExecutable` and
`CFBundleIdentifier` its `Info.plist` gives (`Resolver::refuse_extension_app` in
`secret_guard/resolve.rs`, `SecretGuard::extension_bundle_of` in `secret_guard.rs`).

The rule names no extension: it is about the folder, so it holds for any extension that brings a
helper app. It is separate from Biorouter Copilot's own rule for its engine (AD-12 in
[the Copilot design](../design/copilot-agent-desktop.md)), which it does not change. Reading,
listing and inspecting the folder stay allowed (`ls`, `cat`, `find`, `du`, `plutil -p`,
`codesign -dv`, `open` on the folder itself, `open -R` on a bundle, `open -a TextEdit` on its
`Info.plist`, copying that file out), with or without a wrapper in front (`sudo -u root cat …`),
as does starting an app that is not in it. The refusal tells the model to use the extension's own
tools.

The floor is not only the agent's. Crew's local file registration
([`crates/biorouter-server/src/crew/local_files.rs`](../../crates/biorouter-server/src/crew/local_files.rs))
refuses an upload source or a download destination the floor names (Q3-01), consulting only the
machine-wide statements: the floor and the global `.biorouterignore`.

A `.biorouterignore` adds patterns: the project's own file for paths inside the project, and the
global `<config>/.biorouterignore` for every path. Either can reopen one specific file with a
gitignore negation (`!path`), because user patterns are layered after the floor. A pattern with a
slash is anchored, so reopening a store at any depth takes `**/`: `!**/.kube/config` in the global
file reopens every kubeconfig, for the agent and for Crew, and leaves the rest of the floor
standing (`q4_56_a_negation_reopens_one_store`).

### Templates

`.env.*` also matches the templates a repository commits next to its `.env`, which hold variable
names and no values. Since issue #522 the floor reopens them by exact name
(`DEFAULT_SECRET_TEMPLATE_EXCEPTIONS`): `.env.example`, `.env.sample`, `.env.template`,
`.env.dist`, `.env.tpl`, `.env.defaults` and `.env.<anything>.example`, at any depth. Both guard
constructors add these negations right after the floor and before any `.biorouterignore`, so a
project or the global file can deny a template again with an ordinary pattern
(`issue522_explicit_project_policy_can_still_deny_a_template`). A template is read in any case:
`.env.Example` and `.ENV.EXAMPLE` both open the template on APFS and NTFS. The floor's patterns are
case-sensitive, so `.env.Example` matched `**/.env.*` and missed the lowercase exceptions, and was
refused until `SecretGuard::is_denied` reopened it. It reopens such a name only when the floor's
own `**/.env.*` line is the only one that denied it, and the whole path in lowercase is not denied,
so a `.biorouterignore` line that names the template in either case still holds
(`a_template_spelled_in_another_case_is_readable_and_nothing_else_is`). What stays refused:

- every other `.env` name, including `.env.local`, `.env.production` and `.env.example.bak`;
- a template that is a symlink, which is judged as the file it reaches, so a link from
  `.env.example` to `.env` is refused (`issue522_template_alias_cannot_reopen_a_real_secret`);
- a glob that could reach a secret beside the template: with `.env.local` in the folder,
  `cat config/.e*` is refused, while `cat config/.[e]nv.example` is not;
- the destination of `cp config/.env.example config/.env`, which the refusal names on its own;
- a real `.env` name in another case, such as `.Env.local` or `.env.Production`, which folds to a
  name that is no template.

## Where it runs

| Place | What it checks |
|---|---|
| `ExtensionManager::dispatch_tool_call` (`secret_guard_denial`) | Every tool call's arguments, for every extension. The one choke point every call passes. The free text of the shipped Todo tools and the source of `execute_code` are left out; see "Free text" below. |
| Developer server, `validate_shell_command` | `developer__shell`'s command, from the directory it will actually run in. |
| Developer server, `is_ignored` | `text_editor` and `image_processor` paths, after symlinks are resolved. |
| `call_tool_withholding_secrets`, inside the dispatched future | Every tool **result** and error, before any model sees it. |
| Biorouter Copilot tools | Nothing path-shaped for the argument scan to judge: their arguments name a display, an application or an element, not a file. The result redaction applies to their text, not to their screenshots. |
| `/headless/fs/read` and `/headless/fs/artifact` (`refuse_floor` in `routes/shell.rs`) | The file a served page's preview asks for, or any caller holding the daemon secret (which a public chat's shell can recover), after its links are resolved: the floor and the nearest project `.biorouterignore` refuse it whether or not it exists, and a text body the route does return passes the output redaction below. Both routes ask the same question, so they cannot disagree about a file. |

The command argument checks share one resolver, so they cannot disagree about what a command means.

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
  The refusal names the path inside the script and what it resolves to:
  `bash -c 'cat ~/.aws/credentials'` is refused as `~/.aws/credentials`, resolved under `HOME`,
  not as the whole script joined to the working directory. When the script cannot be read far
  enough to name a path, the refusal quotes the script itself, still with no file let through.
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
  The literal pass judges each word of a value, and the whole value as one more path, except
  for a command line of several words, whose words the resolver reads: until issue #522 it took
  a whole command line as one path too, so `cat config/.env.example | head` was refused because
  the text after its last `/` matches `.env.*`. Prose keeps the whole-value candidate, because an
  unknown tool may take a path with spaces (`/keys/My Server.pem`) under a key that does not
  look like one, so a template mentioned in prose and followed by more words is still refused
  there unless the tool is one of those under "Free text" below.
- A project `.biorouterignore` rule applies to files that do not exist yet: with `*.log` in it,
  `cmd > build.log` is refused.

In each case a negation in `.biorouterignore` reopens one specific file on purpose.

One consequence a negation does not undo: **a floor entry's last name reaches further than its
path.** Where part of a path is unknown (next section) and in `find -name`, the known end is
compared with the *end* of each floor pattern, so a pattern's final component refuses on its own.
That is how `find . -name '*.json'` was already refused (it could name `.codex/auth.json`).
Measured when Q4-56 landed, the new entries add `find . -name '*.yml'` and
`find . -name hosts.yml` (from `.config/gh/hosts.yml`), `find . -name config` and
`find . -name 'config*'` (from `.kube/config` and `.docker/config.json`),
`cd "$(git rev-parse --show-toplevel)" && cat config.json`,
`cat "$(git rev-parse --git-dir)/config"` and `cat $UNSET/config.json`. Known paths are
unaffected: `cat .git/config`, `cat src/config.json`, `cat inventory/hosts.yml` and
`git config --list` all run. This comparison reads the built-in floor only, so a
`.biorouterignore` negation does not reach it; spelling the path in full does.
`.config/gh/hosts.yml` buys the least for its cost, because the output redaction already
recognises the `gh[pousr]_` token it can hold, and is the entry to revisit first if the collateral
proves expensive.

### Free text

The scan cannot tell which tool a key belongs to, so a slash path in prose (`text`, `content`,
`message`, `code`, any key it does not know) is judged as a possible file access. That has to
hold for `code`: under Compute or an external runner it is a program that can open the path.

The dispatch boundary does know the tool, and since issue #522 it leaves out the free text of
two shipped Platform extensions (`FreeTextFields` in
[`crates/biorouter/src/agents/extension_manager.rs`](../../crates/biorouter/src/agents/extension_manager.rs)):

- **Todo**: `todo_write`'s `content`, the `items` of `todo_add` and `todo_expand`,
  `todo_update`'s `text` (top level and in each `updates` entry), and `plan_write`'s `plan`.
  The text is stored in the checklist and shown back to the model; the Todo handlers never
  open it. A todo item such as "configuration/.env copy still needs the user" is admitted.
- **Code Execution**: `execute_code`'s `code`, and the `description` of each `tool_graph`
  node. The script runs in a sandbox with no file or process access of its own. Every tool
  call it makes is dispatched back through `dispatch_tool_call` with its evaluated arguments
  and judged there, which is also the only place a path the script assembles at run time
  (`"configuration/" + ".env"`) can be seen.

Only a string in exactly those places is left out, and only from the copy the scan reads; the
tool receives the original arguments. Every other field of those calls, a value of the wrong
JSON type, a client injected under the same name instead of built by the Platform registry,
a nested key such as `todo__x`, and every other extension, Compute included, is scanned as
before.

### Paths that cannot be known before the command runs

- When part of a path is a hole, the path is refused if the directory it would be read from
  directly holds a protected file (`cat ~/.ssh/$(ls ~/.ssh | head -1)`), or if its known end could
  complete a deny pattern (`cd "$(…)" && cat credentials`). A known final name that is a
  template the guard reopens is not such an end, so
  `cat "$(git rev-parse --show-toplevel)/config/.env.example"` runs.
- A glob that cannot be expanded — too many entries, or a walk past its budget — is refused if
  the directory it would expand in holds a protected file.
- A word that is *entirely* unknown (`cat "$(ls *.csv)"`) is not refused: refusing it would
  refuse every command substitution. That residue is what the output redaction is for.

### Here-documents

Every here-document body used to be read three ways: as a script, as code, and again inside the
whole command's code pass. That is right when something runs the body (`bash <<'EOF'`,
`cat <<'EOF' | sh`, `python3 - <<'EOF'`), and it refused ordinary file writes (#546).
`res$IHS[is.finite(res$IHS)]` in an R script read as a path with an unknown middle whose end
could spell `credentials`; a backquoted R expression in Markdown read as a command substitution;
and a 600-word prose body spent the 512 symlink resolutions on words that name nothing, then
failed closed as too complex.

The lexer (`lex.rs`) now records four facts about each here-document: whether the delimiter was
quoted, whether every POSIX shell reads that delimiter the same way, whether a line equal to it
ended the body, and the body's byte range in the input. The delimiter word is always read with
POSIX quote removal, under the Windows grammar as well, because `<<` is POSIX syntax: otherwise
the Windows reading of `\EOF` keeps the backslash, never meets the `EOF` line, and turns the
rest of the input into body. Only that word is read this way; the next word gets the pass's own
grammar back.

A body is **data** when all of this holds:

- Every command in the input is a plain file write, `cat > FILE <<'EOF'` (or `>>`, `>|`, `<<-`,
  redirections in any order), joined only by newlines, `;`, `&&`, `||` or `&`. `FILE` is literal
  text.
- The delimiter is quoted (`'EOF'`, `"EOF"`, `\EOF`, `'E'OF`) and made only of letters, digits,
  `_`, `.` and `-`.
- A line equal to the delimiter ends the body.

Anything else keeps the three readings: an unquoted delimiter, `$'EOF'` (bash and zsh decode it
and dash does not, so the shells disagree on which line ends the body), a missing or empty
delimiter, a carriage return straight after the delimiter (the shell reads it as part of the
delimiter, so its body ends at an earlier `EOF\r` line and the lines after that run), a body
that runs to the end of the input, a pipe, a subshell, a wrapper such as
`sudo`, an extra argument, a target with a variable in it, and any other command in the same
input, including the `bash x.sh` that would run the file just written. Only a scan's top-level
command is classified. A body inside `bash -c`, `eval` or a command substitution stays a script,
because its output can be captured and run.

A data body is still read as a script and as code, one level deeper and with its own copy of the
directory set, because a file written now can run later where the guard cannot look inside it.
A protected path in any spelling, a link into a secret store, an unknown name in a folder that
holds a secret, nesting past four levels and a link past the symlink budget are all refused as
before. Two refusals are dropped, and both are guesses about text this command does not run:

- The known end of a partly unknown word is not matched against the floor.
- A literal path that `lstat` reports as not found is not resolved, so prose does not spend the
  symlink budget. The question is asked of the path as written, through the same lookup `open`
  makes, so a link on the way or a spelling the disk folds still finds the file and is resolved.
  Only "not found" skips; any other answer is resolved, and past the budget fails closed.

A zsh `(D)` or a `shopt -s dotglob` inside a data body applies to that body only. The command's
own code pass and dot-glob check read the command with data bodies blanked, because the body has
already been read.

A body that is not data is also read for quoted strings, each judged as one literal path:
`python3 - <<'EOF'` with `open('.env')` names the file by a string with no `/`, which the code
pass does not look for, and the script reading sees `print(open('.env').read())` as one word. A
data body skips this, because writing a file that mentions `.env` reads nothing. The literal
whitespace pass over the whole command still runs, so a protected path written out in full, such
as `~/.aws/credentials`, is refused inside a data body too.

A refusal for a word inside a body says so, and says how the body was read. A body read as a
script adds that the guard reads it as a script this command may run, and that file content goes
in a call of its own as `cat > FILE <<'EOF'` with the delimiter quoted. A data body adds that the
guard still reads a body written to a file, because the file can be run later. Without it, a model
that wrote the body as text looked for the refused word among its commands and found nothing.
When bodies nest, the outermost one is named, because that is the one the caller wrote.

### Cost

The scan runs on every tool call, so it is bounded: at most 512 symlink resolutions, 64
directory listings of at most 4,096 entries, a 256-directory / 8,192-entry walk for `**`, four
levels of nested shells, and 2,048 path literals pulled out of code.

Each word a command uses as a path spends one of the 512 symlink resolutions and records where its
folder really is. Once they are spent, a word is still judged when it can be placed without one: a
file that is not a link, in a folder already recorded (or reached from one through folders that
are not links), where every name added on the way is one the disk cannot spell another way, which
means printable ASCII, no `~`, `:` or other character Windows reserves, and no trailing `.` or
space, because a case-insensitive or short-name filesystem opens the same file under such a
spelling and only `realpath` reports the real one. Any other certain path past the budget is
refused as too complex: a link, a name outside that set, a folder never seen. A glob in a
directory the listing budget can no longer read is refused as well. So a command naming 600 files
in one folder is judged in full, while 700 benign links and then one to `.netrc` is refused. Until
2026-11-17 everything past the 512th resolution was let through instead, and that command read the
password (SG-30). A path-looking string pulled out of code does not fail closed; past a budget it
keeps the verdict its spelling alone gives.

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
`POST /agent/call_tool` and code execution's sub-calls all receive the redacted result. The served
page's `/headless/fs/read` and `/headless/fs/artifact` pass the text they return through the same
`redact_text`: a file's text and the text extracted from an Office document. A document's own
bytes and an image are returned as they are. Errors are
redacted as well: a failing tool can include command output in its error. A redacted
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
  pass through. That is why the stores holding such passwords are named on the floor instead; a
  store it does not name (a tool's own token file under another name) is not protected.
- **Live shell output** streamed to the desktop as progress notifications is not redacted. It
  reaches only the user's own screen; the model receives the final, redacted result.
- **Images are not redacted.** `redact_call_tool_result` handles text parts and text resources
  only; every other content kind falls through untouched. A Biorouter Copilot `screen_capture` or
  `get_app_state` screenshot therefore reaches the model as it was taken, so a credential visible
  in a terminal or an editor is not withheld. The per-request Biorouter Copilot approval is the only
  control there.
- **A file written by a here-document and run later** is beyond the check, as any script
  written earlier is. A data body is still held to the floor (above), but a path in it whose end
  only *could* name a secret, such as `cd "$(…)"` followed by `cat credentials`, is let through
  when the body only writes a file.
- **The deny set is editable by the agent**: a negation written into `.biorouterignore` reopens a
  file, as it always could.
- **An extension's helper app started in a way the scan cannot read** is not refused: a copy made
  some other way (an archive with `tar` or `zip`, a copy in another language) or of a folder above
  the extensions folder (the config folder or the home folder, as a backup copies them) and
  started from the copy, a copy whose source the command only learns when it runs
  (`find … -exec cp {} …`, `xargs cp`), a program path whose start is only known when the command
  runs (`"$(…)"/Contents/MacOS/…`), a relative path after a wrapper that changes folder first
  (`env -C DIR`, `sudo -D DIR`), which is resolved against the shell's own folder as every other
  word is, a program a script in another language starts (`subprocess.run([...])`), or a launch
  through some other service (`launchctl`). The operating system's own grant still decides what
  the app may do.
- **Windows shells**: `\` is read as a path separator (not a POSIX escape), so a native
  `C:\Users\…\.aws\credentials` — bare, quoted, or after `cd` — is resolved and matched. What is
  *not* modelled is PowerShell's own `$env:` expansion and `Set-Location`, and a backslash path that
  appears only inside a non-shell **code literal** (`python -c "open('C:\\…')"`): the code-literal
  scanner keys on `/`, so a `\`-only path there is missed. The output redactor is the backstop for
  that residue — it withholds the credential bytes from the result whatever the path spelling.

## Tests

```bash
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter-mcp --lib -- secret_guard h1_ bb_23_ extension
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter-mcp --test developer_guard_execution
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --lib -- secret_output extension_manager guardrails::tool_output
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter-server --lib -- crew::local_files
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter-server --lib -- routes::shell
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter-mcp --lib issue522
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter-mcp --test developer_guard_execution issue522
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --features local-dictation --lib issue522
```

The H1 tables run every measured spelling — and the families around them — against a throwaway
HOME holding made-up credentials, through the dispatch scan, through the resolver alone, through
`developer__shell`'s own check. The `q4_56_` tests hold the seven long-tail stores to the floor by
name, by case and through the H1 spellings (`~`, `$HOME`, a glob, `cd`, `bash -c`,
`find -name`), keep their neighbours readable, and prove a negation still reopens one;
`crew::local_files` holds Crew's registration to the same floor, and `routes::shell` holds the
served page's two file routes to it and to the redaction. The `issue546_` tests (in
`secret_guard.rs`, `secret_guard/lex.rs` and the `developer_guard_execution` integration binary)
hold the here-document rules: the four quoted delimiters as data on both hosts, every form that is
not a plain file write, links, spellings and budgets inside data bodies, and the real shell ending
each body where the guard does. The `issue522` tests hold the
templates open through both guard constructors, the resolver and the real Developer tools, keep
their symlinked, globbed and differently named neighbours refused, and, at the dispatch boundary,
admit the Todo and `execute_code` free text while every other field, lookalike client and
Compute's `code` stay refused. `bb_23_` runs the Chrome key helper's
spellings through the dispatch scan, `developer__shell`'s check and the code scan, and keeps a
search for the helper's name allowed. `an_extensions_helper_app_is_never_started_from_a_shell`
starts a fake extension's helper app in every spelling above, behind every wrapper and its option
values, and keeps reading, listing and inspecting it allowed;
`copying_or_linking_an_extensions_helper_app_is_refused` copies, moves and links it and keeps
copying its `Info.plist`, or anything into the folder, allowed;
`opening_inside_an_extensions_bundle_refuses_only_what_starts` keeps `open -R`, `open -e` and
`open -a TextEdit` on its files allowed; `the_extensions_folder_follows_biorouter_path_root`
holds the folder to `BIOROUTER_PATH_ROOT` and the platform folder. The fake key material is
assembled at run time so no key-shaped literal sits in the source. None of the tests reads the
real `~/.aws`, `~/.ssh` or `~/.config/biorouter`.

## Related documentation

- [Privacy tiers](privacy-tiers.md) — the tier barrier this floor sits beside, and the
  safety-not-security ruling both follow
- [Secret storage](secret-storage.md) — where the provider keys the redaction recognises are kept
- [Permission modes](permission-modes.md) — why no mode, Autonomous included, opens this floor
