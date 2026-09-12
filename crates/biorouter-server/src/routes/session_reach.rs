//! Issue #56 Task 58 / [#47]: how far a caller reaches into a session it merely
//! *named*.
//!
//! ⚠ **`session_id` is a request parameter, not a credential.** Authorization is
//! one daemon-wide shared secret and the daemon has no principal, so no
//! session-scoped route can tell whether the caller has any relationship to the
//! session it addresses. AR-11 measured that secret to be recoverable by the
//! agent, so a public-tier agent could recover it and then read a private chat's
//! transcript, or run a turn inside it — defeating every tier gate without
//! touching one.
//!
//! ⚠ **This module does NOT fix [#47], and nothing here should be read as
//! claiming it does.** #47 says in its own words that this is *"a property of
//! the daemon's API surface as a whole, not of one endpoint"*, and the general
//! problem is that the daemon has no principal — an authorization redesign, not
//! a release fix. What this closes is the **privacy slice**: a session-addressing
//! route that reaches a **private** session must be made by a caller whose own
//! capability covers that session, or must carry the user-action proof.
//! #47 stays open with its residual narrowed to, and stated so it cannot be
//! read as smaller than it is:
//!
//! * a caller holding the daemon secret still reaches every **public** session
//!   it can name — that is not a privacy boundary and this gate is deliberately
//!   inert there;
//! * it still reaches every session-addressing route NOT on
//!   [the gated list](self#the-gated-list). `POST /agent/cancel` requires
//!   user-action proof on a daemon that holds a key and is on the list on one
//!   that does not (SD-11); `POST /interrupt` requires the proof on **either**
//!   kind and so is on neither (SD-11a — `routes::reply::authorize_steer` says
//!   why the steer did not move with the Stop); `GET
//!   /sessions/{id}/extensions`, `GET /sessions/{id}/usage`, `PUT
//!   /sessions/{id}/name`, `PUT /sessions/{id}/user_workflow_values` and
//!   `DELETE /sessions/{id}` were open until QA's 2026-09-10 sweep, which
//!   measured the last one deleting a private chat the read refused (F0), and
//!   they are on the list now. `GET /active_work` and `POST
//!   /active_work/{id}/cancel` remain open — they name no session id in their
//!   path and so enumerate, but carry a `title` and `detail` holding the SHELL
//!   COMMAND or TASK PROMPT of every running job. That is content rather than
//!   metadata, and it is the one row here that a reader should not file
//!   mentally beside "titles and directories". `GET /sessions/running` (ids
//!   only, and `biorouter session list` needs it whole to report liveness
//!   truthfully), `GET /sessions/changes` (a watched row's provider, model and
//!   tier columns), `GET /sessions/insights` and `GET /sessions/activity`
//!   (aggregates) remain open too.
//!   ⚠ **This bullet listed `POST /agent/resume` as open until 2026-09-04, and
//!   it was wrong** — measured against a live private session, `/agent/resume`
//!   answers 403 without the capability header and 200 with it, because
//!   `resume_agent` has called [`session_reach`] directly since before this
//!   sentence was last touched. A residual that names a route which is in fact
//!   guarded is worse than one that omits it: it invites someone to "close" a
//!   gate that is already there, and it makes the rest of the enumeration read
//!   as measured when it is remembered. The list began as the five
//!   the ruling named, and `/reply` dominates them, but "dominates" is an
//!   argument about capability rather than a proof about every route — which is
//!   how `/export` and `/events` sat outside it while returning the same bytes
//!   as `GET /sessions/{id}`, and how `GET /diagnostics/{id}` sat outside it
//!   afterwards while returning those same bytes *inside a zip*. All three are
//!   on it now, and the third one is why the sentence above is worded as an
//!   enumeration: two sweeps have each ended one route short. **The residual
//!   below is a snapshot of an enumeration, not a proof of completeness**; the only
//!   mechanical part of this is the wiring census
//!   (`crates/biorouter/tests/privacy_guard_wiring.rs`), and even that pins the
//!   guards, not the routes;
//! * both read and write halves of `/knowledge/active` are gated when they name
//!   a session. Machine-wide selection requests name no chat and remain outside
//!   the session boundary;
//! * ~~**`GET /sessions` and `GET /sessions/sidebar` are still open, and they
//!   enumerate wholesale.**~~ **ANSWERED 2026-09-11 (QA M1): they FILTER.** QA
//!   measured `GET /sessions` returning all 5,543 rows — 792 private, each with
//!   id, title, working directory and privacy reason — to a caller the singular
//!   read refuses, which undercut the whole reason [`SESSION_OUT_OF_REACH`] is
//!   one sentence for two answers. The decision this bullet left open is now
//!   made: a listing shows a caller exactly the rows this gate would admit it to
//!   ([`HttpCaller::lists_session`]), so the list is the union of what per-id
//!   probing could learn and nothing more. **Filter, not refuse**: a refused
//!   list would break every client on the public chats the gate is deliberately
//!   inert on. ⚠ **The sidebar's pagination was the second half of this and got
//!   it wrong.** It answered M1 by scanning the unfiltered ordering, dropping
//!   the private rows in Rust and resuming from the position it had reached — so
//!   two continuation values subtracted gave the exact number of private chats
//!   between two visible ones, and because `updated_at` is stamped on every
//!   token written, polling the route reported when a private chat was running.
//!   Since the adversarial review of 2026-09-12 the tier is a **SQL predicate**
//!   and the page resumes from a keyset of the last row it RETURNED, so there is
//!   nothing hidden left to count. `GET /schedule/{id}/sessions` takes the same
//!   filter;
//! * **knowledge bases take the same decision** since the same sweep (QA H2):
//!   every `/knowledge/bases/{id}…` route sits behind [`gate_knowledge_base`],
//!   and `GET /knowledge/bases` and `/knowledge/active` omit what the caller
//!   cannot reach. The target is the base's tier, an absent or malformed id is
//!   [`TargetTier::Unreadable`], and the words are [`KNOWLEDGE_BASE_OUT_OF_REACH`];
//! * a `biorouter serve` daemon's own interface — a request carrying the served
//!   document's cookie — is given its operator's configured tier on those
//!   listing and knowledge-base surfaces, which were open to it before they
//!   were gated, and on NOTHING this function decides ([`HttpCaller`],
//!   `docs/deployment/serve-decisions.md` SD-10);
//! * **`workspace_read_conversation` was open too, and it is CLOSED — but by a
//!   different instrument, and a reader must not credit this module for it.**
//!   That MCP tool (`crates/biorouter/src/agents/workspace_extension.rs`) used
//!   to load any named session *with messages* and check only
//!   `session_type == Hidden`, so a model read a private transcript through a
//!   tool call, needing no daemon secret at all. It was **not fixable with this
//!   instrument**: a tool call is by definition the model, so it can never carry
//!   a proof of the user, and refusing it for want of one would refuse it
//!   always. It is fixed with §7's `may_read`, which
//!   [`biorouter::privacy::visibility`] ships and which
//!   `workspace_read_conversation`, `workspace_list`, `workspace_send_prompt`
//!   and `workspace_open` now call — comparing the CALLER'S CAPABILITY with the
//!   target's classification instead of asking for a human. Its refusal answers
//!   "private" and "no such conversation" in one sentence, for the same reason
//!   [`SESSION_OUT_OF_REACH`] does. `workspace_close` and `workspace_set_tools`
//!   enforce the same `may_write` rule as `workspace_send_prompt` — which is
//!   the tier and nothing else, the one-hop lineage clause having been
//!   retired; `workspace_watch` is parent-scoped through the
//!   caller's registered background handles rather than an arbitrary session
//!   write;
//! * and the daemon still has no principal, which is the actual subject of #47.
//!
//! # The blast radius is wider than "private chats"
//!
//! `Unreadable` is refused **identically** to `Private`, which Step 4.3 requires:
//! a refusal that answered "no such chat" would be the per-id oracle described
//! above. The consequence is a behaviour change rather than a wording one, and it
//! is bigger than the headline. Over HTTP, on [the gated
//! routes](self#the-gated-list), an unproven caller naming a session this daemon
//! cannot read is refused **whatever tier that session would have had** — an id
//! that never existed, one that was deleted, one a client held across a store
//! reset, one not persisted yet.
//!
//! `biorouter session send <id>` is the concrete case, and it is also the case
//! that **used to be enforced backwards**. The CLI posts `/reply` and can never
//! carry a proof of a human — a terminal is precisely the surface a model with
//! shell access drives — so under a proof-only rule it was refused for a private
//! chat whatever model it was running, while the desktop app was admitted for
//! the same chat *while running the same public model*. The rule the design
//! actually states is `caller capability >= target classification`, which is the
//! rule [`biorouter::privacy::visibility::may_read`] states for the tool surface
//! and the rule Gate A states for a bind; measuring the surface instead of the
//! capability got the answer exactly wrong in both directions.
//!
//! So reach now has **two sufficient conditions**, and this is the one thing to
//! read carefully if you are extending this module:
//!
//! * the caller's capability covers the target — stated on the request as
//!   [`CALLER_PROVIDER_HEADER`] (a provider NAME, resolved to a tier by *this*
//!   daemon's registry, never a tier the caller asserts); or
//! * the request carries the user-action proof, which is how the desktop app
//!   reaches a private chat it has open on a public model.
//!
//! ⚠ **Only REACH moved.** Raising a session's classification and declassifying
//! one still take the user-action proof and nothing else — a capability is a fact
//! about a model, and neither of those is a decision a model may make. Those
//! gates live in `routes/session.rs` and `privacy::declassify`, and this change
//! does not touch them.
//!
//! An unknown session is still answered exactly as a private one is, at every
//! (capability, proof) pair, so the refusal is no more of an oracle than it was.
//!
//! ⚠ The residual is unchanged and is stated in full above: a caller holding the
//! daemon secret can spell any installed provider's name in that header, exactly
//! as it could already reach every public session. The header makes the gate able
//! to express the right rule; it does not make the daemon able to authenticate
//! anyone, which is [#47].
//!
//! # The gated list
//!
//! | Route | Why it is on the list |
//! |---|---|
//! | `POST /reply` | Runs an agent turn, with tools, in the named session. It **strictly dominates** the rest: a caller who can run a turn in a session can already do anything that session can do. |
//! | `GET /sessions/{session_id}` | Returns the transcript. |
//! | `GET /sessions/{session_id}/export` | The **same** transcript: `SessionManager::export_session` is `get_session(id, true)` then `to_string_pretty`. Added by the wiring sweep — an unguarded sibling of the row above, reachable from the generated TS client as `exportSession`. |
//! | `GET /sessions/{session_id}/events` | The same transcript **plus a live tail**: the stream opens with an `UpdateConversation` snapshot of the whole stored conversation. Added by the wiring sweep; it is the route `biorouter session watch <id>` drives. |
//! | `GET /diagnostics/{session_id}` | The same transcript **in a zip**: `generate_diagnostics` writes `session.json` straight from `SessionManager::export_session`, and ships this session's log files — which carry its prompts — beside it. Added by the second wiring sweep; it is the third route on this list whose entire payload is `get_session(id, true)` under a different name. |
//! | `POST /agent/update_working_dir` | Repoints the session at a directory of the caller's choosing and restarts its agent. |
//! | `POST /agent/add_extension` | Attaches tools to the session. |
//! | `GET|POST /knowledge/active` | Reads or repoints the session's knowledge bases and write target. |
//! | `POST /agent/resume` | Loads the session's stored conversation into a live agent. Gates directly, like the rows above. |
//! | `GET /agent/callable_tool_count` | Counts the named session's MODEL-FACING tools, and answers through `get_or_create_agent` — so it CREATES an agent for a session that has none. Added 2026-09-12 by the SD-8 review of #260, which found it with no gate of any kind; the renderer had merely stopped calling it. ⚠ Its sibling `GET /agent/tools` is **deliberately** not here: that one is the unfiltered permission-editor surface, so a person can administer private tools a public model cannot see. |
//! | `POST /agent/continuation/recover` | Resumes a parked continuation in the named session. Gates directly. |
//! | `POST /agent/update_from_session` | Adopts another session's provider configuration. Gates directly. |
//! | `POST /agent/update_provider` · `restart` · `stop` · `remove_extension` | Gate through [`authorize_agent_control`](../agent/fn.authorize_agent_control.html), which calls [`session_reach`] and then reads the row. |
//! | `POST /agent/cancel` · `/agent/continuation/abandon` | Stop and settle the named session's turn. **On a daemon that holds no user-action key only** (serve decision SD-11): there `routes::reply::authorize_turn_control` gates them through the same `authorize_agent_control` as the row above, so a Stop admits exactly the callers `/agent/stop` does. A daemon that holds a key asks them for the proof instead, which reaches every chat. `POST /interrupt` is NOT here: it asks for the proof on both kinds of daemon, so it never reaches this gate — see `routes::reply::authorize_steer`. |
//! | `DELETE /sessions/{session_id}` | QA 2026-09-10 F0: deleted a private chat the read refused, four of four. Gated before the turn is cancelled or anything parked is released. |
//! | `PUT /sessions/{session_id}/name` · `user_workflow_values` | Writes into the chat; the second re-applies its workflow to the live agent. |
//! | `POST /sessions/{session_id}/edit_message`, `editType: edit` | Truncates the chat in place. (`diverge` keeps DR-19's stricter proof gate.) |
//! | `GET /sessions/{session_id}/extensions` · `usage` | The chat's extensions by name (M2's sibling); its usage, whose 200/404 was an existence oracle. |
//! | `GET /agent/tools` · `GET /agent/callable_tool_count` | QA M2: a private chat's private-connector tool names. Both mint an agent for the chat, so the gate runs first. The empty `session_id` of the settings page names no chat. |
//! | `POST /workflows/create` | Loads the chat's whole transcript and returns what a model makes of it. |
//! | `POST /skills/session` | Writes a skill's instructions into the chat's next turn. |
//! | `POST /knowledge/bases/{id}/ingest-conversation` | Every chat the request names, checked before any is loaded. |
//!
//! Every row since the 2026-09-10 sweep answers with [`SESSION_OUT_OF_REACH`]
//! as PLAIN TEXT — the bytes `GET /sessions/{session_id}` returns — rather than
//! through the route's own error envelope, so one boundary has one body.
//!
//! ⚠ **Two spellings, one list.** The `update_provider` · `restart` · `stop` ·
//! `remove_extension` row and the `cancel` · `continuation/abandon` row reach
//! the gate through a helper rather than by naming it, which is why a scan for
//! the literal `session_reach(` reports those six as ungated and why the ordering
//! test below uses two of them as over-read controls. They are NOT exempt — measured
//! live, each of the first four answers 403 without the capability header and
//! proceeds with it, and `tests/turn_control_no_user_key.rs` measures the other
//! two on a keyless daemon. A future sweep that greps for the call must follow
//! `authorize_agent_control` too, or it will "discover" six holes that are not
//! there and, worse, trust the same grep when it reports a real one.
//!
//! # Why `X-User-Action` and not a new mechanism, for the proof half
//!
//! The instrument already exists ([`biorouter_server::auth::user_action_proof`],
//! Task 18A) and it is the right one here for a reason worth writing down: the
//! daemon holds only the **digest**, while the key lives in the Electron main
//! process and is never in the daemon's environment. So the very recoverability
//! AR-11 measured does **not** hand an agent this proof. That asymmetry is the
//! whole reason the mechanism works, and it must not be undone by caching the
//! key daemon-side for convenience.
//!
//! [#47]: https://github.com/BaranziniLab/biorouter/issues/47

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::session::session_manager::SessionManager;
use biorouter_mcp::knowledge::service::KnowledgeService;
// Issue #56 DR-16. `src/routes/` is compiled into the `biorouterd` binary as
// well as the lib and cannot name `crate::auth`, so this is the shared
// direction — the same import `routes::session` and `routes::knowledge` use.
use biorouter_server::auth::{served_operator_capability, user_action_proof, UserActionProof};
use std::path::Path;
use std::sync::Arc;

/// The header a Biorouter client names the model it is running under.
///
/// ⚠ **The NAME of a provider, never a tier.** A caller that could send
/// `X-Caller-Tier: private` would be asserting its own answer; a caller that
/// sends `versa_azure` is stating a fact this daemon resolves for itself,
/// against its own installed provider registry
/// ([`biorouter::workflow::privacy::declared_provider_tier`]). A name this
/// install does not publish, an unparseable value and an absent header all
/// resolve to [`ProviderTier::Public`] — the fail-safe side — so the header can
/// only ever be a claim the daemon has independently confirmed is *possible*.
///
/// ⚠ **This is not authentication, and it is not sold as one.** A caller
/// holding the daemon secret can spell any installed provider's name here, and
/// the module header already records that such a caller reaches everything
/// anyway (the daemon has no principal — [#47]). What the header buys is that
/// the gate can express the rule the ruling actually states, *caller capability
/// ≥ target classification*, instead of the proxy it used to enforce.
///
/// ⚠ **It has a prose home now, and it did not before.** The header shipped
/// documented only here, in the source of the gate that reads it — which is
/// exactly the wrong place for its only reader, a person or a program writing
/// automation against the daemon. `docs/deployment/programmatic-session-access.md`
/// is the user-facing page: the fail-safe table, `curl` for the JSON read and the
/// SSE stream, and a per-route audit of what this gate does and does not cover.
/// Keep the two in step — the doc states the resolution rules this function
/// implements, so a change here is a change there.
///
/// ⚠ **That page is where the mechanism lives, and the refusal only points at
/// it.** [`SESSION_OUT_OF_REACH`] names the page and not this header, on
/// purpose: the operator reading the page is not the caller being refused, and
/// a refused public-model caller handed a header to add has been handed the
/// retry that refusal exists to foreclose.
///
/// [#47]: https://github.com/BaranziniLab/biorouter/issues/47
pub const CALLER_PROVIDER_HEADER: &str = "X-Caller-Provider";

/// What a caller with neither the capability nor the user's proof is told when
/// it names a session it may not reach.
///
/// ⚠ **ONE sentence for "that chat is private" and for "there is no such
/// chat", deliberately.** Such a caller must not learn whether a session
/// exists, or anything about it, from the shape of the refusal — otherwise the
/// refusal itself becomes an oracle that enumerates the machine's private chats
/// one id at a time. §14.4's content rule holds: it names the boundary and
/// nothing about the chat — no id, no title, no working directory, no tier.
///
/// It forecloses the retry for the reason every refusal in this feature does: a
/// model that reads a refusal as transient loops on it.
///
/// ⚠ **It names BOTH ways through**, because there are two and a refusal that
/// named one would send half its readers somewhere that cannot help them. The
/// oldest wording said only "this request carried no proof it came from the
/// person at the keyboard", which was the whole defect: a terminal running
/// Versa was told to go and be a human, when what it needed was to be told it
/// already had the capability and merely was not saying so.
///
/// ⚠ **It SIGNPOSTS the programmatic channel; it does not hand out a recipe,
/// and it must never be reworded into one.** A version of this constant shipped
/// that named [`CALLER_PROVIDER_HEADER`], gave an example value that resolves
/// Private on a real install, and told the reader to send it. That is the one
/// direction this arm may not grow. The reader of this 403 is a MODEL far more
/// often than a person; the arm exists to foreclose the retry; and a caller
/// handed a header to add has been handed a way to read itself out of "do not
/// retry as you are", which is the whole force of the sentence above it.
/// `routes/status.rs` states the rule for the arm beside this one in as many
/// words: *a refusal that hands the reader a working command is the opposite
/// move on the adjacent arm*.
///
/// ⚠ **The defect that wording was written against is real, and the signpost is
/// what answers it.** An agent read this refusal end to end, took "reachable
/// from a session running a private model" for a state it had no way to enter,
/// and reported the absence of a channel that has always shipped. So the
/// refusal says the channel exists and names the page that documents it, in the
/// register the keyless arm already uses for its own hint: addressed to whoever
/// operates the daemon, phrased as a setup decision that is theirs rather than
/// this caller's, and placed BEFORE the closing "stop and ask the user" so the
/// last thing a model reads is the stop. The header, an example value and the
/// `curl` live on that page, `docs/deployment/programmatic-session-access.md`,
/// whose reader is an operator rather than the caller being refused.
///
/// ⚠ **The sentence is about the CALLER'S OWN situation and nothing else**,
/// which is the line [`SESSION_REACH_NO_KEY`] already walks and the only line on
/// which this constant may grow. It is fixed text: it does not vary with the
/// target, is not derived from it, and is emitted identically for `Private` and
/// for [`TargetTier::Unreadable`], so the byte-for-byte indistinguishability
/// that keeps this refusal from being a per-id oracle is untouched. Anything
/// that named the chat, even to say the channel would have worked for it, would
/// rebuild the oracle in the act of being helpful.
pub const SESSION_OUT_OF_REACH: &str =
    "That chat is private, or there is no chat with that id. This request was made on a public \
     model and carried no proof it came from the person at the keyboard, and the two answers are \
     deliberately the same so that nothing about the chat is disclosed. Nothing was read and \
     nothing was changed. Do not retry as you are; the same call will be refused again, and no \
     setting, hook or permission mode changes it. A private chat is reachable from a session \
     running a private model, one the institution hosts or one that runs on this machine, or \
     from the desktop app when the person at the keyboard acts. Pointing a program that already \
     runs under such a model at this daemon is a setup decision for whoever operates it, and the \
     Biorouter documentation covers it under 'Reaching a private chat from a script'. If this \
     task genuinely needs that chat, stop and ask the user to open it for you.";

/// …and when this daemon was handed no user-action key at all.
///
/// A separate sentence, per Task 18A's open question 23: reporting "this daemon
/// cannot verify a human" as "you are not a human" sends the person at the
/// keyboard hunting for a permission they can never obtain. `just run-server`, a
/// hand-run `biorouterd agent` and every headless deployment land here, and
/// private sessions are unreachable over HTTP on such a daemon. That is the
/// fail-closed direction open question 23 already accepted, and it must not be
/// softened with an env-var escape — the daemon's environment is exactly what
/// AR-11 measured to be recoverable.
///
/// ⚠ It says nothing about the named chat either, so it is no more of an oracle
/// than [`SESSION_OUT_OF_REACH`]: it separates *credential states of the
/// caller*, which the caller already knows, never *states of the session*.
pub const SESSION_REACH_NO_KEY: &str =
    "This daemon was started without a user-action key, so it cannot verify that a request came \
     from the person at the keyboard, and reaching into a private chat requires that proof. \
     Nothing was read and nothing was changed. This control is unavailable on this daemon; use \
     the desktop app.";

/// [`SESSION_OUT_OF_REACH`] for a knowledge base the caller named — the same
/// decision, from the same function, with the subject's noun changed and
/// nothing else (issue #56, QA 2026-09-10 H2).
///
/// ⚠ **ONE sentence for "that base is private" and for "there is no such
/// base"**, for the reason the chat constant gives. A base's id and name are
/// user-authored content — the plan's Task 10D ruled that directly enumerating
/// them is the content crossing, not a side channel — so a refusal that told a
/// private base from an absent one would enumerate the machine's private bases
/// one guess at a time. The existence oracle AR-5 accepts is a different door
/// (`create_base`'s "already exists") and nothing here widens it.
///
/// ⚠ Every constraint on [`SESSION_OUT_OF_REACH`] binds this one, and the leak
/// guards below are run against both: it names no base, no page and no path; it
/// is fixed text; it signposts the operator page without naming the header; and
/// its last words are the stop.
///
/// ⚠ **The KB tool path says something different, deliberately.** A model
/// calling `kb_read_page` is told [`biorouter_mcp::knowledge::tier::KB_PRIVATE_REFUSAL`]
/// ("switch this chat to a private model"), which is the remedy for a chat. An
/// HTTP caller has no chat to switch; what it has is this daemon's reach rule,
/// the one [`SESSION_OUT_OF_REACH`] states for a chat.
pub const KNOWLEDGE_BASE_OUT_OF_REACH: &str =
    "That knowledge base is private, or there is no knowledge base with that id. This request was \
     made on a public model and carried no proof it came from the person at the keyboard, and the \
     two answers are deliberately the same so that nothing about the knowledge base is disclosed. \
     Nothing was read and nothing was changed. Do not retry as you are; the same call will be \
     refused again, and no setting, hook or permission mode changes it. A private knowledge base \
     is reachable from a session running a private model, one the institution hosts or one that \
     runs on this machine, or from the desktop app when the person at the keyboard acts. Pointing \
     a program that already runs under such a model at this daemon is a setup decision for \
     whoever operates it, and the Biorouter documentation covers it under 'Reaching a private chat \
     from a script'. If this task genuinely needs that knowledge base, stop and ask the user to \
     open it for you.";

/// …and [`SESSION_REACH_NO_KEY`]'s sibling, for a daemon that was handed no
/// user-action key at all — a `biorouter serve` daemon among them (SD-7), whose
/// browser reads this when it is pointed at a private base its operator's tier
/// does not cover.
pub const KNOWLEDGE_BASE_REACH_NO_KEY: &str =
    "This daemon was started without a user-action key, so it cannot verify that a request came \
     from the person at the keyboard, and reaching into a private knowledge base requires that \
     proof. Nothing was read and nothing was changed. This control is unavailable on this daemon; \
     use the desktop app.";

/// The named session, reduced to the one bit this gate turns on.
///
/// Three states rather than two because the third has to be *represented* in
/// order to be provably answered the same way as `Private`; folding it in at the
/// type level would make the indistinguishability a definition rather than a
/// tested claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetTier {
    /// The row was read, and it is public.
    Public,
    /// The row was read, and it is private.
    Private,
    /// The row could not be read: no such session, a recycled id, a store
    /// error. **Answered exactly as `Private` is.**
    Unreadable,
}

/// A refusal from [`refuse_unless_reachable`].
///
/// Carries a `&'static str` rather than a `String` so the two constants above
/// are the only two things it can ever say — which is what makes "these two
/// inputs produce the identical response" checkable by equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionOutOfReach {
    pub status: StatusCode,
    pub message: &'static str,
}

impl IntoResponse for SessionOutOfReach {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

impl From<SessionOutOfReach> for super::errors::ErrorResponse {
    fn from(refusal: SessionOutOfReach) -> Self {
        Self {
            status: refusal.status,
            message: refusal.message.to_string(),
        }
    }
}

impl SessionOutOfReach {
    /// The same refusal, worded for a knowledge base.
    ///
    /// A mapping between the constant pairs rather than a second decision: the
    /// verdict — which of the two arms, and that it refused at all — is
    /// [`refuse_unless_reachable`]'s, and this changes only the noun. Private,
    /// because nothing outside this module should be choosing a refusal's words
    /// apart from the decision that produced it.
    fn for_knowledge_base(self) -> Self {
        let message = if self.message == SESSION_REACH_NO_KEY {
            KNOWLEDGE_BASE_REACH_NO_KEY
        } else {
            KNOWLEDGE_BASE_OUT_OF_REACH
        };
        Self { message, ..self }
    }
}

impl From<SessionClassification> for TargetTier {
    /// A row the caller already holds — a listing's — is readable by
    /// construction, so it is never [`TargetTier::Unreadable`].
    fn from(classification: SessionClassification) -> Self {
        match classification {
            SessionClassification::Private => Self::Private,
            SessionClassification::Public => Self::Public,
        }
    }
}

/// May a caller in this credential state reach a session in this state?
///
/// ⚠ **Extracted so the claim is asserted rather than grepped for.** None of the
/// gated handlers can be driven from a unit test cheaply — `AppState::new()`
/// opens the developer's REAL session database — so a scan for
/// `session_reach(` would keep passing against a call whose result was
/// discarded. This mapping is pure, so every corner of it is driven for real by
/// the `tests` module below (not linked: it is `#[cfg(test)]`, so rustdoc cannot
/// resolve it), and the one thing a pure function cannot see — that the gate
/// runs before the route touches anything — stays a source scan and says so.
///
/// `enforced` is DR-15's master opt-out, taken as an argument rather than read
/// here so that "the switch is off" is a corner this function can be tested at.
/// With tiers off the gate is entirely inert — including for `Unreadable`, so a
/// user who opted out still gets their 404 rather than a 403 for a chat that
/// simply is not there.
pub fn refuse_unless_reachable(
    enforced: bool,
    tier: TargetTier,
    caller: ProviderTier,
    proof: UserActionProof,
) -> Result<(), SessionOutOfReach> {
    if !enforced {
        return Ok(());
    }
    match tier {
        // A public chat is reachable by anything holding the daemon secret,
        // exactly as it was before this gate existed. A barrier that fired on
        // every chat is one people route around, and it would break every
        // client that has never sent this header.
        TargetTier::Public => Ok(()),
        // ⚠ **CAPABILITY FIRST — this is the inversion.** The rule is *caller
        // capability ≥ target classification*, which is the same rule
        // `privacy::visibility::may_read` states for the tool surface and the
        // same rule Gate A states for a bind. It was previously enforced as
        // *proof-of-human*, and the two are not the same question: a CLI running
        // Versa has the capability and can never have the proof (a terminal is
        // exactly the surface a model with shell access drives), while the
        // desktop app running Versa had the proof and was admitted. So the
        // capable caller was refused and the proven one allowed — backwards.
        //
        // The user-action arms below are KEPT, not replaced: the desktop app is
        // a legitimate caller whose reach comes from the person at the keyboard
        // rather than from the model it happens to be running, and removing
        // them would refuse every GUI read of a private chat opened on a public
        // model. Reach now has two sufficient conditions; raising a session's
        // tier and declassifying still have exactly one, and it is the proof.
        TargetTier::Private | TargetTier::Unreadable if caller.is_private() => Ok(()),
        TargetTier::Private | TargetTier::Unreadable => match proof {
            UserActionProof::Proven => Ok(()),
            UserActionProof::Unproven => Err(SessionOutOfReach {
                status: StatusCode::FORBIDDEN,
                message: SESSION_OUT_OF_REACH,
            }),
            UserActionProof::NoKeyInstalled => Err(SessionOutOfReach {
                status: StatusCode::FORBIDDEN,
                message: SESSION_REACH_NO_KEY,
            }),
        },
    }
}

/// The capability the request claims, resolved against **this install's**
/// provider registry.
///
/// The header carries a provider NAME; the tier is this daemon's own answer to
/// "what does this install think that provider is". An absent header, a name
/// this install does not publish, and a value that is not valid UTF-8 all give
/// [`ProviderTier::Public`] — the fail-safe side, and also the historical
/// behaviour for every client that has never sent it.
///
/// ⚠ **The DECLARED tier, not an instance's.** `declared_provider_tier` reads
/// `ProviderMetadata::tier`, and the two can disagree in the permissive
/// direction (`ollama` re-pointed off the machine by `OLLAMA_HOST` ships Private
/// and resolves Public). That residual is inherited rather than introduced —
/// `workflow::privacy` and the CLI's start-time refusal already reason from the
/// declared tier for the same reason: there is no instance here to ask, and
/// constructing one would need the provider's credentials just to answer a
/// routing question.
///
/// ⚠ **Deliberately NOT `pub`.** This module is one of `COMPLETE_MODULES` in the
/// wiring census (`crates/biorouter/tests/privacy_guard_wiring.rs`): every
/// public function in it must carry a census row classifying it as a reach
/// decision. This is not one — it resolves an input to [`session_reach`], which
/// is the guard and which does carry a row — and its only caller is that
/// function, three lines below. Making it public to save an import would either
/// break the census or add a row that misdescribes what it is.
async fn caller_capability(headers: &HeaderMap) -> ProviderTier {
    let Some(name) = headers
        .get(CALLER_PROVIDER_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return ProviderTier::Public;
    };
    biorouter::workflow::privacy::declared_provider_tier(name).await
}

/// Read the named session's tier, failing closed on anything that is not a
/// readable public row.
///
/// Metadata only (`with_messages: false`): resolving the tier must not be a way
/// to load the very transcript the gate is about to refuse.
pub async fn target_tier(manager: &SessionManager, session_id: &str) -> TargetTier {
    match manager.get_session(session_id, false).await {
        Ok(session) if session.privacy_tier == SessionClassification::Private => {
            TargetTier::Private
        }
        Ok(_) => TargetTier::Public,
        Err(_) => TargetTier::Unreadable,
    }
}

/// The whole gate, as one call: **resolve the target session's tier first, and
/// require the user-action proof when that tier is Private.**
///
/// ⚠ **Call this before the handler does anything else.** Task 49's grant route
/// establishes the ordering and says why; this is the same rule with a session
/// as its subject. A handler that fetched the agent, or validated the request,
/// or took the turn lock first would hand an unproven caller a side channel —
/// "this chat is busy", "this extension is not enabled here", "no such session"
/// — that discloses exactly what the refusal is worded to withhold.
pub async fn session_reach(
    manager: &SessionManager,
    session_id: &str,
    headers: &HeaderMap,
) -> Result<(), SessionOutOfReach> {
    // DR-15's master opt-out, read INSIDE the gate. A direct read, not a
    // `CallCapability`: an HTTP request naming a session is not a tool call and
    // has no admitted capability to inherit.
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    // Short-circuit BEFORE the store read, so the opt-out costs nothing per
    // request rather than a database round trip per request.
    if !enforced {
        return Ok(());
    }
    refuse_unless_reachable(
        enforced,
        target_tier(manager, session_id).await,
        caller_capability(headers).await,
        user_action_proof(headers),
    )
}

/// Who is asking, resolved ONCE per request and threaded through every decision
/// that request needs — the HTTP counterpart of `CallCapability`, and for the
/// same reason: a listing that re-read the master switch or re-resolved the
/// caller per row could half-believe two answers.
///
/// It carries the two facts [`session_reach`] turns on — the capability the
/// request states ([`CALLER_PROVIDER_HEADER`]) and the user-action proof — and a
/// third that only a `biorouter serve` daemon ever sets:
/// `auth::served_operator_capability`, the operator's configured tier, earned by
/// presenting the served document's cookie.
///
/// ⚠ **The third input is read by the surfaces this type serves, and never by
/// [`session_reach`].** Listings and knowledge bases were fully open to a serve
/// daemon's browser before they were gated, so honouring the operator's tier
/// there keeps that browser's reach exactly where it was. The transcript gate
/// refused that browser every private chat before this type existed, and
/// feeding the operator's tier into it would admit what it refused — the one
/// thing this change may not do. Whether a serve operator on a private provider
/// should reach a private transcript is a decision still to be made, and it is
/// recorded as open in `docs/deployment/serve-decisions.md` SD-10, not taken here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpCaller {
    /// DR-15's master opt-out, sampled with everything else.
    enforced: bool,
    /// What the request states it runs under, resolved by this daemon's
    /// registry — [`caller_capability`].
    stated: ProviderTier,
    /// A serve daemon's operator tier, for a request from its served document.
    /// `Public` on every other daemon and for every other request.
    served_operator: ProviderTier,
    proof: UserActionProof,
}

/// Resolve the caller behind one request. See [`HttpCaller`].
pub async fn http_caller(headers: &HeaderMap) -> HttpCaller {
    HttpCaller {
        enforced: biorouter::privacy::privacy_tiers_enabled(),
        stated: caller_capability(headers).await,
        served_operator: served_operator_capability(headers),
        proof: user_action_proof(headers),
    }
}

impl HttpCaller {
    /// Private if either capability input is: a program stating a private
    /// provider, or a serve daemon's own interface on a private one.
    fn capability(&self) -> ProviderTier {
        if self.stated.is_private() || self.served_operator.is_private() {
            ProviderTier::Private
        } else {
            ProviderTier::Public
        }
    }

    /// May this caller be shown a chat of this classification in a listing?
    ///
    /// Exactly [`refuse_unless_reachable`]'s answer for the row, so a listing is
    /// the union of what the singular gate admits one id at a time and cannot
    /// tell a caller anything per-id probing is worded to withhold. **Omission,
    /// not redaction**: a row carries an LLM-written title and a working
    /// directory, both content (§11.4), which is the rule `workspace_list`
    /// already applies to a model.
    pub fn lists_session(&self, classification: SessionClassification) -> bool {
        refuse_unless_reachable(
            self.enforced,
            TargetTier::from(classification),
            self.capability(),
            self.proof,
        )
        .is_ok()
    }

    /// The reach gate for a knowledge base the caller named — the same pure
    /// decision a chat gets, with the base's tier as the target and
    /// [`KNOWLEDGE_BASE_OUT_OF_REACH`] as its words.
    ///
    /// An id that is not well-formed, and one that names no base, are
    /// [`TargetTier::Unreadable`] and so are refused exactly as a private base
    /// is — to a caller that proves nothing. A caller that does prove it is the
    /// user is let through to the handler, which tells them the truth (400 or
    /// 404). DR-15's opt-out is inert all the way down, including for the
    /// absent id, so a user who turned tiers off still gets their 404.
    pub fn reach_knowledge_base(&self, root: &Path, kb_id: &str) -> Result<(), SessionOutOfReach> {
        if !self.enforced {
            return Ok(());
        }
        refuse_unless_reachable(
            self.enforced,
            knowledge_base_tier(root, kb_id),
            self.capability(),
            self.proof,
        )
        .map_err(SessionOutOfReach::for_knowledge_base)
    }

    /// May this caller **mint** a knowledge-base id — `POST /knowledge/bases`
    /// (adversarial security review 2026-09-12, MEDIUM)?
    ///
    /// ⚠ **It takes no id, and that is the fix rather than an omission.** Create
    /// refuses an id that is taken, so an answer that depended on the id would
    /// tell its caller which ids are taken — and a private base's id is content
    /// the listing deliberately omits ([`KNOWLEDGE_BASE_OUT_OF_REACH`] says so
    /// in as many words). The old route answered a colliding private id with
    /// `400 kb '<id>' already exists at /Users/…/knowledge/<id>` and a free one
    /// with `200`, so a short dictionary of plausible names enumerated the
    /// machine's private bases, with the absolute path thrown in.
    ///
    /// The question asked instead is about the **namespace**: a not-yet-existing
    /// id has exactly the tier [`TargetTier::Unreadable`] names, and a caller
    /// that may not be told about such an id may not take one either. Because
    /// the id is never read, the refusal is the same for every id — which is the
    /// property, stated as a type rather than as a promise.
    ///
    /// What this costs, stated plainly: a caller holding nothing but the daemon
    /// secret can no longer create a knowledge base over HTTP. The desktop sends
    /// the user's proof, a program stating a private provider passes on its
    /// capability, and a `biorouter serve` operator on a private provider passes
    /// on theirs. A public serve operator is refused, and that is the same
    /// answer they already get for every private base on that machine.
    ///
    /// DR-15's opt-out is inert here as everywhere: with tiers off, creation is
    /// exactly what it was.
    pub fn mints_knowledge_base(&self) -> Result<(), SessionOutOfReach> {
        refuse_unless_reachable(
            self.enforced,
            TargetTier::Unreadable,
            self.capability(),
            self.proof,
        )
        .map_err(SessionOutOfReach::for_knowledge_base)
    }
}

/// A named knowledge base, reduced to the bit the gate turns on.
///
/// ⚠ **Absent is not public here**, though it is in
/// [`biorouter_mcp::knowledge::tier::is_private`], and both are right for their
/// callers. The tier store reads an absent base as public because "nothing is
/// there to leak" and refusing would stop a public chat creating one. At this
/// gate the question is what a REFUSAL says, and a caller told "private" for one
/// id and "not found" for another has been handed an oracle; so an absent (or
/// malformed) id is answered as a private one. Creating a base is `POST
/// /knowledge/bases`, which names no existing id and is not behind this gate.
fn knowledge_base_tier(root: &Path, kb_id: &str) -> TargetTier {
    use biorouter_mcp::knowledge::{paths, tier};
    if paths::validate_kb_id(kb_id).is_err() || !paths::kb_root(root, kb_id).is_dir() {
        return TargetTier::Unreadable;
    }
    if tier::is_private(root, kb_id) {
        TargetTier::Private
    } else {
        TargetTier::Public
    }
}

/// `GET|POST /knowledge/active` — the gated route whose router does not have an
/// [`AppState`](crate::state::AppState) to resolve a tier with.
///
/// ⚠ **A middleware rather than a line in the handler, and that is a plumbing
/// constraint rather than a design preference.** `knowledge::router` is
/// deliberately state-typed on `Arc<KnowledgeService>` so it can be tested
/// without constructing an `AppState` (and `crates/biorouter-server/tests/
/// knowledge_routes.rs` does exactly that — 46 tests on this branch, measured,
/// not carried over). Widening that state would touch every one of its ~35
/// handlers and every one of those tests, to gate a single route. So the gate is
/// layered onto the nested router instead, where it is the first thing the
/// request meets.
///
/// It buffers the body ONLY for the route it gates. Everything else under
/// `/knowledge` — including 25 MB multipart ingests — passes through untouched.
pub async fn gate_knowledge_active(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::state::AppState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // `nest` strips the prefix before the inner router sees the request, and a
    // layer added to that inner router runs inside it — so the path here is
    // `/active`. The prefixed spelling is accepted too, so that moving this
    // layer to the outer router (or a change in how `nest` rewrites the URI)
    // cannot silently turn a security gate into a no-op. Which spelling arrives
    // is pinned by `the_knowledge_active_gate_is_actually_wired`.
    let path = request.uri().path();
    let active_path = path == "/active" || path == "/knowledge/active";
    if !active_path {
        return next.run(request).await;
    }

    if request.method() == axum::http::Method::GET {
        let session_id = request.uri().query().and_then(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .find(|(key, _)| key == "session_id")
                .map(|(_, value)| value.into_owned())
        });
        if let Some(session_id) = session_id {
            if let Err(refusal) =
                session_reach(state.session_manager(), &session_id, request.headers()).await
            {
                return refusal.into_response();
            }
        }
        return next.run(request).await;
    }
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    // A selection edit is a handful of ids. The cap is not a policy, it is a
    // refusal to buffer something unbounded in a middleware.
    let Ok(bytes) = axum::body::to_bytes(body, 1024 * 1024).await else {
        return (StatusCode::BAD_REQUEST, "request body too large").into_response();
    };
    // A body this does not understand is passed on unchanged: the handler owns
    // the 400, and answering it here would make the gate a second, divergent
    // parser of the same request.
    if let Some(session_id) = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|body| {
            body.get("session_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
    {
        if let Err(refusal) =
            session_reach(state.session_manager(), &session_id, &parts.headers).await
        {
            return refusal.into_response();
        }
    }

    next.run(axum::extract::Request::from_parts(
        parts,
        axum::body::Body::from(bytes),
    ))
    .await
}

/// Every `/knowledge/bases/{id}…` route, behind ONE layer (issue #56, QA
/// 2026-09-10 H2).
///
/// The tool path refused a public caller a private base at
/// `KnowledgeServer::call_tool`; these routes called the service directly and
/// handed the same base's pages, graph, history, location and a `.brkb` of the
/// whole tree to a caller holding nothing but the daemon secret. The plan had
/// left them ungated on the premise that "the Knowledge view is the user, not a
/// model" — true of the renderer, and false of the secret, which a public chat's
/// own shell recovered with `ps eww` (AR-11). The user is now told apart the
/// way every other private surface tells them apart: by the proof the desktop
/// sends, or by the private capability a program states.
///
/// ⚠ **A layer on a sub-router of exactly the routes that name a base, not a
/// list of routes.** `knowledge::base_routes` puts every `{id}` route in one
/// router and `knowledge::router` `route_layer`s this onto it, so the gate reads
/// the `id` the router itself matched — percent-decoded exactly as each handler's
/// `Path` sees it. Reads and writes alike: a caller that may not read a base may
/// not rewrite, restore, merge or delete it either, which is F0's lesson applied
/// here before anyone measured it.
///
/// ⚠ **"and any route added later is gated by construction" was a claim axum does
/// not support, and this doc made it until 2026-09-12.** `Router::route_layer`
/// wraps the routes present when it is *called* and returns a new map; a route
/// registered afterwards is not wrapped, and nothing says so. `base_routes` is
/// therefore now ungated by construction and the layer is applied to its return
/// value at its single call site, which is the shape that makes appending a
/// route safe — see that function, and
/// `every_route_that_names_a_base_is_inside_the_gated_sub_router`, which reads
/// the file rather than trusting either doc.
///
/// It runs before the handler's own extractors, so a refused request never has
/// its body parsed, its model constructed or its base looked up.
pub async fn gate_knowledge_base(
    axum::extract::State(svc): axum::extract::State<Arc<KnowledgeService>>,
    params: axum::extract::RawPathParams,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let kb_id = params
        .iter()
        .find(|(key, _)| *key == "id")
        .map(|(_, value)| value.to_owned());
    // Unreachable through `knowledge::router`, where every route this layer
    // wraps captures `{id}`. Refused rather than waved through, so that a route
    // moved in here without the capture fails closed instead of open.
    let Some(kb_id) = kb_id else {
        return (StatusCode::FORBIDDEN, KNOWLEDGE_BASE_OUT_OF_REACH).into_response();
    };
    let caller = http_caller(request.headers()).await;
    if let Err(refusal) = caller.reach_knowledge_base(svc.root(), &kb_id) {
        return refusal.into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::body_of;

    const PROOFS: [UserActionProof; 3] = [
        UserActionProof::Proven,
        UserActionProof::Unproven,
        UserActionProof::NoKeyInstalled,
    ];

    const CAPABILITIES: [ProviderTier; 2] = [ProviderTier::Public, ProviderTier::Private];

    /// **The inversion, stated as an assertion.** A caller running a model
    /// hosted inside the institution reaches a private chat *because of that*,
    /// with no proof of a human anywhere — including on a daemon that was never
    /// handed a user-action key at all.
    ///
    /// This is the case the gate used to get exactly backwards: `biorouter
    /// session send <id>` running Versa was refused, while the desktop app
    /// running Versa was allowed, on a rule that measured the surface rather
    /// than the capability.
    #[test]
    fn a_private_capability_reaches_a_private_chat_without_any_human_proof() {
        for proof in PROOFS {
            assert!(
                refuse_unless_reachable(true, TargetTier::Private, ProviderTier::Private, proof)
                    .is_ok(),
                "a caller whose capability already covers this chat was refused ({proof:?})"
            );
        }
    }

    /// The whole rule, at every corner, in the direction that matters: a private
    /// target is out of reach of a caller that has NEITHER the capability nor
    /// the user's proof.
    #[test]
    fn a_public_caller_without_the_users_proof_cannot_reach_a_private_chat() {
        assert!(refuse_unless_reachable(
            true,
            TargetTier::Private,
            ProviderTier::Public,
            UserActionProof::Proven
        )
        .is_ok());
        for proof in [UserActionProof::Unproven, UserActionProof::NoKeyInstalled] {
            assert!(
                refuse_unless_reachable(true, TargetTier::Private, ProviderTier::Public, proof)
                    .is_err(),
                "a caller holding nothing but the daemon secret reached a private chat ({proof:?})"
            );
        }
    }

    /// …and a public target is completely unaffected, for every caller.
    ///
    /// This is the half a gate written only for the refusal loses. A barrier
    /// that fired on every chat would break every client that has never sent
    /// either header — which is all of them, for every public chat — and DR-16's
    /// posture is a CONDITION, not a wall in front of the user.
    #[test]
    fn a_public_target_is_reachable_by_every_caller() {
        for capability in CAPABILITIES {
            for proof in PROOFS {
                assert!(
                    refuse_unless_reachable(true, TargetTier::Public, capability, proof).is_ok(),
                    "the gate is a wall in front of the user, not a condition \
                     ({capability:?}, {proof:?})"
                );
            }
        }
    }

    /// Issue #56 Task 58, Step 4.3. A caller cannot distinguish "no such
    /// session" from "private session" **from the response**.
    ///
    /// Byte-for-byte on both the status and the message, because either half
    /// alone is an oracle: a 403 and a 404 enumerate the machine's private chats
    /// just as well as two different sentences do.
    ///
    /// ⚠ It holds for the ADMITTED corners too, and that is not vacuous: a
    /// private-capability caller is admitted for both, so it learns which it was
    /// from what the handler does next — which is fine, because a caller whose
    /// capability already covers private chats is not an enumerator of them.
    /// What must never differ is the REFUSAL, and this asserts every pair.
    #[test]
    fn no_such_session_and_a_private_session_are_the_same_refusal() {
        for capability in CAPABILITIES {
            for proof in PROOFS {
                assert_eq!(
                    refuse_unless_reachable(true, TargetTier::Unreadable, capability, proof),
                    refuse_unless_reachable(true, TargetTier::Private, capability, proof),
                    "the refusal tells a caller whether the chat exists \
                     ({capability:?}, {proof:?})"
                );
            }
        }
    }

    /// Open question 23. A daemon that was handed no user-action key refuses
    /// rather than allows — including the person at the keyboard — and says so
    /// in different words, because reporting "this daemon cannot verify a human"
    /// as "you are not a human" sends them hunting for a permission they can
    /// never obtain.
    ///
    /// ⚠ Still true, and still *reachable*: the inversion gives a keyless daemon
    /// a way through (bring the capability), but a public-capability caller on
    /// one is refused exactly as before. That is the point of open question 23's
    /// separate sentence — a headless `biorouterd agent` has no keyboard to
    /// prove anything from, so telling it to be a human is a dead end.
    #[test]
    fn a_keyless_daemon_refuses_rather_than_allows_and_says_which() {
        let keyless = refuse_unless_reachable(
            true,
            TargetTier::Private,
            ProviderTier::Public,
            UserActionProof::NoKeyInstalled,
        )
        .expect_err("a keyless daemon must refuse");
        let unproven = refuse_unless_reachable(
            true,
            TargetTier::Private,
            ProviderTier::Public,
            UserActionProof::Unproven,
        )
        .expect_err("an unproven caller must be refused");
        assert_eq!(keyless.message, SESSION_REACH_NO_KEY);
        assert_eq!(unproven.message, SESSION_OUT_OF_REACH);
        assert_ne!(
            keyless.message, unproven.message,
            "a daemon with no user-action key must not be told it is the model"
        );
        // …and neither of them is a way to learn the tier: both say the same
        // thing whether the chat is private or absent, which is the assertion
        // above, and both are 403.
        assert_eq!(keyless.status, StatusCode::FORBIDDEN);
        assert_eq!(unproven.status, StatusCode::FORBIDDEN);
    }

    /// DR-15's master opt-out turns the whole gate off — including for an
    /// `Unreadable` target, so a user who opted out still gets their 404 for a
    /// chat that simply is not there rather than a 403 for one that is.
    #[test]
    fn the_master_switch_turns_the_whole_gate_off() {
        for tier in [
            TargetTier::Public,
            TargetTier::Private,
            TargetTier::Unreadable,
        ] {
            for capability in CAPABILITIES {
                for proof in PROOFS {
                    assert!(
                        refuse_unless_reachable(false, tier, capability, proof).is_ok(),
                        "the master opt-out did not reach this gate \
                         ({tier:?}, {capability:?}, {proof:?})"
                    );
                }
            }
        }
    }

    /// The header carries a NAME and this daemon resolves the tier itself, so a
    /// caller cannot mint a capability by asserting one.
    ///
    /// Driven against the real registry (`declared_provider_tier`), not a
    /// fixture: the value that matters is what *this install* publishes, and a
    /// stubbed table would keep passing after a provider's tier changed.
    #[tokio::test]
    async fn the_capability_header_is_resolved_against_the_real_registry() {
        let of = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(
                CALLER_PROVIDER_HEADER,
                axum::http::HeaderValue::from_str(value).unwrap(),
            );
            headers
        };

        assert_eq!(
            caller_capability(&of("versa_azure")).await,
            ProviderTier::Private,
            "an institutional provider must resolve Private, or the CLI can never reach its \
             own private chats"
        );
        // A public model, an unknown name, an empty value and no header at all
        // are all Public — the fail-safe side, and the historical behaviour for
        // every client that has never sent this.
        assert_eq!(
            caller_capability(&of("anthropic")).await,
            ProviderTier::Public
        );
        assert_eq!(
            caller_capability(&of("private")).await,
            ProviderTier::Public,
            "a caller that spells a TIER rather than a provider must not be believed"
        );
        assert_eq!(
            caller_capability(&of("no-such-provider-xyz")).await,
            ProviderTier::Public
        );
        assert_eq!(caller_capability(&of("   ")).await, ProviderTier::Public);
        assert_eq!(
            caller_capability(&HeaderMap::new()).await,
            ProviderTier::Public
        );
    }

    /// **The wiring half.** A capability the caller never sends is a capability
    /// nobody has: the gate would be correct and every CLI command would still
    /// be refused, which is this campaign's signature failure.
    ///
    /// A cross-crate source scan because the CLI carries no HTTP client and
    /// cannot be driven against this router from here — its daemon requests are
    /// hand-built strings. What is asserted is that the one place those strings
    /// are built names this exact header, so a rename on either side turns the
    /// build red rather than silently un-wiring the gate.
    #[test]
    fn the_cli_sends_the_capability_header_this_gate_reads() {
        let cli = include_str!("../../../biorouter-cli/src/commands/session_watch.rs");

        // (1) The CLI spells this exact header, as a declaration rather than
        //     anywhere in its prose — a mention in a comment is not wiring, and
        //     this campaign has already shipped a grep gate that passed on one.
        assert!(
            cli.contains(&format!(
                "const CALLER_PROVIDER_HEADER: &str = \"{CALLER_PROVIDER_HEADER}\""
            )),
            "the CLI no longer declares `{CALLER_PROVIDER_HEADER}`, so every session-addressing \
             command it runs is a public-capability caller and the inversion buys nothing"
        );

        // (2) …and it is emitted from the one place that composes a request's
        //     headers, which BOTH request builders call. `watch` is a GET and
        //     `send` / `attach` are POSTs; a header put on one builder only
        //     leaves the other half of the surface refused, which is exactly the
        //     "guarded at three doors of four" failure.
        assert!(
            cli.contains("out.push_str(CALLER_PROVIDER_HEADER);"),
            "nothing in the CLI writes `{CALLER_PROVIDER_HEADER}` onto a request"
        );
        for builder in [
            "pub(crate) fn build_get_request",
            "pub(crate) fn build_post_request",
        ] {
            let body = body_of(cli, builder);
            assert!(
                body.contains("auth.headers()"),
                "{builder} composes its headers itself instead of through the one place that \
                 emits `{CALLER_PROVIDER_HEADER}`"
            );
        }
    }

    /// §14.4's content rule, and the marker rule beside it.
    ///
    /// The refusal must name the boundary and nothing about the chat, and it
    /// must not claim to be one of the other two user-proof refusals: their
    /// toasts say *switch this chat's model* and *branch it from the chat
    /// window*, and both would send the user somewhere that cannot help.
    #[test]
    fn the_refusal_names_the_boundary_and_nothing_about_the_chat() {
        for message in [SESSION_OUT_OF_REACH, SESSION_REACH_NO_KEY] {
            assert!(
                !message.contains(biorouter::privacy::refusal::USER_ACTION_REFUSAL_MARKER),
                "this refusal is claiming to be the model picker's: {message}"
            );
            assert!(
                !message.contains(crate::routes::session::COPY_OF_PRIVATE_REFUSAL_MARKER),
                "this refusal is claiming to be the branch refusal's: {message}"
            );
        }
        // The retry is foreclosed in the arm a model actually reaches. The
        // keyless arm is the human's, and tells them where to go instead.
        assert!(SESSION_OUT_OF_REACH.contains("Do not retry"));
    }

    /// **A signpost for the operator, never a recipe for the caller.**
    ///
    /// This replaced a version of the constant that named
    /// [`CALLER_PROVIDER_HEADER`], gave an example value that resolves Private
    /// on a real install, and told the reader to send it.
    ///
    /// ⚠ The defect that version was written against is real, and its half of
    /// this test keeps its force: an agent read the refusal in full, took
    /// "reachable from a session running a private model" for a state it had no
    /// way to enter, and reported the absence of a channel that has always
    /// shipped. `contains("private model")` would have passed throughout. But
    /// the cure went past the wall it was decorating. The reader of this 403 is
    /// a MODEL far more often than a person, and a refused caller told which
    /// header to add reads itself straight out of "do not retry as you are";
    /// `routes/status.rs` states the rule for the arm beside this one in as
    /// many words, that a refusal handing the reader a working command is the
    /// opposite move on the adjacent arm.
    ///
    /// So both halves are asserted together and neither is optional: the
    /// refusal names the page that documents the channel (discoverable, so no
    /// reader concludes there is none), and it carries no header name and no
    /// example value (not actionable, so no reader takes it for the retry). The
    /// mechanism is asserted to be ON that page, which is what stops a future
    /// "help the caller" edit from moving it back here and still passing.
    #[test]
    fn the_refusal_signposts_the_operator_page_without_handing_over_a_recipe() {
        // Read at compile time, so renaming or moving the page breaks this
        // build rather than leaving the refusal pointing at nothing.
        let doc = include_str!("../../../../docs/deployment/programmatic-session-access.md");
        let title = doc
            .lines()
            .next()
            .and_then(|first| first.strip_prefix("# "))
            .expect("the signposted page must open with an H1 title");

        assert!(
            SESSION_OUT_OF_REACH.contains(title),
            "the refusal leaves a program believing there is no programmatic channel at all, \
             which is the report that prompted this sentence: it must name `{title}`"
        );
        assert!(
            doc.contains(CALLER_PROVIDER_HEADER),
            "the page the refusal signposts no longer carries `{CALLER_PROVIDER_HEADER}`, so \
             the signpost points at nothing an operator can act on"
        );

        // …and the refusal hands the refused caller nothing to retry with.
        assert!(
            !SESSION_OUT_OF_REACH.contains(CALLER_PROVIDER_HEADER),
            "the refusal names the header, which to a refused model reads as the retry it was \
             just told not to attempt"
        );
        assert!(
            !SESSION_OUT_OF_REACH.contains("versa_azure"),
            "the refusal carries a provider value that resolves Private on a real install, and \
             with the header name that is a working recipe rather than a refusal"
        );

        // The stop clause keeps its unqualified force, and the signpost sits
        // BEFORE it, so the last thing a model reads is the stop.
        assert!(
            SESSION_OUT_OF_REACH.contains(
                "Do not retry as you are; the same call will be refused again, and no setting, \
                 hook or permission mode changes it."
            ),
            "the unqualified stop clause is gone, and foreclosing the retry is why this arm is \
             worded the way it is"
        );
        assert!(
            SESSION_OUT_OF_REACH
                .trim_end()
                .ends_with("stop and ask the user to open it for you."),
            "the signpost was moved after the stop, so the last thing a model reads is a place \
             to go rather than an instruction to stop"
        );
    }

    /// **The keyless arm names the header even less than the other one does.**
    ///
    /// [`SESSION_REACH_NO_KEY`] is a different state — this daemon was handed no
    /// user-action key at all (`just run-server`, a hand-run `biorouterd
    /// agent`) — and the header does not help there for the caller that arm is
    /// written for: [`refuse_unless_reachable`] admits on capability *before* it
    /// ever looks at the proof, so a caller the header would have helped never
    /// reaches this string at all.
    ///
    /// ⚠ Neither arm may carry the mechanism now (see
    /// [`the_refusal_signposts_the_operator_page_without_handing_over_a_recipe`]),
    /// and this is the worse of the two places a "be helpful to the script" edit
    /// could land: the reader here has no capability to declare, so a header
    /// offered to them is purely an invitation to guess a provider name.
    #[test]
    fn the_keyless_arm_does_not_borrow_the_capability_hint() {
        assert!(
            !SESSION_REACH_NO_KEY.contains(CALLER_PROVIDER_HEADER),
            "the keyless refusal is answering a question its reader did not ask"
        );
        // The two arms stay distinguishable, which is open question 23's whole
        // point and which a copy-paste of the hint would erode.
        assert_ne!(SESSION_OUT_OF_REACH, SESSION_REACH_NO_KEY);
    }

    /// **The oracle property, restated against the sentence that was added.**
    ///
    /// [`no_such_session_and_a_private_session_are_the_same_refusal`] already
    /// asserts the two answers are equal at every (capability, proof) pair. This
    /// asserts the other half of why that holds and will keep holding: the
    /// signpost is CONSTANT text about the caller's own situation, so there is
    /// no input from which a future edit could make it vary with the target. A
    /// sentence like "that page would have got you into this chat" would satisfy
    /// the equality test only until someone made it conditional.
    #[test]
    fn the_capability_hint_is_constant_and_never_derived_from_the_target() {
        for capability in CAPABILITIES {
            for proof in PROOFS {
                let private = refuse_unless_reachable(true, TargetTier::Private, capability, proof);
                let absent =
                    refuse_unless_reachable(true, TargetTier::Unreadable, capability, proof);
                assert_eq!(private, absent);
                if let Err(refusal) = private {
                    // Whatever it says, it is one of the two constants — the
                    // `&'static str` in `SessionOutOfReach` is what makes that
                    // checkable, and it is why the type does not carry a String.
                    assert!(
                        refusal.message == SESSION_OUT_OF_REACH
                            || refusal.message == SESSION_REACH_NO_KEY,
                        "a refusal composed per-request can vary with the target"
                    );
                }
            }
        }
        // The refusal names no part of a session. `SessionSummary` carries four
        // fields, and the guard that stood here tested for their snake_case
        // NAMES: `session_id`, `working_dir`, `privacy_tier`, `"name"`. A leak
        // arrives as a VALUE and never as the field's name, so those tests
        // could not fire on any refusal this gate is able to produce. Two were
        // dead on arrival in a way worth naming: `session_id` could not have
        // caught an id being spliced in, which is the one leak that matters
        // most here, and `"name"` had been quoted precisely so it would stop
        // tripping on the constant's own "provider name" — a guard bent to fit
        // the text it guards.
        //
        // A guard that cannot fail is not a guard. So each one below is a
        // predicate shaped like the leak it names, and each is run against a
        // positive control that really does leak that field BEFORE it is run
        // against the constants. If a control stops firing, the guard has been
        // blunted and this test says so instead of passing.
        struct LeakGuard {
            field: &'static str,
            leaks: fn(&str) -> bool,
            control: &'static str,
        }
        let guards = [
            LeakGuard {
                // A session id is `YYYYMMDD_N`, so an id spliced into a refusal
                // shows up as ASCII digits. Neither constant carries one.
                field: "id",
                leaks: |message| message.chars().any(|c| c.is_ascii_digit()),
                control: "That chat, 20260904_1, is private.",
            },
            LeakGuard {
                // An LLM-written title is arbitrary text, so the only shape it
                // reliably takes is that it has to be quoted to be readable.
                field: "name",
                leaks: |message| message.contains('"') || message.contains('\u{201c}'),
                control: "The chat \"Patient cohort review\" is private.",
            },
            LeakGuard {
                // A working directory arrives as a path.
                field: "working_dir",
                leaks: |message| message.contains('/') || message.contains('\\'),
                control: "That chat runs in /Users/someone/patients.",
            },
            LeakGuard {
                // The tier leaks by being ASSERTED. This refusal may call a
                // chat private only while offering "or there is no chat with
                // that id" in the same breath, which is the whole reason the
                // two answers are one answer.
                field: "privacy_tier",
                leaks: |message| {
                    message.contains("chat is private")
                        && !message.contains("or there is no chat with that id")
                },
                control: "That chat is private.",
            },
        ];
        for guard in guards {
            assert!(
                (guard.leaks)(guard.control),
                "the `{}` guard does not fire on a refusal that really does leak that field, so \
                 it cannot fail and is guarding nothing",
                guard.field
            );
            for message in [SESSION_OUT_OF_REACH, SESSION_REACH_NO_KEY] {
                assert!(
                    !(guard.leaks)(message),
                    "this refusal now discloses the target's `{}`, which is a fact about the \
                     chat: {message}",
                    guard.field
                );
            }
        }
    }

    /// Issue #56 Task 58, Step 3: **resolve the tier before doing anything
    /// else.** The half [`refuse_unless_reachable`]'s own tests cannot see.
    ///
    /// Ordering, not merely presence: a gate placed after the turn lock, after
    /// the agent fetch or after the transcript read hands an unproven caller the
    /// side channel the refusal is worded to withhold. Each row names the FIRST
    /// thing its handler does that touches the session, and the assertion is
    /// that the gate is earlier in the body than that.
    ///
    /// A source scan because none of these handlers can be driven cheaply from a
    /// unit test — `AppState::new()` opens the developer's REAL session
    /// database. Every route on the list is also driven over HTTP by
    /// [`super::bypass_tests`] except `POST /agent/add_extension` (whose admitted
    /// arm mints a real agent), `GET|POST /knowledge/active` (a middleware, which
    /// a body scan cannot see and
    /// [`super::bypass_tests::the_knowledge_active_gate_is_actually_wired`]
    /// drives instead) and the two SD-11 turn-control routes, which are gated
    /// only on a daemon with no user-action key and so are driven by their own
    /// keyless binary, `tests/turn_control_no_user_key.rs`; this is what holds
    /// the ORDERING, which no status code can show.
    ///
    /// ⚠ **Every route added to the gated list gets a row here.** `/export`,
    /// `/events` and `/diagnostics` each shipped a gate that this table did not
    /// name, and a gate nothing names can be deleted without a red build — which
    /// is the failure the whole census exists for.
    #[test]
    fn every_gated_route_resolves_the_tier_before_it_touches_the_session() {
        let session_rs = include_str!("session.rs");
        let reply_rs = include_str!("reply.rs");
        let agent_rs = include_str!("agent.rs");
        let events_rs = include_str!("session_events.rs");
        let status_rs = include_str!("status.rs");
        let workflow_rs = include_str!("workflow.rs");
        let skills_rs = include_str!("skills.rs");
        let knowledge_rs = include_str!("knowledge.rs");
        for (src, func, gate_call, first_touch, what) in [
            (
                reply_rs,
                "pub async fn reply",
                "session_reach(",
                "try_begin_turn_idempotent_with_continuation(",
                "the turn lock, whose 409 says whether this chat is busy",
            ),
            (
                reply_rs,
                "pub async fn recover_continuation",
                "session_reach(",
                "recover_continuation_for_owner(",
                "the pending continuation ownership state",
            ),
            // SD-11: the turn-control routes. Their gate is a helper, because on
            // a daemon that holds a key the answer is the proof and on one that
            // holds none it is `authorize_agent_control` — the ordering is the
            // same either way, and it is what is asserted here.
            (
                reply_rs,
                "pub async fn recover_continuation",
                "authorize_turn_control(",
                "recover_continuation_for_owner(",
                "the pending continuation ownership state",
            ),
            (
                reply_rs,
                "pub async fn cancel_turn(",
                "authorize_turn_control(",
                "cancel_turn_bounded(",
                "the turn registry, whose answer says whether this chat is busy",
            ),
            (
                reply_rs,
                "pub async fn abandon_continuation_lease",
                "authorize_turn_control(",
                "state.abandon_continuation_lease(",
                "the continuation registry",
            ),
            (
                session_rs,
                "async fn get_session(",
                "session_reach(",
                ".get_session(&session_id,",
                "the session read, with or without conversation history",
            ),
            (
                session_rs,
                "async fn export_session(",
                "session_reach(",
                "export_session(&session_id)",
                "the transcript read, under another name",
            ),
            (
                events_rs,
                "pub async fn observe_session_events(",
                "session_reach(",
                "session_events::subscribe(",
                "the bus subscription, which would outlive the refusal, and the \
                 full-conversation snapshot frame right behind it",
            ),
            (
                status_rs,
                "async fn diagnostics(",
                "session_reach(",
                "generate_diagnostics(",
                "the diagnostics bundle, whose `session.json` IS the transcript",
            ),
            (
                agent_rs,
                "async fn agent_add_extension",
                "authorize_agent_control(",
                "get_agent(",
                "the agent fetch, which creates one if absent",
            ),
            (
                agent_rs,
                "async fn update_working_dir",
                "session_reach(",
                "try_begin_turn_idempotent(",
                "the turn lock, whose 409 says whether this chat is busy",
            ),
            // ── QA 2026-09-10: F0, M2, and the sweep F0 asked for ──
            (
                session_rs,
                "async fn delete_session(",
                "session_reach(",
                "cancel_turn(",
                "the turn cancel and the parked-card release, each an effect on the chat, \
                 ahead of the delete itself",
            ),
            (
                session_rs,
                "async fn update_session_name(",
                "session_reach(",
                ".user_provided_name(",
                "the rename",
            ),
            (
                session_rs,
                "async fn update_session_user_workflow_values(",
                "session_reach(",
                "apply_user_workflow_values(",
                "the row write and the workflow re-applied to the live agent",
            ),
            (
                session_rs,
                "async fn edit_message(",
                "session_reach(",
                "edit_in_place(",
                "the in-place truncation",
            ),
            (
                session_rs,
                "async fn get_session_extensions(",
                "session_reach(",
                "session_extensions(",
                "the row read that names the chat's extensions",
            ),
            (
                session_rs,
                "async fn get_session_usage(",
                "session_reach(",
                "get_session_model_usage(",
                "the usage read, whose 200/404 said whether the id existed",
            ),
            (
                agent_rs,
                "async fn get_tools(",
                "session_reach(",
                "permission_editor_tools(",
                "the agent fetch, which mints an agent for the chat",
            ),
            (
                agent_rs,
                "async fn get_callable_tool_count(",
                "session_reach(",
                // The delegate, which is where the agent fetch lives: the gate is
                // in the wrapper so the refusal keeps its own plain-text body
                // rather than this route's JSON envelope, exactly as `get_tools`
                // does beside it.
                "model_visible_tool_count(",
                "the agent fetch, which mints an agent for the chat",
            ),
            (
                workflow_rs,
                "async fn create_workflow(",
                "session_reach(",
                "workflow_from_session(",
                "the transcript load and the model that summarises it",
            ),
            (
                skills_rs,
                "pub async fn set_session_skills(",
                "session_reach(",
                "session_skills::apply(",
                "the per-chat skill write",
            ),
            (
                knowledge_rs,
                "pub async fn ingest_conversation(",
                "session_reach(",
                ".get_session(sid, true)",
                "the transcript load",
            ),
        ] {
            let handler = body_of(src, func);
            let gate = handler.find(gate_call).unwrap_or_else(|| {
                panic!("{func} does not consult its session-reach gate (`{gate_call}`)")
            });
            let touch = handler
                .find(first_touch)
                .unwrap_or_else(|| panic!("`{first_touch}` is no longer in {func}"));
            assert!(
                gate < touch,
                "{func} reaches {what} before it resolves the target session's tier"
            );
        }

        // The OVER-READ controls, so the scan is provably not vacuous: a handler
        // in each file whose body does not name the gate must come back without
        // it, or `body_of` is over-reading past a function end and every
        // assertion above is passing on someone else's body.
        //
        // ⚠ **"Does not name the gate" is not "is not gated", and two of these
        // rows are the difference.** `update_agent_provider` and
        // `agent_remove_extension` ARE gated — through
        // `agent::authorize_agent_control`, which calls `session_reach` and then
        // reads the row — and measured live against a private session each
        // answers 403 without the capability header and proceeds with it. They
        // are controls for the EXTRACTOR, not exemptions from the gate, and the
        // comment here said otherwise until 2026-09-04. `interrupt` is the
        // genuinely ungated control: it asks for the user's proof instead of
        // reach — on a keyless daemon too, which is the one way it differs from
        // the Stop beside it (SD-11a, `reply::authorize_steer`).
        // `get_session_extensions` was this file's other one until QA's
        // 2026-09-10 sweep gated it, so it is a control no longer;
        // `get_session_insights` and `running_sessions` replace it — machine-wide
        // aggregates that name no chat — on the two sides of this file's gated
        // handlers. Two more reply.rs controls sit on either side of the rows
        // that file contributes.
        //
        // BOTH sides in `agent.rs`: `agent_remove_extension` sits after the two
        // gated handlers' neighbourhood and `update_agent_provider` before it,
        // and a control on one side only passes against an extractor that
        // over-reads towards the other.
        for (src, control) in [
            (reply_rs, "fn attach_names_a_missing_turn("),
            (reply_rs, "pub async fn interrupt"),
            (reply_rs, "pub fn routes("),
            (session_rs, "async fn get_session_insights("),
            (session_rs, "async fn running_sessions("),
            (agent_rs, "async fn agent_remove_extension"),
            (agent_rs, "async fn update_agent_provider"),
            // BOTH sides in the two files this sweep added, for the same reason:
            // `system_info` sits before `diagnostics` and `routes` after it,
            // `bus_lag_resync_frame` before `observe_session_events` and `routes`
            // after it.
            (events_rs, "pub(crate) async fn bus_lag_resync_frame"),
            (events_rs, "pub fn routes("),
            (status_rs, "async fn system_info("),
            (status_rs, "pub fn routes("),
        ] {
            // Both spellings the rows above use, so a control is a control for
            // every row it could be over-reading into.
            let body = body_of(src, control);
            for gate in ["session_reach(", "authorize_turn_control("] {
                assert!(
                    !body.contains(gate),
                    "the body scan is over-reading: {control} is not on the gated list and \
                     reported the gate (`{gate}`)"
                );
            }
        }
    }

    // ─── QA 2026-09-10: the caller, the knowledge-base target, the words ───

    fn caller(
        stated: ProviderTier,
        served_operator: ProviderTier,
        proof: UserActionProof,
    ) -> HttpCaller {
        HttpCaller {
            enforced: true,
            stated,
            served_operator,
            proof,
        }
    }

    /// A listing admits exactly what the singular gate admits, at every corner
    /// — so it can never tell a caller more than per-id probing does, and never
    /// less than the desktop app and a private program are owed.
    #[test]
    fn a_listing_is_the_singular_gate_applied_row_by_row() {
        for stated in CAPABILITIES {
            for proof in PROOFS {
                let who = caller(stated, ProviderTier::Public, proof);
                for classification in [
                    SessionClassification::Public,
                    SessionClassification::Private,
                ] {
                    assert_eq!(
                        who.lists_session(classification),
                        refuse_unless_reachable(
                            true,
                            TargetTier::from(classification),
                            stated,
                            proof
                        )
                        .is_ok(),
                        "{stated:?} {proof:?} {classification:?}"
                    );
                }
            }
        }
        // The two shapes QA cares about, spelled out.
        let secret_only = caller(
            ProviderTier::Public,
            ProviderTier::Public,
            UserActionProof::Unproven,
        );
        assert!(secret_only.lists_session(SessionClassification::Public));
        assert!(!secret_only.lists_session(SessionClassification::Private));
        let desktop = caller(
            ProviderTier::Public,
            ProviderTier::Public,
            UserActionProof::Proven,
        );
        assert!(desktop.lists_session(SessionClassification::Private));
    }

    /// A serve daemon's own interface keeps the reach its operator's provider
    /// implies on the surfaces this type serves — and a serve daemon on a public
    /// provider gives it none, which is the same answer as a secret-only caller.
    #[test]
    fn the_served_operator_standing_is_a_capability_and_only_that() {
        let private_operator = caller(
            ProviderTier::Public,
            ProviderTier::Private,
            UserActionProof::NoKeyInstalled,
        );
        assert!(private_operator.lists_session(SessionClassification::Private));
        let public_operator = caller(
            ProviderTier::Public,
            ProviderTier::Public,
            UserActionProof::NoKeyInstalled,
        );
        assert!(!public_operator.lists_session(SessionClassification::Private));
        assert!(public_operator.lists_session(SessionClassification::Public));
    }

    /// ⚠ **The transcript gate never reads the served-operator standing**, and
    /// this is the assertion that keeps it so: feeding it there would admit a
    /// serve daemon's browser to private transcripts it has always been refused
    /// — the one direction this change may not move. `session_reach` resolves
    /// its capability from the header alone; the served input is read by
    /// `http_caller`, which `session_reach` does not call.
    #[test]
    fn the_transcript_gate_does_not_read_the_served_operator_standing() {
        let session_reach_body = crate::routes::body_of(
            include_str!("session_reach.rs"),
            "pub async fn session_reach(",
        );
        assert!(
            !session_reach_body.contains("served_operator")
                && !session_reach_body.contains("http_caller("),
            "the transcript gate now reads the serve operator's standing, which would admit a \
             browser to private transcripts it was always refused"
        );
        assert!(session_reach_body.contains("caller_capability(headers)"));
    }

    /// A knowledge base's target, at each of its corners: a private base; a
    /// public one; one that does not exist; and an id that could not name one.
    /// The last two are answered as the first, to a caller that proves nothing.
    #[test]
    fn a_knowledge_base_target_answers_absent_and_malformed_as_private() {
        let root = tempfile::tempdir().unwrap();
        let svc =
            biorouter_mcp::knowledge::service::KnowledgeService::new(root.path().to_path_buf());
        svc.create_base("notes", "Notes", None).unwrap();
        svc.create_base("omop", "OMOP", None).unwrap();
        biorouter_mcp::knowledge::tier::raise_unlocked(root.path(), "omop", true).unwrap();

        assert_eq!(
            knowledge_base_tier(root.path(), "notes"),
            TargetTier::Public
        );
        assert_eq!(
            knowledge_base_tier(root.path(), "omop"),
            TargetTier::Private
        );
        assert_eq!(
            knowledge_base_tier(root.path(), "no-such-base"),
            TargetTier::Unreadable
        );
        for malformed in ["../sessions", "Bad--Id", "", "a/b"] {
            assert_eq!(
                knowledge_base_tier(root.path(), malformed),
                TargetTier::Unreadable,
                "{malformed:?}"
            );
        }

        let secret_only = caller(
            ProviderTier::Public,
            ProviderTier::Public,
            UserActionProof::Unproven,
        );
        let private_refusal = secret_only
            .reach_knowledge_base(root.path(), "omop")
            .unwrap_err();
        assert_eq!(private_refusal.message, KNOWLEDGE_BASE_OUT_OF_REACH);
        assert_eq!(private_refusal.status, StatusCode::FORBIDDEN);
        for other in ["no-such-base", "../sessions"] {
            assert_eq!(
                secret_only.reach_knowledge_base(root.path(), other),
                Err(private_refusal),
                "{other:?} was answered differently from a private base"
            );
        }
        assert!(secret_only
            .reach_knowledge_base(root.path(), "notes")
            .is_ok());

        // The person at the keyboard reaches all of them; the handler then tells
        // them the truth about the absent and malformed ones.
        let desktop = caller(
            ProviderTier::Public,
            ProviderTier::Public,
            UserActionProof::Proven,
        );
        for id in ["omop", "notes", "no-such-base", "../sessions"] {
            assert!(
                desktop.reach_knowledge_base(root.path(), id).is_ok(),
                "{id}"
            );
        }

        // A keyless daemon says so in the knowledge base's words.
        let keyless = caller(
            ProviderTier::Public,
            ProviderTier::Public,
            UserActionProof::NoKeyInstalled,
        );
        assert_eq!(
            keyless
                .reach_knowledge_base(root.path(), "omop")
                .unwrap_err()
                .message,
            KNOWLEDGE_BASE_REACH_NO_KEY
        );

        // DR-15: with tiers off nothing is refused — not even the absent id, so a
        // user who opted out still gets their 404 from the handler.
        let off = HttpCaller {
            enforced: false,
            ..secret_only
        };
        for id in ["omop", "no-such-base"] {
            assert!(off.reach_knowledge_base(root.path(), id).is_ok(), "{id}");
        }
    }

    /// The knowledge-base refusals obey every rule the chat ones do, checked by
    /// the same predicates: fixed text, no digit, quote or path, the stop
    /// clause last, the operator page named without the header, and neither
    /// renderer marker.
    #[test]
    fn the_knowledge_base_refusals_keep_every_rule_the_chat_refusals_keep() {
        for message in [KNOWLEDGE_BASE_OUT_OF_REACH, KNOWLEDGE_BASE_REACH_NO_KEY] {
            assert!(!message.chars().any(|c| c.is_ascii_digit()), "{message}");
            assert!(
                !message.contains('"') && !message.contains('\u{201c}'),
                "{message}"
            );
            assert!(
                !message.contains('/') && !message.contains('\\'),
                "{message}"
            );
            assert!(!message.contains(CALLER_PROVIDER_HEADER), "{message}");
            assert!(!message.contains("versa_azure"), "{message}");
            assert!(
                !message.contains(biorouter::privacy::refusal::USER_ACTION_REFUSAL_MARKER),
                "{message}"
            );
            assert!(
                !message.contains(crate::routes::session::COPY_OF_PRIVATE_REFUSAL_MARKER),
                "{message}"
            );
            // It may call a base private only while offering "no such base".
            assert!(
                !message.contains("base is private")
                    || message.contains("or there is no knowledge base with that id"),
                "{message}"
            );
        }
        let doc = include_str!("../../../../docs/deployment/programmatic-session-access.md");
        let title = doc
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("# "))
            .unwrap();
        assert!(KNOWLEDGE_BASE_OUT_OF_REACH.contains(title));
        assert!(KNOWLEDGE_BASE_OUT_OF_REACH.contains(
            "Do not retry as you are; the same call will be refused again, and no setting, hook \
             or permission mode changes it."
        ));
        assert!(KNOWLEDGE_BASE_OUT_OF_REACH
            .trim_end()
            .ends_with("stop and ask the user to open it for you."));
        // "Started without a user-action key" is what the keyless knowledge-base
        // tier binary keys on, and what a serve operator's browser reads.
        assert!(KNOWLEDGE_BASE_REACH_NO_KEY.contains("started without a user-action key"));
        assert_ne!(KNOWLEDGE_BASE_OUT_OF_REACH, SESSION_OUT_OF_REACH);
        assert_ne!(KNOWLEDGE_BASE_REACH_NO_KEY, SESSION_REACH_NO_KEY);
    }

    /// The knowledge route's gate is a middleware, so the scan above cannot see
    /// it — but the wiring can still be lost in a refactor of `configure`, and a
    /// layer that is never applied is a security control that silently does
    /// nothing.
    ///
    /// `the_knowledge_active_gate_is_actually_wired` is what proves it FIRES;
    /// this is the cheap tripwire that survives a move of that test.
    #[test]
    fn the_knowledge_router_carries_the_reach_gate() {
        let mod_rs = include_str!("mod.rs");
        let configure = body_of(mod_rs, "pub fn configure");
        assert!(
            configure.contains("session_reach::gate_knowledge_active"),
            "the knowledge router no longer carries the session-reach gate"
        );
    }

    // The router-shape scan that used to live here — "every `{id}` route sits
    // behind `gate_knowledge_base`" — moved to
    // `every_route_that_names_a_base_is_inside_the_gated_sub_router`, beside
    // `base_addressing_routes`, when the adversarial review of 2026-09-12 found
    // that the claim it checked was weaker than the doc it was checking. It now
    // also asserts that every gated route is actually PROBED, which needs that
    // list in scope; and it was tied to a `.route_layer(` inside `pub fn
    // router(`, which is exactly the shape that had to change.
}

#[cfg(test)]
mod bypass_tests {
    use super::*;
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };
    use crate::state::AppState;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use biorouter::conversation::message::Message;
    use biorouter::model::ModelConfig;
    use biorouter::session::SessionType;
    use serial_test::serial;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// A string that appears in the seeded chat and nowhere else, so "the
    /// transcript came back" is an assertion rather than an impression.
    ///
    /// ⚠ **Unmistakably a fixture, and deliberately not shaped like a record.**
    /// These tests seed into the developer's REAL session database —
    /// `AppState::new()` opens it — so a row that ever escapes [`SeededChat`]'s
    /// cleanup lands in their own sidebar. A marker that read like a patient
    /// identifier would then be a privacy incident invented by the test suite of
    /// the privacy feature.
    const MARKER_IN_THE_TRANSCRIPT: &str = "task58-transcript-marker-not-real-data";

    async fn get_session_with(
        state: Arc<AppState>,
        session_id: &str,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        let app = crate::routes::session::routes(state);
        let mut builder = Request::builder()
            .method("GET")
            .uri(format!("/sessions/{session_id}"));
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let res = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn session_metadata_read_omits_history_without_weakening_private_reach() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed_private_chat(&state, "Metadata-only synthetic fixture").await;
        let mut session = state
            .session_manager()
            .get_session(private.id(), false)
            .await
            .unwrap();
        session.extension_data.set_extension_state(
            "todo",
            "v1",
            serde_json::json!({
                "items": [{"id":"1", "text":"Inspect synthetic summary", "status":"pending"}]
            }),
        );
        state
            .session_manager()
            .update(private.id())
            .extension_data(session.extension_data)
            .apply()
            .await
            .unwrap();

        for query in ["", "?metadata_only=false", "?metadata_only=true"] {
            let path = format!("{}{query}", private.id());
            let (status, body) =
                get_session_with(state.clone(), &path, Some(TEST_USER_ACTION_KEY)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                body.contains(MARKER_IN_THE_TRANSCRIPT),
                query != "?metadata_only=true"
            );
            let json: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(json["id"], private.id());
            assert_eq!(
                json["extension_data"]["todo.v1"]["items"][0]["text"],
                "Inspect synthetic summary"
            );
        }
        let path = format!("{}?metadata_only=true", private.id());
        let (status, body) = get_session_with(state.clone(), &path, None).await;
        let (missing_status, missing_body) =
            get_session_with(state.clone(), "29990101_99999?metadata_only=true", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(status, missing_status);
        assert_eq!(body, missing_body);
        assert!(!body.contains("Inspect synthetic summary"));
        let (status, _) = get_session_with(
            state,
            &format!("{}?metadata_only=invalid", private.id()),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    /// `GET /sessions/{id}/export` — the same transcript as
    /// [`get_session_with`], `to_string_pretty`'d.
    async fn get_export_with(
        state: Arc<AppState>,
        session_id: &str,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        let app = crate::routes::session::routes(state);
        let mut builder = Request::builder()
            .method("GET")
            .uri(format!("/sessions/{session_id}/export"));
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let res = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// `GET /sessions/{id}/events` — the same transcript as an opening SSE
    /// frame, then a live tail.
    ///
    /// ⚠ **The body must be read as a PREFIX, never with `to_bytes`.** An
    /// admitted observer's stream stays open for the life of the session, so
    /// draining it to completion hangs the test suite forever rather than
    /// failing it. This reads until the snapshot frame has arrived (or the whole
    /// finite body of a refusal has), then drops the stream — which is also what
    /// tears down the spawned observer task.
    async fn get_events_with(
        state: Arc<AppState>,
        session_id: &str,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        use futures::StreamExt;
        let app = crate::routes::session_events::routes(state);
        let mut builder = Request::builder()
            .method("GET")
            .uri(format!("/sessions/{session_id}/events"));
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let res = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let mut stream = res.into_body().into_data_stream();
        let mut collected: Vec<u8> = Vec::new();
        // The timeout is the backstop for the case this helper exists to avoid:
        // a stream that neither sends the snapshot nor ends. Its expiry is not
        // an assertion — whatever arrived is returned and the caller judges it.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            while let Some(Ok(chunk)) = stream.next().await {
                collected.extend_from_slice(&chunk);
                if collected.len() >= 1_000_000
                    || String::from_utf8_lossy(&collected).contains("UpdateConversation")
                {
                    break;
                }
            }
        })
        .await;
        (status, String::from_utf8_lossy(&collected).into_owned())
    }

    /// `GET /diagnostics/{id}` — the support bundle, whose `session.json` is
    /// `SessionManager::export_session` verbatim.
    ///
    /// Returns the raw bytes because the payload is a **Deflated** zip: a
    /// `contains(MARKER)` over the compressed body would pass whether or not the
    /// transcript is in there, which is exactly the test that claims a guarantee
    /// it does not have. [`session_json_in`] decompresses instead.
    async fn get_diagnostics_with(
        state: Arc<AppState>,
        session_id: &str,
        user_action: Option<&str>,
    ) -> (StatusCode, Vec<u8>) {
        let app = crate::routes::status::routes(state);
        let mut builder = Request::builder()
            .method("GET")
            .uri(format!("/diagnostics/{session_id}"));
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let res = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, bytes.to_vec())
    }

    /// `session.json` out of a diagnostics zip, decompressed.
    fn session_json_in(zip_bytes: &[u8]) -> String {
        use std::io::Read;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
            .expect("the diagnostics response is not a zip");
        let mut entry = archive
            .by_name("session.json")
            .expect("the diagnostics zip carries no session.json");
        let mut text = String::new();
        entry.read_to_string(&mut text).unwrap();
        text
    }

    /// `POST /reply`.
    ///
    /// ⚠ The caller MUST already hold this session's turn lock. A `/reply` that
    /// gets past every check spawns a real agent turn against the developer's
    /// real configuration — which on this machine means real provider
    /// credentials in the Keychain. Holding the lock makes "the gate let it
    /// through" observable as a 409 from the turn lock, with nothing spawned and
    /// no streaming body to drain.
    async fn post_reply_with(
        state: Arc<AppState>,
        session_id: &str,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        let app = crate::routes::reply::routes(state);
        let mut builder = Request::builder()
            .method("POST")
            .uri("/reply")
            .header("content-type", "application/json");
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let body = serde_json::json!({
            "session_id": session_id,
            "user_message": Message::user().with_text("summarise this chat"),
        });
        let res = app
            .oneshot(
                builder
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// `POST /agent/update_working_dir`.
    ///
    /// ⚠ Like [`post_reply_with`], the caller MUST already hold this session's
    /// turn lock: the first thing this handler does after the gate is claim that
    /// lock, and a request that gets past both repoints a real chat at a
    /// directory and RESTARTS its agent there. Holding the lock makes "the gate
    /// let it through" observable as a 409, with nothing moved.
    async fn post_update_working_dir(
        state: Arc<AppState>,
        session_id: &str,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        post_json(
            crate::routes::agent::routes(state),
            "/agent/update_working_dir",
            serde_json::json!({
                "session_id": session_id,
                "working_dir": "/tmp/task58_session_reach",
            }),
            user_action,
        )
        .await
    }

    /// `POST /agent/add_extension`.
    ///
    /// The command names nothing that exists, so an accepted request has no MCP
    /// server to spawn — but it would still reach `get_agent`, which MINTS an
    /// agent from the developer's own configuration. That is why only the
    /// refusing arm of this route is driven, and it is refused before that call.
    async fn post_add_extension(
        state: Arc<AppState>,
        session_id: &str,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        post_json(
            crate::routes::agent::routes(state),
            "/agent/add_extension",
            serde_json::json!({
                "session_id": session_id,
                "config": {
                    "type": "stdio",
                    "name": "task58-probe",
                    "description": "",
                    "cmd": "/nonexistent/task58",
                    "args": [],
                    "timeout": null,
                },
            }),
            user_action,
        )
        .await
    }

    /// One JSON POST at `router`, with the proof-of-user header when supplied.
    async fn post_json(
        router: axum::Router,
        uri: &str,
        body: serde_json::Value,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let res = router
            .oneshot(
                builder
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn post_knowledge_active(
        state: Arc<AppState>,
        body: serde_json::Value,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        let app = crate::routes::configure(state, "task-58-secret".to_string());
        let mut builder = Request::builder()
            .method("POST")
            .uri("/knowledge/active")
            .header("content-type", "application/json");
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let res = app
            .oneshot(
                builder
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn get_knowledge_active(
        state: Arc<AppState>,
        session_id: &str,
        user_action: Option<&str>,
    ) -> (StatusCode, String) {
        let app = crate::routes::configure(state, "task-58-secret".to_string());
        let encoded: String = url::form_urlencoded::byte_serialize(session_id.as_bytes()).collect();
        let mut builder = Request::builder()
            .method("GET")
            .uri(format!("/knowledge/active?session_id={encoded}"));
        if let Some(key) = user_action {
            builder = builder.header("X-User-Action", key);
        }
        let res = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// A chat seeded by this module, which deletes itself when the test's scope
    /// ends — **including on a panic**, which a `delete_session` at the end of
    /// the test body cannot do.
    ///
    /// ⚠ The store here is the developer's REAL session database, so a row that
    /// outlives a failing assertion is a chat in their own sidebar, forever, with
    /// no obvious provenance. `block_in_place` is what lets an async delete run
    /// from `Drop`, and it is only legal on a multi-threaded runtime — so
    /// `#[tokio::test(flavor = "multi_thread")]` on every test below is a
    /// requirement of this type, not a habit.
    struct SeededChat {
        state: Arc<AppState>,
        id: String,
    }

    impl SeededChat {
        fn id(&self) -> &str {
            &self.id
        }
    }

    impl Drop for SeededChat {
        fn drop(&mut self) {
            let state = self.state.clone();
            let id = std::mem::take(&mut self.id);
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    // Reported, never panicked: on the unwind path a second panic
                    // aborts the process and takes the real failure's message with
                    // it, which would turn a legible assertion into a bare abort.
                    if let Err(e) = state.session_manager().delete_session(&id).await {
                        eprintln!("task 58: could not clean up the seeded chat {id}: {e}");
                    }
                })
            });
        }
    }

    /// One chat at `tier` with a marker message in it. A private one is raised
    /// the way a real one gets there — by binding a private provider — rather
    /// than by writing the column, so what these tests refuse is the same state
    /// a user's own chat reaches. The returned guard deletes it.
    async fn seed_chat(
        state: &Arc<AppState>,
        label: &str,
        tier: SessionClassification,
    ) -> SeededChat {
        let manager = state.session_manager();
        let session = manager
            .create_session(
                PathBuf::from("/tmp/task58_session_reach"),
                label.to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        manager
            .add_message(
                &session.id,
                &Message::user().with_text(MARKER_IN_THE_TRANSCRIPT),
            )
            .await
            .unwrap();
        if tier == SessionClassification::Private {
            manager
                .update(&session.id)
                .provider_name("versa_azure")
                .model_config(ModelConfig::new("gpt-4o").unwrap())
                .raise_privacy(SessionClassification::Private, "turn:versa_azure")
                .apply()
                .await
                .unwrap();
        }
        SeededChat {
            state: state.clone(),
            id: session.id,
        }
    }

    async fn seed_private_chat(state: &Arc<AppState>, label: &str) -> SeededChat {
        seed_chat(state, label, SessionClassification::Private).await
    }

    /// Issue #56 Task 58 / #47. **The bypass itself, as a named regression
    /// test.** This is the test that would have caught the hole, and it is the
    /// one that must never be deleted.
    ///
    /// Hold nothing but the daemon secret — which AR-11 measured the agent can
    /// recover — name a private session, and try the two things that dominate
    /// every other session-addressing route: read its transcript, and run a turn
    /// in it. A caller who can run a turn in a session can already do anything
    /// that session can do.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn holding_the_secret_you_cannot_read_a_private_transcript_or_run_a_turn_in_it() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed_private_chat(&state, "Task 58 private (test fixture)").await;
        let private_id = private.id();

        // 1. READ. A caller holding only the daemon secret must not get the
        //    transcript.
        let (status, body) = get_session_with(state.clone(), private_id, None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a caller holding only the daemon secret read a private transcript"
        );
        assert!(
            !body.contains(MARKER_IN_THE_TRANSCRIPT),
            "the refusal carried the private conversation in its body"
        );

        // 2. …and the person at the keyboard still can.
        let (status, body) =
            get_session_with(state.clone(), private_id, Some(TEST_USER_ACTION_KEY)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the user cannot read their own chat"
        );
        assert!(
            body.contains(MARKER_IN_THE_TRANSCRIPT),
            "the user's own read did not return the conversation"
        );

        // 3. RUN A TURN. `/reply` dominates every other route on the list.
        //    The turn lock is held for the whole exchange so that a request the
        //    gate lets through stops at a 409 instead of spawning a real turn
        //    against real provider credentials.
        let turn_guard = state
            .try_begin_turn_idempotent(private_id, tokio_util::sync::CancellationToken::new(), None)
            .expect("no turn is running in a session created a moment ago");

        let (status, _) = post_reply_with(state.clone(), private_id, None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a caller holding only the daemon secret ran an agent turn, with tools, in a \
             private chat"
        );

        let (status, _) =
            post_reply_with(state.clone(), private_id, Some(TEST_USER_ACTION_KEY)).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "the user's own turn was refused; 409 here is the turn lock this test holds, \
             i.e. the request got past the barrier as it should"
        );

        drop(turn_guard);
    }

    /// Issue #56 Task 58, Step 4.1: **a public target is unaffected.**
    ///
    /// The half a gate written only for its refusal never drives, and the
    /// expensive one to get wrong. [`super::target_tier`] resolves a real row out
    /// of the real store, so if it ever mis-read a valid public session — a
    /// column renamed, a default that flips, a transient store error swallowed
    /// into `Unreadable` — then EVERY header-less caller would be refused on
    /// EVERY public chat, and every other assertion in this module would still
    /// pass. [`super::tests::a_public_target_is_reachable_by_every_caller`] pins
    /// the decision; this pins that a real public row on this machine arrives at
    /// that decision as `Public`.
    ///
    /// Each route is driven to a status only its own body can produce, so
    /// "not a 403" is nowhere the assertion:
    ///
    /// * `GET /sessions/{id}` returns the transcript, and the marker is in it;
    /// * `POST /reply` and `POST /agent/update_working_dir` each stop at the turn
    ///   lock this test holds — the first thing each does after the gate — so
    ///   their 409 is a status the gate cannot produce. Holding it is also what
    ///   keeps a request that got through from spawning a real turn against real
    ///   provider credentials, or repointing a chat and restarting its agent.
    ///
    /// ⚠ `POST /agent/add_extension` is deliberately NOT driven on this arm, and
    /// that is a cost of the route rather than an omission: passing its gate
    /// means `get_agent` mints a real agent from the developer's own
    /// configuration and the handler then attaches an extension to it. Its
    /// refusing arm is driven by
    /// [`the_other_two_gated_routes_refuse_a_private_chat_over_http`], and there
    /// is nothing between its gate and its handler that the other three do not
    /// also have — it is the same `session_reach(` call, with `Public` returning
    /// `Ok(())` unconditionally.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_public_chat_is_reachable_over_http_by_a_caller_that_proves_nothing() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let public = seed_chat(
            &state,
            "Task 58 public (test fixture)",
            SessionClassification::Public,
        )
        .await;

        // READ. The whole transcript, to a caller carrying nothing but the
        // daemon secret — which is exactly the request the gate must not touch.
        // All four spellings of that read, because a gate added to one of them
        // with the wrong sense would refuse every header-less client on every
        // public chat, and nothing else here would notice.
        let (status, body) = get_session_with(state.clone(), public.id(), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the gate is a wall in front of the user: it refused a PUBLIC chat ({body})"
        );
        assert!(
            body.contains(MARKER_IN_THE_TRANSCRIPT),
            "a public read came back without the conversation: {body}"
        );

        let (status, body) = get_export_with(state.clone(), public.id(), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "/sessions/{{id}}/export refused an unproven caller on a PUBLIC chat ({body})"
        );
        assert!(
            body.contains(MARKER_IN_THE_TRANSCRIPT),
            "a public export came back without the conversation: {body}"
        );

        let (status, body) = get_events_with(state.clone(), public.id(), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "/sessions/{{id}}/events refused an unproven caller on a PUBLIC chat ({body})"
        );
        assert!(
            body.contains(MARKER_IN_THE_TRANSCRIPT),
            "a public observer opened without the conversation: {body}"
        );

        let (status, bytes) = get_diagnostics_with(state.clone(), public.id(), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "/diagnostics/{{id}} refused an unproven caller on a PUBLIC chat"
        );
        assert!(
            session_json_in(&bytes).contains(MARKER_IN_THE_TRANSCRIPT),
            "a public diagnostics bundle came back without the conversation"
        );

        let turn_guard = state
            .try_begin_turn_idempotent(
                public.id(),
                tokio_util::sync::CancellationToken::new(),
                None,
            )
            .expect("no turn is running in a session created a moment ago");

        let (status, body) = post_reply_with(state.clone(), public.id(), None).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "/reply refused an unproven caller on a PUBLIC chat; 409 here is the turn lock \
             this test holds, i.e. the request reached the handler as it should ({body})"
        );

        let (status, body) = post_update_working_dir(state.clone(), public.id(), None).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "/agent/update_working_dir refused an unproven caller on a PUBLIC chat; 409 here \
             is the turn lock this test holds ({body})"
        );

        drop(turn_guard);
    }

    /// The two routes on the gated list that
    /// [`holding_the_secret_you_cannot_read_a_private_transcript_or_run_a_turn_in_it`]
    /// does not drive, on their refusing arm, over HTTP.
    ///
    /// Neither is additional capability — `/reply` dominates both, and it is
    /// already covered. They are here because "the gate is on this route" is
    /// otherwise only [`super::tests::every_gated_route_resolves_the_tier_before_it_touches_the_session`],
    /// and a source scan cannot see a `session_reach(` call whose result was
    /// discarded.
    ///
    /// The refusal is asserted by its full text, not by its status, because these
    /// two return it through `ErrorResponse` — a JSON envelope — while
    /// `GET /sessions/{id}` returns it as plain text. One sentence has to survive
    /// both shapes, or a client cannot recognise the boundary it hit.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_other_two_gated_routes_refuse_a_private_chat_over_http() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed_private_chat(&state, "Task 58 routes (test fixture)").await;

        // Held for the same reason as everywhere else here: if the gate ever
        // stopped firing, this request would repoint a real chat and restart its
        // agent rather than merely returning the wrong status.
        let turn_guard = state
            .try_begin_turn_idempotent(
                private.id(),
                tokio_util::sync::CancellationToken::new(),
                None,
            )
            .expect("no turn is running in a session created a moment ago");

        let (status, body) = post_update_working_dir(state.clone(), private.id(), None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a caller holding only the daemon secret repointed a private chat at a directory \
             of its choosing and restarted its agent there ({body})"
        );
        assert!(
            body.contains(SESSION_OUT_OF_REACH),
            "the refusal did not survive the JSON envelope: {body}"
        );

        drop(turn_guard);

        let (status, body) = post_add_extension(state.clone(), private.id(), None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a caller holding only the daemon secret attached tools to a private chat ({body})"
        );
        assert!(
            body.contains(SESSION_OUT_OF_REACH),
            "the refusal did not survive the JSON envelope: {body}"
        );
    }

    /// `GET /sessions/{id}/export` — the same transcript as
    /// [`holding_the_secret_you_cannot_read_a_private_transcript_or_run_a_turn_in_it`]
    /// reads, `to_string_pretty`'d and reachable from the generated TS client as
    /// `exportSession`.
    ///
    /// ⚠ **This route's gate shipped with no test of any kind** — it was on
    /// neither the ordering scan next door nor this module's HTTP list — so
    /// deleting the two lines in `export_session` turned nothing red. That is
    /// the same shape as the hole the gate closes, one level up.
    ///
    /// ⚠ **Both arms.** A refusal-only test passes equally well against a route
    /// that 403s everything, so the proving arm asserts the marker really comes
    /// back — which is what makes the refusing arm a refusal *of the transcript*
    /// rather than of the route.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_export_sibling_refuses_a_private_transcript_over_http() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed_private_chat(&state, "Task 58 export (test fixture)").await;

        let (status, body) = get_export_with(state.clone(), private.id(), None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a caller holding only the daemon secret exported a private transcript"
        );
        assert!(
            !body.contains(MARKER_IN_THE_TRANSCRIPT),
            "the refusal carried the private conversation in its body: {body}"
        );

        let (status, body) =
            get_export_with(state.clone(), private.id(), Some(TEST_USER_ACTION_KEY)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the user cannot export their own chat: {body}"
        );
        assert!(
            body.contains(MARKER_IN_THE_TRANSCRIPT),
            "the user's own export did not return the conversation: {body}"
        );
    }

    /// `GET /sessions/{id}/events` — the same transcript as an opening
    /// `UpdateConversation` frame, and then a live tail of everything said next.
    ///
    /// ⚠ **The one partial pin this route had was not about privacy.**
    /// `session_events::tests::observing_an_unknown_session_is_refused` asserts
    /// 403 for an id that does not exist, which the gate happens to produce via
    /// `Unreadable` — so deleting the gate does turn that test red, but it says
    /// nothing about a private chat's transcript, which is the property the gate
    /// exists for. A future reader "restoring" the honest 404 there would reopen
    /// this hole and see a green suite.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_events_stream_refuses_a_private_transcript_over_http() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed_private_chat(&state, "Task 58 events (test fixture)").await;

        let (status, body) = get_events_with(state.clone(), private.id(), None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a caller holding only the daemon secret opened a live observer on a private \
             chat and was handed its whole conversation as the first frame"
        );
        assert!(
            !body.contains(MARKER_IN_THE_TRANSCRIPT),
            "the refusal carried the private conversation in its body: {body}"
        );

        let (status, body) =
            get_events_with(state.clone(), private.id(), Some(TEST_USER_ACTION_KEY)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the user cannot watch their own chat: {body}"
        );
        assert!(
            body.contains(MARKER_IN_THE_TRANSCRIPT),
            "the user's own observer opened without the conversation: {body}"
        );
    }

    /// `GET /diagnostics/{id}` — the support bundle, whose `session.json` is
    /// `SessionManager::export_session` verbatim and whose `logs/` entries carry
    /// this session's prompts.
    ///
    /// ⚠ **The marker is asserted through the DECOMPRESSOR.** The zip is
    /// Deflated, so `contains(MARKER)` over the response bytes answers "no"
    /// whether or not the transcript is in there — a test written that way would
    /// pass with the gate deleted, which is worse than no test at all.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_diagnostics_bundle_refuses_a_private_transcript_over_http() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed_private_chat(&state, "Task 58 diagnostics (test fixture)").await;

        let (status, bytes) = get_diagnostics_with(state.clone(), private.id(), None).await;
        let refusal = String::from_utf8_lossy(&bytes).into_owned();
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a caller holding only the daemon secret downloaded a private chat's transcript \
             as a diagnostics zip"
        );
        assert!(
            !refusal.starts_with("PK"),
            "the refusal is a zip, so the bundle was generated and returned anyway"
        );
        assert!(
            !refusal.contains(MARKER_IN_THE_TRANSCRIPT),
            "the refusal carried the private conversation in its body: {refusal}"
        );

        let (status, bytes) =
            get_diagnostics_with(state.clone(), private.id(), Some(TEST_USER_ACTION_KEY)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the user cannot download diagnostics for their own chat"
        );
        assert!(
            session_json_in(&bytes).contains(MARKER_IN_THE_TRANSCRIPT),
            "the user's own bundle came back without the conversation, so the refusal above \
             is not evidence that the gate withholds a transcript"
        );
    }

    /// Issue #56 Task 58, Step 4.3, over HTTP: the refusal is not an oracle.
    ///
    /// [`super::tests::no_such_session_and_a_private_session_are_the_same_refusal`]
    /// pins the decision; this pins that the ROUTE does not add a distinguisher
    /// of its own on the way out — a different status, a different body, a
    /// validation answer that only a real id could produce.
    ///
    /// And the other direction, which is the part that keeps this from being
    /// satisfied by a route that refuses everything: a caller who DOES prove it
    /// is the user is told the truth, 200 for the one that exists and 404 for
    /// the one that does not.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn an_unproven_caller_cannot_tell_a_private_chat_from_one_that_does_not_exist() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let chat = seed_private_chat(&state, "Task 58 oracle (test fixture)").await;
        // Syntactically valid (so `is_valid_session_id` cannot answer for the
        // store) and not a session on this machine.
        let absent_id = "task58-no-such-session-0000";

        let private = get_session_with(state.clone(), chat.id(), None).await;
        let absent = get_session_with(state.clone(), absent_id, None).await;
        assert_eq!(
            private, absent,
            "an unproven caller can tell a private chat from one that does not exist"
        );
        assert_eq!(private.0, StatusCode::FORBIDDEN);

        // The user is told the difference, which is what makes the equality
        // above a property of the *refusal* rather than of a route that answers
        // 403 to everything.
        let (status, _) =
            get_session_with(state.clone(), chat.id(), Some(TEST_USER_ACTION_KEY)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) =
            get_session_with(state.clone(), absent_id, Some(TEST_USER_ACTION_KEY)).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "the person at the keyboard is entitled to know the chat is not there"
        );
    }

    /// The knowledge gate is a middleware, and a middleware that is layered onto
    /// the wrong router — or matched against the wrong spelling of the path,
    /// which is exactly what `nest` makes ambiguous — is a security control that
    /// silently does nothing. So it is exercised through the REAL router tree
    /// (`routes::configure`), not through a hand-built one.
    ///
    /// ⚠ **Every request here must be one the handler REJECTS, and that is a
    /// constraint of the route rather than a style.** `set_active` writes into
    /// the developer's real knowledge directory — a selection this test invented,
    /// silently replacing a list the person at this machine curated. An earlier
    /// version of this test sent `{"hidden_kbs": []}` at machine scope and did
    /// exactly that: `~/.config/biorouter/knowledge/.hidden-kbs` was rewritten to
    /// `[]` on every run, so anyone who had hidden a base and then ran the server
    /// suite silently got it back.
    ///
    /// So the two pass-through arms name a primary that does not exist.
    /// `apply_selection_unlocked` decides everything before it commits anything,
    /// and an unknown primary fails in the decide half — which makes the 400 a
    /// stronger signal than the old `assert_ne!(403)` as well as a harmless one:
    /// only `set_selection` echoes the kb id back, so nothing but the real
    /// handler could have produced this body.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_knowledge_active_gate_is_actually_wired() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed_private_chat(&state, "Task 58 knowledge (test fixture)").await;
        let public = seed_chat(
            &state,
            "Task 58 knowledge public (test fixture)",
            SessionClassification::Public,
        )
        .await;
        // A base id that exists on no machine, so the handler refuses to pin it
        // and returns before its commit half.
        const NO_SUCH_KB: &str = "task58-no-such-kb";

        let (status, body) = post_knowledge_active(
            state.clone(),
            serde_json::json!({ "session_id": private.id(), "hidden_kbs": [] }),
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a caller holding only the daemon secret repointed a private chat's knowledge \
             bases: {body}"
        );

        let (status, body) = get_knowledge_active(state.clone(), private.id(), None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "an unproven caller read a private chat's knowledge selection: {body}"
        );
        let (status, body) =
            get_knowledge_active(state.clone(), private.id(), Some(TEST_USER_ACTION_KEY)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the user-action proof should reach the private selection: {body}"
        );

        // Step 4.1's other half, for this route: a PUBLIC chat is untouched by
        // the layer and reaches the handler, which answers on its own terms.
        //
        // ⚠ Since QA's 2026-09-10 sweep the handler's own terms, for an
        // unproven caller naming a base that does not exist, are the
        // KNOWLEDGE-BASE refusal — the one it gives for a private base, so that
        // pinning is not a way to ask which ids exist. That body is still one
        // only the handler can produce (the layer's is `SESSION_OUT_OF_REACH`),
        // so it proves the layer let the request through as well as the old
        // 400 did. The person at the keyboard still gets the 400 that names
        // the id, from `set_selection`.
        for session in [Some(public.id()), None] {
            let mut body = serde_json::json!({ "primary_kb": NO_SUCH_KB });
            if let Some(id) = session {
                body["session_id"] = serde_json::json!(id);
            }
            let (status, answer) = post_knowledge_active(state.clone(), body.clone(), None).await;
            assert_eq!(
                (status, answer.as_str()),
                (StatusCode::FORBIDDEN, KNOWLEDGE_BASE_OUT_OF_REACH),
                "{session:?}: the layer refused an unproven caller the session gate should have \
                 let through, or the handler told it whether the base exists"
            );
            let (status, answer) =
                post_knowledge_active(state.clone(), body, Some(TEST_USER_ACTION_KEY)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{session:?}: {answer}");
            assert!(
                answer.contains(NO_SUCH_KB),
                "this 400 did not come from `set_selection`: only it echoes the kb id: {answer}"
            );
        }
    }

    // ─── QA 2026-09-10 (H2 / M1 / M2 / F0): the rest of the chat surface ───
    //
    // Every test below drives the REAL router tree with the headers each
    // caller really sends. "Secret only" is the caller QA measured: a public
    // chat's shell that recovered the daemon secret with `ps eww`. The daemon
    // cannot tell it from any other client, so it is a public model.

    /// The proof-of-user header, exactly as the desktop app sends it.
    const PROOF: (&str, &str) = ("X-User-Action", TEST_USER_ACTION_KEY);

    /// A caller stating that it runs under an institution-hosted model — the
    /// CLI's shape, and the capability half of the gate.
    const PRIVATE_CAPABILITY: (&str, &str) = (CALLER_PROVIDER_HEADER, "versa_azure");

    /// One request through `routes::configure`, the tree `commands::agent`
    /// serves, so a gate wired onto the wrong router is measured rather than
    /// assumed. `check_token` is layered outside `configure`, so every request
    /// here already holds the daemon secret — which is the whole premise.
    async fn call(
        state: Arc<AppState>,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, String) {
        let app = crate::routes::configure(state, "qa-h2-f0-sweep-secret".to_string());
        let mut builder = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let body = match body {
            Some(json) => {
                builder = builder.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&json).unwrap())
            }
            None => Body::empty(),
        };
        let res = app.oneshot(builder.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Every route that names ONE chat and answered a secret-only caller when
    /// QA measured it, as `(method, uri, body)` for a given id.
    ///
    /// ⚠ **Destructive last.** Before this change the first row deleted the
    /// chat outright, which would turn every later row into a probe of an
    /// absent id and hide what each of them did to a real one.
    fn chat_addressing_routes(id: &str) -> Vec<(&'static str, String, Option<serde_json::Value>)> {
        vec![
            ("GET", format!("/sessions/{id}/extensions"), None),
            ("GET", format!("/sessions/{id}/usage"), None),
            ("GET", format!("/agent/tools?session_id={id}"), None),
            (
                "GET",
                format!("/agent/callable_tool_count?session_id={id}"),
                None,
            ),
            (
                "POST",
                "/workflows/create".to_string(),
                Some(serde_json::json!({ "session_id": id })),
            ),
            (
                "PUT",
                format!("/sessions/{id}/name"),
                Some(serde_json::json!({ "name": "renamed by an unproven caller" })),
            ),
            (
                "PUT",
                format!("/sessions/{id}/user_workflow_values"),
                Some(serde_json::json!({ "userWorkflowValues": {} })),
            ),
            (
                "POST",
                "/skills/session".to_string(),
                Some(serde_json::json!({ "sessionId": id, "add": ["qa-h2-probe-skill"] })),
            ),
            (
                "POST",
                format!("/sessions/{id}/edit_message"),
                Some(serde_json::json!({ "timestamp": 0, "editType": "edit" })),
            ),
            ("DELETE", format!("/sessions/{id}"), None),
        ]
    }

    /// **F0, and the sweep it asked for.** QA held nothing but the daemon
    /// secret and was refused a private chat's transcript — then deleted the
    /// same chat, four of four. Every route that names a chat now asks the
    /// read's own gate, so each one answers an unproven caller exactly as
    /// `GET /sessions/{id}` does: the same status, the same bytes, and the same
    /// answer for a chat that does not exist.
    ///
    /// Mismatches are collected rather than asserted one at a time, so a
    /// regression reports every door it reopened instead of the first.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn every_route_that_names_a_private_chat_refuses_it_exactly_as_the_read_does() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed_private_chat(&state, "QA F0 sweep (test fixture)").await;
        // Syntactically a session id, and not a row on this machine.
        let absent = "29990101_424242";

        let before = state
            .session_manager()
            .get_session(private.id(), true)
            .await
            .unwrap();

        let (read_status, read_body) = get_session_with(state.clone(), private.id(), None).await;
        assert_eq!(read_status, StatusCode::FORBIDDEN);
        assert_eq!(
            read_body, SESSION_OUT_OF_REACH,
            "the read path's refusal is what every route below is compared against"
        );

        let mut leaks = Vec::new();
        for target in [private.id(), absent] {
            for (method, uri, body) in chat_addressing_routes(target) {
                let (status, got) = call(state.clone(), method, &uri, body, &[]).await;
                if status != read_status || got != read_body {
                    leaks.push(format!("{method} {uri} -> {status}: {got:.160}"));
                }
            }
        }
        assert!(
            leaks.is_empty(),
            "a caller holding nothing but the daemon secret was answered differently from \
             `GET /sessions/{{id}}` by {} route(s):\n  {}",
            leaks.len(),
            leaks.join("\n  ")
        );

        // …and nothing moved: the chat is still there, under its own name, with
        // its transcript and its extension state.
        let after = state
            .session_manager()
            .get_session(private.id(), true)
            .await
            .expect("an unproven caller removed a private chat");
        assert_eq!(
            after.name, before.name,
            "an unproven caller renamed a private chat"
        );
        assert_eq!(
            serde_json::to_value(&after.conversation).unwrap(),
            serde_json::to_value(&before.conversation).unwrap(),
            "an unproven caller changed a private chat's transcript"
        );
        assert_eq!(
            serde_json::to_value(&after.extension_data).unwrap(),
            serde_json::to_value(&before.extension_data).unwrap(),
            "an unproven caller wrote into a private chat's per-chat state"
        );
    }

    /// The other half, which "refuse the unproven caller" alone would satisfy
    /// by refusing everyone: the person at the keyboard (the proof) and a
    /// program running under a private model (the capability) both still get
    /// through. Each route is driven to a status only its own body can produce,
    /// chosen so nothing expensive or irreversible runs: the turn lock (409), a
    /// queued child (424), a chat with no workflow (404) or no transcript (an
    /// `error` field).
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_person_at_the_keyboard_and_a_private_caller_still_reach_each_one() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();

        for credential in [PROOF, PRIVATE_CAPABILITY] {
            let private = seed_private_chat(&state, "QA F0 admitted arm (test fixture)").await;
            let id = private.id();
            let headers = [credential];

            let (status, body) = call(
                state.clone(),
                "GET",
                &format!("/sessions/{id}/extensions"),
                None,
                &headers,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{credential:?} extensions: {body}");
            let (status, body) = call(
                state.clone(),
                "GET",
                &format!("/sessions/{id}/usage"),
                None,
                &headers,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{credential:?} usage: {body}");

            let (status, body) = call(
                state.clone(),
                "PUT",
                &format!("/sessions/{id}/name"),
                Some(serde_json::json!({ "name": "renamed by the user" })),
                &headers,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{credential:?} rename: {body}");

            // No workflow was ever attached, so the handler's own 404 is the
            // proof it ran.
            let (status, body) = call(
                state.clone(),
                "PUT",
                &format!("/sessions/{id}/user_workflow_values"),
                Some(serde_json::json!({ "userWorkflowValues": {} })),
                &headers,
            )
            .await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{credential:?} workflow values: {body}"
            );

            let (status, body) = call(
                state.clone(),
                "POST",
                "/skills/session",
                Some(serde_json::json!({ "sessionId": id, "add": ["qa-h2-probe-skill"] })),
                &headers,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{credential:?} skills: {body}");

            // Held so an admitted in-place edit stops at the lock instead of
            // truncating the chat.
            let turn_guard = state
                .try_begin_turn_idempotent(id, tokio_util::sync::CancellationToken::new(), None)
                .expect("no turn is running in a session created a moment ago");
            let (status, body) = call(
                state.clone(),
                "POST",
                &format!("/sessions/{id}/edit_message"),
                Some(serde_json::json!({ "timestamp": 0, "editType": "edit" })),
                &headers,
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT, "{credential:?} edit: {body}");
            drop(turn_guard);

            // DELETE last: admitted, it removes the row, which is the point.
            let (status, body) = call(
                state.clone(),
                "DELETE",
                &format!("/sessions/{id}"),
                None,
                &headers,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{credential:?} delete: {body}");
            assert!(
                state
                    .session_manager()
                    .get_session(id, false)
                    .await
                    .is_err(),
                "an admitted delete left the row behind"
            );
        }

        // `/workflows/create` on a chat whose provider cannot be built here
        // (no credentials in the sandbox) answers with a 200 whose `error`
        // field is the handler's own — measured before this change as
        // "Failed to create workflow: Provider not set". Nothing reaches a
        // model, and the gate cannot produce that body.
        let empty = seed_private_chat_without_messages(&state, "QA F0 empty (test fixture)").await;
        let (status, body) = call(
            state.clone(),
            "POST",
            "/workflows/create",
            Some(serde_json::json!({ "session_id": empty.id() })),
            &[PROOF],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "workflows/create: {body}");
        let answer: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(
            answer["error"].is_string() && !body.contains(SESSION_OUT_OF_REACH),
            "workflows/create did not reach its own handler: {body}"
        );
        // The admitted request built an agent for the chat. Dropped here: this
        // database recycles `YYYYMMDD_N` ids once a row is deleted, and a
        // cached agent left under this id would be found by the next test's
        // fresh chat and read as something that test's request created.
        let _ = state.agent_manager.remove_session(empty.id()).await;

        // The two tool routes, on a QUEUED child: admitted, each reaches the
        // not-ready answer (424) rather than minting an agent for the chat.
        let child = seed_queued_private_child(&state).await;
        for uri in [
            format!("/agent/tools?session_id={}", child.chat.id()),
            format!("/agent/callable_tool_count?session_id={}", child.chat.id()),
        ] {
            let (status, body) = call(state.clone(), "GET", &uri, None, &[PROOF]).await;
            assert_eq!(status, StatusCode::FAILED_DEPENDENCY, "{uri}: {body}");
        }
    }

    /// **M2, as QA measured it.** `GET /agent/tools?session_id=<private>`
    /// handed a secret-only caller the private chat's tool names while
    /// `add_extension` on the same chat refused. Asserted on the queued-child
    /// shape so the admitted arm is observable without an agent being built.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_private_chats_tool_surface_is_refused_as_its_transcript_is() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let child = seed_queued_private_child(&state).await;
        // Measure THIS request: an agent cached under a recycled id by an
        // earlier test is not one this request created.
        let _ = state.agent_manager.remove_session(child.chat.id()).await;
        assert!(state.peek_agent(child.chat.id()).await.is_none());
        for uri in [
            format!("/agent/tools?session_id={}", child.chat.id()),
            format!("/agent/callable_tool_count?session_id={}", child.chat.id()),
        ] {
            let (status, body) = call(state.clone(), "GET", &uri, None, &[]).await;
            assert_eq!(
                (status, body.as_str()),
                (StatusCode::FORBIDDEN, SESSION_OUT_OF_REACH),
                "{uri} answered a secret-only caller"
            );
        }
        assert!(
            state.peek_agent(child.chat.id()).await.is_none(),
            "a refused caller still materialised an agent for the chat"
        );
    }

    /// A public chat is untouched on every one of these routes, for a caller
    /// that proves nothing — the gate is a condition on the target, never a
    /// wall in front of the client.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_public_chat_is_untouched_by_the_sweep() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let public = seed_chat(
            &state,
            "QA F0 public (test fixture)",
            SessionClassification::Public,
        )
        .await;
        let id = public.id();
        for (method, uri, expected) in [
            ("GET", format!("/sessions/{id}/extensions"), StatusCode::OK),
            ("GET", format!("/sessions/{id}/usage"), StatusCode::OK),
            ("DELETE", format!("/sessions/{id}"), StatusCode::OK),
        ] {
            let (status, body) = call(state.clone(), method, &uri, None, &[]).await;
            assert_eq!(status, expected, "{method} {uri}: {body}");
        }
    }

    /// **M1.** `GET /sessions` returned every row — 5,543 of them, 792 private,
    /// each with its title, directory and privacy reason — to a caller the
    /// singular read refuses. A listing now shows a caller exactly the rows the
    /// singular gate would admit it to, so it cannot learn from the list what
    /// per-id probing is worded not to tell it.
    ///
    /// Answered here is `session_reach.rs`'s open question: **filter, not
    /// refuse.** A refused list would break every client for the public chats
    /// the gate is deliberately inert on.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn every_listing_shows_a_caller_only_the_chats_it_could_open() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed_private_chat(&state, "QA M1 private (test fixture)").await;
        let public = seed_chat(
            &state,
            "QA M1 public (test fixture)",
            SessionClassification::Public,
        )
        .await;

        for (headers, sees_private) in [
            (&[][..], false),
            (&[PROOF][..], true),
            (&[PRIVATE_CAPABILITY][..], true),
        ] {
            for uri in ["/sessions", "/sessions?include_subagents=true"] {
                let (status, body) = call(state.clone(), "GET", uri, None, headers).await;
                assert_eq!(status, StatusCode::OK, "{uri}: {body}");
                assert!(
                    body.contains(public.id()),
                    "{uri} {headers:?} lost a public chat"
                );
                assert_eq!(
                    body.contains(private.id()),
                    sees_private,
                    "{uri} {headers:?}: private chat listed = {}",
                    body.contains(private.id())
                );
                if !sees_private {
                    assert!(
                        !body.contains("QA M1 private"),
                        "{uri} leaked the private chat's title without its id"
                    );
                }
            }
            let ids = sidebar_ids(&state, 50, headers).await;
            assert!(ids.contains(&public.id().to_string()));
            assert_eq!(ids.contains(&private.id().to_string()), sees_private);
        }
    }

    /// Paging a FILTERED sidebar must still walk every visible row exactly
    /// once: a filter applied after `LIMIT` would hand back short, ragged pages
    /// and let `has_more` count the rows it hid.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_filtered_sidebar_pages_through_every_visible_chat_exactly_once() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let mut seeded = Vec::new();
        for i in 0..4 {
            seeded.push(
                seed_chat(
                    &state,
                    &format!("QA M1 paging public {i} (test fixture)"),
                    SessionClassification::Public,
                )
                .await,
            );
            seeded.push(
                seed_private_chat(&state, &format!("QA M1 paging private {i} (test fixture)"))
                    .await,
            );
        }
        let rows = sidebar_ids(&state, 3, &[]).await;
        let mut deduped = rows.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(
            rows.len(),
            deduped.len(),
            "a filtered page repeated a row: {rows:?}"
        );
        for chat in &seeded {
            let tier = state
                .session_manager()
                .get_session(chat.id(), false)
                .await
                .unwrap()
                .privacy_tier;
            assert_eq!(
                rows.contains(&chat.id().to_string()),
                tier == SessionClassification::Public,
                "{} ({tier:?}) was {} the unproven sidebar",
                chat.id(),
                if rows.contains(&chat.id().to_string()) {
                    "in"
                } else {
                    "missing from"
                }
            );
        }
    }

    /// `GET /schedule/{id}/sessions` lists a schedule's runs by name and
    /// directory — the same rows, through a different door.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_schedules_run_list_is_filtered_like_every_other_listing() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        const SCHEDULE: &str = "qa-m1-probe-schedule";
        let private = seed_private_chat(&state, "QA M1 scheduled private (test fixture)").await;
        let public = seed_chat(
            &state,
            "QA M1 scheduled public (test fixture)",
            SessionClassification::Public,
        )
        .await;
        for chat in [&private, &public] {
            state
                .session_manager()
                .update(chat.id())
                .schedule_id(Some(SCHEDULE.to_string()))
                .apply()
                .await
                .unwrap();
        }
        for (headers, sees_private) in [(&[][..], false), (&[PROOF][..], true)] {
            let (status, body) = call(
                state.clone(),
                "GET",
                &format!("/schedule/{SCHEDULE}/sessions?limit=50"),
                None,
                headers,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(body.contains(public.id()));
            assert_eq!(
                body.contains(private.id()),
                sees_private,
                "{headers:?}: {body}"
            );
        }
    }

    /// The knowledge-base layer, through the tree the daemon SERVES: nested
    /// under `/knowledge` by `configure`, beneath `gate_knowledge_active`. Every
    /// other test of it drives `knowledge::router` bare, and a layer that reads
    /// its `{id}` from the matched route is exactly the kind of thing `nest` can
    /// change underneath it.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_knowledge_base_gate_fires_under_the_served_router_tree() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let kb = format!("qa-h2-served-{}", std::process::id());
        let root = state.knowledge_service.root().to_path_buf();
        state
            .knowledge_service
            .create_base(&kb, "QA H2", None)
            .unwrap();
        let page = root.join(&kb).join("knowledge").join("x.md");
        std::fs::create_dir_all(page.parent().unwrap()).unwrap();
        std::fs::write(&page, "# x\n\nqa-h2-served-marker\n").unwrap();
        biorouter_mcp::knowledge::tier::raise_unlocked(&root, &kb, true).unwrap();

        let uri = format!("/knowledge/bases/{kb}/page?path=knowledge/x.md");
        let (status, body) = call(state.clone(), "GET", &uri, None, &[]).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::FORBIDDEN, KNOWLEDGE_BASE_OUT_OF_REACH),
            "the served tree handed a secret-only caller a private base's page"
        );
        let (status, body) = call(state.clone(), "GET", &uri, None, &[PROOF]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("qa-h2-served-marker"));
        let (status, body) = call(state.clone(), "GET", "/knowledge/bases", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !body.contains(&kb),
            "the served list named a private base: {body}"
        );

        let _ = state.knowledge_service.delete_base_async(&kb, None).await;
    }

    /// Appears in the seeded knowledge pages and nowhere else.
    const KB_SENTINEL: &str = "qa-h2-lib-sweep-marker-not-real-data";

    /// Two bases in the served tree's knowledge store, each with one page and
    /// one commit; the first is then ratcheted private the way a private chat's
    /// ingest leaves it. Deleted on drop — including when an assertion fails —
    /// because every test in this binary shares that store.
    struct SeededBases {
        state: Arc<AppState>,
        private: String,
        public: String,
        /// The private base's newest commit, for the history-shaped routes.
        sha: String,
    }

    impl Drop for SeededBases {
        fn drop(&mut self) {
            let root = self.state.knowledge_service.root().to_path_buf();
            for id in [&self.private, &self.public] {
                let _ = self.state.knowledge_service.delete_base(id);
                let _ = std::fs::remove_dir_all(root.join(id));
            }
        }
    }

    async fn seed_bases(state: &Arc<AppState>, label: &str) -> SeededBases {
        let pid = std::process::id();
        let mut seeded = SeededBases {
            state: state.clone(),
            private: format!("qa-{label}-private-{pid}"),
            public: format!("qa-{label}-public-{pid}"),
            sha: String::new(),
        };
        for (id, name) in [
            (seeded.private.clone(), "QA private base (test fixture)"),
            (seeded.public.clone(), "QA public base (test fixture)"),
        ] {
            let (status, body) = call(
                state.clone(),
                "POST",
                "/knowledge/bases",
                Some(serde_json::json!({ "id": id, "name": name })),
                &[PROOF],
            )
            .await;
            assert_eq!(status, StatusCode::OK, "creating {id}: {body}");
            let (status, body) = call(
                state.clone(),
                "PUT",
                &format!("/knowledge/bases/{id}/pages/knowledge/x.md"),
                Some(serde_json::json!({
                    "content": biorouter_mcp::knowledge::page_fixtures::valid_page(
                        "note",
                        "X",
                        &format!("# X\n\n{KB_SENTINEL} in {id}"),
                    ),
                    "commit_message": "seed",
                })),
                &[PROOF],
            )
            .await;
            assert_eq!(status, StatusCode::OK, "seeding {id}: {body}");
        }
        let root = state.knowledge_service.root().to_path_buf();
        biorouter_mcp::knowledge::tier::raise_unlocked(&root, &seeded.private, true).unwrap();
        let (status, body) = call(
            state.clone(),
            "GET",
            &format!("/knowledge/bases/{}/history", seeded.private),
            None,
            &[PROOF],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let history: serde_json::Value = serde_json::from_str(&body).unwrap();
        seeded.sha = history[0]["commit_sha"].as_str().unwrap().to_string();
        seeded
    }

    /// **`POST /knowledge/bases` was an existence oracle**, as a named
    /// regression test (adversarial security review 2026-09-12, MEDIUM).
    ///
    /// Create refuses an id that is taken, and it used to say so for a
    /// **private** base — to a caller holding nothing but the daemon secret, with
    /// the machine's absolute config path in the body. KB ids are user-authored
    /// names, so a short dictionary enumerated the private bases the listing
    /// deliberately omits.
    ///
    /// What is asserted is indistinguishability, byte for byte: the id of a
    /// private base, the id of a public base and an id that has never existed all
    /// get the SAME answer. That is only checkable if the answer does not depend
    /// on the id, which is why `mints_knowledge_base` does not take one.
    ///
    /// And the user is unaffected: with the proof, the collision is reported
    /// truthfully, a free id is created — and neither answer names a path.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn creating_a_base_cannot_be_used_to_ask_which_private_bases_exist() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let bases = seed_bases(&state, "create-oracle").await;
        let absent = format!("qa-create-oracle-absent-{}", std::process::id());

        let mint = |id: String, headers: Vec<(&'static str, &'static str)>| {
            let state = state.clone();
            async move {
                call(
                    state,
                    "POST",
                    "/knowledge/bases",
                    Some(serde_json::json!({ "id": id, "name": "minted by a probe" })),
                    &headers,
                )
                .await
            }
        };

        // The caller AR-11 measured: the daemon secret and nothing else.
        let private = mint(bases.private.clone(), vec![]).await;
        let public = mint(bases.public.clone(), vec![]).await;
        let free = mint(absent.clone(), vec![]).await;
        assert_eq!(
            private,
            (
                StatusCode::FORBIDDEN,
                KNOWLEDGE_BASE_OUT_OF_REACH.to_string()
            ),
            "a private base's id was answered differently from every other id"
        );
        assert_eq!(
            public, private,
            "a taken PUBLIC id and a taken PRIVATE id must be answered the same way here, or \
             the difference between them is the oracle"
        );
        assert_eq!(
            free, private,
            "an id that does not exist was answered differently from a private base's id, which \
             is the oracle: 403 means taken by something this caller may not see"
        );
        // …and the person at the keyboard is told the truth, without a path.
        let (status, collision) = mint(bases.private.clone(), vec![PROOF]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{collision}");
        assert!(
            collision.contains("already exists"),
            "the user was not told the id is taken: {collision}"
        );

        // No body on this route names a directory, whichever caller asked.
        let root = state
            .knowledge_service
            .root()
            .to_string_lossy()
            .into_owned();
        for body in [&private.1, &public.1, &free.1, &collision] {
            assert!(
                !body.contains(&root),
                "an error body on the create path named the knowledge root: {body}"
            );
            assert!(
                !body.contains(&format!("/{}", bases.private)),
                "an error body on the create path named a base's directory: {body}"
            );
        }

        let (status, body) = mint(absent.clone(), vec![PROOF]).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the user can no longer create a knowledge base: {body}"
        );
        let _ = state.knowledge_service.delete_base(&absent);
        let _ = std::fs::remove_dir_all(state.knowledge_service.root().join(&absent));
    }

    /// `crates/biorouter-server/src/routes/knowledge.rs`, as text, for the router
    /// shape assertions below. `include_str!` rather than a runtime read so a
    /// moved file is a compile error rather than a skipped check.
    const KNOWLEDGE_ROUTES_SOURCE: &str = include_str!("knowledge.rs");

    /// The body of a top-level `fn` in a `routes/*.rs` file: from its first `{`
    /// to the first line that is a bare `}` in column 0.
    ///
    /// A brace counter would be the obvious implementation and is the wrong one
    /// here: the bodies this reads are full of `"{id}"` and `"{*page_path}"`
    /// literals. Column-0 `}` is what `cargo fmt` guarantees for a top-level
    /// item, and nothing inside a function body can produce one.
    ///
    /// ⚠ **Line comments are removed**, and the first draft of this did not do
    /// that — a comment in `router()` reading "applied here, to the whole of
    /// `base_routes()`" made the single-consumer assertion below count two. A
    /// scanner that reads prose as code is the failure mode `privacy_guard_wiring`
    /// was written to avoid; the same applies here. Nothing in these bodies puts
    /// `//` inside a string literal, which is the case this does not handle.
    #[allow(clippy::string_slice)] // every index comes from `find`: a char boundary
    fn top_level_fn_body(source: &str, signature: &str) -> String {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("`{signature}` is not in knowledge.rs any more"));
        let after = &source[start..];
        let open = after
            .find('{')
            .unwrap_or_else(|| panic!("`{signature}` has no body"));
        let rest = &after[open + 1..];
        let end = rest
            .find("\n}\n")
            .unwrap_or_else(|| panic!("`{signature}` is not closed in column 0"));
        rest[..end]
            .lines()
            .map(|line| match line.find("//") {
                Some(at) => &line[..at],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every `.route("<path>", <verbs>)` in a router-builder body, as
    /// `(path, methods)`.
    #[allow(clippy::string_slice)] // every index comes from `find`: a char boundary
    fn registered_routes(body: &str) -> Vec<(String, Vec<String>)> {
        body.split(".route(")
            .skip(1)
            .map(|chunk| {
                let quote = chunk
                    .find('"')
                    .unwrap_or_else(|| panic!("a `.route(` with no path literal: {chunk:.80}"));
                let rest = &chunk[quote + 1..];
                let close = rest.find('"').expect("unterminated route path literal");
                let path = rest[..close].to_string();
                let args = &rest[close + 1..];
                let methods = ["get", "post", "put", "delete", "patch"]
                    .into_iter()
                    .filter(|verb| args.contains(&format!("{verb}(")))
                    .map(str::to_uppercase)
                    .collect();
                (path, methods)
            })
            .collect()
    }

    /// Does `uri` (a concrete request path, `/knowledge` prefix and query string
    /// included) match the axum route pattern `pattern` (`/bases/{id}/…`)?
    fn uri_matches_route(uri: &str, pattern: &str) -> bool {
        let path = uri.split('?').next().unwrap_or(uri);
        let path = path.strip_prefix("/knowledge").unwrap_or(path);
        let mut actual = path.trim_start_matches('/').split('/');
        let expected: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
        for (index, segment) in expected.iter().enumerate() {
            if segment.starts_with("{*") {
                // A wildcard capture eats the whole remainder, which must be
                // non-empty.
                return actual.next().is_some();
            }
            let Some(got) = actual.next() else {
                return false;
            };
            if segment.starts_with('{') {
                if got.is_empty() {
                    return false;
                }
            } else if *segment != got {
                return false;
            }
            if index + 1 == expected.len() {
                return actual.next().is_none();
            }
        }
        false
    }

    /// **The guarantee the doc used to assert and axum does not provide**
    /// (adversarial security review 2026-09-12, MEDIUM).
    ///
    /// `Router::route_layer` wraps the routes that exist when it is called and
    /// nothing added afterwards, so "every `/bases/{id}` route is gated" is a
    /// property of how `knowledge.rs` is *written*, not of what axum promises.
    /// The restructure makes the natural edit safe — the layer is applied to
    /// `base_routes()`'s return value at its one call site — and this reads the
    /// file to check the three things that restructure depends on, plus the one
    /// thing the restructure cannot give: that the probe list the H2 tests drive
    /// actually reaches every route registered.
    #[test]
    fn every_route_that_names_a_base_is_inside_the_gated_sub_router() {
        let gated = top_level_fn_body(
            KNOWLEDGE_ROUTES_SOURCE,
            "fn base_routes() -> Router<Arc<KnowledgeService>>",
        );
        let outer = top_level_fn_body(KNOWLEDGE_ROUTES_SOURCE, "pub fn router(svc: Arc<Knowledge");

        // 1. No route that names a base is registered outside the sub-router.
        for (path, _) in registered_routes(&outer) {
            assert!(
                !path.contains('{'),
                "`{path}` captures a path parameter and is registered on the UNGATED outer \
                 router. Every route that names a base belongs in `base_routes()`."
            );
        }

        // 2. The sub-router does not gate itself, and its one consumer does —
        //    immediately, so nothing can be appended between the two.
        assert!(
            !gated.contains("route_layer"),
            "`base_routes()` applies its own layer again. That is the snapshot trap: a route \
             appended after that call is silently ungated."
        );
        assert_eq!(
            outer.matches("base_routes()").count(),
            1,
            "`base_routes()` has more than one consumer; each would need its own gate"
        );
        let consumed = outer.split_once("base_routes()").expect("checked above").1;
        assert!(
            consumed.trim_start().starts_with(".route_layer("),
            "`base_routes()` is consumed without `.route_layer(` immediately after it"
        );
        assert!(
            consumed.contains("gate_knowledge_base"),
            "the layer applied to `base_routes()` is not `gate_knowledge_base`"
        );

        // 3. …and the probe list the H2 tests drive covers every one of them, so
        //    "each route refuses" is measured rather than assumed.
        let probes = base_addressing_routes("probe-kb", "deadbeef", "other-kb");
        let registered = registered_routes(&gated);
        assert!(
            registered.len() >= 20,
            "only {} `/bases/{{id}}` routes were found; the scanner has stopped reading \
             knowledge.rs",
            registered.len()
        );
        for (path, methods) in &registered {
            assert!(
                !methods.is_empty(),
                "no HTTP method was read off `{path}`; the scanner needs a new verb"
            );
            for method in methods {
                assert!(
                    probes
                        .iter()
                        .any(|(probe_method, uri, _)| probe_method == method
                            && uri_matches_route(uri, path)),
                    "`{method} {path}` is gated but never probed: add it to \
                     `base_addressing_routes` so the H2 tests drive it against a real private \
                     base"
                );
            }
        }
    }

    /// The scanner's own corners, so a silently-matching-everything matcher
    /// cannot make the assertion above vacuous.
    #[test]
    fn the_route_scanner_reads_paths_methods_and_matches_exactly() {
        let parsed = registered_routes(
            r#"
            .route("/bases/{id}", get(a).put(b).delete(c))
            .route("/bases/{id}/pages/{*page_path}", get(d).put(e))
            .route("/active", post(f))
            "#,
        );
        assert_eq!(
            parsed,
            vec![
                (
                    "/bases/{id}".to_string(),
                    vec!["GET".to_string(), "PUT".to_string(), "DELETE".to_string()]
                ),
                (
                    "/bases/{id}/pages/{*page_path}".to_string(),
                    vec!["GET".to_string(), "PUT".to_string()]
                ),
                ("/active".to_string(), vec!["POST".to_string()]),
            ]
        );

        assert!(uri_matches_route("/knowledge/bases/kb1", "/bases/{id}"));
        assert!(uri_matches_route(
            "/knowledge/bases/kb1/page?path=knowledge/x.md",
            "/bases/{id}/page"
        ));
        assert!(uri_matches_route(
            "/knowledge/bases/kb1/pages/knowledge/x.md",
            "/bases/{id}/pages/{*page_path}"
        ));
        assert!(!uri_matches_route(
            "/knowledge/bases/kb1/pages",
            "/bases/{id}/pages/{*page_path}"
        ));
        assert!(!uri_matches_route(
            "/knowledge/bases/kb1/tier",
            "/bases/{id}"
        ));
        assert!(!uri_matches_route(
            "/knowledge/bases/kb1",
            "/bases/{id}/tier"
        ));
        assert!(!uri_matches_route(
            "/knowledge/bases/kb1/graph",
            "/bases/{id}/tier"
        ));
    }

    /// Every route under `/knowledge/bases/{id}` in the served tree, as
    /// `(method, uri, body)`. Macros name a provider the registry does not
    /// know, so an admitted one stops with a 400 long before any model.
    ///
    /// ⚠ **Destructive last**, for the reason the chat sweep gives.
    ///
    /// ⚠ **This list is checked for completeness**, by
    /// `every_route_that_names_a_base_is_inside_the_gated_sub_router`: a route
    /// added to `knowledge::base_routes` and not added here fails that test.
    fn base_addressing_routes(
        id: &str,
        sha: &str,
        other: &str,
    ) -> Vec<(&'static str, String, Option<serde_json::Value>)> {
        let model = serde_json::json!({ "provider": "qa-h2-no-such-provider", "model": "m" });
        let page = biorouter_mcp::knowledge::page_fixtures::valid_page(
            "note",
            "X",
            "overwritten by an unproven caller",
        );
        let base = format!("/knowledge/bases/{id}");
        vec![
            ("GET", base.clone(), None),
            ("GET", format!("{base}/tier"), None),
            ("GET", format!("{base}/graph"), None),
            ("GET", format!("{base}/location"), None),
            ("GET", format!("{base}/page?path=knowledge/x.md"), None),
            ("GET", format!("{base}/pages"), None),
            ("GET", format!("{base}/pages/knowledge/x.md"), None),
            ("GET", format!("{base}/history"), None),
            (
                "POST",
                format!("{base}/preview"),
                Some(serde_json::json!({ "commit_sha": sha, "path": "knowledge/x.md" })),
            ),
            ("GET", format!("{base}/export"), None),
            (
                "POST",
                format!("{base}/query"),
                Some(serde_json::json!({ "question": "what is in it?", "model": model })),
            ),
            (
                "POST",
                format!("{base}/lint"),
                Some(serde_json::json!({ "model": model })),
            ),
            ("POST", format!("{base}/sources/s1/reclassify"), None),
            (
                "POST",
                format!("{base}/tier"),
                Some(serde_json::json!({ "tier": "public" })),
            ),
            (
                "POST",
                format!("{base}/merge"),
                Some(serde_json::json!({ "source_kb_id": other })),
            ),
            (
                "PUT",
                format!("{base}/sources/s1/credibility"),
                Some(serde_json::json!({})),
            ),
            (
                "PUT",
                base.clone(),
                Some(serde_json::json!({ "name": "renamed by an unproven caller" })),
            ),
            (
                "PUT",
                format!("{base}/default-model"),
                Some(serde_json::json!({ "model": model })),
            ),
            (
                "PUT",
                format!("{base}/pages/knowledge/x.md"),
                Some(serde_json::json!({ "content": page, "commit_message": "overwrite" })),
            ),
            (
                "POST",
                format!("{base}/raw"),
                Some(serde_json::json!({ "text": "an unproven raw source", "title": "t" })),
            ),
            (
                "POST",
                format!("{base}/ingest"),
                Some(serde_json::json!({ "source": { "text": "t" }, "model": model })),
            ),
            (
                "POST",
                format!("{base}/ingest-conversation"),
                Some(serde_json::json!({ "session_ids": ["29990101_1"], "model": model })),
            ),
            (
                "POST",
                format!("{base}/restore"),
                Some(serde_json::json!({ "commit_sha": sha })),
            ),
            ("DELETE", base, None),
        ]
    }

    /// **H2, through the tree the daemon serves and in the binary CI runs.**
    /// Every route that names a knowledge base answers a caller holding only
    /// the daemon secret, on a private base, exactly as the page read does —
    /// the same status and the same bytes — and answers a base that does not
    /// exist the same way. The person at the keyboard still reads all of it,
    /// and a public base is untouched.
    ///
    /// `tests/knowledge_routes.rs` (`h2_http_barrier`) sweeps the bare router
    /// as well, but CI runs `cargo test --workspace --lib --bins`, so that
    /// binary is not what keeps this door shut; this test is.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn every_route_that_names_a_private_base_refuses_it_exactly_as_the_read_does() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let bases = seed_bases(&state, "h2-sweep").await;
        let absent = format!("qa-h2-sweep-absent-{}", std::process::id());
        let page_read = |id: &str| format!("/knowledge/bases/{id}/page?path=knowledge/x.md");

        let (read_status, read_body) =
            call(state.clone(), "GET", &page_read(&bases.private), None, &[]).await;
        assert_eq!(
            (read_status, read_body.as_str()),
            (StatusCode::FORBIDDEN, KNOWLEDGE_BASE_OUT_OF_REACH),
            "the read path's refusal is what every route below is compared against"
        );

        let mut leaks = Vec::new();
        for id in [bases.private.as_str(), absent.as_str()] {
            for (method, uri, body) in base_addressing_routes(id, &bases.sha, &bases.public) {
                let (status, got) = call(state.clone(), method, &uri, body, &[]).await;
                if status != read_status || got != read_body {
                    leaks.push(format!("{method} {uri} -> {status}: {got:.160}"));
                }
            }
        }
        assert!(
            leaks.is_empty(),
            "a caller holding nothing but the daemon secret was answered differently from the \
             page read by {} route(s):\n  {}",
            leaks.len(),
            leaks.join("\n  ")
        );

        // …and nothing moved: still there, still private, same page.
        let root = state.knowledge_service.root().to_path_buf();
        assert!(biorouter_mcp::knowledge::tier::is_private(
            &root,
            &bases.private
        ));
        let on_disk =
            std::fs::read_to_string(root.join(&bases.private).join("knowledge/x.md")).unwrap();
        assert!(
            on_disk.contains(KB_SENTINEL),
            "an unproven caller rewrote a private page"
        );

        // The listing omits the private base — its id and its name — from the
        // same caller, and shows it to the user.
        let (status, body) = call(state.clone(), "GET", "/knowledge/bases", None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains(&bases.public), "{body}");
        assert!(
            !body.contains(&bases.private) && !body.contains("QA private base"),
            "the served list named a private base to a secret-only caller: {body}"
        );
        let (status, body) = call(state.clone(), "GET", "/knowledge/bases", None, &[PROOF]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(&bases.private), "{body}");

        // The other half: "refuse everyone" would pass everything above.
        for (method, uri, body) in base_addressing_routes(&bases.private, &bases.sha, "")
            .into_iter()
            .filter(|(method, _, _)| *method == "GET")
        {
            let (status, got) = call(state.clone(), method, &uri, body, &[PROOF]).await;
            assert_eq!(status, StatusCode::OK, "{uri}: {got:.200}");
        }
        let (status, got) = call(
            state.clone(),
            "GET",
            &page_read(&bases.private),
            None,
            &[PROOF],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{got}");
        assert!(got.contains(KB_SENTINEL), "{got}");
        let (status, _) = call(
            state.clone(),
            "GET",
            &format!("/knowledge/bases/{absent}"),
            None,
            &[PROOF],
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "the user is entitled to know the base is not there"
        );
        let (status, got) = call(state.clone(), "GET", &page_read(&bases.public), None, &[]).await;
        assert_eq!(status, StatusCode::OK, "a public base was refused: {got}");
        assert!(got.contains(KB_SENTINEL));
    }

    /// The browser token a `biorouter serve` launch would have minted. Distinct
    /// from every other cookie value in this binary's tests.
    const SERVED_TOKEN: &str = "5d0c9b8a7f6e5d4c3b2a19f8e7d6c5b4";

    /// **SD-10, in the binary CI runs.** A `serve` daemon's own interface —
    /// told apart by the served document's cookie — keeps the listing and
    /// knowledge-base reach its operator's private provider implies; the same
    /// request without the cookie, or with the wrong one, is a public caller;
    /// and the cookie opens no transcript — `GET /sessions/{id}` and `DELETE`
    /// refuse it exactly as they refuse the secret alone.
    ///
    /// ⚠ It installs the operator standing into this test binary for good (a
    /// `OnceLock`, as in the daemon). That is harmless to every other test here
    /// because the standing is earned only by a request carrying this exact
    /// cookie, and none of them sends it. The keyless arm — how `serve` really
    /// starts its daemon — needs a binary with no user-action key, and is
    /// `tests/serve_operator_reach.rs`.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_served_interface_keeps_its_listing_reach_and_gains_no_transcript() {
        install_test_user_action_key();
        // `biorouter_server::`, not `crate::`: this module is also compiled into
        // the `biorouterd` bin, which has no `auth` module of its own and reads
        // the library's — the same static `http_caller` reads in either binary.
        biorouter_server::auth::install_served_operator(
            SERVED_TOKEN.to_string(),
            ProviderTier::Private,
        );
        let cookie = format!("biorouter_session={SERVED_TOKEN}");
        let mut probe = HeaderMap::new();
        probe.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
        assert_eq!(
            served_operator_capability(&probe),
            ProviderTier::Private,
            "a different serve operator was installed into this binary first; this test's \
             premise does not hold"
        );

        let state = AppState::new().await.unwrap();
        let private = seed_private_chat(&state, "SD-10 served private (test fixture)").await;
        let bases = seed_bases(&state, "sd10").await;
        let served = [("cookie", cookie.as_str())];
        let wrong = [(
            "cookie",
            "biorouter_session=00000000000000000000000000000000",
        )];

        for (headers, operator) in [(&served[..], true), (&[][..], false), (&wrong[..], false)] {
            let (status, body) = call(state.clone(), "GET", "/sessions", None, headers).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(
                body.contains(private.id()),
                operator,
                "GET /sessions {headers:?}"
            );
            let ids = sidebar_ids(&state, 50, headers).await;
            assert_eq!(ids.contains(&private.id().to_string()), operator);

            let (status, body) =
                call(state.clone(), "GET", "/knowledge/bases", None, headers).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(
                body.contains(&bases.private),
                operator,
                "GET /knowledge/bases {headers:?}"
            );
            let (status, body) = call(
                state.clone(),
                "GET",
                &format!(
                    "/knowledge/bases/{}/page?path=knowledge/x.md",
                    bases.private
                ),
                None,
                headers,
            )
            .await;
            if operator {
                assert_eq!(status, StatusCode::OK, "{body}");
                assert!(body.contains(KB_SENTINEL), "{body}");
            } else {
                assert_eq!(
                    (status, body.as_str()),
                    (StatusCode::FORBIDDEN, KNOWLEDGE_BASE_OUT_OF_REACH),
                    "{headers:?}"
                );
            }
        }

        // The cookie earns nothing at the transcript gate: the read and the
        // delete refuse the served interface exactly as the secret alone.
        for method in ["GET", "DELETE"] {
            let uri = format!("/sessions/{}", private.id());
            let (status, body) = call(state.clone(), method, &uri, None, &served).await;
            assert_eq!(
                (status, body.as_str()),
                (StatusCode::FORBIDDEN, SESSION_OUT_OF_REACH),
                "{method} {uri} with the served cookie"
            );
        }
        assert!(
            state
                .session_manager()
                .get_session(private.id(), false)
                .await
                .is_ok(),
            "the served cookie deleted a private chat"
        );
    }

    /// One sidebar page, as the route's own clients take it: `cursor` passed
    /// back unchanged, never computed.
    async fn sidebar_page(
        state: &Arc<AppState>,
        limit: u32,
        cursor: Option<&str>,
        headers: &[(&str, &str)],
    ) -> serde_json::Value {
        // The cursor is base64url without padding, so every byte of it is
        // already safe in a query string — no escaping, and a client that had to
        // escape it would be a client that had parsed it.
        let uri = match cursor {
            Some(cursor) => format!("/sessions/sidebar?limit={limit}&cursor={cursor}"),
            None => format!("/sessions/sidebar?limit={limit}"),
        };
        let (status, body) = call(state.clone(), "GET", &uri, None, headers).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        serde_json::from_str(&body).unwrap()
    }

    fn page_ids(page: &serde_json::Value) -> Vec<String> {
        page["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap().to_string())
            .collect()
    }

    /// Every id the sidebar hands this caller, walking `next_cursor` to the end.
    async fn sidebar_ids(
        state: &Arc<AppState>,
        limit: u32,
        headers: &[(&str, &str)],
    ) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..10_000 {
            let page = sidebar_page(state, limit, cursor.as_deref(), headers).await;
            ids.extend(page_ids(&page));
            if page["has_more"] != serde_json::Value::Bool(true) {
                return ids;
            }
            cursor = Some(
                page["next_cursor"]
                    .as_str()
                    .expect("has_more without next_cursor")
                    .to_string(),
            );
        }
        panic!("the sidebar never reported its last page");
    }

    /// **The count oracle, as a named regression test** (adversarial security
    /// review 2026-09-12, HIGH). This is the test that would have caught it.
    ///
    /// The sidebar filters its rows for a caller that may not open a private
    /// chat, and it used to resume the next page from the position it had
    /// reached in the UNFILTERED ordering. So the continuation value counted the
    /// rows it had hidden: ask for page 1 twice with N private chats created in
    /// between and the value moves by exactly N. `updated_at` is stamped on
    /// every token written in this tree, so a private chat merely *running a
    /// turn* moves it — which turns a listing into a live activity monitor on
    /// chats the singular read refuses outright.
    ///
    /// Two assertions, and the first is the one that fails on the old code:
    ///
    /// 1. the continuation value does not move when hidden chats appear; and
    /// 2. the walk still reaches the same visible rows across that churn — a
    ///    position-based resume does not, because the position it was given now
    ///    points at a different row.
    ///
    /// The sleep is load-bearing: `updated_at` is `datetime('now')`, one-second
    /// granularity, so without it the seeded rows tie and SQLite breaks the tie
    /// by `id ASC` — which would put the private rows *below* the boundary and
    /// leave the old code's value accidentally unmoved.
    ///
    /// ⚠ The measurement is **retried**, and that is a statement about this
    /// binary rather than about the route. The head of the listing is the whole
    /// machine's newest visible chat; `#[serial]` keeps the other serial tests
    /// out, but a non-serial test that creates a chat can land a foreign row at
    /// the head inside the second this waits — which makes the test's PREMISE
    /// false (a different visible row) rather than its subject wrong. A repeated
    /// displacement still fails, and says so.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn the_sidebar_continuation_value_is_not_a_count_of_the_chats_it_hid() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();

        // Two visible rows, in one `datetime('now')` second. Which of them sorts
        // first does not matter — only that the pair is stable across the churn
        // below, which it is, because nothing here touches them again.
        let _visible_a = seed_chat(
            &state,
            "count-oracle visible A (test fixture)",
            SessionClassification::Public,
        )
        .await;
        let _visible_b = seed_chat(
            &state,
            "count-oracle visible B (test fixture)",
            SessionClassification::Public,
        )
        .await;

        // "Secret only" — the caller AR-11 measured, and the one this gate
        // answers as a public model.
        let secret_only: &[(&str, &str)] = &[];
        const HIDDEN: usize = 3;
        let mut displacements = Vec::new();

        for _ in 0..3 {
            let before = sidebar_page(&state, 1, None, secret_only).await;
            let first_page_ids = page_ids(&before);
            let token_before = before["next_cursor"].clone();
            assert!(
                !token_before.is_null(),
                "two visible chats were just seeded and the first page reported no next page: \
                 {before}"
            );
            let second_page_ids =
                page_ids(&sidebar_page(&state, 1, token_before.as_str(), secret_only).await);

            // Now the hidden rows, stamped into a strictly later second so they
            // sort above everything seeded above. The guards drop at the end of
            // each attempt, so a retry starts from the state this one did.
            tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
            let mut hidden = Vec::new();
            for i in 0..HIDDEN {
                hidden.push(
                    seed_private_chat(&state, &format!("count-oracle hidden {i} (test fixture)"))
                        .await,
                );
            }

            let after = sidebar_page(&state, 1, None, secret_only).await;
            if page_ids(&after) != first_page_ids {
                displacements.push(format!("{first_page_ids:?} -> {:?}", page_ids(&after)));
                continue;
            }
            assert_eq!(
                after["next_cursor"], token_before,
                "the continuation value moved when {HIDDEN} private chats were created. Its \
                 displacement IS their count, and because `updated_at` is stamped on every token \
                 written, polling this route reports when a private chat is running."
            );

            // …and the value the caller was given still walks to the same row,
            // which a position into the unfiltered ordering no longer does once
            // that ordering has shifted underneath it.
            assert_eq!(
                page_ids(&sidebar_page(&state, 1, token_before.as_str(), secret_only).await),
                second_page_ids,
                "the same continuation value reached a different visible row after private chats \
                 were created"
            );
            return;
        }

        panic!(
            "the head of the listing moved under every attempt, so nothing was measured — \
             another test in this binary is creating chats: {displacements:?}"
        );
    }

    /// A private chat with no message at all — `/workflows/create` answers such
    /// a chat before it builds an agent.
    async fn seed_private_chat_without_messages(state: &Arc<AppState>, label: &str) -> SeededChat {
        let manager = state.session_manager();
        let session = manager
            .create_session(
                PathBuf::from("/tmp/task58_session_reach"),
                label.to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        manager
            .update(&session.id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
        SeededChat {
            state: state.clone(),
            id: session.id,
        }
    }

    /// A private subagent registered as still initializing: its tool routes,
    /// once admitted, answer 424 without building an agent.
    struct QueuedChild {
        chat: SeededChat,
        handle: Arc<biorouter::agents::subagent_handle::BackgroundSubagent>,
    }

    impl Drop for QueuedChild {
        fn drop(&mut self) {
            self.handle
                .complete(biorouter::agents::SubagentResult::from_error(
                    "QA M2 queued-child fixture cleaned up",
                ));
        }
    }

    async fn seed_queued_private_child(state: &Arc<AppState>) -> QueuedChild {
        let manager = state.session_manager();
        let session = manager
            .create_session(
                PathBuf::from("/tmp/task58_session_reach"),
                "QA M2 queued child (test fixture)".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        manager
            .update(&session.id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register_initializing(
            "qa-m2-parent",
            session.id.clone(),
            "QA M2 queued child",
            tokio_util::sync::CancellationToken::new(),
        );
        QueuedChild {
            chat: SeededChat {
                state: state.clone(),
                id: session.id,
            },
            handle,
        }
    }

    /// `DELETE /knowledge/bases/{id}` through the real router tree, with the
    /// proof — as the Knowledge view sends it.
    async fn delete_knowledge_base(state: Arc<AppState>, kb_id: &str) -> (StatusCode, String) {
        let app = crate::routes::configure(state, "task-58-secret".to_string());
        let res = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/knowledge/bases/{kb_id}"))
                    .header("X-User-Action", TEST_USER_ACTION_KEY)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn selection_json(body: &str) -> serde_json::Value {
        serde_json::from_str(body).unwrap_or_else(|_| panic!("not a selection: {body}"))
    }

    /// QA 2026-09-10 F14, the daemon's half, end to end through the real router,
    /// the reach gate and a PRIVATE chat — the configuration every chat on a
    /// UCSF install is in.
    ///
    /// QA read the blank `.active-kb` it found after deleting the primary as
    /// "the daemon does not repair the selection". The blank IS the repair for a
    /// delete (D2 in `docs/knowledge-base/multi-kb-implementation-plan.md`):
    /// hiding promotes to the next base, deleting clears to the explicit
    /// no-primary, and a chat that merely inherited keeps inheriting. What this
    /// pins is the rest of the contract the Knowledge view now relies on instead
    /// of re-deriving it: nothing is left pointing at the deleted base, in any
    /// scope, and the person at the keyboard can choose again — for a private
    /// chat — and have it stick.
    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn deleting_the_primary_leaves_no_pointer_at_it_and_the_user_can_choose_again() {
        use biorouter_mcp::knowledge::service::PrimaryUpdate;

        install_test_user_action_key();
        // A throwaway knowledge root: this test creates bases and moves
        // pointers, which it must never do in a real one.
        let knowledge_root = tempfile::tempdir().unwrap();
        let state = AppState::new_with_knowledge_root(knowledge_root.path().to_path_buf())
            .await
            .unwrap();
        let svc = state.knowledge_service.clone();
        let pinning = seed_private_chat(&state, "F14 pinning chat (test fixture)").await;
        let inheriting = seed_private_chat(&state, "F14 inheriting chat (test fixture)").await;
        svc.create_base("soul", "Soul", None).unwrap();
        svc.create_base("doomed", "Doomed", None).unwrap();

        // The machine default names the base about to go, so the inheriting
        // chat shows it as its primary too; the other chat pins it itself — as
        // the person does, with the proof, through the gate.
        svc.set_selection(None, None, PrimaryUpdate::Set("doomed"))
            .unwrap();
        let (status, body) = post_knowledge_active(
            state.clone(),
            serde_json::json!({ "session_id": pinning.id(), "primary_kb": "doomed" }),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for chat in [pinning.id(), inheriting.id()] {
            let (status, body) =
                get_knowledge_active(state.clone(), chat, Some(TEST_USER_ACTION_KEY)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(selection_json(&body)["primary_kb"], "doomed", "{body}");
        }

        let (status, body) = delete_knowledge_base(state.clone(), "doomed").await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

        // No scope reports the deleted base, as primary or as a member…
        for chat in [pinning.id(), inheriting.id()] {
            let (status, body) =
                get_knowledge_active(state.clone(), chat, Some(TEST_USER_ACTION_KEY)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let selection = selection_json(&body);
            assert!(selection["primary_kb"].is_null(), "{body}");
            assert_eq!(selection["kb_ids"], serde_json::json!(["soul"]), "{body}");
        }
        let machine = svc.selection(None).unwrap();
        assert_eq!(machine.primary_kb, None);

        // …and none is left STORING it. The two pointers that named it are the
        // explicit no-primary — a blank file, which must not fall back to Soul —
        // and the chat that only inherited was left inheriting: no file of its
        // own was invented for it.
        let active_kb = std::fs::read_to_string(knowledge_root.path().join(".active-kb")).unwrap();
        assert_eq!(
            active_kb.trim(),
            "",
            "the machine pointer still names something"
        );
        let sessions = knowledge_root.path().join(".active-kb-sessions");
        let stored: Vec<String> = std::fs::read_dir(&sessions)
            .unwrap()
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        assert_eq!(
            stored,
            vec![String::new()],
            "exactly one chat pinned the base, and its pointer must now be blank"
        );
        assert_eq!(svc.get_primary_for_session(inheriting.id()).unwrap(), None);

        // The person chooses again — for a PRIVATE chat, which needs the proof —
        // and it sticks: in the answer, in a fresh read, and on disk.
        let (status, body) = post_knowledge_active(
            state.clone(),
            serde_json::json!({ "session_id": inheriting.id(), "primary_kb": "soul" }),
            Some(TEST_USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(selection_json(&body)["primary_kb"], "soul", "{body}");
        let (status, body) =
            get_knowledge_active(state.clone(), inheriting.id(), Some(TEST_USER_ACTION_KEY)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(selection_json(&body)["primary_kb"], "soul", "{body}");
        assert_eq!(
            svc.get_primary_for_session(inheriting.id())
                .unwrap()
                .as_deref(),
            Some("soul")
        );

        // The same write without the proof is still refused, and moves nothing.
        let (status, _) = post_knowledge_active(
            state.clone(),
            serde_json::json!({ "session_id": pinning.id(), "primary_kb": "soul" }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(svc.get_primary_for_session(pinning.id()).unwrap(), None);
    }
}
