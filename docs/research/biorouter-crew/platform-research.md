# BioRouter Crew: external platform and protocol research

> September 22 design update: [the implementation plan](implementation-plan.md) now follows the user's no-administrator, home-based deployment requirement, 2–50-user scope, cluster Public/Private toggle and built-in saved-SSH/MCP integration. It also contains the required three-user AWS/dev-app computer-use testing plan. Earlier deployment recommendations in this research snapshot are superseded; recorded source findings and probe results remain historical evidence, not rootless product acceptance.

Research snapshot: 2026-09-21 America/Los_Angeles (2026-09-22 UTC). Primary sources only support technical conclusions. GitHub API observations below are a snapshot, not a claim about future maintenance. No third-party server was installed, executed, or penetration-tested for this comparison. Security findings are source review findings at the identified commits.

## Recommendation

Build a small native Crew service around BioRouter's existing execution and privacy controls, with an SSH-launched per-user bridge and a dedicated, unprivileged shared broker. Follow the user's preference for broadly portable Linux operation, text files and text streaming: versioned JSONL on the control/event transport, a broker-owned append journal, and ordinary files for metadata and attachments. Do not require a database, Redis, NATS, or a new specialized service stack for the first release. Reuse mature components for SSH, authenticated IPC, filesystem operations and agent session transport; avoid making an existing chat product the authority for Unix execution or private/public data flow.

This recommendation follows from the required combination: an existing SSH account is the identity; agents execute as that account; another channel member cannot prompt, approve, cancel, or reconfigure that user's agent; and every context, attachment, tool result, and model call retains its privacy policy. None of the compared products establishes that full boundary. A richer chat interface does not remove the need to implement it.

The strongest alternatives depend on which requirement can change:

- **Matrix/Synapse** is the strongest reusable room/event/media protocol if Crew can accept a separate account service, database, and substantial policy integration.
- **Zulip** is the strongest ready-made research-team collaboration product when topic organization, search, uploads, and bots matter more than a native SSH-account experience.
- **Ergo + WeeChat** is the strongest lightweight existing terminal chat choice. It needs a separate artifact service and account-to-Unix mapping.
- **room** is the closest architectural reference for local agent coordination. Its identity, compatibility paths, and file-based querying require substantial redesign before a multi-user institutional deployment.
- **Devzat / ssh-chat** demonstrate SSH chat well, but do not meet Crew's full collaboration, attachment, and execution requirements.
- **XtermChat**, as identified here, should not be adopted for sensitive Crew workloads; source findings below are disqualifying for that use.

These are engineering fit judgments, not security certifications or claims that custom code is inherently safer. Native Crew takes responsibility for authorization, durability, recovery, upgrade compatibility, and adversarial testing. Plain files reduce deployment dependencies but do not remove the crash-consistency work normally performed by a database.

## Correcting the initial comparison

“A separate service exists” and “a new network port must be exposed” are different questions. An HTTP, WebSocket, IRC, or gRPC service can run on loopback and be reached through SSH forwarding. Conversely, a Unix socket alone does not prove trusted identity, room authorization, or storage confidentiality. OpenSSH supports local forwarding, including Unix-domain endpoints, and `ProxyJump` supports a comma-separated sequence of jumps. Institutional SSH policy can restrict forwarding or remote commands; actual host feasibility must be tested separately. [OpenSSH ssh manual](https://man.openbsd.org/ssh), [OpenSSH configuration manual](https://man.openbsd.org/ssh_config#ProxyJump)

Likewise, terminal image rendering is not equivalent to storing an arbitrary binary attachment and authorizing its later download. Log output is not necessarily queryable durable history. A unique nickname or a public-key hash is not necessarily the authenticated Unix account on the institution's existing server.

| Candidate | Rooms, membership, history and DMs | Files and images | Agent/programmatic interface | Existing SSH account is authoritative? | Deployment interpretation | Crew fit |
|---|---|---|---|---|---|---|
| **knoxio-labs/room** | Rooms, private/unlisted/DM configuration, tokens, NDJSON history; see compatibility caveats | No complete arbitrary-attachment lifecycle established in inspected docs/protocol | JSON stdin/stdout, one-shot CLI, REST/WebSocket | **No:** claimed username/token; no established kernel UID binding | UDS broker; optional HTTP/WS | Useful design reference, not a ready security core |
| **quackduck/devzat** | Dynamic rooms, DMs; backlog is limited and disabled for private deployment's main room | Do not equate display features with durable authorized attachments; full artifact lifecycle not established | gRPC plugin API and commands | **No:** separate SSH public-key identity/display name | Separate SSH listener; can be made reachable through a tunnel | Good SSH-chat demo |
| **shazow/ssh-chat** | Primarily one chat; optional chat log; lightweight private messaging | No rich attachment lifecycle established | SSH interaction / Go integration | **No:** separate key allowlist and chat identity | Separate SSH listener, configurable bind | Too small for Crew |
| **Ergo + WeeChat** | Registered channels, invite controls, accounts, DMs, configurable persistent history | WeeChat DCC is a separate transfer path; add governed artifact service | IRCv3, client scripting | **No:** SASL/registered account mapping needed | IRC daemon on UDS/loopback plus per-user client | Best mature terminal baseline |
| **XtermChat (dnysaz)** | SQLite rooms/history, password rooms, creator checks; identity weaknesses below | Arbitrary stored attachments not established | Flask REST, terminal/web clients, monitoring bot | **No:** supplied username and PIN/device ID | HTTP service; public bind is not necessary in principle | Reject for sensitive deployment |
| **Matrix / Synapse** | Rich room membership, permissions, history, DMs, spaces, profiles | Standard media repository and file/image event types | Client-server HTTP API, sync, SDKs, appservices | **No:** Matrix identity; custom trusted mapping needed | Homeserver + database/media; tunnelable | Strongest protocol reuse option |
| **Zulip** | Channels, topics, permissions, DMs, history and search | Upload API and sharing model | REST/events API, owned bots | **No:** Zulip account | Full application stack; tunnelable | Best ready-made human collaboration alternative |
| **Mattermost** | Teams/channels/DMs/history; features vary by edition | Built-in sharing | REST, WebSocket, bots, webhooks | **No:** Mattermost/SSO account | App + PostgreSQL; tunnelable | Strong product, edition/license and integration costs |
| **Rocket.Chat** | Channels/private groups/DMs, permission system | Configurable storage and upload API | REST/realtime APIs and apps | **No:** application account | Full application/database stack; tunnelable | Strong product, larger operational surface |
| **Tinode** | Group/one-to-one messaging, granular access, synchronization | Images/files and large-object storage | JSON/WebSocket or protobuf/gRPC | **No:** Tinode/custom auth account | Go service + database/media | Capable messaging engine, fewer Crew semantics |
| **NATS + JetStream** | Durable pub/sub building blocks; no team/chat domain model | Object storage exists; artifact ACL lifecycle remains application work | Mature clients including Rust | **No:** NATS auth identity | Additional broker, loopback/private-network possible | Consider after multi-node scale needs are demonstrated |
| **Native Rust Crew broker** | Must implement required subset | Must implement governed blob storage | Typed Crew API + per-user BioRouter/ACP runner | **Yes, by design, once tested:** SSH execution + peer credentials | Dedicated service UDS, per-user stdio bridge | Best match to non-negotiable boundaries |

The sections below supply sources and qualifications for this matrix. “Not established” means the inspected primary sources did not demonstrate the required end-to-end behavior; it does not assert that no extension or fork could provide it.

## The five suggested projects

### room

The repository is **[knoxio-labs/room](https://github.com/knoxio-labs/room)**, MIT licensed. Its documented architecture is a Rust broker, a Unix socket, TUI/agent clients, and NDJSON persistence. CLI operations include join, send, query, poll and watch; query can span rooms and apply subscription filters. Importantly, the documented CLI query path reads history files directly. It is designed around agent coordination, not just human chat. [README](https://github.com/knoxio-labs/room/blob/66d9527c6c1cc699e1f3d963d81a5f8e97e3ce3c/README.md)

Current code does implement private, unlisted and DM access checks against creator/invite lists. It would be incorrect to describe room as having no ACLs merely because an older design document starts with that problem. However, `issue_token` accepts a username and checks name collision; it does not establish a Unix UID. The authorization function accepts legacy rooms without configuration. [Pinned auth implementation](https://github.com/knoxio-labs/room/blob/66d9527c6c1cc699e1f3d963d81a5f8e97e3ce3c/crates/room-daemon/src/broker/auth.rs)

The authentication documentation explicitly retains deprecated unauthenticated `SEND:<username>` and plain-username interactive handshakes. It also describes the first joiner as host, host re-election on restart, and weaker persistence of revocation in single-room mode. These semantics differ from Crew's stable creator ownership and authenticated institutional identity. [Authentication documentation](https://github.com/knoxio-labs/room/blob/66d9527c6c1cc699e1f3d963d81a5f8e97e3ce3c/docs/authentication.md), [handshake parser](https://github.com/knoxio-labs/room/blob/66d9527c6c1cc699e1f3d963d81a5f8e97e3ce3c/crates/room-daemon/src/broker/handshake.rs)

REST bearer-token paths now exist alongside UDS/WS. Therefore “no exposed service required” is accurate for UDS mode, but “only a Unix socket implementation exists” would be outdated. [REST implementation](https://github.com/knoxio-labs/room/blob/66d9527c6c1cc699e1f3d963d81a5f8e97e3ce3c/crates/room-daemon/src/broker/ws/rest.rs)

**Implication:** borrow ideas such as typed events, cursors, room subscriptions and agent-friendly CLI tools. Do not reuse the identity model, first-join ownership, or direct shared-history access as Crew's trust boundary. A shared filesystem deployment would need explicit permission analysis even if the socket authorization were strengthened.

### Devzat

**[quackduck/devzat](https://github.com/quackduck/devzat)** is MIT licensed and implements a custom SSH chat server. Public keys identify users, while the SSH username can initialize a display name. It supports rooms and DMs; the default server port is 2221. That is a different authentication service from logging into an institution's existing OpenSSH account. [README](https://github.com/quackduck/devzat/blob/a4a1be9e0dd5c7d771462970aa8359ae4b918c12/README.md)

The private-server mode uses an allowlist of key-derived IDs. Its manual states that `#main` backlog is disabled in private mode, which is a significant mismatch with persistent asynchronous teamwork. Bridges and external integrations are optional and would require explicit policy decisions in Crew. [Admin manual](https://github.com/quackduck/devzat/blob/a4a1be9e0dd5c7d771462970aa8359ae4b918c12/Admin%27s%20Manual.md)

The gRPC plugin API can listen to events, register commands and send messages; it is a real integration surface. It also permits middleware interception and configurable sender display names, so it is not itself proof of per-user agent ownership. [Plugin API](https://github.com/quackduck/devzat/blob/a4a1be9e0dd5c7d771462970aa8359ae4b918c12/plugin/README.md)

**Implication:** useful for terminal UX inspiration or a nonsensitive demonstration. Crew would still need durable channel semantics, richer file handling, Unix identity, per-user runners and policy gates.

### ssh-chat

**[shazow/ssh-chat](https://github.com/shazow/ssh-chat)** is an MIT-licensed Go SSH server that gives clients a chat prompt. It offers a configurable bind address, administrator and connection allowlists based on public keys, and an optional log file. Its default bind in the documented CLI is `0.0.0.0:2022`; an intentionally restricted deployment must change that. It is a simple chat service, not a documented teams/channels/artifact platform. [README](https://github.com/shazow/ssh-chat/blob/844e5d5b2675eb011a37ff36d02d0583a371fa29/README.md)

**Implication:** it solves connecting a terminal to chat, but nearly all Crew-specific domain work remains. A recent repository commit should not be confused with a recent packaged release; the latest release API returned v1.10 from 2020.

### Ergo + WeeChat

**[Ergo](https://github.com/ergochat/ergo)** is an MIT-licensed Go IRC server with registered accounts/channels, SASL, client certificates, history and bouncer functionality. Channel registration preserves owner/topic/modes. **[WeeChat](https://github.com/weechat/weechat)** is a GPL-3.0-or-later terminal client with plugins and scripting. [Ergo README](https://github.com/ergochat/ergo/blob/6e25291c9b7450d63db59f360e1b6d90fa83e345/README.md), [WeeChat README](https://github.com/weechat/weechat/blob/5122c58cd97ebd78fa7a3e40f5ee141571eb308d/README.md)

The current Ergo manual's persistent-history section supports SQLite, PostgreSQL and MySQL. An earlier paragraph on the same page still describes MySQL alone; use the detailed current configuration section and validate the selected release. The manual covers invite-only channels, persistent permissions, UDS/loopback listeners and a dedicated unprivileged service account. It does not turn an IRC nickname into an authoritative SSH UID. [Pinned operator manual](https://github.com/ergochat/ergo/blob/6e25291c9b7450d63db59f360e1b6d90fa83e345/docs/MANUAL.md#persistent-history)

WeeChat supports DCC file transfers, but those are an additional transfer mechanism, not a centralized attachment store with Crew's room ACLs, retention, previews and download auditing. [WeeChat user guide](https://weechat.org/files/doc/devel/weechat_user.en.html#xfer)

**Implication:** excellent for “team terminal chat through SSH” with mature protocols. Less attractive for the full native Crew sidebar, attachments and agent policy model because substantial custom infrastructure remains.

### XtermChat: identity and concrete source concerns

The matching project is **[dnysaz/xtc-server](https://github.com/dnysaz/xtc-server)** plus **[dnysaz/xtc-client](https://github.com/dnysaz/xtc-client)**, both MIT licensed. This disambiguation matters: similar terminal-chat names refer to unrelated projects. The client documents a Flask/SQLite server, CLI and local web interface, room passwords and a device-derived identity; web users select a five-digit PIN. This is not institutional SSH authentication. [Client documentation](https://github.com/dnysaz/xtc-client/blob/c0624fc1146b98296a7037173c5b2824369c3690/readme.md)

At server commit `73e4792a9db01ecb469506172e17135bc5936745`, source review found:

1. `connection.get_messages` returns a `pin` field for every message. The messages route returns that result, after checking a room password where configured. A credential used as identity is therefore included in message data. [Message storage/retrieval](https://github.com/dnysaz/xtc-server/blob/73e4792a9db01ecb469506172e17135bc5936745/connection.py)
2. `/send` checks or registers the supplied username/PIN but does not check the destination room password or membership. `/bot/list/all` exposes bot PINs, and its handler has no authentication check. `/login` directly compares stored PIN values. [Server routes](https://github.com/dnysaz/xtc-server/blob/73e4792a9db01ecb469506172e17135bc5936745/server.py#L220)
3. Room creation and owner operations are tied to caller-supplied identity fields. Even where a creator check exists, this is not equivalent to authenticating the effective Unix UID. [Room implementation](https://github.com/dnysaz/xtc-server/blob/73e4792a9db01ecb469506172e17135bc5936745/room.py)

These are bounded static findings, not a complete vulnerability audit, CVE assignment, or claim that a live deployment was exploited. They suffice to rule out this snapshot as a security foundation for Crew. No private data was used and no requests were sent to a deployed XtermChat service.

## Additional options

### Matrix/Synapse

Matrix supplies rooms, membership, per-room permissions, history synchronization, profiles, DMs, spaces, media/file events and optional end-to-end encryption. Its client-server protocol supports custom clients and agent integrations. The durable event model is a particularly useful reference for Crew. Matrix identities are application identities rather than Unix identities. [Matrix client-server specification](https://spec.matrix.org/latest/client-server-api/)

Synapse can listen behind a local reverse proxy; a tunnel-based deployment does not inherently need public federation. Its configuration explicitly allows an empty federation-domain allowlist to deny federation with all servers. Private deployments would also need to address remote media, URL previews, push services, identity services and any integration egress rather than assuming that disabling federation closes every path. [Reverse-proxy configuration](https://element-hq.github.io/synapse/latest/reverse_proxy.html), [Synapse configuration](https://element-hq.github.io/synapse/develop/usage/configuration/config_documentation.html#federation_domain_whitelist)

**Tradeoff:** rich protocol reuse versus a second identity/control plane and operating stack. E2EE adds device and key-management obligations; a model can only use context it is authorized to decrypt. Search, auditing and agent participation must be designed with the chosen encryption mode. Synapse's current repository license is AGPL-3.0; do not assume historical Apache licensing applies to the current Element-hosted code. [Synapse repository/license](https://github.com/element-hq/synapse)

### Zulip

Zulip offers channel/topic organization and configurable permissions; this is attractive for long-running analyses with many parallel agent jobs. Its API includes messages, real-time events, users and channel management. File upload is a documented authenticated API operation. [Channel permissions](https://zulip.com/help/channel-permissions), [API](https://zulip.com/api/), [Upload API](https://zulip.com/api/upload-file)

Zulip has owned bots, and its current help says only a bot's owner accesses that bot's API key. That is useful but different from restricting which human may prompt the bot to execute Unix commands: Crew must still enforce authenticated sender-to-owner checks in its runner. [Manage a bot](https://zulip.com/help/manage-a-bot)

Its production deployment is a full application with supporting services, not a tiny binary each SSH user can independently start and thereby share a workspace. The server is Apache-2.0 licensed. [Production requirements](https://zulip.readthedocs.io/en/latest/production/requirements.html), [Zulip source](https://github.com/zulip/zulip)

**Tradeoff:** strongest path to a ready collaboration experience; requires adopting accounts and organization semantics, plus a BioRouter bot/runner bridge and privacy enforcement across every API path.

### Mattermost

Mattermost closely matches Slack-style teams, channels, DMs and files, and has integration APIs. Its current offerings differ materially: the free Entry mode documents a 10,000-message visibility/search limit and omits compliance capabilities; Team Edition is described separately. Feature access cannot be inferred from “self-hosted.” [Current editions](https://docs.mattermost.com/product-overview/editions-and-offerings), [API reference](https://api.mattermost.com/)

Licensing must be evaluated for the actual artifact and integration. The current root license distinguishes official compiled distributions from source builds: source compilation generally uses AGPLv3 or a commercial license; selected admin/configuration/web assets are Apache-2.0, with stated exceptions. An unqualified “MIT” row would be misleading. [Current licensing text](https://github.com/mattermost/mattermost/blob/f0b63f8ed11ea4b375f940e76eea189c6f8cd657/LICENSE.txt)

**Tradeoff:** a credible institution-operated collaboration product, especially where paid administration/compliance features are already procured. It is not a drop-in SSH-account broker; Crew would still need ownership-aware runners, artifact/context policy and an integration boundary.

### Rocket.Chat

Rocket.Chat supports public/private channels and room membership, plus file storage controls and authenticated upload APIs. Those are closer to the desired rich-chat experience than terminal chat projects. [Channels](https://docs.rocket.chat/docs/channels), [File upload configuration](https://docs.rocket.chat/docs/file-upload), [Upload API](https://developer.rocket.chat/apidocs/upload-media-files-to-a-room)

The repository's license is MIT outside designated enterprise directories and third-party components; enterprise directories use their own license. Treat it as a mixed-license product rather than assuming every feature is MIT. [License](https://github.com/RocketChat/Rocket.Chat/blob/f50636a1fcd21cc5b922479680519369ad7afa6c/LICENSE)

**Tradeoff:** comprehensive product reuse but another identity, administration and operational stack. Review actual edition entitlement, offline operation, notifications and integrations before counting a feature as available in an isolated institutional deployment.

### Tinode

Tinode provides Go messaging infrastructure, group/one-to-one messaging, granular permissions, user discovery, images/files, configurable media storage and a scriptable CLI. Transport is JSON over WebSocket/long polling or protobuf/gRPC. Its server is GPL-3.0 and clients use Apache-2.0. The project labels itself beta quality and explicitly says it is not pursuing a Slack replacement; full-text search and encryption capabilities listed as planned must not be counted as delivered. [Tinode README](https://github.com/tinode/chat/blob/a4d12e3ffdefa9021235deeb81415492e7a727e5/README.md), [API](https://github.com/tinode/chat/blob/a4d12e3ffdefa9021235deeb81415492e7a727e5/docs/API.md)

**Tradeoff:** viable backend candidate with custom authentication support, but Crew still owns Unix identity, team/workspace semantics, privacy labels, search policy and user-owned execution. It offers less advantage than Matrix for standard protocol reuse or Zulip for a ready product.

### NATS/JetStream versus a small native broker

Core NATS is ephemeral pub/sub; JetStream adds durable streams, replay, acknowledgments and consumer state. Redelivery means consumers must tolerate duplicates. NATS has subject-based authorization, but a subject permission is not automatically a room membership, attachment-access decision, or Unix execution identity. [JetStream](https://docs.nats.io/concepts/jetstream), [Authorization](https://docs.nats.io/learn/security/authorization)

**Tradeoff:** valuable when independent services, distributed delivery or failover justify it. For the user's preferred first single-host Crew service, use a single writer, append journal and typed text event stream. Keep journal sequence IDs and replay boundaries explicit so a later deployment can add a different storage engine or event bus without changing the public room model. Do not treat a pub/sub broker as the authority for chat history or artifact ACLs.

### A simple file-based design is viable, with explicit recovery rules

`room` is already the most relevant lightweight file-based alternative in this comparison. Adding another terminal chat toy would not remove the UID, authorization and durability work. Ergo is a useful mature text-protocol reference; its durable history uses database backends, so it does not satisfy a strict no-database requirement without further work.

For Crew, propose the following small storage contract, to be proven with fault-injection tests before implementation is called robust:

- One broker writes a workspace journal. Ordinary users never append to it directly. Do not coordinate multiple writers by having each SSH account append into a shared file.
- Events are versioned UTF-8 JSON records, one per line, with a monotonically increasing sequence, operation ID, actor, workspace/channel scope and policy label. Bound record sizes and reject malformed records with an actionable error. JSON strings escape embedded newlines.
- A committed operation is acknowledged only after its durable journal write succeeds. An idempotency key allows safe retry after disconnection when the client cannot know whether its previous write committed.
- On startup, scan/verify the journal and rebuild state; handle a partial final record through a documented recovery procedure. Corruption in the middle is a failure requiring recovery, not permission to silently skip an authorization event.
- Write snapshots and metadata through temporary files, explicit synchronization and atomic replacement on the same filesystem. The journal is authoritative; snapshots and search indexes are rebuildable accelerators.
- For attachment publication, first stage and verify the blob, make it durable, then append the event that makes it visible. Unreferenced staged blobs can be reclaimed; a journal event must not acknowledge an attachment that was never durably stored.
- Use a broker lock and startup ownership check to prevent competing writers. Qualify the selected institutional filesystem's locking, append, rename, synchronization and recovery behavior. A shared NFS home is not automatically equivalent to local POSIX storage.
- Preserve room deletion/archival as an authorization event and retain stable creator identity across restart. Compaction, retention and backups must preserve required policy and audit information and must be recoverable after interruption.

This keeps the runtime stack small. The tradeoff is that Crew must implement and verify these transaction/recovery semantics rather than inheriting them from SQLite or PostgreSQL. Faster indexing and multiple writers are later options, not prerequisites for the requested collaboration design.

## Transport, identity and agent architecture implications

These are proposed design requirements derived from the comparison, not features proven in the current BioRouter build.

1. **Retain the institution's SSH portal.** Launch a fixed, noninteractive command such as `biorouter crew bridge --stdio` through OpenSSH. Let approved SSH configuration handle authentication, host verification and jumps. Avoid nested ad hoc shell construction or silently weakening host-key checking. Use a separate transfer stream or framed binary protocol for files so one large upload cannot stall chat/control events.
2. **Make identity server-derived.** The bridge connects locally to a broker UDS; the broker derives peer UID/GID through kernel credentials and resolves a server-side principal. Tokio provides a `peer_cred` API. Do not accept JSON `username`, `$USER`, display name, avatar, or a browser-supplied header as authority. [Tokio UnixStream peer credentials](https://docs.rs/tokio/latest/tokio/net/struct.UnixStream.html#method.peer_cred)
3. **Scope the principal.** Use a provisioned workspace UUID plus a managed Unix-account identity, not username alone across narrows and leo. Account deletion/recreation and UID reuse require an explicit lifecycle policy. An SSH alias is a connection profile; several aliases or login nodes can refer to one workspace. Shared Unix accounts cannot distinguish individual coworkers without adding another authentication factor.
4. **Separate broker from runners.** A dedicated unprivileged service owns the journal, metadata files, blobs and audit records. User-owned runner processes execute only as their authenticated UID and hold that user's credentials. The broker must not become a general `sudo` or arbitrary-command dispatcher. Its service account can read broker-managed room content by design; host root remains inside the infrastructure trust boundary.
5. **Use owner checks at every agent control endpoint.** Verify principal, agent owner, workspace, channel membership and privacy policy before prompt, cancel, approve, resume, model change or file access. Store immutable actor/owner IDs separately from nicknames. A room mention or quoted command is context, not an authorization to operate someone else's agent.
6. **Use ACP narrowly.** ACP defines agent/client sessions, prompts, updates, tool interactions and permissions over JSON-RPC. It is useful between BioRouter and a user-owned agent process. It does not replace multi-user room authorization or institutional data-flow policy. [ACP protocol overview](https://agentclientprotocol.com/protocol/v1/overview)
7. **Check membership at access time.** Every history page, live-event delivery, search, context assembly, download and preview must authorize the caller. Removing membership must terminate subscriptions and revoke future retrieval; a cached earlier download cannot be clawed back. Keep the journal, metadata and blobs inaccessible to ordinary users outside the broker API; otherwise API checks are bypassable through filesystem reads.
8. **Separate visibility from data classification.** A channel can be broadly visible within a workspace yet carry restricted data. Use independent membership/visibility and data-policy fields. “Public model allowed” must never follow from an SSH transport, room name, or login hostname alone.
9. **Bound automatic cross-channel context.** Discovery may search only channels the user currently has permission to access and that are allowed by the selected task's policy. Include provenance and channel IDs in selected context. Labels propagate to derived text, summaries, embeddings, exports, attachments and agent output. A restricted source cannot become public by copying it into a public conversation or by summarizing it.
10. **Design for multiple hosts.** A Unix socket is local to one kernel; a shared home directory does not make a UDS reachable from another login node. The shared broker needs a known service host, and peer-credential identity requires the bridge to execute there or an explicitly trusted authenticated gateway. Do not infer cross-node identity from NFS paths or numeric UIDs alone.

## HIPAA context: the consequential distinction

SSH protects a transport channel; it does not make a server, application, model endpoint, or complete workflow HIPAA compliant. HHS describes administrative, physical and technical safeguards, risk analysis, access management and audit review obligations. Crew's design must supply evidence useful to the institution's approval process rather than presenting SSH use as that approval. [HHS Security Rule summary](https://www.hhs.gov/hipaa/for-professionals/security/laws-regulations/index.html)

HHS cloud guidance also addresses business-associate agreements and responsibilities of cloud services that maintain ePHI. AWS access, an encrypted volume, a private IP or an institution-hosted model name does not by itself establish that a particular service/account/configuration is approved for this workload. [HHS cloud guidance](https://www.hhs.gov/hipaa/for-professionals/special-topics/health-information-technology/cloud-computing/index.html)

For the implementation plan, this means explicit institution-approved provider endpoints and egress policy; encryption and key custody for server/local storage and backups; audit records; retention and recovery; and policy for local downloads, notifications and previews. Arbitrary file sharing should mean arbitrary bytes can be stored and downloaded subject to policy, not that uploaded HTML, SVG, macros or executables run inside Crew. Provider routing is one of several egress paths: link previews, external image loads, telemetry, embeddings and bots need the same boundary.

## Maintenance and license observations

GitHub REST endpoints `/repos/{owner}/{repo}`, `/commits?per_page=1` and `/releases/latest` were queried live for these observations. Commit dates are default-branch commit metadata, not guarantees of support. Every repository below reported `archived: false`. “No latest release” means that endpoint returned HTTP 404, not that there are no tags. All dates below are UTC and were already in the past at the research timestamp.

| Repository | Observed default-branch commit date | Latest release observed | License interpretation |
|---|---|---|---|
| [knoxio-labs/room](https://api.github.com/repos/knoxio-labs/room) | 2026-03-26 | [v3.6.1 — 2026-03-26](https://github.com/knoxio-labs/room/releases/tag/v3.6.1) | MIT |
| [quackduck/devzat](https://api.github.com/repos/quackduck/devzat) | 2026-07-23 | [release-a4a1be9 — 2026-07-23](https://github.com/quackduck/devzat/releases/tag/release-a4a1be9) | MIT |
| [shazow/ssh-chat](https://api.github.com/repos/shazow/ssh-chat) | 2026-01-10 | [v1.10 — 2020-08-03](https://github.com/shazow/ssh-chat/releases/tag/v1.10) | MIT |
| [ergochat/ergo](https://api.github.com/repos/ergochat/ergo) | 2026-09-01 | [v2.19.1 — 2026-08-05](https://github.com/ergochat/ergo/releases/tag/v2.19.1) | MIT |
| [weechat/weechat](https://api.github.com/repos/weechat/weechat) | 2026-09-20 | [v4.10.1 — 2026-09-05](https://github.com/weechat/weechat/releases/tag/v4.10.1) | GPL-3.0-or-later |
| [dnysaz/xtc-server](https://api.github.com/repos/dnysaz/xtc-server) | 2026-03-16 | No latest release returned | MIT |
| [dnysaz/xtc-client](https://api.github.com/repos/dnysaz/xtc-client) | 2026-03-16 | No latest release returned | MIT |
| [element-hq/synapse](https://api.github.com/repos/element-hq/synapse) | 2026-09-21 | [v1.161.0 — 2026-09-15](https://github.com/element-hq/synapse/releases/tag/v1.161.0) | AGPL-3.0 |
| [zulip/zulip](https://api.github.com/repos/zulip/zulip) | 2026-09-21 | [12.3 — 2026-09-21](https://github.com/zulip/zulip/releases/tag/12.3) | Apache-2.0 |
| [mattermost/mattermost](https://api.github.com/repos/mattermost/mattermost) | 2026-09-21 | [v11.11.0 — 2026-09-07](https://github.com/mattermost/mattermost/releases/tag/v11.11.0) | Mixed; source/build/distribution distinctions above |
| [RocketChat/Rocket.Chat](https://api.github.com/repos/RocketChat/Rocket.Chat) | 2026-09-22 | [8.8.1 — 2026-09-09](https://github.com/RocketChat/Rocket.Chat/releases/tag/8.8.1) | MIT outside enterprise/third-party exceptions |
| [tinode/chat](https://api.github.com/repos/tinode/chat) | 2026-09-13 | [v0.25.3 — 2026-07-04](https://github.com/tinode/chat/releases/tag/v0.25.3) | Server GPL-3.0; clients Apache-2.0 |
| [nats-io/nats-server](https://api.github.com/repos/nats-io/nats-server) | 2026-09-21 | [v2.15.0 — 2026-09-17](https://github.com/nats-io/nats-server/releases/tag/v2.15.0) | Apache-2.0 |

No benchmark, multi-user isolation result, deployment approval or compliance determination is inferred from these release and license observations. The companion feasibility report should separately establish the actual SSH hosts' command execution, transport, filesystem and process-supervision behavior using synthetic fixtures.
