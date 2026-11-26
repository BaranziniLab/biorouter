# Security checklist for outside skills, plugins and extensions

> **What this is.** The checklist every deep reader and every independent security reviewer applies to an adoption candidate, the grades, the bar Biorouter sets, and the patterns that were rejected in the November 2026 triage.
> **Status:** Current, distilled from the 2026-11 research, glebis, ecosystem and harness triage streams.
> **Audience:** agents running stage 3 (first pass) and stage 4 (independent review) of the external skill triage, and the reviewers of implemented items.

## How a review runs

- **Two passes.** The deep reader does a first pass on every item it does not skip. A different Opus agent then reviews each candidate adversarially and is told the reader's grade is a claim to test, not a result.
- **Read everything the item can run.** Every file of the item: `SKILL.md`, references, scripts, hooks, plugin and package manifests, lock files, MCP server code, and any install or release workflow the item is distributed through. Follow every command the item tells an agent to run, and every script those scripts call.
- **Static reading only.** Never execute anything from the source. A behaviour that only running could confirm is recorded as unconfirmed and becomes a required change or a test the implementer runs later, under `sandbox-exec` with the network denied and invented data.
- **Evidence is a path and a line.** Every finding names the file and line (or the command) that shows it, a severity and the change that would remove it.
- **Write the review and return it.** The schema carries `path`, `grade`, `findings` (severity, title, evidence, required change) and `adoption_conditions`.

## The checklist

1. **Network.** Every endpoint the item contacts and what it sends: research data, sequences, patient data, file contents, transcripts, mail. Telemetry, analytics and update checks.
2. **Credentials.** What keys or tokens it needs, where it stores them (environment, file, keychain), and how it passes them. A key on a command line is visible in `ps`; a key in a URL ends up in logs and error messages. Can a key reach a host other than its own (redirects, model chosen URLs, a "custom" target)?
3. **Execution.** Shell or code execution, and how commands are built: injection through file names, URLs, user text or model text. Evaluation of model written code or markup. Flags that skip permissions. Sandboxing, time and size limits for generated code.
4. **Filesystem scope.** Reads or writes outside the working folder: dotfiles, browser profiles, keychains, another app's private database, fixed `/tmp` paths, `git add -A`, whole file rewrites.
5. **Personal data.** Mail, messages, browser history, calendars, meeting recordings, health and wellbeing data, agent transcripts. Where each goes.
6. **Dependencies.** Unpinned installs, `curl | sh`, `npx`, `uvx` or `pip` of unpinned packages, postinstall hooks, vendored binaries, large model or data downloads without a checksum.
7. **Persistence.** Cron jobs, launch agents, systemd units, hooks, background daemons, anything that starts itself again.
8. **Prompt injection.** Fetched web pages, PDFs, mail, papers or database records fed to the model with authority. A skill that turns an incoming message into an agent turn. Text in the repo that addresses an AI agent.
9. **Outbound or destructive actions without confirmation.** Sends, posts, uploads, pushes, purchases, deletes, financial actions.
10. **Privacy tiers.** Anything that would send private data or PHI to a public service or to a model the chat's tier does not allow, or reach another institution's private connector. Biorouter's tiers must never be bypassed (CLAUDE.md, "Privacy tiers").
11. **Secret guard.** Anything that reads, or tells the agent to read, a credential file Biorouter's secret guard protects, such as `~/.aws/credentials`, SSH private keys or `.env` (CLAUDE.md, "Secret guard").
12. **Claims against code.** Documentation that promises more than the code does: a dry run that writes, a secret skip that does not skip, a "local only" tool that calls an API.
13. **Third party terms.** Data provider terms (non-commercial APIs, share alike data), naming or trademark limits in a NOTICE.
14. **Clinical and wellbeing content.** Medical advice without scope limits, a crisis protocol that is referenced but missing, real patient data in examples.

## Grades and severities

| Grade | Meaning | What happens |
|---|---|---|
| `safe` | Nothing above applies, or only info findings | Adopt as proposed |
| `safe-with-changes` | Each finding has a stated change that removes it | Every required change becomes an acceptance criterion of the work item |
| `unsafe` | A high finding with no change that removes it, or a design that is the risk | Rejected; its method may inform an original Biorouter skill as ideas only |

| Severity | Typical shape |
|---|---|
| high | A credential or private data can leave the machine or reach the wrong host; code runs or a message goes out without the person's yes; a destructive action without confirmation |
| medium | A leak that needs an unusual input; an unpinned dependency; a missing limit on generated code; a claim the code does not keep |
| low | Hardening, wording, a narrow edge case with no data at risk |
| info | Context for the implementer |

## The Biorouter bar

Biorouter users handle PHI and unpublished research, agents have a shell, chats have privacy tiers, and Smart approval asks before writes. So an adopted item:

- is strictly opt in: anything that reads personal data, sends anything outward or costs money runs only after the person says yes in the chat;
- uses Biorouter's own tools and extensions for network access instead of shell fetches around them;
- takes keys from Biorouter's secret store or the environment, never from a command line, a URL or a file in the working folder;
- treats every fetched text as data, escaped before it reaches a prompt, and says so in the skill;
- keeps generated code inside a stated sandbox with time and size limits;
- never sends a private chat's data to a public model or service.

## Patterns rejected in 2026-11

These were graded unsafe, and only their methods survived as ideas. Expect the same shapes again:

- an encryption password on the command line, and every original value printed to stdout by a `--json` flag;
- private mail, chat posts and notes sent to whatever model a command line tool defaults to;
- unconfirmed sends of any local file to fuzzily matched recipients, by an agent that reads untrusted messages;
- a daemon that turns any incoming chat message into an agent turn with shell and edit tools;
- `git add -A` and `--no-verify` inside a "dry run" that then pushes;
- browser history from every synced device placed in model context, and scraping of deleted entries;
- force quitting another app and writing into its private database;
- agent transcript text sent to a cloud embedding API on the command line;
- a wellbeing skill whose distress protocol file was missing and which named no crisis line;
- browser automated tax and invoicing actions, with the author's tax and bank identifiers published in the repo;
- headless agent runs with permissions skipped, started from a launch agent, with meeting titles reaching code.

## After adoption

- The implementer meets every required change, adds a test for each change that involves code, and runs data tools only on invented data with the network denied.
- An independent Sonnet reviewer checks that each condition was met; re-run on Opus only if the Sonnet verdict is unsure.
- A fix of a high or medium finding gets its own independent review, told the earlier findings and asked to look for new ways in that the fix opened, with a generated variety of inputs.
- A review loop needs a round cap and a stop rule set before it starts, for example "no high, and mediums only as documented limits". At the cap, bring the residual list to the owner.
- For a detector of sensitive data, design it to fail closed from the start: keep only what can be verified, and let the person keep more by name. Patching heuristics round by round never converged in 2026-11.
- Review the published zip or `.brxt`, not only the folder it was built from.

## Related documentation

- [../SKILL.md](../SKILL.md): the triage process and its rules.
- [licence-policy.md](licence-policy.md): what may be copied at all.
- [implement-release-list.md](implement-release-list.md): where the per item review and the published artifact review sit.
