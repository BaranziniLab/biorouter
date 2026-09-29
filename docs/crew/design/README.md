# Crew design and reference

The design and reference documents for Crew's code: the broker's wire contract, how people and places are named, the desktop interface, the `biorouter crew` command line in depth, the SSH rules and the Linux build. They describe how Crew works now and are kept in step with the code: a row that a later decision changed says so in place, and where a document and the code disagree, the code is what shipped. Source comments cite these files, and `node --test scripts/check-crew-manual.test.mjs` checks the UI spec, the broker protocol, the naming design and the CLI guide against the code they describe.

This folder is for developers and reviewers of Crew. If you use Crew, read the [Crew user manual](../README.md) one level up instead. The plans, reviews and test evidence of the campaign that built Crew, from the first research on 2026-09-21 to the merge on 2026-09-27, are in [the Crew build campaign](../../history/biorouter-crew/README.md): read them to learn why something is shaped the way it is, not what it does today.

| Document | What it holds |
|---|---|
| [Broker protocol](protocol-contract.md) | The wire contract of the broker in `crates/biorouter-crew`: transport and identity, `hello` v1 and v2 and its capabilities, the collaboration methods and what they project, names, joining by invitation and device code, attachments, owned-agent grants, presence, durability and storage faults, quotas and the rootless setup checklist |
| [Naming design](naming-design.md) | How people, workspaces, teams and channels are named instead of numbered: display rules, name keys, uniqueness, the daemon resolver, joining by invitation and device code, decisions D1 to D17 and slices S0 to S4 (S4 is deferred) |
| [UI redesign specification](ui-redesign-spec.md) | The Slack-like Crew desktop interface as approved on 2026-09-23, with the rows later decisions changed marked in place: layout, every screen, the SSH failure table, component architecture, the copy deck, identity display, revoke, privacy, motion, accessibility and test migration |
| [Native CLI guide](cli-guide.md) | `biorouter crew` in depth, for developers and testers: the shared desktop daemon and its credentials, names, joining, messages, files, agents, chat grants, the JSON output contract, retries and recovery. The [command-line manual](../command-line.md) is the user's page |
| [SSH hop policy](ssh-hop-policy.md) | The rules Crew applies to every SSH hop before it signs in: the configuration each hop needs, host-key trust, multiplexing, and what is refused |
| [Linux portability](linux-portability.md) | How the Linux broker is built for the release target and its glibc floor, how to qualify and install it without administrator access, and one dated rootless smoke |

## Related documentation

- [Crew user manual](../README.md): how to use Crew, for lab members, hosts and IT staff
- [Crew build campaign](../../history/biorouter-crew/README.md): the plans, reviews and evidence behind these documents
- [Privacy tiers](../../security/privacy-tiers.md): the privacy rules Crew's private and public boundaries rest on
- [Documentation organization](../../organization.md): why living design sits here and campaign records under `history/`
