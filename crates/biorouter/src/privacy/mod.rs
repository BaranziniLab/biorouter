//! Privacy tiers (issue #56). Two lattices over two different domains.
//!
//! * [`ProviderTier`] is CAPABILITY — what a session may *do*. It reduces with
//!   [`ProviderTier::least`] over the components of the provider bound right
//!   now. It is a pure function of live state and is never stored.
//! * [`SessionClassification`] is CLASSIFICATION — how sensitive a session's
//!   *contents* are. It reduces with `max` over events in time and is stored in
//!   `sessions.privacy_tier`, where it is a permanent ratchet.
//!
//! They do not interconvert. There is exactly one crossing, [`floor`], and a
//! repo-grep test in `Task 7` asserts its caller count.
//!
//! Invariant, proven by induction in the design (§4): for any sequence of legal
//! binds, `capability(S) >= classification(S)`. The bind admits `P` only when
//! `tier(P) >= classification(S)`; the ratchet then sets
//! `classification := max(old, floor(tier(P))) <= floor(tier(P))`.

use serde::{Deserialize, Serialize};

pub mod affiliation;
pub mod alt_provider;
pub mod capability;
pub mod config_keys;
// The (caller, target) state behind `visibility::requires_first_crossing_approval`.
pub mod crossing;
pub mod declassify;
pub mod disclosure;
#[cfg(test)]
mod disclosure_tests;
pub mod extensions;
pub mod grant;
pub mod master_switch;
pub mod mixing;
pub mod provenance;
pub mod refusal;
pub(crate) mod registry_live;
mod registry_private;
pub mod system_auth;
// The three platform prompters and the test seam (DR-24). Each carries its own
// file-level `cfg`, so these declarations are unconditional and the audit in
// `system_auth.rs` can assert that exactly two files gate on the seam's cfg pair.
pub mod system_auth_macos;
pub mod system_auth_peer;
pub mod system_auth_polkit;
pub mod system_auth_seam;
pub mod system_auth_windows;
pub mod visibility;

pub use affiliation::{CrossAffiliation, ExtensionAffiliation, ModelAffiliation};
pub use alt_provider::{assert_alt_provider_allowed, assert_alt_provider_matches_session};
pub use capability::CallCapability;
pub use config_keys::is_capability_key;
pub use extensions::{
    classify_extension, classify_extension_entry, private_extension_ids, resolve_extension,
    ExtensionClassification,
};
// ⚠ **The cross-affiliation proof-of-user is deliberately NOT re-exported here.**
// `grant::tests::the_proof_of_user_is_constructed_in_exactly_one_place` asserts
// that the set of files naming it is exactly the one HTTP handler behind the
// user-action header, and a convenience re-export would put this module in that
// set for nothing — which is also how a `use` of it becomes one line shorter to
// write from somewhere it must never be written at all.
pub use refusal::{raise_needs_user_action, PrivacyRefusal};

/// The master privacy-tier switch (R7, DR-15). `true` — the default — means
/// every gate and the classification ratchet are live. `false` means none of
/// them are: nothing is refused, and nothing ratchets.
///
/// A **re-export**, not a wrapper: the storage is
/// [`biorouter_mcp::privacy_toggle`], because several enforcement points are in
/// `biorouter-mcp` and that crate cannot see this one. Re-export rather than
/// `fn privacy_tiers_enabled() { biorouter_mcp::…() }` so the token at every
/// call site is identical in both crates and Task 30's audit is one pattern.
///
/// ⚠ **Where it is read, and where it deliberately is not.** A gate on a
/// TOOL-CALL path must not read it: [`CallCapability::sample`] reads the
/// provider tier and this flag together, at one instant, and threads the pair,
/// so those gates ask [`CallCapability::enforced`] /
/// [`CallCapability::restricts_private_data`] instead. A second read there is
/// the race the capability exists to close — pass Gate C with tiers on, then
/// relax a later barrier with tiers off. Every gate that is NOT on a tool-call
/// path (a provider bind, a turn, a spawn, a search, a route) has no capability
/// to consult and reads this directly.
pub use biorouter_mcp::privacy_toggle::privacy_tiers_enabled;

/// Load the master switch from disk. Called ONCE per process, at startup, after
/// the config is available — by BOTH hosts of this library: `biorouterd`
/// (`commands/agent.rs`) and the `biorouter` CLI (its `main`). A host that skips
/// it enforces, which is safe and is exactly why the CLI's omission went a whole
/// round unnoticed.
///
/// ⚠ **What it reads changed in Task 42 (DR-22), and the name did not.** The
/// value now lives in [`master_switch`]'s own record beside `config.yaml`, not
/// in `config.yaml` itself — because Task 30 closed the HTTP channel while the
/// FILE channel stayed open, and a next-launch disable of a control the agent is
/// subject to is not a control. The function keeps its name because three call
/// sites and a page of design prose refer to it by that name, and because it is
/// still what it says: the load, from this install's configuration directory.
///
/// The value is never read through [`crate::config::Config::get_param`], whose
/// middle branch resolves an environment variable. The threat that closes is
/// specific and cheap: the agent has `developer__shell`, so if the value were
/// env-readable then `BIOROUTER_PRIVACY_TIERS=off biorouterd` — or a line in the
/// user's shell profile — is a one-token disable. Because the authoritative
/// value then lives in daemon memory, every gate's read is a relaxed atomic load
/// and is safe on a hot path.
///
/// A load error resolves to ON, for the same reason absence does: the failure of
/// the loader must not be a way to disable the control.
///
/// ⚠ **An OFF answer is announced, not merely obeyed** (H3, 2026-09-10 security
/// test drive): [`master_switch::load`] logs one WARN naming the record and how
/// it got its value, and the report is [`master_switch::remember`]ed here —
/// beside the atomic, by the same writer — for the config surface the app reads.
pub fn load_privacy_tiers_from_config() {
    let report = master_switch::load(crate::config::Config::global());
    let on = report.enabled;
    master_switch::remember(report);
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(on);
}

/// What [`load_privacy_tiers_from_config`] would store, for an explicit
/// [`crate::config::Config`].
///
/// The seam exists so a test can exercise the real resolution against a scratch
/// configuration directory instead of the developer's own — this function
/// *writes* (the migration does), and a test that reached the process-global
/// `Config` would create a record in `~/.config/biorouter`.
///
/// ⚠ **Not a second reader of the switch.** It is the disk resolution; the
/// authoritative value is the atomic, and gates read
/// [`privacy_tiers_enabled`]. Calling this from a gate would re-introduce
/// exactly the per-gate file read hardening measure (3) exists to forbid.
/// Load DR-27's cross-institution mixing policy from disk. Called ONCE per
/// process, at start-up, beside [`load_privacy_tiers_from_config`] and by the
/// same two hosts — pinned to those call sites by
/// `mixing::tests::every_host_that_loads_the_master_switch_also_loads_the_mixing_policy`,
/// because a host that loads one and not the other is the omission that would go
/// unnoticed.
///
/// A host that skips it enforces `standard`: the default, and exactly what this
/// tree did before DR-27. See [`mixing::load_from_record`] for why the value is
/// installed at start-up rather than resolved on first use.
pub fn load_mixing_policy_from_record() {
    mixing::load_from_record(crate::config::Config::global());
}

pub fn resolve_privacy_tiers(config: &crate::config::Config) -> bool {
    // `load` runs the migration (once, on the first start-up after the upgrade,
    // and never again — the only read of the retired `config.yaml` key in the
    // tree), reads the record, and resolves nothing recorded OR unreadable to
    // ON. It also logs the OFF warning, so this seam exercises exactly what the
    // loader says as well as what it stores.
    master_switch::load(config).enabled
}

/// The key the master switch is addressed by — over `/config/upsert`, over
/// `/config/read`, and by the Settings panel. One spelling, shared by the
/// route's two refusals, by the migration and by the renderer.
///
/// ⚠ **It is no longer where the value is STORED** (Task 42, DR-22). The
/// record lives in [`master_switch`]; this key names the switch on the wire and
/// is *ignored* in `config.yaml` from the migration onward. A reader that
/// consults `config.yaml` for it has not closed the channel DR-22 names, it has
/// added a second one.
pub const PRIVACY_TIERS_CONFIG_KEY: &str = "BIOROUTER_PRIVACY_TIERS";

/// Is this key the master privacy switch? **One predicate, every verb** — the
/// same argument [`is_capability_key`] makes, and for the same reason it was
/// made there: a rule that holds for `POST /config/upsert` and not for
/// `POST /config/remove` is not a rule, it is a door with one lock.
///
/// A DELETE of this key is not the absence of a write. It removes the key from
/// disk, so the next start-up reads *absent* and resolves to ON — while the
/// running daemon's atomic keeps whatever it had. Every consequence of that is
/// in the safe direction, which is exactly why it went unnoticed; the reason to
/// close it anyway is that the divergence is real, silent, and the next person
/// to reason about "where can this value change" would be wrong.
pub fn is_privacy_tiers_key(key: &str) -> bool {
    key == PRIVACY_TIERS_CONFIG_KEY
}

/// The key the master switch's RECORD REPORT is served under, beside
/// [`PRIVACY_TIERS_CONFIG_KEY`] on `/config/read` and `/config` (H3): where the
/// record lives and which door last wrote it — [`master_switch::SwitchReport`],
/// as the process last loaded or wrote it. The renderer mirrors the spelling in
/// `settings/privacy/privacyTiers.ts`.
///
/// ⚠ **A report, never a setting, and it has no writer on the wire.** Both read
/// paths answer it from [`master_switch::remembered`] and overwrite whatever
/// `config.yaml` holds under the same name, so an agent that `/config/upsert`s a
/// flattering copy writes a line nothing reads. That is why neither write verb
/// refuses it — the reason `/config/read`'s synthetic `model-limits` key needs
/// no refusal either — and why no reader may ever consult `config.yaml` for it:
/// the moment one does, "turned off outside the app" becomes something the
/// agent can pre-empt.
pub const PRIVACY_TIERS_RECORD_KEY: &str = "BIOROUTER_PRIVACY_TIERS_RECORD";

/// Is this key the switch's record report? See [`PRIVACY_TIERS_RECORD_KEY`].
pub fn is_privacy_tiers_record_key(key: &str) -> bool {
    key == PRIVACY_TIERS_RECORD_KEY
}

/// The typed phrase Settings → Privacy sends with the flip. A **UX guard against
/// an accidental or model-composed config write**, not an authorization
/// boundary: the phrase is a fixed string in the shipped source, so a caller
/// holding the daemon secret replays it. What it buys is that the flip cannot be
/// a side effect of an ordinary `/config/upsert` — which is the reachable path,
/// because a model *can* compose one of those through a tool.
pub const PRIVACY_TIERS_DISABLE_PHRASE: &str = "DISABLE PRIVACY TIERS";

/// `Some(false)` for a YAML `false` or for the words `off` / `false` / `no`
/// (case-insensitively); `Some(true)` for any other bool or string. A shape that
/// is neither gives `None`, which the caller resolves to ON.
///
/// The three words rather than only `off`, and the bool as well as the string,
/// because the alternative is worse in one specific direction: a user who
/// hand-edited `config.yaml` to `BIOROUTER_PRIVACY_TIERS: false` and was told
/// nothing would believe the feature is off while it is on. Over-reading an
/// "off" costs protection the user asked to drop; under-reading one costs them
/// a belief that is false.
///
/// ⚠ **`trim` for the same reason, and it is not cosmetic.** The renderer's
/// mirror of this function (`settings/privacy/privacyTiers.ts`) trims, and the
/// two must agree or a hand-edited `BIOROUTER_PRIVACY_TIERS: " off "` renders as
/// off in Settings → Privacy while the daemon goes on enforcing — the user is
/// then told something false about the control they just used, which is the
/// exact failure the paragraph above rejects.
pub fn privacy_tiers_value_is_on(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::String(s) => {
            let s = s.trim();
            Some(
                !(s.eq_ignore_ascii_case("off")
                    || s.eq_ignore_ascii_case("false")
                    || s.eq_ignore_ascii_case("no")),
            )
        }
        _ => None,
    }
}

/// CAPABILITY — the least-privileged model currently bound to a session.
///
/// Deliberately **not** `Ord`: `max` over this type is always a bug. A mixed
/// lead/worker composite is `least(lead, worker)`, so a private lead with a
/// public worker has **public** reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProviderTier {
    Public,
    Private,
}

impl ProviderTier {
    /// The capability reduction. Public is less privileged, so it wins.
    pub fn least(a: Self, b: Self) -> Self {
        match (a, b) {
            (Self::Private, Self::Private) => Self::Private,
            _ => Self::Public,
        }
    }

    /// `const` so [`CallCapability::restricts_private_data`] — the predicate
    /// every barrier asks — can itself be a `const fn`.
    pub const fn is_private(self) -> bool {
        matches!(self, Self::Private)
    }
}

impl Default for ProviderTier {
    /// Fail-**safe**, not fail-open: Public is the *less* privileged tier, so a
    /// provider module that forgets `tier()` gets less reach, never more.
    fn default() -> Self {
        Self::Public
    }
}

/// CLASSIFICATION — the most sensitive thing a session has ever touched.
///
/// `Ord` is derived and `Public < Private`, so `max` is the accumulation and is
/// spellable. Monotone in time; the storage layer refuses to lower it (see
/// `SessionUpdateBuilder`'s `CASE WHEN` emission).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum SessionClassification {
    Public,
    Private,
}

impl SessionClassification {
    pub const PUBLIC_SQL: &'static str = "public";
    pub const PRIVATE_SQL: &'static str = "private";

    /// Named constructor for `#[serde(default = "…")]`, which takes a **path to
    /// a function**, not a variant. Task 6's `Session::privacy_tier` field uses
    /// `#[serde(default = "SessionClassification::public")]`; without this the
    /// struct does not compile (`expected function, found variant`).
    ///
    /// Serde's default is the *deserialization* fallback for a JSON document
    /// with no `privacy_tier` — an exported/imported session file, not a
    /// database row. Public is right here and Private is right for the DB read,
    /// and they differ on purpose: Task 22's `import_session` never trusts the
    /// deserialized value as authority to be public (it raises to Private and
    /// only ever raises), while `from_stored` below fails closed because a
    /// missing *column* is a projection bug rather than an absent field.
    pub fn public() -> Self {
        Self::Public
    }

    pub fn as_sql(self) -> &'static str {
        match self {
            Self::Public => Self::PUBLIC_SQL,
            Self::Private => Self::PRIVATE_SQL,
        }
    }

    /// Parse a stored value. **Fails closed**, deliberately breaking the
    /// tree's `try_get(..).ok().flatten()` convention for optional columns
    /// (`session_manager.rs:1971-1977`): an unrecognised or absent value is a
    /// bug in a projection, and `branch_point_msg_uid`'s absence from
    /// `list_sessions_by_types` is the live proof that a projection does get
    /// missed. Private paints every row with a badge the user will report on
    /// day one, and is safe until they do.
    pub fn from_stored(raw: &str) -> Self {
        match raw {
            Self::PUBLIC_SQL => Self::Public,
            Self::PRIVATE_SQL => Self::Private,
            other => {
                tracing::error!(
                    value = other,
                    "unrecognised sessions.privacy_tier; reading Private (fail-closed)"
                );
                Self::Private
            }
        }
    }

    pub fn is_private(self) -> bool {
        matches!(self, Self::Private)
    }
}

/// The ONE crossing between the two lattices: the classification floor a turn
/// run under `tier` establishes. `pub(crate)` on purpose — a repo-grep test
/// asserts the caller count, so a third crossing cannot appear unnoticed.
pub(crate) fn floor(tier: ProviderTier) -> SessionClassification {
    match tier {
        ProviderTier::Private => SessionClassification::Private,
        ProviderTier::Public => SessionClassification::Public,
    }
}

/// A session may bind `incoming` only when the provider is at least as private
/// as the session's contents.
///
/// ⚠ This is Gate A's predicate written in Rust, and **Gate A does not call
/// it**: the live gate is the `WHERE` clause of
/// `SessionStorage::bind_provider_if_allowed` — `AND (privacy_tier = 'public'
/// OR ? = 1)` — because a predicate evaluated here would leave a window between
/// the read and the write that a concurrent ratchet fits inside. What this
/// function serves is `visible_to` (Gate D) and the induction below, so the two
/// spellings must stay one predicate:
/// `the_sql_predicate_and_bind_allowed_agree_on_every_combination`
/// (`agents::agent::gate_a_bind_tests`) runs the real statement over all four
/// (tier, classification) combinations and asserts the outcomes agree. Relax
/// either one and that test fails.
pub fn bind_allowed(incoming: ProviderTier, target: SessionClassification) -> bool {
    match target {
        SessionClassification::Public => true,
        SessionClassification::Private => incoming.is_private(),
    }
}

/// Gate A's predicate **plus DR-16's**, for the surface where a MODEL asks for
/// the bind: it may LOWER a conversation's capability, never RAISE it.
///
/// [`bind_allowed`] is deliberately asymmetric — it refuses only the *downward*
/// bind, because putting a MORE private model in front of a conversation can
/// never leak that conversation's contents. That reasoning is sound for the
/// user's own act and incomplete for a model's: the raise is not dangerous to
/// the conversation's *contents*, it is dangerous because of what the raised
/// conversation may then **reach**. A session bound to a private provider has
/// Private capability, which unlocks `chatrecall` over private conversations,
/// private knowledge bases and an unfiltered Gate E roster — and, on its next
/// turn, ratchets `sessions.privacy_tier` to Private permanently, locking the
/// user's conversation out of every public model for good.
///
/// DR-16 rules that the raise "is the user's decision, not yours"
/// ([`refusal::raise_needs_user_action`]). The three sites that enforce it are
/// all HTTP routes, where `X-User-Action` can carry proof that a person asked;
/// a model-facing tool has no person on the other end to supply that proof, so
/// it gets the refusal rather than the question. See
/// [`refusal::PrivacyRefusal::ToolTierRaise`].
///
/// What is left is the *sideways* bind, in both directions of the pair: the
/// provider's tier must equal the target's classification. That is the two
/// rules composed, not a third one — `capability(S) >= classification(S)` is
/// the invariant, [`bind_allowed`] forbids dropping below the floor, and this
/// forbids a model raising the ceiling above it.
///
/// ⚠ The baseline is the target's **stored classification**, not its currently
/// bound provider's tier (which is what the HTTP sites compare). Two reasons:
/// the classification is the value that ratchets, and is therefore the one a
/// concurrent turn cannot move in the unsafe direction; and the pre-flight
/// already holds it, where reading the target's live provider would mean
/// reaching for its agent, which the pre-flight must not create.
pub fn tool_bind_allowed(incoming: ProviderTier, target: SessionClassification) -> bool {
    bind_allowed(incoming, target) && incoming.is_private() == target.is_private()
}

/// Gate D / the §7 matrix's VIS rule: a caller sees a target only when the
/// target's classification does not exceed the caller's capability.
pub fn visible_to(caller: ProviderTier, target: SessionClassification) -> bool {
    bind_allowed(caller, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DR-16 at the model-facing surface, as the predicate: all four
    /// combinations, and the asymmetry `bind_allowed` carries on its own spelled
    /// out beside the one this adds.
    ///
    /// The `Private → Public` row is the whole finding: `bind_allowed` answers
    /// `true` there, because putting a MORE private model in front of a public
    /// chat leaks nothing — and `tool_bind_allowed` answers `false`, because a
    /// MODEL asking for it is granting that chat reach it did not have and
    /// ratcheting it permanently. Both answers are correct for their own
    /// caller; the bug was having only the first one on the tool path.
    #[test]
    fn a_model_may_lower_a_capability_but_never_raise_one() {
        use ProviderTier::{Private, Public};
        use SessionClassification as C;

        // Sideways, both directions of the pair: the only binds a model may ask
        // for, and the ones that must keep working.
        assert!(tool_bind_allowed(Public, C::Public));
        assert!(tool_bind_allowed(Private, C::Private));

        // Downward. Gate A already refuses it, and this must not disagree —
        // two predicates over the same four cells that differ anywhere except
        // the raise cell would be two policies.
        assert!(!bind_allowed(Public, C::Private));
        assert!(!tool_bind_allowed(Public, C::Private));

        // The raise. `bind_allowed` permits it on purpose; the model-facing
        // surface must not.
        assert!(
            bind_allowed(Private, C::Public),
            "Gate A's asymmetry is deliberate — if this flipped, `tool_bind_allowed` is now \
             redundant and the change that flipped it needs a much harder look than this test"
        );
        assert!(
            !tool_bind_allowed(Private, C::Public),
            "a model just raised a public conversation's capability to Private"
        );

        // Stated as a rule rather than as four cells, so a fifth cell (a third
        // tier) cannot be added while satisfying the cells above.
        for (incoming, target) in [
            (Public, C::Public),
            (Public, C::Private),
            (Private, C::Public),
            (Private, C::Private),
        ] {
            assert_eq!(
                tool_bind_allowed(incoming, target),
                incoming.is_private() == target.is_private(),
                "a model-facing bind is exactly the sideways one ({incoming:?} → {target:?})"
            );
        }
    }

    #[test]
    fn capability_is_a_least_and_classification_is_a_max() {
        use ProviderTier::{Private, Public};
        // CAPABILITY: least privileged wins. A private lead with a public worker
        // has public reach, because the transcript already goes to the worker.
        assert_eq!(ProviderTier::least(Private, Public), Public);
        assert_eq!(ProviderTier::least(Private, Private), Private);
        assert_eq!(ProviderTier::least(Public, Public), Public);

        // CLASSIFICATION: most sensitive wins, and it is Ord so `max` is spellable.
        assert!(SessionClassification::Private > SessionClassification::Public);
        assert_eq!(
            SessionClassification::Public.max(SessionClassification::Private),
            SessionClassification::Private
        );

        // The ONE crossing.
        assert_eq!(floor(Private), SessionClassification::Private);
        assert_eq!(floor(Public), SessionClassification::Public);
    }

    #[test]
    fn an_unparseable_or_absent_classification_reads_private() {
        assert_eq!(
            SessionClassification::from_stored("private"),
            SessionClassification::Private
        );
        assert_eq!(
            SessionClassification::from_stored("public"),
            SessionClassification::Public
        );
        // Fail closed, loudly: a bug in a projection paints every session Private —
        // immediately visible, immediately fixed, safe meanwhile. The match is
        // case-SENSITIVE on purpose: `as_sql` is the only writer and it only ever
        // emits lowercase, so `"PUBLIC"` in the column is an anomaly, and an
        // anomaly reads Private like any other unrecognised value. Do not "fix"
        // this by lowercasing the input — that is a leniency, and leniency here
        // fails open.
        assert_eq!(
            SessionClassification::from_stored("PUBLIC"),
            SessionClassification::Private
        );
        assert_eq!(
            SessionClassification::from_stored("nonsense"),
            SessionClassification::Private
        );
        assert_eq!(
            SessionClassification::from_stored(""),
            SessionClassification::Private
        );
    }

    #[test]
    fn deriving_ord_on_provider_tier_would_be_caught_here() {
        // If someone makes ProviderTier orderable, `least` becomes
        // interchangeable with `min` and a reviewer stops noticing which is meant.
        // This is the semantic assertion: the two lattices disagree on the same
        // pair, which is the entire point of having two of them.
        //
        // The companion static check is the derive list above — exactly one type
        // in this file carries an ordering derive, and it is not this one. A grep
        // counts that token, so do not spell it here.
        let cap = ProviderTier::least(ProviderTier::Private, ProviderTier::Public);
        let cls = SessionClassification::Private.max(SessionClassification::Public);
        assert_eq!(cap, ProviderTier::Public);
        assert_eq!(cls, SessionClassification::Private);
    }

    /// Occurrences of `needle` in `code` whose preceding character is not `.`, so
    /// `session_manager.rs`'s `pos.floor()` — an `f64` method with nothing to do
    /// with this module — can never be counted as a crossing.
    ///
    /// Byte-wise on purpose: `.` is ASCII, and no byte of a multi-byte UTF-8
    /// character can equal it, so testing the preceding byte is exact and cannot
    /// split a character the way slicing the string would.
    fn count_calls_not_method(code: &str, needle: &str) -> usize {
        let bytes = code.as_bytes();
        code.match_indices(needle)
            .filter(|(at, _)| *at == 0 || bytes[*at - 1] != b'.')
            .count()
    }

    #[test]
    fn floor_is_crossed_only_where_a_capability_establishes_a_classification() {
        // The two lattices are independent by construction; `floor` is the only
        // crossing. This test is what keeps that true over the next thirty tasks.
        //
        // It asserts the exact (file, count) SET, not a total. Three reasons, each
        // learned from the first version of this test, which could not pass at any
        // point in this plan:
        //
        //  (a) A total is defeated by an UNRELATED symbol. The first version
        //      matched `line.contains("floor(")`, which already matches
        //      `session_manager.rs`'s `let lo = pos.floor() as usize;` — an f64
        //      method with nothing to do with this module. (This comment used to
        //      cite a line number for it; the number had already drifted by 63
        //      lines, so the symbol is the anchor and the number is gone.) Its
        //      "baseline 0" was really 1 before a single line of #56 existed.
        //      Matching only the QUALIFIED path `privacy::floor(` cannot see a
        //      float method, ever.
        //  (b) A total is defeated by the plan's OWN later additions, silently: a
        //      task that adds two crossings where the number says one still passes
        //      if another task removed one. A set names the file.
        //  (c) The failure message has to be actionable. `assert_eq!` on a Vec of
        //      (file, count) prints exactly which file changed.
        //
        // The qualified-path rule needs a second assertion to hold: nobody may
        // `use` the bare name, or rule (a) is evaded by an import — through
        // `crate::privacy`, and equally through the `super`/`self` paths a module
        // beside this one would use. Inside `src/privacy/` the audit does not rely
        // on the import assertion at all; see `beside_the_definition` below.
        //
        // SCOPE, written down because the plan claims more than this delivers. The
        // plan says this test catches "a refactor that adds
        // `From<ProviderTier> for SessionClassification` and sprinkles `.into()` at
        // four sites". It does not, and no wording of it could: that refactor adds
        // no `floor` call to count, and the `From` impl itself would land in this
        // file, which the audit skips. What this test pins is the set of `floor`
        // CALLERS — narrower than advertised, still the thing that keeps the two
        // lattices from merging, and not to be cited in a later review as though
        // the broader claim had been established here.
        //
        // EXPECTED grows twice, and each growth is one uncommented line in the diff
        // that causes it — Task 13 (Gate B's ratchet) and Task 23 (the spawn stamp).
        // A test written to accept `<= 2` would let a third crossing appear
        // unnoticed, which is the entire point of having this test.
        const EXPECTED: &[(&str, usize)] = &[
            // Task 13: Gate B's ratchet, in `Agent::reply`. The ONE place a
            // capability establishes a classification.
            ("crates/biorouter/src/agents/agent.rs", 1),
            // Task 23: the spawn stamp, in `apply_settings_overrides`. The
            // child's CAPABILITY establishes the CLASSIFICATION its row is born
            // with. The extension filter beside it deliberately does NOT cross:
            // it compares two `ProviderTier`s through Gate C's own predicate.
            ("crates/biorouter/src/agents/subagent_tool.rs", 1),
        ];

        // CARGO_MANIFEST_DIR is <workspace>/crates/biorouter; go up twice.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        // This audit is only as good as its walk, and while EXPECTED is empty a walk
        // that reads NOTHING produces the identical green result as a walk that finds
        // nothing. So every way it can silently do no work is made loud instead: a
        // wrong root, an unreadable directory, an unreadable file, and a scan that
        // comes back implausibly small each fail rather than passing vacuously.
        let crates = root.join("crates");
        assert!(
            crates.is_dir(),
            "the audit walks {}; if that path is wrong, every assertion below passes \
             for the wrong reason",
            crates.display()
        );
        let mut calls: std::collections::BTreeMap<String, usize> = Default::default();
        let mut imports: Vec<String> = vec![];
        let mut scanned = 0usize;
        for entry in walkdir::WalkDir::new(&crates) {
            let entry = entry.expect("the audit must not silently skip an unreadable directory");
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            // The definition, and the induction test below, live here. Match the
            // EXACT path: `ends_with` also exempted any future
            // `crates/<other-crate>/src/privacy/mod.rs` from the audit entirely —
            // a blind spot shaped exactly like the one this test exists to prevent.
            if rel == "crates/biorouter/src/privacy/mod.rs" {
                continue;
            }
            scanned += 1;
            let src = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("the audit could not read {rel}: {e}"));
            // A file that lives BESIDE the definition reaches `floor` through
            // `super`, not through `privacy::` — and Task 8 creates exactly such a
            // sibling (`privacy::extensions`). For those files the audit counts the
            // CALL instead of trusting an import spelling, because the form this
            // evasion would really take is not a named import anyone would notice:
            // it is the `use super::*;` that every `mod tests` in this tree already
            // writes, which makes the bare name available with nothing to grep for.
            let beside_the_definition = rel.starts_with("crates/biorouter/src/privacy/");
            for (i, line) in src.lines().enumerate() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                let crossings = if beside_the_definition {
                    // Every `floor(` that is not a `.floor()`: covers `privacy::floor(`,
                    // `super::floor(`, `self::floor(` and the bare glob-imported name.
                    count_calls_not_method(code, "floor(")
                } else {
                    // Occurrences, not lines. EXPECTED asserts exact COUNTS, so two
                    // crossings written on one line have to read as two; a
                    // `contains` scored them as one and under-counted the set it is
                    // the whole job of this test to pin.
                    code.matches("privacy::floor(").count()
                };
                if crossings > 0 {
                    *calls.entry(rel.clone()).or_default() += crossings;
                }
                if code.contains("use crate::privacy::floor")
                    || code.contains("use super::floor")
                    || code.contains("use self::floor")
                    || ((code.contains("privacy::{")
                        || code.contains("use super::{")
                        || code.contains("use self::{"))
                        && code.contains("floor"))
                {
                    imports.push(format!("{rel}:{}", i + 1));
                }
            }
        }
        assert!(
            scanned >= 400,
            "only {scanned} .rs files were scanned (612 at Task 7). The walk is no longer \
             covering the tree, and a broken walk reports the same empty caller set as a \
             clean one."
        );
        assert!(
            imports.is_empty(),
            "`floor` must be called through its qualified `privacy::floor(..)` path so this \
             audit can see it; do not import the bare name: {imports:#?}"
        );
        let found: Vec<(String, usize)> = calls.into_iter().collect();
        // `found` comes out of a BTreeMap, so it is path-sorted; `want` would
        // otherwise keep EXPECTED's declaration order. The two entries Tasks 13 and
        // 23 uncomment happen to be declared in sorted order, but relying on that
        // makes the NEXT entry fail for a reason that has nothing to do with the
        // invariant, behind a message that blames the invariant. Sort both.
        let mut want: Vec<(String, usize)> = EXPECTED
            .iter()
            .map(|(f, n)| ((*f).to_string(), *n))
            .collect();
        want.sort();
        assert_eq!(
            found, want,
            "the set of `floor` callers changed. A new crossing between the capability and \
             classification lattices is a design change, not a refactor: argue it in the PR."
        );
    }

    /// Every word of length `n` over `alphabet`, in lexicographic order by index.
    /// `alphabet.len().pow(n)` of them — the induction below uses 2^6 = 64, which
    /// is exhaustive over every bind order a session of six binds can experience.
    fn all_sequences_of_length<T: Copy>(n: usize, alphabet: &[T]) -> Vec<Vec<T>> {
        let mut out: Vec<Vec<T>> = vec![vec![]];
        for _ in 0..n {
            out = out
                .into_iter()
                .flat_map(|prefix| {
                    alphabet.iter().map(move |sym| {
                        let mut next = prefix.clone();
                        next.push(*sym);
                        next
                    })
                })
                .collect();
        }
        out
    }

    #[test]
    fn capability_never_falls_below_classification_for_any_legal_sequence() {
        use ProviderTier::{Private, Public};
        // The design's induction (§4), made executable. Every legal bind followed
        // by its ratchet must preserve capability >= classification.
        //
        // SCOPE. `visible_to` delegates to `bind_allowed`, and `bind_allowed` is
        // also this loop's admission gate — so the assertion and the gate move
        // together. This pins `floor` for consistency WITH Gate A's predicate; it
        // cannot see a relaxation OF that predicate, and must not be read as
        // covering it. What covers it is
        // `the_sql_predicate_and_bind_allowed_agree_on_every_combination`
        // (`agents::agent::gate_a_bind_tests`), which runs Gate A's live `WHERE`
        // clause over all four (tier, classification) combinations and asserts the
        // outcome equals `bind_allowed` — the debt this comment used to record as
        // owed by "whichever task first wires Gate A".
        for binds in all_sequences_of_length(6, &[Private, Public]) {
            let mut classification = SessionClassification::Public;
            let mut capability = Public;
            // The induction's BASE case. Asserting it is not decoration: without a
            // read here the initializer is dead, `unused_assignments` fires, and
            // `scripts/clippy-lint.sh`'s `-D warnings` turns the whole lint gate
            // red. Silencing that with an `#[allow]` would have thrown away the one
            // half of §4's proof the plan's loop does not cover.
            assert!(
                visible_to(capability, classification),
                "a fresh session must start with capability >= classification"
            );
            for incoming in binds {
                if !bind_allowed(incoming, classification) {
                    continue; // Gate A refuses; state unchanged
                }
                capability = incoming; // the bind succeeds
                classification = classification.max(floor(capability)); // Gate B ratchets
                assert!(
                    visible_to(capability, classification),
                    "capability {capability:?} fell below classification {classification:?}"
                );
            }
        }
    }
}
