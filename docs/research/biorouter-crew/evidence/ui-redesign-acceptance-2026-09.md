# Crew UI redesign acceptance evidence, 2026-09

> **What this is.** The acceptance record of the Crew UI redesign and naming campaign (plan §16): four live QA rounds with fresh novice critics on a disposable AWS fixture, the security results of each round, the seven final acceptance lanes on the final build `461f7899`, the fixture's provenance, and what is still unverified.
> **Status:** Current. It records completed runs from 2026-09-24 14:13Z to 2026-09-25 15:11Z and is complete except the fixture teardown receipts, which the close-out's Finish phase adds to [Fixture teardown](#fixture-teardown). A result here holds for the build it names; later commits need their own runs.
> **Audience:** Maintainers deciding what Crew can claim after this campaign, reviewers of the security-sensitive commits, and whoever resumes Crew acceptance.

The campaign ran the redesigned desktop Crew view and the name-first identity work against real
Unix accounts on one disposable EC2 instance, with every agent turn on the private Versa
`versa_azure` / `gpt-5.5-2026-04-24` model. All data was synthetic. Identifiers used below:
`T-nn`, `Q2-nn`, `Q3-nn` and `Q4-nn` are the findings of rounds 1–4 in the triage reports;
P0/P1/P2 are the triage's severity ranks, P0 the most severe. `G04`–`G15` and `P01`–`P10` are the gates and parity rows of the
[status ledger](../implementation-status.md). Finding IDs inside a final lane (`NEW-1`, `F1`,
`D-1` and so on) are local to that lane. "Unaided" means a novice with only the app would
finish; a step that needed a hint counts against it.

Raw evidence stays local under `/private/tmp/crew-ui-redesign/` and is not published:
screenshots, transcripts, CLI logs, journals and receipts. Every lane swept its own evidence
for secret values and found none. Synthetic fixture evidence does not establish institutional
deployment or HIPAA compliance.

## What this establishes, and what it does not

- **Established on `461f7899`:** three members each shared a file by drop and had their own agent
  compute a correct, source-named result from it (G04/G13 lane); an ordinary chat was granted,
  used and revoked from the GUI, and its next turn was refused before any Crew data left, including
  with the workspace unreachable (revoke lane); mixed GUI and CLI collaboration with identical
  message and file IDs across eight or nine sources (G11 lane); the institution gate refused a UCSF model on a
  foreign-institution workspace in the GUI and the CLI with zero egress; the live observer
  source-ACL test and the six-command Versa self-test passed again.
- **Established across the rounds:** the unaided novice task rate rose from 39% to 91%, P0s went
  from 5 to 0 after round 1, and three brand-new host and joiner pairs completed their journeys.
  Every broker boundary the security critics tested held in every round.
- **Not established:** the real native share confirmation's wording in the GUI lanes (the stage
  answered it automatically), the daemon's worker allowlist exercised by a model, Windows and Linux
  desktops, MFA and jump hosts (the fixture used keys only), load, storage faults, and hosted CI on
  `461f7899`. [What remains unverified](#what-remains-unverified) lists each one.
- **One assertion failed:** G12 (d). A policy change after a grant was refused correctly, but the
  person saw the broker's raw JSON envelope instead of the plain sentence.

## Builds under test

Each round ran on one clean commit. The native CLI and daemon were frozen from `target/debug`
into an immutable directory, the Linux x86_64 broker was built in `rust:1.92-bullseye` (glibc
floor 2.31, maximum imported 2.30), and the renderer came from `npm run build:e2e`.

| Round | HEAD | `biorouter` / `biorouterd` (sha256) | Linux broker (sha256) | Renderer entry |
|---|---|---|---|---|
| 1 | `5959ebdc` | `62660f8d…8ede` / `569a6d85…6aec` | `f8cc4fe7…8afa` | `index.html` `738a3110…2418` |
| 2 | `5b079379` | `6414bf7e…26b1` / `0cfb288a…ecc1` | `59ceaf92…1cfdc` | `index-Cj4mKt24.js` |
| 3 | `ac02e57a` | `7dd362b4…9c39` / `5dd7f284…15af` | `f3c1e19b…d158f` | `index-DmGxtEGT.js` |
| 4 | `b1c87edc` | `13636d25…0d50` / `e2a4d7c9…ef99e` | `f3c1e19b…d158f` (byte-identical to round 3) | `index-RXzcFP0L.js` |
| Final | `461f7899` | see below | see below | `index-C2JhvNFw.js` |

The final build, `461f7899f2b5567ce64da9555a90448e2faf6d37` ("feat(crew): build the broker with
join-by-name by default"), was clean from 14:00:58Z to 14:22:46Z on 2026-09-25:

| Artifact | sha256 | Notes |
|---|---|---|
| `native-461f7899/biorouter` | `ef845bcafb76788e22580fea86a93467ef0b1d61e396e9ba972a2c684c5e90c6` | arm64 Mach-O, 1.91.2, mode 0555; `--institution` in both privacy helps |
| `native-461f7899/biorouterd` | `f79ed54654f918e44b714af3255c6b127e827e46c3a2feb2f8de6bb09bc9ab51` | `biorouter-server 1.91.2` |
| `linux-broker-461f7899/biorouter-crew` | `2d08981b29d15ea3cfbb52af7c0d4e0e006c22bd094c234ebff5b0d4c3657ec1` | 3,240,856 B; `join_by_name_v1` and `direct_add_v1` present; runs on ubuntu:24.04 and debian-11 |
| Same broker built with no feature flag | `2d08981b…57ec1` | byte-identical, so `join-by-name` is in the default build |
| Renderer `index.html` | `08f573c9601ab74c97b025d1eeaa880fa126e332caa42a7c860e694f6e848e3d` | entry `index-C2JhvNFw.js` `cb53df53…7367`, one React copy |

The first final `build:e2e` (entry `index-B9Mrli7R.js`) was rejected: it bundled five React
copies and every renderer stuck at "Loading BioRouter…". The cause was a git-ignored
self-referencing symlink `ui/desktop/node_modules/node_modules` combined with Electron Forge's
`resolve.preserveSymlinks: true`, not the source. The symlink was removed and the build re-run.

## Fixture provenance

Three EC2 runs served the campaign, all in `us-west-2`, all tagged
`Project=biorouter-crew-qa`, all with only ordinary Unix accounts that have no sudo.

| Run | Instance | Used for | Outcome |
|---|---|---|---|
| `crew-ui-qa-20260924T013533Z-b9155424` | `i-0d7277eaa90124c45` | Resume-time checks: the baseline novice run, fourth-profile CLI refusal, the first live fairness pass, the broker upgrade and downgrade | The deadline supervisor ran cleanup at 2026-09-24T13:36:06Z and recorded `cleanup_verified` (instance terminated; volume, security group and key pairs absent; local keys removed) |
| `crew-ui-live-20260924T141727Z-10eda2e0` | `i-001b9863cb7c170bc` | Nothing: no host key appeared in the console within 360 s | Cleaned up by its own failure path; an independent check at 14:24:50Z found the instance terminated and the volume, security group and key pair not found |
| `crew-ui-live-20260924T142514Z-50d6da52` | `i-00efab0e3576ff58d` (`52.33.141.141`) | Rounds 1–4 and the final lanes | Active at this writing; see [Fixture teardown](#fixture-teardown) |

Controls on the live run:

- **Instance:** `t3.small`, Ubuntu 24.04 (`ami-07a134137a631b892`), 20 GiB encrypted gp3, IMDSv2 required.
  `create-key-pair`, `create-security-group` and `run-instances` were each dry-run before anything was created.
- **Host key:** taken from EC2 `GetConsoleOutput` only, never from `ssh-keyscan`:
  `SHA256:gcAL2xuU4waySml/eD37dGCgdb4p5S0i8F8kfpcsTsA` (ED25519), pinned with `StrictHostKeyChecking yes`.
- **Security group `sg-034b925eff57a0c0e`:** TCP 22 from `169.230.180.159/32`, and from 2026-09-25T01:12Z
  also `128.218.42.202/32` (rule `sgr-06dfcd2e553b28434`). Nothing wider was ever opened.
- **VPN egress rule:** in round 3 the Mac moved to a full-tunnel VPN, so new SSH left from `128.218.42.202`
  and timed out until that /32 was authorized. The old /32 was kept in case the VPN dropped back.
  From then on every daemon, app and SSH process was started outside the Bash tool's sandbox, which
  also blocks new connections.
- **Accounts:** eleven ordinary users, UIDs 10001–10011, each with its own ed25519 key: six at provision
  (alice, bob, carol, foreign, dave, erin), frank in round 2, henry and gina in round 3, iris and jack in
  round 4. Each new account was registered in the fixture state before it was created, so cleanup deletes its key.
- **Profiles:** one local profile per person with its own shared daemon and development Electron app.
  Each carries the working Versa key in its own `secrets.yaml` (the same key in every profile, copied
  without printing). The fourth profile's daemon ran behind a CONNECT recorder that forwards nothing,
  whose log read 0 bytes at every recorded check in every round.
- **Deadline:** created with `ExpiresAt` 2026-09-25T02:25:14Z. Extended at 2026-09-24T21:20Z to
  2026-09-25T14:25:14Z (supervisor PID 710) and at 2026-09-25T10:35Z to 2026-09-26T02:25:14Z
  (supervisor PID 67165). The resource tags still read the original time; nothing local reads them.

### Fixture teardown

Pending. The close-out's Finish phase runs
`python3 /private/tmp/crew-ui-redesign/live/fixture/tools/cleanup.py /private/tmp/crew-ui-redesign/live/fixture/aws/fixture-state.json`
and records its receipts here: the instance, volume `vol-0500a9e73aee9221f`, security group
`sg-034b925eff57a0c0e`, key pair `crew-ui-live-20260924T142514Z-50d6da52-bootstrap` and all eleven
local user keys. If Finish does not run, supervisor PID 67165 runs the same cleanup at
2026-09-26T02:25:14Z.

## Live QA rounds

Each round redeployed the stage in place: brokers upgraded as each user over their own key and
restarted by their owners with identical arguments, daemons and apps restarted from the new
artifacts. Every broker journal was byte-identical across every restart (chen-lab 92 → 170 → 223 →
233 lines at rounds 2, 3, 4 and final), and snapshots before and after matched in every part.
Fresh critics then drove the real apps through Chrome DevTools, with drag and paste sent as CDP
events and the native share confirmation answered by the development gate
`BIOROUTER_DEV_AUTO_CONFIRM_SHARE=1` from round 3 on.

| | Round 1 | Round 2 | Round 3 | Round 4 |
|---|---|---|---|---|
| Build | `5959ebdc` | `5b079379` | `ac02e57a` | `b1c87edc` |
| Critics | Alice (host); Bob, Carol, Dave, Erin (join); security; a11y | Alice, Frank (new), Erin, Bob, Carol, Dave; security; a11y | Henry and Gina (new), Dave, Bob, Erin, Carol; security | Iris and Jack (new), Gina, Bob, Erin, Carol; security |
| Novice tasks | 38 | 49 | 60 | 56 |
| Unaided | 15 | 28 | 51 | 51 |
| With friction or partial | 17, plus 1 answered "no" | 14 | 8 | 3 |
| Failed | 5 | 7 | 1 | 2 |
| **Unaided rate** | **39%** | **57%** | **85%** | **91%** |
| Findings P0 / P1 / P2 | 5 / 23 / 41 | 0 / 16 / 62 | 0 / 5 / 58 | 0 / 3 / 54 |
| Earlier findings verified fixed | — | 49 of round 1 | 54 of round 2 | 36 of round 3 |
| Design score (Carol) | about 4½ | 7 | 8.5 | 9 |
| Local gate before the round | workspace `--lib --bins` exit 0 | ALL_GREEN | ALL_GREEN | ALL_GREEN |

- **Round 1.** The five P0s: an accepted team invitation collapsed every member's open channel; a host
  could not get admitted people into channels; every Copy failed (the renderer permission allowlist
  denied clipboard writes); the Join dialog's Institution field kept only the first letter; focus was
  invisible under Increase Contrast and forced colours. The failures traced to three causes: channel
  membership (P0-2), Copy (P0-3) and the native-picker-only share (T-26).
- **Round 2.** All five P0s were gone. The seven failures had five causes: the native picker after a
  drop (Q2-16, three tasks), Keys and security (Q2-02), radios in forced colours (Q2-11), re-grant
  taking three hops (Q2-10) and an agent silently substituting a file (Q2-15). The most-reported problem
  was coming back after five idle minutes (Q2-01). The fixes led to D-DROP, D-KEEPALIVE, D-HOST,
  D-ALIAS and D-AVATAR (plan §16).
- **Round 3.** The one failure: an agent's result did not name its source and read the older of two
  same-named files (Q3-02). The P1s: the renderer's direct file door (Q3-01), Files calling a sent file
  unsent (Q3-03), an open chat not noticing an outage until refocused (Q3-04) and file buttons in the
  Tab order (Q3-05).
- **Round 4.** Both failures were Bob's "does Crew reconnect by itself?" checks, each recovered by one
  click: a drop found first by Crew's own view was never re-dialled (Q4-01), and "Can't connect" stayed
  stale after the daemon reconnected (Q4-02). The third P1 was file names truncated first (Q4-03). All
  three were fixed before the final build and re-checked there (below).

### Brand-new host and joiner pairs

| Round | Host | Joiners | Journey result |
|---|---|---|---|
| 1 | Alice hosted `chen-lab` from her own account | Bob, Carol, Dave, Erin, each by invitation and device code | All joined; every join step succeeded but none fully unaided (the Institution field, Copy and guessed institution). Alice could not get the four into channels (P0-2) |
| 3 | Henry, a PI who had never used a terminal, hosted `ito-lab` with **Start it for me** (done in about 4 s) | Gina, a first-week novice, joined in 4 clicks and a paste | Henry 14 of 15 steps unaided; Gina 15 of 18 (the one failure was Q3-02). As tasks, 17 of 21 unaided (81%) |
| 4 | Iris, a lab manager who had never used a terminal, hosted `wong-lab` the same way | Jack, a first-day technician, joined in 4 clicks and a paste (the join took under 4 s) | Iris 13 of 13 steps unaided; Jack 15 of 16 acted steps unaided, one partly (the agent chat's tool rows). As tasks, 21 of 22 unaided (95%) |

Round 2 also added Frank as a brand-new member of the existing `chen-lab`: 12 of his 13 steps
succeeded, 10 of them unaided. Across the rounds bob, carol, dave, erin, frank, gina and jack
joined real workspaces by invitation and device code, and the security lanes made their own
wrong-code, replay, system-account and uninvited claims, all refused. With the adversarial reviews,
that is the evidence behind enabling `join-by-name` by default.

## Security results per round

Every refusal was checked against the broker journal, not only the client, and every institution
check against the fourth profile's CONNECT recorder.

| Property | Round 1 | Round 2 | Round 3 | Round 4 |
|---|---|---|---|---|
| UCSF model refused on the `foreign-synthetic` workspace, nothing dispatched | Held; recorder 0 B, journal identical | Held; Start disabled with the reason; CLI and raw route refused | Held (GUI, CLI, raw route) | Held; 400 `institution_refusal` |
| A member acting as host or owner | Held; 6 CLI attempts `forbidden` | Held; 16 direct-add attempts refused, 0 journal records | Held; 16 raw and 3 CLI attempts refused | Held; 5 attempts `forbidden`, a replayed idempotency key refused the same way |
| Wrong device code, replayed invitation | Held; but the host was told "Approved." and the joiner nothing (F1, F2) | Held; both people now told correctly, and `auth.join` written only after the correct code | Not re-tested | Not re-tested |
| System accounts | `root` and `nobody` accepted as invitees (P2) | Invite by name refused 16 names; the legacy `{uid, public_key}` path accepted UID 0 (P1) | Fixed: 10 system UIDs refused, 0 records for 16 attempts | — |
| Removed member's open observer | — | Ended within 0.15–1.4 s; 0 of 3 canaries delivered | 0 of 6 canaries across a re-dial; but a revoked device's bridge stayed up reading `connected` (Q3-12) | Q3-12 fixed: `unknown device` → `disconnected`, bridge retired, no re-dial over 24 s |
| Renderer file door | — | — | **P1 (Q3-01):** page script registered `secrets.yaml` and an SSH private key through `POST /crew/files` with no dialog | Closed for the checklist (credentials, links, case variants, hard links, renamed keys → 400 `crew_file_is_credential`). New: page script could download into `~/.bashrc` (reported P1, triaged P2 as defense in depth) and five credential stores passed the floor (P2) |
| D-DROP native confirmation | — | — | Held; the real sheet names the true path, Cancel shares nothing | Held with the auto-confirm gate off |
| D-HOST one-click hosting | — | — | Held: 403 without proof, 400 for extra fields and shell metacharacters, 409 for used or unknown setups | Held |
| Machine IDs on Crew screens | Mostly held; two screens showed IDs | Held across 8 apps | Held across 10 renderers and 33 surfaces | Held across 11 renderers |

The round-4 download finding and the missing stores were fixed after round 4, before the final build:
`b7585ba1a` (`.netrc`, `.pgpass`, `.git-credentials`, `.docker/config.json`, `.kube/config`,
`gh` hosts on the BR-23 floor), `6aca6c18a` and `7339e6d9d` (settings, login and autostart download
destinations refused, executables never replaced, cloud drives inside `~/Library` kept open). Those
three are covered by unit tests in the closeout gate, not by a live security re-run on `461f7899`.
The structural fix, a registration proof only the main process holds, is deferred (plan §16).

## Final acceptance lanes on `461f7899`

Seven lanes ran on 2026-09-25 between 14:26Z and 15:11Z, each checking the build in place (the
worktree clean at `461f7899`, daemons on `native-461f7899/biorouterd` by `lsof`, renderers loading
`index-C2JhvNFw.js`, brokers `2d08981b…` by `/proc/<pid>/exe`) and each corroborating model output
independently: read-only SSH as the owning account, read-only `sessions.db`, broker journals and
hashes.

| Lane | Gate | Verdict |
|---|---|---|
| Three users, three files, three agents | G04, G13 | **7 of 7 assertions PASS**; the native share confirmation NOT RUN (dev gate) |
| Revoke end to end | P09, G12, resume priority 3 | **Every core assertion PASS**; bridge-drop cases N/A (the daemon re-dialled first), server-unreachable case PASS; 5 findings (P2/P3) |
| Denial matrix | G12 | **4 PASS, (b) PASS with partial coverage, (d) FAIL on wording** |
| Institution gate on the fourth profile | G05, G12 | **PASS**; one control NOT RUN |
| Round-4 P1 re-check by a fresh novice | UI acceptance | **Q4-01, Q4-02, Q4-03, Q4-35 PASS**; 4 of 4 tasks unaided; 1 new finding |
| Mixed GUI and CLI | G11 | **11 of 11 assertions PASS** plus the CLI name-resolution checks |
| Fairness and self-test | resume priorities 2 and 4 | **Both PASS** |

### Three users, three files, three agents (G04, G13)

Alice, Carol and Dave each dropped a synthetic CSV into chen-lab `#general`, posted it, then asked
their own agent through **Ask my agent** for a stated statistic.

| | Alice | Carol | Dave |
|---|---|---|---|
| File (sha256) | `alice-qpcr-ct-g04.csv` (`74b70737…9dea`, 105 B) | `carol-cell-counts-g04.csv` (`39f9ee7f…6f34`, 122 B) | `dave-od600-g04.csv` (`e2977655…4cfe`, 103 B) |
| Upload transfer ID | `2f02b41432bb9a77c7c28977c9788041` | `0e2bfd0c7247a6984eb5ac5daf7a4e7d` | `fc233159f1abb9e1e70239ca30ee7867` |
| Blob ID | `4fbee6b4-f66a-452e-9ef6-75b1cbf49e01` | `9741d4de-0253-4679-9096-907b7eda1238` | `0da84d17-8cb0-4999-9bd4-5867526bb128` |
| File message | `221ce161-aa57-4969-ac58-7af340e2611c` | `805bc673-2aa7-4a95-923e-366e19d21d2b` | `5fd28c1e-01d1-445e-93a2-76ae5a3461ab` |
| Run ID | `194e84d1-5106-42e7-b5ae-90d424733161` | `bad43534-1197-46b1-b8bf-a5480e79302e` | `20c4c01b-6b31-4396-96ee-149152668b6d` |
| Result message | `8d864c31-796e-48c9-880e-01b1d6e78cd3` | `be8d0af2-5c9b-4f10-9e58-cd418b2fa820` | `fe4092a4-599f-4c9b-bb1c-c82bb36cb5be` |
| Expected → posted | mean `ct` 22.5, n = 6 → 22.5, 6 | sum `cells` 8730, n = 7 → 8,730, 7 | median `od600` 0.377, n = 7 → 0.377, 7 |

1. **Shared through the GUI, transfer IDs recorded: PASS 3/3.** The daemon deletes an upload's receipt
   once the message is sent, so the transfer IDs come from the broker journal, where they prefix each
   upload's idempotency keys.
2. **Results correct: PASS 3/3,** against a local Python computation, row counts included.
3. **Each post names its file: PASS 3/3.** Each agent session holds exactly one tool call, a
   `blob.read` of its own blob that succeeded first time, and the bytes it returned hash to the local file.
4. **All three files visible to all three members: PASS 3×3,** in CLI history as each person and in
   each person's Files tab.
5. **Host blob hash equals the local file's: PASS 3/3** (read-only SSH as `crew_alice`).
6. **A peer downloads another member's file through the CLI with a matching hash: PASS 3/3,** as a
   rotation (Carol ← Alice, Dave ← Carol, Alice ← Dave).
7. **Each run's grant ended after completion: PASS 3/3.** `grants list` shows `expired: true` an hour
   before `expires_at`, the GUI shows **Ended**, and the journal holds the completed `run.project`
   followed by the owner's `run.revoke`.

Each run took 6–8 s from Start to its posted result. Observations (P2): the CLI calls a lapsed
round-1 grant "Active" because it reads only the local `expired` flag; the CLI says "Expired" where
the GUI says "Ended" for a revoked grant; the new task chat was missing from Recents.

### Revoke end to end (P09, G12)

Gina's ordinary chat `20260925_6` on private Versa was granted `ito-lab` `#data` from the GUI five
times and revoked five times. Broker `ito-lab` PID 30514 ran `2d08981b…`.

| # | Run | Revoked through | Answer | Broker `run.revoke` |
|---|---|---|---|---|
| 1 | `90a15e4b-ecaf-48c2-b46a-c945979cefe8` | Chat access pane | 200, `remote_revocation_confirmed: true` | journal seq 72 |
| 2 | `cc53390a-3b02-44f2-b788-9d0d48e1eb4d` | `#data` → Agent access tab | 200 | seq 77 |
| 3 | `abc56974-6164-4a14-9538-b4c01530b66e` | Chat access bar after the bridge was killed | 200 | seq 79 |
| 4 | `48619568-acd8-44d8-a154-342d5634cec8` | Chat access bar 4 s into a bridge drop | 200 | seq 81 |
| 5 | `216fd939-6013-4670-8ce6-5c926a6024db` | Chat access bar with the server unreachable | **503 `crew_revocation_unconfirmed`** | none until Retry, then seq 83 |

- **Grant and use: PASS.** Before any grant the chat's Crew calls were refused. After **Allow**, the
  model posted "REE-FINAL-1" (message `04b9b046-7ca9-4cbc-ae53-e59c0de68ef9`) and, after a re-grant,
  "REE-FINAL-2" (`4efe1a7d-8fca-4cef-ae99-1e78ead4deec`); both appeared live in Henry's GUI and in CLI history.
- **After revokes 1 and 2: PASS.** An inline confirm, then "Access revoked…"; the CLI grant list and the
  saved record show `expired: true`; the journal records `run.revoke`; the chat reads "Crew access to
  #data was removed, so this chat can't continue…" with Start a new chat and Grant access again, and Send
  is disabled across a reload. A turn started with **Edit in place** was refused by the daemon before
  it started. After each revoke there were 0 stored rows, 0 Crew calls, no model request and no journal change.
- **Bridge dropped (tests A and A′): not applicable.** Killing the exact bridge PID never produced an
  unconfirmed revoke: the keepalive re-dialled within 20 s (1.6 s in A′) and each revoke was confirmed.
- **Server unreachable (test B): PASS.** With Gina's SSH config pointed at a closed port, the revoke
  answered 503 with `stopped_on_this_device: true`; the grant was expired and saved locally at once; the
  next turn was refused with nothing sent; the UI said "Stopped on this device. Reconnect to confirm with
  the workspace." and never claimed Revoked; the broker was not marked revoked. After the network
  returned, only **Retry** confirmed it (seq 83).
- **Findings.** F1 (P2): Edit in place bypasses the revoked chat's composer hold and truncates the stored
  transcript before the daemon refuses the turn. F2 (P2): while the workspace is offline the Crew view
  offers no Revoke. F3 (P2): reconnecting does not retry an unconfirmed revocation, and the copy says it
  will; for 3 min 46 s the broker still held that run live (refused locally, so nothing left the device).
  F4 (P2): the CLI's text `grants list` shows a lapsed grant as Active and task grants as Chat. F5 (P3):
  two revocations of one chat show as identical rows.

### Denial matrix (G12)

Run on wong-lab (Iris and Jack), and on chen-lab for (c) only.

| # | Assertion | Verdict |
|---|---|---|
| a | connect, grant, revoke of a live grant, host start, members add and file registration with a missing, random or another person's real `X-User-Action` → 403 with no effect | **PASS.** 65 of 65 refused (36 raw socket, 17 CLI, 12 renderer page script). The bridge PID, grant and transfer lists and journal hash did not change; the same requests with the right proof took effect |
| b | A worker attempting `enrollment.approve`, `team.add_member`, `run.revoke` or `policy.set` through `crew__request` is refused | **PASS, partial coverage.** No human action happened and the journal gained nothing, but GPT-5.5 declined all four prompts to send an out-of-schema method, so the daemon's worker allowlist was not exercised live. A direct `/agent/call_tool` was refused by the tier gate first; a human-proven `run.*` through `/request` got 403 `crew_typed_run_required` |
| c | Bob cancelling Alice's run or revoking her grant | **PASS.** 17 attempts (CLI 4, raw 7, GUI 6) refused; Alice's grant still worked |
| d | A stale grant after a policy change is refused with the plain "settings changed" sentence | **FAIL on wording; the refusal PASS.** Both interfaces refused with no model call, no rows and no journal record, but showed `Crew broker refused request: {"code":"grant_expired",…}` (D-1). The plain sentence appears only when the connection's own epoch moves, which was verified in both interfaces |
| e | Credential paths refused on registration | **PASS.** 21 refusals of 400 `crew_file_is_credential` (keys, the real `secrets.yaml`, three symlinks, a `..` route); `ordinary.csv` and `id_ed25519.pub` accepted |
| f | A removed member's transfer of a restricted file | **PASS.** The CLI refused; the raw and GUI routes started and then stopped at size 0 with no file written, the broker answering `forbidden: channel unavailable` |

Other findings: F-1 (P3) the refused download says "Reselect the original local file" instead of "you
are no longer in that channel"; P-1 (P3) `privacy set-personal private` on an already-private connection
re-saves it, bumps its epoch (expiring every grant) and drops the bridge.

### Institution gate on the fourth profile

The fourth profile (`crew_foreign`, workspace `foreign-lab`, Private · `foreign-synthetic`) with the
UCSF-affiliated Versa model, from 14:28Z to 14:37Z:

- **GUI:** Ask my agent kept Start disabled with "gpt-5.5-2026-04-24 is approved for UCSF. foreign-lab
  uses foreign-synthetic…", and a real click did nothing. Bypassing the button from page script sent the
  GUI's own `POST …/runs`, which the daemon answered **400** `institution_refusal`: the daemon is the gate.
- **CLI:** `tasks start … --provider versa_azure --model gpt-5.5-2026-04-24 --allow-posting` and
  `grants grant 20260925_1 general` both exited 1 with `crew_request_refused`.
- **Zero egress:** the recorder log was 0 B at five checks across the window; a second copy of the
  recorder on another port did log `CONNECT example.com:443`, so the instrument works. The foreign
  journal stayed byte-identical (`e9d294d7…9a14`, 67 lines) and no run, grant or session was created.
- **Positive control:** Alice's same-model task on chen-lab (`ucsf`) was admitted as run
  `e23de04d-c4f0-455f-b051-44fd7b253902` (journal seq 285, `provider_affiliation` `ucsf`) and completed.
- **Not run:** a proof on this build that the daemon honours the proxy, because it would have made the
  shared 0-byte log non-zero; round 1's lane proved it on an earlier build.
- **Findings:** F1 (P2) `grants grant` prints the daemon's raw sentence behind `Daemon returned 400:`
  while `tasks start` prints the desktop's; F2 (P3) the pane repeats the mismatch sentence after a bypass.
  The round-1 escaped apostrophe (`model\'s`) no longer reproduces.

### Round-4 P1 re-check by a fresh novice

- **Q4-01 (Erin): PASS, 0 clicks.** With Crew on screen, the bridge was killed and the server made
  unreachable. The daemon logged the drop at once, and a new bridge started exactly at the documented
  schedule (drop + 20 + 60 + 180 s); the UI showed Connected 10.5 s later, 2 min 26 s after the network
  returned. Round 4 had recorded no re-dial for 7 min 37 s.
- **Q4-02 (Erin, second outage): PASS with a residual.** After one failed Connect, the daemon re-dialled
  by itself and the UI showed Connected 45.6 s after the network returned, with no click (round 4:
  about 5 minutes stale). The residual is NEW-1 (P2): a red bar keeps the old raw error ("Crew SSH failure
  [ssh_eof; child_before_cleanup=exit_255]… outcome may be unknown…") above a live, connected channel for
  at least 5 minutes. Likely cause, not tested: the automatic follow never clears the failure `connect()`
  recorded in `state/useCrewConnections.ts`.
- **Q4-03 (Jack at 1048 px): PASS.** File names are whole in the timeline and the Files tab with the pane
  open or closed, and on Carol's original failing case the metadata now gives way first.
- **Q4-35 (Iris): PASS.** Team ⋯ → "Members of Wong Lab…" opens a dialog titled "Members of Wong Lab"
  that lists members first.

### Mixed GUI and CLI (G11)

Bob's Electron app was stopped at 14:59:30Z; his daemon (PID 42618) and his server-side bridge (PID
31129) stayed up throughout. Bob's CLI created `#g11-mixed`
(`f8cba6b4-4fe3-4b17-8f34-79f749beed15`) and added Carol and Alice by name.

| | Message ID | Posted by |
|---|---|---|
| M1 | `1bc7a8f1-1b77-44e8-953d-5ecf464c5e09` | Bob, CLI |
| M2 (file) | `9eca97d2-ebaa-4fd4-9b0f-71e96bd24c7f` | Bob, CLI |
| M3 | `5e355802-88f7-42fe-8309-044e10e71ad7` | Alice, GUI |
| M4 | `11409766-18e2-420c-878d-f9bdee8a7db7` | Carol, GUI |
| M5 | `fe1ef8cb-5c64-44ae-bc01-16a61f62ac34` | Bob, reopened GUI |

The file is blob `c4991a58-796a-4eea-a6c5-57c501eb3bb7`, 300,000 B, sha256
`4bd9b24f5b2d067cafb2867add416aa6c9e56e3434636375220483b7d2b32ab7`, from upload transfer
`027848a822e25a7e2e8acf6c945b4387`.

All 11 assertions passed: Bob's app closed with his daemon up; Bob posts, uploads and posts a file,
watches (text and stream JSON, all five messages) and lists members from the CLI; every message's ID,
body, actor and attachments are identical across 9 sources for M1–M2 and 8 for M3–M5 (his CLI send
and watch, CLI history as all three people, all three GUIs, the journal), and Alice's GUI **Copy message
ID** gave M1's ID; the file ID matches everywhere and Carol's GUI **Copy file ID** gave it; authorship
shows display names (`"Carol Nguyen" (@crew_carol)`) in the GUI and the CLI; Alice's GUI post reached
Bob's CLI watch about 2 s after it was journaled; Carol's GUI Save through the real native save sheet
produced a file whose hash equals Bob's source and the host blob; and Bob's reopened app attached to the
same daemon and showed the CLI-era state. The CLI name checks passed: teams by name in any case,
members by `@username`, channels as `'#name'` or `team/channel`, a case-only username refused with
"Did you mean", a display name never selecting anyone, nothing sent when a name fails to resolve, and
IDs only with `--show-ids`. Findings (low): `members add` prints the bare `@handle` even when the person
has a display name, and an unknown connection's refusal carries a `Daemon returned 400:` prefix.

### Fairness and self-test

- **Live observer source-ACL test: PASS on the first run.**
  `cargo test -j 8 -p biorouter-server --lib routes::crew_observation::live_acceptance::real_source_acl_revocation_clears_enqueued_and_waiting_observer_frames -- --ignored --exact --nocapture`
  ran 15:04:24–15:04:31Z: `1 passed; 0 failed; 877 filtered out`, test binary sha256 `49386ed1…1021`
  built from the clean HEAD. Alice's Versa task run `b41616f0-8601-4796-82eb-3b6a5137b1f9` produced the
  derived message `9eeb218a-145a-4414-bfb5-10bc14739174` carrying canary `FAIRCANARY-e9d452717d08`;
  Frank's real daemon observed it before revocation; inside the test the frame was admitted, the queue
  filled, Alice's real revocation ran through the final CLI, and a terminal `error` frame with
  `clear: true` and no canary ended the stream. Independently, the chen-lab journal gained exactly one
  record (`membership.revoke`, epoch 28 → 29), Frank's history after the derived cursor was refused
  `stale_cursor`, and the source-derived rows were withheld from Frank only. The frame file was
  reconstructed from the observed message, as in the 2026-09-24 run, not captured byte for byte.
- **Six-command Versa self-test: PASS.** Session `20260923_11` in the preserved native-home19 profile
  resumed through the final CLI (`run --resume --session-id 20260923_11 --text … --output-format text`)
  from 14:58:51Z to 14:59:21Z, exit 0. Exactly six `developer__shell` calls ran, in order, each byte for
  byte the prescribed command; every output is byte-identical to the artifact captures whose hashes are
  in the native manifest; both privacy helps contain `--institution`; the old
  "provider restore parameters are reserved" refusal did not appear. Display observation: the text
  progress renderer printed each tool output twice; the recorded results are correct.

## Findings from the final lanes

None of these is a leak. They were found after the last fix round, so none is fixed on `461f7899`.

| ID | Lane | Severity | Finding |
|---|---|---|---|
| D-1 | Denial matrix | P2 (G12 (d) FAIL) | A workspace-policy change surfaces as the broker's raw JSON envelope at turn start, in the CLI and the GUI; `grants list` still reports those grants `expired: false` |
| NEW-1 | Novice re-check | P2 | A stale raw connect error bar stays above a reconnected channel after a failed Connect |
| F1 | Revoke | P2 | Edit in place bypasses the revoked chat's composer hold and truncates the stored transcript |
| F2 | Revoke | P2 | No Revoke control in the Crew view while the workspace is offline |
| F3 | Revoke | P2 | An unconfirmed revocation is not retried on reconnect, and the copy says it will be |
| F4 / O-1 | Revoke, three-user | P2 | The CLI's text `grants list` prints lapsed grants as Active and task grants as Chat |
| F1 | Institution | P2 | `grants grant`'s institution refusal carries a `Daemon returned 400:` prefix |
| O-2 | Three-user | P2 | "Expired" in the CLI against "Ended" in the GUI for a revoked grant |
| O-3 | Three-user | P2 | A new task chat is missing from Recents |
| — | Mixed | Low | `members add` names the person by `@handle` only; `--show-ids` adds nothing to it; an unknown connection's refusal carries an HTTP prefix |
| F-1 | Denial matrix | P3 | A removed member's refused download says "Reselect the file" |
| P-1 | Denial matrix | P3 | A no-op privacy save bumps the connection epoch and drops the bridge |
| F5 | Revoke | P3 | Two revocations of one chat show as identical rows |
| F2 | Institution | P3 | The mismatch sentence shows twice after a bypassed Start |
| — | Self-test | Display | `run --output-format text` prints each tool output twice |

## What remains unverified

- **The native share confirmation in the GUI lanes.** Every stage app ran with the development
  auto-confirm gate, so the GUI critics never saw the real sheet. Security saw it in rounds 3 and 4, with
  the gate off, and it was accurate.
- **The daemon's worker allowlist exercised live** (G12 (b)): the model never sent an out-of-schema
  method. A deterministic tool-call harness under a private capability is needed.
- **The download-destination rules and the added credential stores on a live build** (`b7585ba1a`,
  `6aca6c18a`, `7339e6d9d`): unit-tested only.
- **The daemon honouring the fourth profile's proxy on `461f7899`** (institution lane 5c).
- **SCOPE-BIND's reissued-session-ID case live:** covered by adversarial unit tests only.
- **Platforms:** macOS desktops only. Windows and Linux desktops, and shared Windows CLI IPC, were not run.
- **SSH authentication breadth:** the fixture used keys only; no MFA, jump host or changed host key
  was exercised in this campaign.
- **Load and faults:** no 10/30/50-user load, storage fault, power loss or concurrent-writer run.
- **Hosted CI on `461f7899`:** not run at this writing. The local tracking ref puts the last pushed
  head at `b1c87edc`, 33 commits earlier, and no hosted result for it is recorded here either.
- **The closeout gate's verdict:** its logs show every command passing except `just generate-openapi`,
  which hit "No space left on device" and was followed by a direct schema regeneration and a passing
  `check-openapi-schema.sh`; no file records the gate runner's own verdict
  (see the [validation report](../validation-report.md#crew-ui-redesign-campaign-2026-09-24-to-2026-09-25)).
- **Human review:** the security-sensitive commits listed in the [handoff](../handoff-2026-09-24.md#close-out-handoff-2026-09-25)
  have had independent agent review, not the human review CLAUDE.md requires before merge.

## Where the raw evidence is

All paths are local to the machine that ran the campaign and are not published.

| Evidence | Path |
|---|---|
| Stage record (builds, fixture, profiles, operating procedures) | `/private/tmp/crew-ui-redesign/live/STAGE.md`, `live/FINAL-ARTIFACTS.json` |
| Round triage and critic reports | `/private/tmp/crew-ui-redesign/live/reports/` (`TRIAGE-r1.md`…`TRIAGE-r4.md`, `security-r1.md`…`security-r4.md`, per-person reports) |
| Structured round results (task tables, groups, gates) | `/private/tmp/crew-ui-redesign/live/round1-result.json`…`round4-result.json` |
| Final lanes | `/private/tmp/crew-ui-redesign/live/evidence-final/<lane>/RESULT.md` with `raw/` |
| Redeploy receipts | `/private/tmp/crew-ui-redesign/live/receipts/` (`r2-*`…`r4-*`, `final-*`) |
| Resume-time evidence (broker upgrade, fourth profile, fairness, self-test) | `/private/tmp/crew-ui-redesign/evidence/`, `/private/tmp/crew-ui-redesign/fixture/evidence/` |
| Screenshots (never published) | `/private/tmp/crew-ui-redesign/live/shots/` |

## Related documentation

- [Implementation status](../implementation-status.md) — the P01–P10 and G01–G15 rows these results update
- [Validation report](../validation-report.md) — the gate commands, counts and hashes behind each round
- [Resume handoff](../handoff-2026-09-24.md) — the close-out handoff, deferred items and the commits that need human review
- [Implementation plan §16](../implementation-plan.md#16-resumed-scope-gui-redesign-and-human-readable-identity-2026-09-23) — the acceptance criteria and the round decisions (D-DROP, D-KEEPALIVE, D-HOST, D-ALIAS, D-AVATAR)
- [UI redesign specification](../ui-redesign-spec.md) — the design the novice critics tested
- [Evidence records](README.md) — the other bounded live runs in this folder
