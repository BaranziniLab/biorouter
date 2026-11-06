//! Affiliation — the third axis (DR-26). *Under whose agreements?*
//!
//! [`super::ProviderTier`] asks how sensitive a thing is. Affiliation asks a
//! different question, and the two do not compose: a HIPAA-compliant LLM
//! approved at one institution has **no** blanket permission over another
//! institution's PHI. Compliance is established per data flow — by BAAs,
//! subcontractor chains, IRB approvals and DUAs — and it does not transfer
//! because both endpoints happen to be called "private". UCSF's Versa reaching
//! the UCSF OMOP agent is the arrangement everyone approved; the same Versa
//! model reaching another institution's connector is a cross-institutional
//! linkage nobody papered. Both pass every tier gate this campaign built, which
//! is why this cannot be expressed by subdividing the tier lattice.
//!
//! This module is the vocabulary, the single comparison, and the words a
//! mismatch is stated in. It decides nothing on its own: a mismatch **warns and
//! asks**, it does not block (DR-19 applied to the third axis), and the grant
//! and the gates that consult them are the tasks that follow.
//!
//! [`cross_affiliation_warning`] lives here rather than beside a gate because
//! DR-26 makes the warning the product: it must name both institutions
//! specifically enough for the user to act on, and a copy composed at each gate
//! would drift into the shrug ("this may be a compliance risk") the ruling
//! rejects.
//!
//! ⚠ **The inversion to get right.** [`ModelAffiliation::Local`] is the *most*
//! permissive value, not a peer of the institutions — see [`compatible`].

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::sync::{LazyLock, Mutex, PoisonError};

/// The institution an agreement is with, as a normalised slug.
///
/// `"UCSF"`, `"ucsf"` and `" ucsf "` are the same institution. A
/// case-sensitive comparison here would make a mismatch appear between an
/// institution and *itself*, which fails **open** in the worst way: it does not
/// leak anything directly, it trains users to click through the warning until
/// the one that mattered goes by too.
///
/// # Why this is interned rather than a `String`
///
/// [`ModelAffiliation`] must be [`Copy`] — Task 48 puts it on
/// [`super::capability::CallCapability`], whose `Copy` derive is load-bearing
/// because that value threads into `async move` blocks owning no `&self`. An
/// owned `String` or `BTreeSet` on the model side would break every gate
/// signature downstream. So the slug is leaked once, on first sight, and the id
/// is a `Copy` pointer to it.
///
/// ⚠ **The leak is bounded by the caller, not by this type.** Every distinct
/// normalised slug reaching [`Self::new`] is leaked for the life of the
/// process, and nothing here caps their number or their length. That is safe
/// *today* by construction, not by promise: the only institution strings in the
/// tree are a provider's own gateway-host decision and the compiled snapshot
/// beside `PRIVATE_EXTENSIONS`, whose header records that there is no network
/// path to the registry from Rust at all — the only fetch is Electron's
/// `registry:fetch`. Task 47 is where a registry-sourced value first arrives,
/// and the cap belongs **there**, at the parse that admits it, where refusing
/// is meaningful: an institution the registry does not publish is already
/// specified to be a *mismatch*, which is the fail-closed answer.
///
/// It cannot be enforced in this constructor. Truncating an over-long slug or
/// stripping a non-ASCII one rewrites two distinct institutions into one id,
/// which grants a cross-institutional flow that should have warned — strictly
/// worse than leaking the string. See
/// `distinct_institution_names_never_collide`, which fails if anyone adds
/// either.
///
/// ⚠ **Equality compares the string CONTENTS, not the pointer.** That is
/// deliberate belt-and-braces: were the interner ever to hand out two pointers
/// for one slug, pointer equality would make an institution mismatch itself —
/// the precise failure above. Content equality cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstitutionId(&'static str);

impl InstitutionId {
    /// Normalise and intern. Total: every input yields an id, including an
    /// empty one, which matches only other empties and therefore only ever
    /// costs a mismatch warning — the safe direction.
    ///
    /// ⚠ **The normaliser is [`name_to_key`](crate::config::extensions::name_to_key),
    /// reused rather than re-derived.** That function is what makes today's
    /// extension classification work at all; two different normalisers for two
    /// axes is how `cdwagent` ends up classified Private under one and
    /// mismatched under the other.
    pub fn new(name: &str) -> Self {
        Self(intern(crate::config::extensions::name_to_key(name)))
    }

    /// The normalised slug. `'static` because it is interned, so this composes
    /// into a warning message without borrowing the id.
    ///
    /// ⚠ **Normalised is not sanitised, and this is the identity, not display
    /// text.** [`name_to_key`](crate::config::extensions::name_to_key)
    /// lowercases and strips whitespace; it does not remove control characters
    /// or bidi overrides, and it must not — a lossy rewrite here would collide
    /// two institutions (above). Task 47 specifies that an institution the
    /// registry does not publish is surfaced **raw** in the warning, so the
    /// composer of that warning is what escapes for its medium. DR-26 requires
    /// the warning name both institutions specifically enough to act on, and a
    /// slug that reorders the sentence around it fails that requirement.
    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for InstitutionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Serialize for InstitutionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for InstitutionId {
    /// Deserialising is a third door a raw institution string comes through,
    /// beside a provider decider and the registry snapshot, and it normalises
    /// like the other two. A `"UCSF"` in `registry.json` that landed as an id
    /// distinct from `ucsf` would be the self-mismatch this type exists to
    /// prevent, arriving by a different route.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::new(&raw))
    }
}

/// Interned slugs, so a repeated institution is leaked once rather than per
/// construction. Poisoning is recovered from rather than propagated: this table
/// is an allocation cache, a panic elsewhere tells us nothing about its
/// contents, and a gate that panicked here would fail a call that should merely
/// have warned.
static INTERNER: LazyLock<Mutex<HashSet<&'static str>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn intern(slug: String) -> &'static str {
    let mut table = INTERNER.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(existing) = table.get(slug.as_str()) {
        return existing;
    }
    let leaked: &'static str = Box::leak(slug.into_boxed_str());
    table.insert(leaked);
    leaked
}

/// A **set** of institutions, interned so that the whole set stays [`Copy`].
///
/// # Why the model side is a set at all
///
/// DR-26's model side used to be one institution, and a lead/worker composite
/// spanning two of them had no representable answer:
/// [`crate::providers::composite_affiliation`] returned a fake institution id
/// whose only virtue was that no real extension was tagged with it. That is a
/// sentinel doing the work of a missing variant, and it is wrong in a specific,
/// silent way — it is safe only while nobody registers an institution with that
/// id, which is a property of *a string nobody has chosen yet* rather than of
/// the design. It also made the one legitimate cross-institutional flow
/// impossible to express: a connector whose allowlist names **both**
/// institutions is exactly what a DUA papers, and a pair covered by both is
/// exactly who may use it — yet the sentinel was in no allowlist, so that flow
/// was refused along with the ones that should be.
///
/// # Why it is interned rather than a plain `BTreeSet`
///
/// The same reason [`InstitutionId`] is. [`ModelAffiliation`] must stay [`Copy`]
/// because it rides on [`super::capability::CallCapability`], whose `Copy`
/// derive is load-bearing — that value threads into `async move` blocks owning
/// no `&self`. A `BTreeSet` there would break the derive and cascade into every
/// gate signature downstream. So the set is leaked once, on first sight, and
/// this is a `Copy` pointer to it.
///
/// ⚠ **The leak is bounded by the caller, not by this type** — as for
/// [`InstitutionId`], and one step more sharply, because the number of *sets* is
/// combinatorial in the number of ids. It is safe today by construction: the
/// only sets this build creates are one singleton per affiliated provider and
/// one union per distinct lead/worker pair, over ids that come from a provider's
/// own gateway-host decision and the compiled registry snapshot. The cap belongs
/// at the parse that admits a registry-sourced institution, for the reason
/// [`InstitutionId::new`] records.
///
/// ⚠ **Equality compares the set CONTENTS, not the pointer.** The derives on a
/// `&'static T` delegate to `T`, so `PartialEq`, `Ord` and `Hash` all read
/// through to the `BTreeSet`. That is the belt-and-braces [`InstitutionId`]
/// keeps for the same reason: were the interner ever to hand out two pointers
/// for one set, pointer identity would make a set mismatch *itself*, which fails
/// open by training users to click through the warning.
///
/// ⚠ **It is never empty**, and that is enforced by the constructors rather than
/// hoped for. [`compatible`] asks whether the model's set is a *subset* of the
/// extension's allowlist, and the empty set is a subset of everything — so an
/// empty model affiliation would reach every private extension in the build,
/// which is `Local`'s blanket permission granted to a model that is not local.
/// [`Self::new`] answers `None` instead, which forces the caller to say what it
/// means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstitutionSet(&'static BTreeSet<InstitutionId>);

impl InstitutionSet {
    /// The one-institution case, which is every provider this build ships.
    pub fn of(id: InstitutionId) -> Self {
        Self(intern_set(BTreeSet::from([id])))
    }

    /// A set from anything iterable — `None` for an empty one, which is not a
    /// representable model affiliation (see the type's own doc).
    pub fn new(ids: impl IntoIterator<Item = InstitutionId>) -> Option<Self> {
        let set: BTreeSet<InstitutionId> = ids.into_iter().collect();
        if set.is_empty() {
            return None;
        }
        Some(Self(intern_set(set)))
    }

    /// The institutions, ascending. `'static` because the set is interned, so
    /// this composes into a warning without borrowing the handle.
    pub fn iter(&self) -> impl Iterator<Item = InstitutionId> + 'static {
        self.0.iter().copied()
    }

    /// The underlying set, for the one comparison that needs it.
    pub fn as_set(&self) -> &'static BTreeSet<InstitutionId> {
        self.0
    }

    /// The single institution covering this model, or `None` when more than one
    /// does.
    ///
    /// ⚠ **`None` here means "not exactly one", and every caller must treat the
    /// two-or-more case explicitly** rather than folding it onto the
    /// single-institution path. That is why this returns an `Option` instead of
    /// a "first" institution: a spanning pair has no representative, and
    /// silently picking one clears the flow to the other.
    pub fn sole(&self) -> Option<InstitutionId> {
        let mut ids = self.0.iter();
        match (ids.next(), ids.next()) {
            (Some(only), None) => Some(*only),
            _ => None,
        }
    }

    /// Both sets' institutions — the fold a composite provider needs.
    pub fn union(self, other: Self) -> Self {
        Self(intern_set(self.0.union(other.0).copied().collect()))
    }
}

/// Interned sets, on exactly the terms [`INTERNER`] holds ids: an allocation
/// cache whose poisoning is recovered from rather than propagated, because a
/// panic elsewhere says nothing about its contents and a gate that panicked here
/// would fail a call that should merely have warned.
static SET_INTERNER: LazyLock<Mutex<HashSet<&'static BTreeSet<InstitutionId>>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn intern_set(set: BTreeSet<InstitutionId>) -> &'static BTreeSet<InstitutionId> {
    let mut table = SET_INTERNER.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(existing) = table.get(&set) {
        return existing;
    }
    let leaked: &'static BTreeSet<InstitutionId> = Box::leak(Box::new(set));
    table.insert(leaked);
    leaked
}

/// Whose compliance regime covers the model bound right now. Orthogonal to
/// [`super::ProviderTier`] — see DR-26.
///
/// There is no `None` variant: a public model's affiliation is not a value in
/// this type, it is the absence of one. The tier gates already keep a public
/// model away from private data, so affiliation never applies to it, and giving
/// "public" a seat here would invite a gate to compare it with something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModelAffiliation {
    /// Runs on this machine; the data never leaves it.
    Local,
    /// Covered by **every** named institution's agreements — `{ucsf}` for both
    /// Versa providers, derived from the same gateway-host check that decides
    /// their tier, so a Versa module repointed elsewhere loses Private *and*
    /// `ucsf` together.
    ///
    /// A set rather than one institution, mirroring [`ExtensionAffiliation`],
    /// because a lead/worker composite discloses the whole transcript to both
    /// endpoints and is therefore covered by both institutions at once. The two
    /// sets are read in **opposite** directions, which is the whole of
    /// [`compatible`]: this one is what the model *carries*, and the extension's
    /// is what it *admits*.
    ///
    /// ⚠ **Never empty** — see [`InstitutionSet`]. `Institutions(∅)` would be a
    /// subset of every allowlist and so reach every private extension, which is
    /// `Local`'s blanket permission handed to a model that is not local.
    Institutions(InstitutionSet),
}

impl ModelAffiliation {
    /// The one-institution case — `Institutions({id})`. The spelling every
    /// provider in this build uses, and the mirror of
    /// [`ExtensionAffiliation::institution`].
    pub fn institution(id: InstitutionId) -> Self {
        Self::Institutions(InstitutionSet::of(id))
    }

    /// Covered by all of these — `None` for an empty iterator, which is not a
    /// representable affiliation.
    pub fn institutions(ids: impl IntoIterator<Item = InstitutionId>) -> Option<Self> {
        InstitutionSet::new(ids).map(Self::Institutions)
    }

    /// The institutions covering this model, or `None` for [`Self::Local`].
    pub fn institution_set(&self) -> Option<InstitutionSet> {
        match self {
            Self::Local => None,
            Self::Institutions(set) => Some(*set),
        }
    }

    /// The bound institution when there is exactly one, and `None` for
    /// [`Self::Local`] **or** for a model covered by two or more.
    ///
    /// ⚠ **Named `sole_` rather than `institution` because the two-or-more case
    /// must not be silently answered.** A model spanning two institutions has no
    /// representative: picking either one clears the flow to the other, which is
    /// the disclosure DR-26 exists to prevent. Every caller here treats `None`
    /// as "not covered by any single institution" and takes the restrictive
    /// path.
    pub fn sole_institution(&self) -> Option<InstitutionId> {
        match self {
            Self::Local => None,
            Self::Institutions(set) => set.sole(),
        }
    }
}

/// Whose data an extension holds, and therefore which private models may reach
/// it.
///
/// `Institution(x)` from DR-26's table is spelled `Institutions({x})` here —
/// **one shape, not two**, because a second spelling is a second thing every
/// comparison, serialiser and warning would have to handle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ExtensionAffiliation {
    /// Safe for **any** private model. The default for a private extension with
    /// no institutional constraint.
    #[default]
    Any,
    /// An explicit allowlist. A one-element set is the single-institution case;
    /// an **empty** set permits nothing, and is emphatically not [`Self::Any`]
    /// — conflating them would turn a typo in the registry into a granted flow.
    Institutions(BTreeSet<InstitutionId>),
}

impl ExtensionAffiliation {
    /// The single-institution case — `Institutions({id})`.
    pub fn institution(id: InstitutionId) -> Self {
        Self::Institutions(BTreeSet::from([id]))
    }

    /// An allowlist from anything iterable.
    pub fn institutions(ids: impl IntoIterator<Item = InstitutionId>) -> Self {
        Self::Institutions(ids.into_iter().collect())
    }
}

/// Keep only the institutions permitted by both extension-side authorities.
///
/// This is a conservative merge of two descriptions of the same extension,
/// not the model-to-extension reach decision implemented by [`compatible`].
/// `Any` contributes no restriction; two allowlists intersect, including to an
/// empty set that permits no institution.
pub fn restrict_extension_affiliation(
    current: ExtensionAffiliation,
    authority: ExtensionAffiliation,
) -> ExtensionAffiliation {
    match (current, authority) {
        (ExtensionAffiliation::Any, affiliation) | (affiliation, ExtensionAffiliation::Any) => {
            affiliation
        }
        (
            ExtensionAffiliation::Institutions(current),
            ExtensionAffiliation::Institutions(authority),
        ) => {
            ExtensionAffiliation::Institutions(current.intersection(&authority).copied().collect())
        }
    }
}

/// **The** affiliation comparison. Is extension `ext` reachable from a session
/// bound to model affiliation `model`, without a cross-affiliation grant?
///
/// Both endpoints are assumed already Private; affiliation is asked only after
/// tier has been. `false` does not mean "blocked" — it means *mismatch*, which
/// warns and offers the user an explicit, once-per-triple approval (DR-19,
/// DR-26). A blocked-outright design is one researchers route around by turning
/// the feature off, and legitimate cross-institutional work under a real DUA
/// exists.
///
/// There must be exactly **one** of these. A gate that hand-compares
/// affiliations is a second implementation of the table below, and the two will
/// disagree on the row nobody thought about.
///
/// | model | ext | |
/// |---|---|---|
/// | `Local` | anything | compatible |
/// | `Institutions(M)` | `Any` | compatible |
/// | `Institutions(M)` | `Institutions(A)` with `M ⊆ A` | compatible |
/// | `Institutions(M)` | `Institutions(A)` otherwise | **mismatch** |
///
/// ⚠ **SUBSET, and the direction is the whole rule.** A model covered by
/// `{ucsf, stanford}` reaching a `{ucsf}` connector must be **refused**: the
/// Stanford half is in the pipeline, so UCSF's data reaches Stanford. Membership
/// — "the allowlist names one of the model's institutions", which is the obvious
/// reading and what a one-institution model made indistinguishable — allows
/// exactly that flow. For a single-institution model the subset test reduces to
/// the old `contains`, so nothing this build ships changes behaviour; the
/// difference appears only on the pair the previous encoding could not express.
pub fn compatible(model: &ModelAffiliation, ext: &ExtensionAffiliation) -> bool {
    let ModelAffiliation::Institutions(bound) = model else {
        // `Local` is the MOST permissive affiliation, not a peer of the
        // institutions, and it returns here BEFORE any comparison happens.
        //
        // The reason is not a rule about which sets contain what: a local model
        // may reach everything private because **no transfer occurs at all**.
        // There is no disclosure, so there is nothing for an agreement to
        // govern. Expressing this as equality — or as membership in a set that
        // happens to contain `Local` — makes the local model match only itself
        // and breaks the single most important case, while every other row of
        // the table still passes. See DR-26.
        return true;
    };

    match ext {
        ExtensionAffiliation::Any => true,
        // Every institution the model is covered by must be one the extension
        // admits. `allowed.contains(any_of(bound))` would clear a pair that also
        // discloses to an institution the allowlist never named.
        ExtensionAffiliation::Institutions(allowed) => bound.as_set().is_subset(allowed),
    }
}

/// The **union-of-owners** rule (DR-26 / Task 50): may a session bound to
/// `model` reach a subject whose content belongs to *every* institution in
/// `owners`?
///
/// ⚠ **This is not [`compatible`] with a set argument, and the difference is the
/// whole of Step 1.** [`ExtensionAffiliation::Institutions`] is an **allowlist**
/// — a model named in it is compatible. A knowledge base's set, and a chat's,
/// are unions of what they INGESTED or TOUCHED, so every member is an owner the
/// reader must be covered by: a chat that reached both institutions' connectors
/// is recallable from neither of their models. `owners.contains(bound)` — the
/// obvious reading, and literally what the allowlist does — passes every other
/// row and fails exactly there.
///
/// It is nevertheless not a second *ruling*: it is [`compatible`] asked once per
/// owner (through [`owner_compatible`]) and conjoined, which is why it is
/// written that way rather than as its own match. An empty set is therefore
/// compatible with everything (`all` over nothing), which is correct here and
/// the opposite of an empty allowlist.
pub fn owners_compatible(
    model: Option<ModelAffiliation>,
    owners: &BTreeSet<InstitutionId>,
) -> bool {
    owners.iter().all(|owner| owner_compatible(model, *owner))
}

/// One owner of the union, asked of this model: [`compatible`]'s table against
/// the single-institution allowlist an owner is.
///
/// ⚠ **Named and extracted so the decision and its words cannot ask it
/// differently** — which they did. [`owners_compatible`] conjoined this
/// predicate while [`cross_affiliation_owners`] re-asked the *allowlist*
/// question over the whole owner set, and for a model covered by exactly the
/// institutions a subject is owned by those two are not the same question:
/// `M ⊆ O` is true while `M ⊆ {o}` is false for every `o` in `O`. So the rule
/// refused and the composer said there was nothing to warn about, and at chat
/// recall the composer IS the decision.
fn owner_compatible(model: Option<ModelAffiliation>, owner: InstitutionId) -> bool {
    let ext = ExtensionAffiliation::institution(owner);
    match model {
        Some(model) => compatible(&model, &ext),
        // A private model that states nothing mismatches every claimed
        // owner — `unstated_model`'s arm, not a short circuit to compatible.
        None => unstated_model("", &ext).is_none(),
    }
}

/// The words a union-of-owners mismatch is stated in, or `None` when there is
/// none.
///
/// The copy comes from the same two composers every other surface uses, so a
/// chat-recall refusal and a dispatch refusal cannot describe one institutional
/// boundary differently. `subject` is what is being reached — "this chat", "this
/// knowledge base" — and never an id, because §14.4 forbids naming content in a
/// refusal.
///
/// ⚠ **`None` here is not "no sentence available", it is *allowed*.**
/// `chatrecall_extension` refuses exactly when this returns `Some`, so this
/// function is [`owners_compatible`] at that surface rather than a rendering of
/// it. The two must therefore agree on every input, which is why they share
/// [`owner_compatible`] instead of each forming a view — and why the composers
/// below are reached directly rather than through [`cross_affiliation`], which
/// would re-ask a different question about the same sets. Pinned over the whole
/// cross product by
/// `tests::the_words_for_a_union_mismatch_exist_exactly_when_the_rule_finds_one`.
pub fn cross_affiliation_owners(
    model: Option<ModelAffiliation>,
    subject: &str,
    owners: &BTreeSet<InstitutionId>,
) -> Option<CrossAffiliation> {
    // The owners this model is NOT covered by — [`owners_compatible`]'s own
    // conjunct negated, so this set is non-empty exactly when that function
    // says mismatch. "Every mismatch the rule finds has copy" is then a
    // structural property of one predicate rather than something two
    // implementations happen to agree on.
    //
    // It is also what the user needs told: which institutions the subject holds
    // that their model is not covered by, rather than the whole owner list.
    let unmatched: BTreeSet<InstitutionId> = owners
        .iter()
        .copied()
        .filter(|owner| !owner_compatible(model, *owner))
        .collect();
    if unmatched.is_empty() {
        return None;
    }

    // ⚠ **Composed, not re-decided.** [`cross_affiliation`] would ask
    // [`compatible`] again — the ALLOWLIST question — about a set the union rule
    // has already ruled on, and answer `None` for any model that is a subset of
    // it. That is how a model spanning exactly a chat's owners came to be
    // refused by the ingest gate and cleared by the recall gate; going through
    // the composers directly makes the disagreement unrepresentable.
    match model {
        Some(ModelAffiliation::Institutions(bound)) => {
            Some(compose_mismatch(bound, subject, &unmatched))
        }
        // `Local` cannot reach here: `owner_compatible` clears it against every
        // owner, so `unmatched` is empty and the guard above returned. Written
        // out rather than folded into a catch-all, so a model variant added
        // later has to be answered here instead of inheriting a `None` that
        // would read as "no mismatch".
        Some(ModelAffiliation::Local) => None,
        None => Some(compose_unstated(subject, &unmatched)),
    }
}

/// This model affiliation as the crate boundary carries it (DR-26 / Task 50
/// Step 0), or `None` when there is nothing to say and the `_meta` key is
/// omitted.
///
/// ⚠ **`biorouter-mcp` cannot see this module**, so the third axis reaches an
/// MCP server the same way the tier does: as a `_meta` key whose spelling is
/// owned by the READER's crate. This function is the one translation, and the
/// wire word is produced by
/// [`biorouter_mcp::knowledge::affiliation::capability_meta_value`] rather than
/// composed here — the correction [`super::super::knowledge`]'s tier key
/// records, applied to this axis from the start.
///
/// `None` — a public model, or a private one that stated nothing — leaves the
/// key off, which the reader treats as
/// [`CallerAffiliation::Unstated`](biorouter_mcp::knowledge::affiliation::CallerAffiliation::Unstated):
/// the most restrictive caller in that table, and the same direction absence
/// takes on the tier key.
pub fn caller_affiliation_meta_value(model: Option<ModelAffiliation>) -> Option<String> {
    biorouter_mcp::knowledge::affiliation::capability_meta_value(&caller_affiliation(model))
}

/// This model affiliation in the vocabulary `biorouter-mcp` owns.
///
/// Split out from [`caller_affiliation_meta_value`] so the cross-crate agreement
/// tests can drive the two tables against each other without going through the
/// wire.
///
/// ⚠ **A model covered by two or more institutions crosses as
/// [`CallerAffiliation::Unstated`], and that is exact rather than a
/// simplification.** The wire word is `institution:<id>`, singular, and the far
/// side's rule is the union-of-owners one: a base owned by `S` is reachable iff
/// the caller clears every owner in `S`. For a model covered by `M` with
/// `|M| ≥ 2` that is `∀o ∈ S: M ⊆ {o}`, which is false for every non-empty `S`
/// and true for the empty one — precisely `Unstated`'s row
/// (`owners.is_empty()`). So no reachability answer is lost.
///
/// ⚠ **The alternative is worse, which is why this is not a stopgap.** Joining
/// the ids into one wire value (`institution:stanford+ucsf`) would be read by
/// any reader as a *real institution* with that name — and this tree ships a
/// separately-installed PATH CLI that routinely lags the app, so old readers are
/// the normal case, not the edge. Such a reader would then hand that fake id to
/// `contributed_owners` and stamp it onto a knowledge base as an owner **no
/// model can ever match**, leaving the base permanently unreachable with no
/// declassification path — the failure that crate's own docs name. Omitting the
/// key says "this side cannot tell you", which is true and which every reader
/// already handles restrictively.
///
/// ⚠ **What it does cost, stated rather than buried:** `contributed_owners`
/// gives `Unstated` no owners, so a spanning model writing into a knowledge base
/// ratchets nothing. The precise answer would be "add both", and carrying it
/// needs a set-valued value on the wire — a protocol change on a boundary whose
/// readers lag. That is the follow-up; contributing nothing is what an older
/// daemon already does and is not a new hole.
pub fn caller_affiliation(
    model: Option<ModelAffiliation>,
) -> biorouter_mcp::knowledge::affiliation::CallerAffiliation {
    use biorouter_mcp::knowledge::affiliation::CallerAffiliation;
    match model {
        Some(ModelAffiliation::Local) => CallerAffiliation::Local,
        Some(ModelAffiliation::Institutions(set)) => match set.sole() {
            Some(id) => CallerAffiliation::Institution(id.as_str().to_string()),
            None => CallerAffiliation::Unstated,
        },
        None => CallerAffiliation::Unstated,
    }
}

/// The published display name for an institution id, or `None` if the registry
/// does not publish one.
///
/// The map is generated into the compiled snapshot beside `PRIVATE_EXTENSIONS`
/// (`registry_private::INSTITUTIONS`) from `registry.json`'s `institutions`
/// field, so there is no second hand-maintained list to drift.
///
/// ⚠ **`None` is not an error and must never be treated as one.** Task 47: an
/// affiliation naming an institution the registry does not publish is a
/// *mismatch* whose raw id is surfaced — failing open on a typo is how a real
/// constraint disappears. The absence of a display name changes only how the
/// institution is rendered, never whether it counts.
pub fn institution_display_name(id: InstitutionId) -> Option<&'static str> {
    super::registry_private::INSTITUTIONS
        .iter()
        .find(|(published, _)| *published == id.as_str())
        .map(|(_, name)| *name)
}

/// How an institution is written in a warning: `UCSF (ucsf)` when the registry
/// publishes a name for it, and the bare id when it does not.
///
/// The id is included even when a display name exists, because the id is what
/// appears in `registry.json`, in `baam.html`'s `data-affiliation` and in a
/// support conversation — a user asked to accept a cross-institutional risk
/// should be able to match the warning to the record.
fn label(id: InstitutionId) -> String {
    match institution_display_name(id) {
        Some(name) => format!("{name} ({id})"),
        None => id.as_str().to_string(),
    }
}

/// Who an extension's allowlist says owns its data, as warning prose.
///
/// Shared by both composers below rather than written twice: they differ in what
/// they say about the MODEL, and letting the extension half drift between them
/// would leave one warning naming an institution by display name and the other
/// by raw id for the same connector.
fn owners_label(owners: &BTreeSet<InstitutionId>) -> String {
    if owners.is_empty() {
        // Not a sentence with a blank in it: an empty allowlist is a real state
        // (a hand-edited snapshot), it permits nothing, and saying so is more
        // useful than naming zero institutions.
        return "no institution at all: its allowlist names none".to_string();
    }
    owners
        .iter()
        .map(|id| label(*id))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One mismatch, in the two renderings its two audiences need.
///
/// ⚠ **They are returned together because they must never disagree about
/// whether there IS a mismatch.** Two composers, each deciding for itself,
/// is the second implementation of DR-26's table this module exists to
/// prevent — and the disagreement would be silent: a tool listed with no mark
/// and then refused at dispatch, or marked and then dispatched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossAffiliation {
    /// The full statement, for a **human** deciding whether to proceed: the
    /// bind prompt, the enablement refusal, Gate C's and Gate F's refusals.
    /// DR-26 requires it be specific enough to act on, so it names every owning
    /// institution and spells out why compliance does not transfer.
    pub warning: String,
    /// The bounded form Gate E prepends to a tool **description**.
    ///
    /// ⚠ **A tool description is a bounded field on a real API.** Versa Azure
    /// carries a private tier here and is an Azure OpenAI deployment, whose
    /// function `description` is capped at 1024 characters; the full
    /// [`Self::warning`] is ~460 of them and Gate E prepends it to *every* tool
    /// of the mismatched extension. Marking a tool must not be the thing that
    /// makes the whole request unsendable — that is a worse failure than the one
    /// being prevented, and it arrives as an opaque provider error.
    ///
    /// It is also the DR-19 answer to the objection that
    /// `get_extensions_info` refuses to mark instructions to keep a compliance
    /// paragraph out of every system prompt: N copies of the paragraph reach
    /// that same prompt through the tool list. The short form is what makes
    /// those two positions consistent.
    pub mark: String,
}

/// The byte ceiling for [`CrossAffiliation::mark`].
///
/// Chosen against the smallest real cap the mark can meet — Azure OpenAI's
/// 1024-character function `description` — with room left for the tool's own
/// description, which is the text the mark must not crowd out. It is asserted
/// on the composer rather than at the surface because the composer is what
/// grows.
pub const MARK_BUDGET: usize = 320;

/// How many owning institutions the **mark** names before summarising.
///
/// The mark is the only rendering with a hard ceiling, and an allowlist is the
/// only unbounded input to it. The complete list is never lost — it stays in
/// [`CrossAffiliation::warning`], which is what the user reads and what the
/// refusal carries.
const MARK_OWNERS_SHOWN: usize = 2;

/// `"A"`, `"A and B"`, `"A, B and C"` — a list a person reads, not a `join`.
fn join_and(mut parts: Vec<String>) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts.remove(0),
        _ => {
            let last = parts.pop().expect("a list of two or more has a last");
            format!("{} and {last}", parts.join(", "))
        }
    }
}

/// Whose agreements cover the bound MODEL, as warning prose.
///
/// ⚠ **The possessive is per institution, not on the joined phrase.** "UCSF and
/// Stanford's agreements" reads as one joint arrangement; a lead/worker pair
/// spanning two institutions is covered by two *separate* sets of agreements,
/// and the sentence has to say so — that separateness is the entire reason the
/// pair is a mismatch for either institution's connector alone.
///
/// For one institution this renders exactly [`label`] plus `'s`, so every
/// sentence this build ships today is unchanged to the byte.
fn covering_agreements(bound: InstitutionSet) -> String {
    join_and(bound.iter().map(|id| format!("{}'s", label(id))).collect())
}

/// A MODEL's whole affiliation as prose, including the two shapes
/// [`covering_agreements`] cannot express — [`ModelAffiliation::Local`] and the
/// absence of a stated affiliation (issue #56, DR-31).
///
/// ⚠ **The model side, never the extension side.** [`owners_label`] renders an
/// allowlist — *whose data this holds* — and this renders what a model is
/// *covered by*. [`compatible`] reads the two in opposite directions, so one
/// renderer for both would invite a sentence that swapped them.
///
/// It exists because DR-31's spawn refusal names an affiliation on **both**
/// sides of a comparison between two models, where the extension-versus-model
/// warnings only ever needed the bound-institution half. The institution
/// rendering itself is [`label`], reused rather than re-derived, so an
/// institution reads the same here as in every cross-affiliation warning —
/// `UCSF (ucsf)` where the registry publishes a display name, the raw slug where
/// it does not (Task 47: an unpublished institution is a *mismatch* surfaced
/// raw, never silently dropped).
///
/// ⚠ **`None` renders as a stated absence, not as an empty phrase.** A private
/// provider with no affiliation is the one pairing this vocabulary says cannot
/// exist, and every gate treats it as *unstated* rather than *unconstrained*. A
/// refusal that rendered it as "" would read as though one side had no opinion,
/// which is the opposite of what the gate just decided.
pub fn model_affiliation_label(model: Option<ModelAffiliation>) -> String {
    match model {
        Some(ModelAffiliation::Local) => "this machine, a local model covered by no \
             institution's agreements because nothing leaves the machine"
            .to_string(),
        Some(ModelAffiliation::Institutions(bound)) => join_and(bound.iter().map(label).collect()),
        None => "no stated institution".to_string(),
    }
}

/// The same institutions for the MARK, which has a byte budget: capped at
/// [`MARK_OWNERS_SHOWN`] exactly as the extension side is, and without the
/// possessive because the mark's sentence does not take one.
///
/// The model side needs the cap for the reason the extension side does — a
/// composite can in principle span more institutions than the mark can name —
/// and the complete list is never lost, because it stays in
/// [`CrossAffiliation::warning`].
fn covered_by_capped(bound: InstitutionSet) -> String {
    let all: Vec<String> = bound.iter().map(label).collect();
    if all.len() <= MARK_OWNERS_SHOWN {
        return join_and(all);
    }
    let shown = all[..MARK_OWNERS_SHOWN].join(", ");
    format!("{shown} and {} more", all.len() - MARK_OWNERS_SHOWN)
}

/// Who an extension's allowlist says owns its data, capped for the mark.
fn owners_label_capped(owners: &BTreeSet<InstitutionId>) -> String {
    if owners.len() <= MARK_OWNERS_SHOWN {
        return owners_label(owners);
    }
    let shown = owners
        .iter()
        .take(MARK_OWNERS_SHOWN)
        .map(|id| label(*id))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{shown} and {} more", owners.len() - MARK_OWNERS_SHOWN)
}

/// The copy shown to a user before a cross-affiliation flow proceeds — `None`
/// when there is nothing to warn about.
///
/// ⚠ **It returns `None` for every compatible pair, and that is the point.** A
/// composer callers had to guard themselves would eventually put a compliance
/// warning on UCSF's Versa reaching the UCSF OMOP agent — the arrangement
/// everyone approved — which is precisely the prompt fatigue DR-19 warns about.
/// The mismatch test and the copy are one call, so they cannot disagree.
///
/// DR-26 requires the warning be specific enough to act on: it names the
/// extension, the institution(s) whose data it holds, and the institution whose
/// agreements cover the bound model. A user can only accept a risk that was
/// stated to them.
///
/// This composes the sentence; it does not decide anything, does not record a
/// grant and does not block. Where it is surfaced — a refusal at dispatch, a
/// mark in discovery, a bind-time prompt — is the tasks that follow.
pub fn cross_affiliation_warning(
    model: ModelAffiliation,
    extension: &str,
    ext: &ExtensionAffiliation,
) -> Option<String> {
    cross_affiliation(model, extension, ext).map(|f| f.warning)
}

/// The same finding as [`cross_affiliation_warning`], carrying **both**
/// renderings — see [`CrossAffiliation`]. Gate E is the caller that needs the
/// short one.
pub fn cross_affiliation(
    model: ModelAffiliation,
    extension: &str,
    ext: &ExtensionAffiliation,
) -> Option<CrossAffiliation> {
    if compatible(&model, ext) {
        return None;
    }
    // Both `let … else` arms are unreachable: `compatible` already returned
    // true for every `Local` model and every `Any` extension, so a mismatch is
    // always a bound institution against an allowlist. They are written as
    // early returns rather than `unreachable!()` because a warning composer is
    // reached from a refusal path, and a panic there converts a control into an
    // outage.
    let ModelAffiliation::Institutions(bound) = model else {
        return None;
    };
    let ExtensionAffiliation::Institutions(owners) = ext else {
        return None;
    };
    Some(compose_mismatch(bound, extension, owners))
}

/// The words for a mismatch that has **already been decided**, with no second
/// opinion about whether there is one.
///
/// ⚠ **Composing and deciding are separate functions for one reason.**
/// [`compatible`] answers the allowlist question, and the union rule
/// ([`owners_compatible`]) is a different question over the same vocabulary. A
/// composer that re-asks the allowlist question can therefore answer "nothing to
/// warn about" for a flow the union rule refused — which it did, for a model
/// covered by exactly the institutions a chat is owned by. Because
/// [`cross_affiliation_owners`] is the decision at chat recall, that silence was
/// a permitted disclosure rather than a missing sentence.
fn compose_mismatch(
    bound: InstitutionSet,
    extension: &str,
    owners: &BTreeSet<InstitutionId>,
) -> CrossAffiliation {
    let held_by = owners_label(owners);
    let covered_by = covering_agreements(bound);

    CrossAffiliation {
        warning: format!(
            "Cross-institutional data flow. The extension `{extension}` holds data belonging to \
             {held_by}, but this chat is bound to a model covered by {covered_by} agreements. \
             Using it would send `{extension}`'s inputs and results across that boundary. \
             Compliance does not transfer between institutions: a model approved at one has no \
             permission over another's data unless a BAA, DUA or IRB approval covers this \
             specific flow. Proceed only if you know one does."
        ),
        // The model's audience needs three facts and no paragraph: whose data
        // this is, whose model it is on, and that calling it will not work
        // unless the user has already accepted this flow. Everything else is in
        // the refusal it gets if it tries anyway.
        //
        // ⚠ The refusal clause is CONDITIONAL because this mark is composed
        // without a session, so it cannot know whether Task 49's grant exists —
        // see `the_mark_does_not_promise_a_refusal_a_grant_may_already_have_cleared`.
        mark: format!(
            "⚠ Cross-institutional: `{extension}` holds data belonging to {}, and this chat's \
             model is covered by {}. A call is refused unless the user has approved \
             this flow; tell them what you need it for and let them decide.",
            owners_label_capped(owners),
            covered_by_capped(bound)
        ),
    }
}

/// **The** gate decision on DR-26's third axis: `Some(warning)` is a mismatch.
///
/// [`super::CallCapability::cross_affiliation_warning`] is this function with
/// the three model axes taken off one sample, and is what every on-the-tool-call
/// path asks. This spelling exists for the surfaces that hold an
/// `Arc<dyn Provider>` directly and have no admitted capability to inherit —
/// `POST /agent/add_extension` is the one DR-26 names. Those callers read the
/// tier and the affiliation off the **same** `Arc`, which is a single read of
/// one object rather than the two program points `CallCapability` exists to
/// collapse, so handing them a sampler would buy nothing.
///
/// ⚠ **Every call to this spelling is a census row** (Task 51):
/// `the_sites_that_decide_how_far_a_caller_reaches_are_exactly_these`, in
/// `crates/biorouter/tests/privacy_capability.rs`, greps `crates/*/src/` for
/// `affiliation::gate_cross_affiliation` and asserts the exact (file, count) set.
/// So reaching for this instead of an admitted [`super::CallCapability`] is a
/// visible, argued choice rather than a quiet one — which it was not until that
/// audit was written, and until then this paragraph claimed a cost that could not
/// be charged.
///
/// There must be exactly one of these. A gate that hand-compares affiliations is
/// a second implementation of DR-26's table, and the two will disagree on the
/// row nobody thought about — pinned by that file's
/// `compatible_is_the_only_function_that_compares_two_affiliations`.
///
/// The three guards and why each is here are documented on
/// [`super::CallCapability::cross_affiliation_warning`].
///
/// ⚠ **Task 52 (DR-27): this is the REFUSAL spelling, so it is narrowed by the
/// mixing policy** — through [`refusing_mismatch`], which is the one place that
/// setting is read. In `open` it answers `None` while
/// [`gate_cross_affiliation`] below goes on resolving. Its two callers are both
/// refusal-shaped in the sense that matters: the subagent spawn DROPS a
/// mismatched extension from the child, and `/agent/add_extension` logs the
/// warning it would otherwise have shown, and neither should happen on a machine
/// whose user asked for cross-institution reach to be silent.
pub fn gate_cross_affiliation_warning(
    enforced: bool,
    model_tier: super::ProviderTier,
    model: Option<ModelAffiliation>,
    extension: &str,
    class: &super::ExtensionClassification,
) -> Option<String> {
    refusing_mismatch(gate_cross_affiliation(
        enforced, model_tier, model, extension, class,
    ))
    .map(|f| f.warning)
}

/// **The** place DR-27's mixing policy is read: a resolved mismatch, filtered
/// down to the ones that actually REFUSE (issue #56, Task 52 Step 3).
///
/// ⚠ **One reader, not one per gate.** Three call sites reading a mode is three
/// places to disagree, and the disagreement would be silent — a tool marked at
/// discovery and then dispatched, or listed and then refused. Every spelling of
/// "the warning a refusal is stated in" runs through here: this module's
/// [`gate_cross_affiliation_warning`],
/// [`super::CallCapability::cross_affiliation_warning`] (the same gate with the
/// three model axes taken off one sample), and — via [`refusing_mismatches`] —
/// the agent's bind-time statement.
///
/// ⚠ **Generic over what is being refused, deliberately.** Not every caller
/// holds a [`CrossAffiliation`] by the time it decides whether to speak: the bind
/// surface holds a list of already-rendered warnings. Widening the type is what
/// let that caller join this reader instead of asking
/// [`super::mixing::mismatch_refuses`] itself, which would have been the second
/// place to disagree.
///
/// ⚠ **It takes the finding rather than the inputs, and that is what keeps
/// `open` from becoming a short circuit.** The resolution has already happened
/// by the time this is called, and [`gate_cross_affiliation`] still returns it —
/// so Gate E's mark, the bind surface and the grant route's subject all keep
/// seeing the mismatch in `open`, which is what DR-27 requires: *the
/// compatibility check still runs, still resolves, and the result is still
/// available to the UI for display.* A mode that returned early from the gate
/// would blind those, and would leave `open → standard` with nothing to
/// re-tighten.
///
/// ⚠ **`open` is not the master switch.** This narrows the *affiliation* axis
/// and touches no other: the tier refusal beside it
/// ([`super::refusal::privacy_refusal`]) never consults the policy, so a public
/// model is still refused a private extension in every mode.
pub fn refusing_mismatch<T>(finding: Option<T>) -> Option<T> {
    finding.filter(|_| super::mixing::mismatch_refuses())
}

/// [`refusing_mismatch`] for a whole surface's worth of mismatches at once:
/// everything a caller was about to refuse or warn over, or nothing at all.
///
/// Defined in terms of the single reader rather than beside it, so the count of
/// places that ask the mixing policy stays at one.
///
/// Its caller is `Agent::cross_affiliation_warnings`, the bind-time statement.
/// That is a *refusal* spelling in the sense that matters — it is the sentence a
/// user is shown so they can decide whether to proceed — so DR-27's `open`, which
/// is "allowed **silently**", must not produce it. The RESOLUTION behind it is
/// untouched and stays mode-blind: `ExtensionManager::extension_reach` still
/// marks, and `Agent::cross_affiliation_grant_subject` still finds its subject,
/// in all three modes.
pub fn refusing_mismatches<T>(found: Vec<T>) -> Vec<T> {
    refusing_mismatch(Some(found)).unwrap_or_default()
}

/// The **resolution** on DR-26's third axis, carrying both renderings — see
/// [`CrossAffiliation`]. The three guards live here, and
/// [`gate_cross_affiliation_warning`] is this function plus DR-27's filter with
/// a field selected off it.
///
/// ⚠ **Mode-blind on purpose** (Task 52). This says whether the flow crosses an
/// institutional boundary, which is a fact about the two endpoints rather than
/// about what the user asked Biorouter to do about it — so Gate E's mark, the
/// bind surface and the grant route's subject keep working in `open`. A caller
/// that DISPLAYS a mismatch reads this; a caller that REFUSES one reads the
/// spelling above.
pub fn gate_cross_affiliation(
    enforced: bool,
    model_tier: super::ProviderTier,
    model: Option<ModelAffiliation>,
    extension: &str,
    class: &super::ExtensionClassification,
) -> Option<CrossAffiliation> {
    if !enforced || !class.tier.is_private() || !model_tier.is_private() {
        return None;
    }
    match model {
        Some(model) => cross_affiliation(model, extension, &class.affiliation),
        None => unstated_model(extension, &class.affiliation),
    }
}

/// The same warning for a **private** model that stated no affiliation at all.
///
/// ⚠ **This arm exists because `None` must not short-circuit to compatible.**
/// DR-26 gives `None` exactly one meaning — a public model, for which the tier
/// gates already hold and affiliation never applies — so a Private tier with no
/// affiliation is a combination the vocabulary says cannot exist. It was
/// nevertheless reachable: `LeadWorkerProvider` overrode `tier()` and not
/// `affiliation()` until `f212cd48`, and the trait default is `None`, so any
/// future provider that overrides one and forgets the other lands here. A gate
/// that reads `None` as "affiliation never applies" fails **open** in precisely
/// the case DR-26 exists to catch.
///
/// The safe direction is to clear an extension with no institutional claim —
/// genuinely unaffected either way — and warn for every named one. It is not
/// *the answer*; the answer is for the provider to state its affiliation.
///
/// ⚠ **This is not the arm a composite spanning two institutions takes**, and
/// the distinction is load-bearing. That composite used to resolve to a sentinel
/// institution that behaved like this arm, which conflated "covered by two
/// institutions" with "covered by nobody we can name" — and cost the pair the
/// one flow it is genuinely entitled to, a connector whose allowlist names both.
/// [`ModelAffiliation::Institutions`] now represents it, and [`compatible`]'s
/// subset rule answers it. What remains here is only the real unknown.
///
/// The copy deliberately does **not** name an institution for the model side.
/// Naming one would be a lie, and DR-26 requires a warning specific enough to
/// act on: "we cannot tell whose agreements cover this model" is actionable
/// (bind a model that says), where an invented institution is not.
pub fn unstated_model_warning(extension: &str, ext: &ExtensionAffiliation) -> Option<String> {
    unstated_model(extension, ext).map(|f| f.warning)
}

/// The same finding as [`unstated_model_warning`], carrying both renderings —
/// see [`CrossAffiliation`].
pub fn unstated_model(extension: &str, ext: &ExtensionAffiliation) -> Option<CrossAffiliation> {
    let ExtensionAffiliation::Institutions(owners) = ext else {
        // `Any` — no institution claims this extension's data, so there is no
        // boundary for an unstated model to cross.
        return None;
    };
    Some(compose_unstated(extension, owners))
}

/// [`compose_mismatch`]'s unstated-model twin, and it is a separate function for
/// the same reason: [`cross_affiliation_owners`] needs the words for a mismatch
/// the union rule already found, without a composer forming its own view of
/// whether there is one.
fn compose_unstated(extension: &str, owners: &BTreeSet<InstitutionId>) -> CrossAffiliation {
    let held_by = owners_label(owners);

    CrossAffiliation {
        warning: format!(
            "Cross-institutional data flow. The extension `{extension}` holds data belonging to \
             {held_by}, and this chat is bound to a private model that does not state whose \
             agreements cover it. Using it would send `{extension}`'s inputs and results to an \
             endpoint no agreement in this build can vouch for. Compliance does not transfer \
             between institutions: a model approved at one has no permission over another's \
             data unless a BAA, DUA or IRB approval covers this specific flow. Bind a model \
             whose institution is known, or proceed only if you know one does."
        ),
        // No institution is named for the model side, here as in the warning:
        // naming one would be a lie, and "we cannot tell" is the actionable
        // statement. The refusal clause is conditional for the reason the stated
        // arm's is.
        mark: format!(
            "⚠ Cross-institutional: `{extension}` holds data belonging to {}, and this chat's \
             model does not state whose agreements cover it. A call is refused unless the user \
             has approved this flow; tell them what you need it for and let them decide.",
            owners_label_capped(owners)
        ),
    }
}

/// What the writer of a private dictionary row knows about the model of the chat it
/// writes from (O-35).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriterModel {
    /// The chat's model was read: `Some` with its affiliation, or `None` for a private
    /// model that states none, which contributes no institution, as `Local` does.
    Known(Option<ModelAffiliation>),
    /// No live agent and no provider in process: the model could not be read.
    Unknown,
}

/// The institutions a private dictionary row records (`dictionary.md`, "Private rows
/// and institutions"): `null`, `{"institutions": []}` or the owners' ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowInstitutions {
    /// `null`: no institution claims the row, so it may shape every private target.
    Unclaimed,
    /// `{"institutions": []}`: the chat's record or model could not be read, or the
    /// union named more than [`DICTIONARY_ROW_MAX_INSTITUTIONS`]; only local models
    /// clear it.
    Unknown,
    /// 1 to 8 institutions whose data the row may hold.
    Owners(BTreeSet<InstitutionId>),
}

/// The most institutions a dictionary row names; a larger union is stored as unknown,
/// which reaches the same targets as any row that names two or more (local models).
pub const DICTIONARY_ROW_MAX_INSTITUTIONS: usize = 8;

impl RowInstitutions {
    /// The ids the store writes: `None` for `null`, an empty list for unknown.
    pub fn stored_ids(&self) -> Option<Vec<String>> {
        match self {
            Self::Unclaimed => None,
            Self::Unknown => Some(Vec::new()),
            Self::Owners(owners) => Some(owners.iter().map(|id| id.as_str().to_string()).collect()),
        }
    }
}

/// The institutions a private dictionary row written from a chat records (O-35, P-16):
/// the union of the institutions behind the chat's model (a lead and worker pair is
/// already folded into one [`ModelAffiliation`]) and those whose connectors the chat
/// touched (`SessionManager::session_affiliations`).
///
/// `Local`, or a private model that states no affiliation, contributes none: the term
/// never left the machine, or no agreement can be named for it, the rule a knowledge
/// base follows for the same two callers (`contributed_owners`). A model covered by two
/// institutions contributes both. An empty union is [`RowInstitutions::Unclaimed`].
///
/// ⚠ **Unknown fails toward the stricter value.** A model that could not be read, or a
/// touched record that answers `Err` (its column could not be read or parsed, which is
/// not "no institutions": chat recall and conversation ingest refuse on it), gives
/// [`RowInstitutions::Unknown`], never `Unclaimed`, which would reach every private
/// target. So does a union of more than 8 ids.
///
/// It compares nothing: which targets a row then reaches is asked of
/// [`gate_cross_affiliation_warning`], one institution at a time, by the bias set
/// (PR-14).
pub fn dictionary_row_institutions(
    model: WriterModel,
    touched: anyhow::Result<BTreeSet<InstitutionId>>,
) -> RowInstitutions {
    let (WriterModel::Known(model), Ok(mut owners)) = (model, touched) else {
        return RowInstitutions::Unknown;
    };
    if let Some(ModelAffiliation::Institutions(set)) = model {
        owners.extend(set.iter());
    }
    match owners.len() {
        0 => RowInstitutions::Unclaimed,
        n if n <= DICTIONARY_ROW_MAX_INSTITUTIONS => RowInstitutions::Owners(owners),
        _ => RowInstitutions::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn inst(s: &str) -> InstitutionId {
        InstitutionId::new(s)
    }

    fn bound(s: &str) -> ModelAffiliation {
        ModelAffiliation::institution(inst(s))
    }

    #[test]
    fn extension_authority_restriction_intersects_without_becoming_reach_logic() {
        let declared = ExtensionAffiliation::institutions([inst("ucsf"), inst("stanford")]);
        let authority = ExtensionAffiliation::institutions([inst("ucsf"), inst("mit")]);
        assert_eq!(
            restrict_extension_affiliation(declared, authority),
            ExtensionAffiliation::institution(inst("ucsf"))
        );
        assert_eq!(
            restrict_extension_affiliation(
                ExtensionAffiliation::Any,
                ExtensionAffiliation::institution(inst("ucsf"))
            ),
            ExtensionAffiliation::institution(inst("ucsf"))
        );
    }

    /// A model covered by several institutions at once — what a lead/worker pair
    /// spanning them folds to.
    fn spanning(names: &[&str]) -> ModelAffiliation {
        ModelAffiliation::institutions(names.iter().map(|n| inst(n)))
            .expect("a spanning fixture names at least one institution")
    }

    fn allowlist(names: &[&str]) -> ExtensionAffiliation {
        ExtensionAffiliation::institutions(names.iter().map(|n| inst(n)))
    }

    /// One fixed-key hasher per call, so two ids hash comparably. A fresh
    /// `RandomState` per id would differ regardless of the values.
    fn hash_of(id: &InstitutionId) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut hasher);
        hasher.finish()
    }

    // ---------------------------------------------------------------- DR-26's
    // compatibility table, one named test per row. The rows are the whole
    // specification of this module; anything else in this file exists to catch
    // an implementation that satisfies the table and is still wrong.

    /// Row 1a: `Local` × `Any`.
    #[test]
    fn local_reaches_an_unconstrained_extension() {
        assert!(compatible(
            &ModelAffiliation::Local,
            &ExtensionAffiliation::Any
        ));
    }

    /// Row 1b, and the mistake this task exists to prevent. An equality test —
    /// or set membership that happens to contain `Local` — passes every other
    /// row in this table and fails exactly here.
    #[test]
    fn local_reaches_every_institutions_extension() {
        assert!(compatible(
            &ModelAffiliation::Local,
            &allowlist(&["stanford"])
        ));
        assert!(compatible(
            &ModelAffiliation::Local,
            &ExtensionAffiliation::institution(inst("ucsf"))
        ));
        assert!(compatible(
            &ModelAffiliation::Local,
            &allowlist(&["stanford", "broad", "ucsf"])
        ));
    }

    /// Row 2: `Institution(X)` × `Any`.
    #[test]
    fn an_institution_model_reaches_an_unconstrained_extension() {
        assert!(compatible(&bound("ucsf"), &ExtensionAffiliation::Any));
    }

    /// Row 3: `Institution(X)` × `Institution(X)`.
    #[test]
    fn an_institution_model_reaches_its_own_institutions_extension() {
        assert!(compatible(
            &bound("ucsf"),
            &ExtensionAffiliation::institution(inst("ucsf"))
        ));
    }

    /// Row 4: `Institution(X)` × `Institutions([… X …])`.
    #[test]
    fn an_institution_model_reaches_an_allowlist_that_names_it() {
        assert!(compatible(
            &bound("ucsf"),
            &allowlist(&["stanford", "ucsf", "broad"])
        ));
    }

    /// Row 5: `Institution(X)` × `Institution(Y)`, X ≠ Y. The operator's case —
    /// UCSF's Versa reaching another institution's private connector.
    #[test]
    fn a_different_institution_is_a_mismatch() {
        assert!(!compatible(
            &bound("ucsf"),
            &ExtensionAffiliation::institution(inst("stanford"))
        ));
        assert!(!compatible(
            &bound("stanford"),
            &ExtensionAffiliation::institution(inst("ucsf"))
        ));
    }

    /// Row 6: `Institution(X)` × `Institutions([…])` without X.
    #[test]
    fn an_allowlist_that_omits_the_bound_institution_is_a_mismatch() {
        assert!(!compatible(
            &bound("ucsf"),
            &allowlist(&["stanford", "broad"])
        ));
    }

    /// ⚠ **Task 56 Step 2, and the row a one-institution build could not state.**
    /// The model side is a SUBSET of the allowlist, not a member of it, and the
    /// direction is the whole rule.
    ///
    /// A pair covered by `{ucsf, stanford}` reaching a `{ucsf}` connector must be
    /// **refused**: the Stanford half is in the pipeline, so UCSF's data reaches
    /// Stanford. `allowlist.contains(any_of(model))` — the obvious reading, and
    /// the one a set-valued model makes newly expressible — allows exactly that
    /// flow, and passes every other row in this table.
    ///
    /// The converse is the flow that must still work: a `{ucsf}` model reaching a
    /// connector whose allowlist names ucsf *and* stanford is a single-institution
    /// disclosure to a connector already cleared for it.
    #[test]
    fn the_model_side_is_a_subset_of_the_allowlist_not_a_member_of_it() {
        let spans = spanning(&["ucsf", "stanford"]);

        // Refused: half the pair is not covered by ucsf's agreements.
        assert!(
            !compatible(&spans, &allowlist(&["ucsf"])),
            "a pair spanning ucsf and stanford discloses to stanford, which ucsf's \
             connector never admitted"
        );
        assert!(!compatible(&spans, &allowlist(&["stanford"])));
        // ...including against an allowlist that names one of them plus a third.
        assert!(!compatible(&spans, &allowlist(&["ucsf", "broad"])));

        // Allowed: the allowlist admits both halves.
        assert!(compatible(&spans, &allowlist(&["ucsf", "stanford"])));
        assert!(compatible(
            &spans,
            &allowlist(&["ucsf", "stanford", "broad"])
        ));
        assert!(compatible(&spans, &ExtensionAffiliation::Any));

        // ...and the other direction, which is what the subset rule must not
        // break: one institution against an allowlist naming several.
        assert!(compatible(
            &bound("ucsf"),
            &allowlist(&["ucsf", "stanford"])
        ));
    }

    /// The empty model set is unrepresentable, and that is a safety property
    /// rather than tidiness.
    ///
    /// [`compatible`] asks `model ⊆ allowlist`, and ∅ is a subset of everything —
    /// so `Institutions(∅)` would reach **every** private extension in the build:
    /// `Local`'s blanket permission handed to a model that is not local, arriving
    /// through a constructor rather than through a decision. The constructor
    /// answers `None` so the caller has to say what it meant.
    #[test]
    fn a_model_covered_by_no_institution_cannot_be_built() {
        assert_eq!(ModelAffiliation::institutions([]), None);
        assert_eq!(InstitutionSet::new([]), None);
        // ...and a non-empty one is built, so the guard is not simply refusing
        // everything.
        assert!(ModelAffiliation::institutions([inst("ucsf")]).is_some());
    }

    /// A model covered by two institutions is covered by neither *alone*, so it
    /// has no sole institution — and every caller that asks for one must get
    /// `None` rather than an arbitrary half.
    ///
    /// Picking either half is the fail-open: it silently clears the flow to the
    /// other institution, which is the disclosure DR-26 exists to prevent.
    #[test]
    fn a_spanning_model_has_no_sole_institution() {
        assert_eq!(
            bound("ucsf").sole_institution(),
            Some(inst("ucsf")),
            "one institution still answers"
        );
        assert_eq!(spanning(&["ucsf", "stanford"]).sole_institution(), None);
        assert_eq!(ModelAffiliation::Local.sole_institution(), None);
    }

    // ------------------------------------------------- The four that catch the
    // real mistakes (Task 45, Step 3).

    /// A case-sensitive comparison would make a mismatch appear between an
    /// institution and itself — which fails OPEN in the worst way, by training
    /// users to click through the warning.
    #[test]
    fn institution_ids_compare_case_insensitively() {
        assert!(compatible(&bound("UCSF"), &allowlist(&["ucsf"])));
        assert!(compatible(&bound("ucsf"), &allowlist(&["UCSF"])));
        assert!(compatible(&bound("UcSf"), &allowlist(&["uCsF"])));
        assert_eq!(inst("UCSF"), inst("ucsf"));
    }

    /// The `" ucsf "` half of the same requirement. `name_to_key` strips **all**
    /// whitespace, not just the ends, which is the discipline this axis reuses
    /// rather than inventing a second normaliser beside it.
    #[test]
    fn institution_ids_ignore_whitespace() {
        assert_eq!(inst(" ucsf "), inst("ucsf"));
        assert_eq!(inst("u c s f"), inst("ucsf"));
        assert_eq!(inst("\tUCSF\n"), inst("ucsf"));
        assert!(compatible(&bound(" UCSF "), &allowlist(&["ucsf"])));
    }

    /// `Any` is the default for a private extension with no institutional
    /// constraint, so it must accept every institution — including ones this
    /// build has never heard of.
    #[test]
    fn any_accepts_every_institution() {
        for name in ["ucsf", "stanford", "broad", "somewhere-new", ""] {
            assert!(
                compatible(&bound(name), &ExtensionAffiliation::Any),
                "Any must accept Institution({name:?})"
            );
        }
    }

    /// The mirror, and it is not symmetric with `Any`. An extension that names
    /// an EMPTY allowlist permits nothing; conflating the two would turn a typo
    /// in the registry into a granted flow.
    #[test]
    fn an_empty_allowlist_is_not_any() {
        let empty = ExtensionAffiliation::Institutions(BTreeSet::new());
        assert_ne!(empty, ExtensionAffiliation::Any);
        for name in ["ucsf", "stanford", "broad"] {
            assert!(
                !compatible(&bound(name), &empty),
                "an empty allowlist must not admit Institution({name:?})"
            );
        }
        // ...and `Local` still reaches it, because `Local` never compares.
        assert!(compatible(&ModelAffiliation::Local, &empty));
    }

    /// `compatible` is total: it answers for every pair, and never panics. A
    /// deterministic generator, so a failure reproduces exactly.
    #[test]
    fn compatible_is_total_over_generated_pairs() {
        let corpus = fuzz_corpus();
        let mut rng = Xorshift(0x5EED_1234_ABCD_0001);
        for _ in 0..4000 {
            let model = if rng.next().is_multiple_of(5) {
                ModelAffiliation::Local
            } else {
                // 1..=3 institutions, so the generator reaches the spanning
                // shapes the sentinel encoding could not express. A generator
                // that only ever produced singletons would leave `compatible`'s
                // subset rule indistinguishable from membership over 4000 draws.
                let n = 1 + (rng.next() % 3) as usize;
                ModelAffiliation::institutions(
                    (0..n).map(|_| inst(rng.pick(&corpus))).collect::<Vec<_>>(),
                )
                .expect("n is at least 1, so the set is never empty")
            };
            let ext = if rng.next().is_multiple_of(4) {
                ExtensionAffiliation::Any
            } else {
                let n = (rng.next() % 4) as usize;
                ExtensionAffiliation::institutions(
                    (0..n).map(|_| inst(rng.pick(&corpus))).collect::<Vec<_>>(),
                )
            };

            // Totality: it returns. (A panic aborts the test.)
            let answer = compatible(&model, &ext);

            // Determinism: a control that answered differently on a second look
            // would be unauditable.
            assert_eq!(answer, compatible(&model, &ext), "{model:?} vs {ext:?}");

            // ...and the answer is DR-26's table, restated independently.
            // Spelled as "every institution the model carries is admitted"
            // rather than as `is_subset`, so this stays a second statement of
            // the rule instead of a copy of the implementation.
            let expected = match (&model, &ext) {
                (ModelAffiliation::Local, _) => true,
                (_, ExtensionAffiliation::Any) => true,
                (
                    ModelAffiliation::Institutions(bound),
                    ExtensionAffiliation::Institutions(set),
                ) => bound.iter().all(|id| set.contains(&id)),
            };
            assert_eq!(answer, expected, "{model:?} vs {ext:?}");
        }
    }

    // --------------------------------------------------------------- Shape and
    // type-level invariants the rest of Phase 6 depends on.

    /// Task 48 puts `ModelAffiliation` on `CallCapability`, whose `Copy` derive
    /// is load-bearing (`capability.rs`: it threads into `async move` blocks
    /// owning no `&self`). An owned `String`/`BTreeSet` inside it would break
    /// every gate signature downstream, so pin it here where the cause is
    /// visible rather than at the far end of the blast radius.
    #[test]
    fn the_model_side_is_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<InstitutionId>();
        // ⚠ Task 56 made the model side set-valued, which is exactly the change
        // that would have broken this had the set not been interned. A
        // `BTreeSet` here is the natural encoding and is not `Copy`.
        assert_copy::<InstitutionSet>();
        assert_copy::<ModelAffiliation>();
    }

    /// [`InstitutionSet`]'s doc claims equality reads the set **contents**, not
    /// the pointer, and rests the interned design's safety on it — the same claim
    /// [`InstitutionId`] makes one level down, and defended here for the same
    /// reason: pointer identity is the obvious "optimisation" for an interner and
    /// would make a set mismatch *itself*, which fails open by training users to
    /// click through the warning.
    #[test]
    fn sets_with_equal_contents_but_distinct_pointers_are_equal() {
        let interned = InstitutionSet::new([inst("ucsf"), inst("stanford")]).expect("non-empty");
        // Bypass the set interner. The interned copy is leaked and still live, so
        // a second allocation of the same contents cannot share its address.
        let bypassed = InstitutionSet(Box::leak(Box::new(
            [inst("stanford"), inst("ucsf")].into_iter().collect(),
        )));
        assert!(
            !std::ptr::eq(interned.0, bypassed.0),
            "fixture is only meaningful if the two pointers really differ"
        );

        assert_eq!(interned, bypassed);
        assert_eq!(
            ModelAffiliation::Institutions(interned),
            ModelAffiliation::Institutions(bypassed)
        );
        // ...and the gate agrees, which is the property that actually matters.
        assert!(compatible(
            &ModelAffiliation::Institutions(bypassed),
            &allowlist(&["ucsf", "stanford"])
        ));
    }

    /// `InstitutionId`'s own doc comment claims equality compares the string
    /// **contents**, not the pointer, and rests the safety of the interned
    /// design on it: were the table ever to hand out two pointers for one slug,
    /// content equality still says "same institution" where pointer identity
    /// would make an institution mismatch *itself* — the fail-open that trains
    /// users to click through the warning.
    ///
    /// Nothing defended that claim. Replacing the derives with pointer
    /// identity — the obvious "optimisation" for an interner, and one that
    /// looks correct because the table normally does return one pointer per
    /// slug — passed every other test in this file (measured, 16/16). So the
    /// stated safety net could have been removed silently. This is the test
    /// that fails instead.
    #[test]
    fn ids_with_equal_contents_but_distinct_pointers_are_equal() {
        let interned = InstitutionId::new("ucsf");
        // Bypass the interner. The interned copy is leaked and therefore still
        // live, so a second allocation of the same bytes cannot share its
        // address — the two pointers differ deterministically, not by luck.
        let bypassed = InstitutionId(Box::leak(String::from("ucsf").into_boxed_str()));
        assert!(
            !std::ptr::eq(interned.0.as_ptr(), bypassed.0.as_ptr()),
            "fixture is only meaningful if the two pointers really differ"
        );

        // `Eq`, which every hand-written comparison would reach for...
        assert_eq!(interned, bypassed);

        // ...`Ord`, which is what `BTreeSet::contains` uses inside
        // `compatible`, so this is the property an actual gate depends on...
        assert!(compatible(
            &ModelAffiliation::institution(interned),
            &ExtensionAffiliation::institution(bypassed)
        ));

        // ...and `Hash`, which must agree with `Eq` or any downstream
        // `HashSet<InstitutionId>` silently holds one institution twice.
        assert_eq!(hash_of(&interned), hash_of(&bypassed));
    }

    /// Two institutions that normalise **differently** must never collapse into
    /// one id. This is the direction that fails open: a collision silently
    /// grants a cross-institutional flow that should have warned, whereas a
    /// spurious distinction only costs a warning the user can clear.
    ///
    /// It is pinned because the tempting answers to an unbounded interner —
    /// truncate an over-long slug, strip a non-ASCII one — are precisely the
    /// lossy transforms that collide, and review proposed them. Neither may be
    /// added to `InstitutionId::new`: a malformed or hostile institution id has
    /// to be **rejected at the parse that admits it** (Task 47, where an
    /// institution the registry does not publish is already a mismatch), never
    /// quietly rewritten into a well-formed one that means something else.
    #[test]
    fn distinct_institution_names_never_collide() {
        let names = [
            "ucsf",
            "ucsfhealth",
            "ucsf-health",
            "ucsf_health",
            "ücsf",
            "stanford",
            "",
            "i",
            "İ",
        ];
        for (i, a) in names.iter().enumerate() {
            for b in &names[i + 1..] {
                assert_ne!(inst(a), inst(b), "{a:?} and {b:?} became one institution");
            }
        }

        // ...including a pair that differs only after a long common prefix,
        // which is exactly what a length cap in the constructor would erase.
        let long_a = format!("{}a", "u".repeat(4096));
        let long_b = format!("{}b", "u".repeat(4096));
        assert_ne!(inst(&long_a), inst(&long_b));
    }

    /// "Represent `Institution(x)` on the extension side as `Institutions({x})`;
    /// one shape, not two." A second spelling is a second thing to compare.
    #[test]
    fn a_single_institution_extension_is_just_a_one_element_allowlist() {
        assert_eq!(
            ExtensionAffiliation::institution(inst("ucsf")),
            allowlist(&["ucsf"])
        );
    }

    /// A private extension with no institutional constraint is safe for any
    /// private model — so the default may not be an allowlist.
    #[test]
    fn the_default_extension_affiliation_is_any() {
        assert_eq!(ExtensionAffiliation::default(), ExtensionAffiliation::Any);
    }

    /// Deserialising is the third place a raw institution string enters the
    /// process (after a provider decider and the registry), and it must
    /// normalise like the other two. A `"UCSF"` in `registry.json` that landed
    /// as a distinct institution from `ucsf` is precisely the self-mismatch
    /// above, arriving by a different door.
    #[test]
    fn deserialising_an_institution_id_normalises_it() {
        let parsed: InstitutionId = serde_json::from_str("\" UCSF \"").unwrap();
        assert_eq!(parsed, inst("ucsf"));
        assert_eq!(serde_json::to_string(&parsed).unwrap(), "\"ucsf\"");
    }

    // ------------------------------------------------ The warning (Task 47).
    //
    // DR-26: a mismatch WARNS, and the warning is the product. "This may be a
    // compliance risk" is a shrug — it must name the institution that owns the
    // extension, the institution whose model is bound, and what will be sent
    // where, because a user can only accept a risk that was stated to them.

    /// A warning is offered for a mismatch and for nothing else. A composer that
    /// produced copy for a compatible pair would put a compliance warning on the
    /// arrangement everyone approved — UCSF's Versa reaching the UCSF OMOP agent
    /// — which is the prompt fatigue DR-19 exists to avoid.
    #[test]
    fn a_compatible_pair_has_no_warning() {
        for (model, ext) in [
            (ModelAffiliation::Local, allowlist(&["stanford"])),
            (ModelAffiliation::Local, ExtensionAffiliation::Any),
            (bound("ucsf"), ExtensionAffiliation::Any),
            (bound("ucsf"), allowlist(&["ucsf"])),
            (bound("ucsf"), allowlist(&["stanford", "ucsf"])),
        ] {
            assert_eq!(
                cross_affiliation_warning(model, "SomeAgent", &ext),
                None,
                "{model:?} vs {ext:?} is compatible and must not warn"
            );
        }
    }

    /// The warning names the extension, the institution that owns its data and
    /// the institution the bound model is covered by — all three, because
    /// dropping any one leaves a sentence the user cannot act on.
    #[test]
    fn the_warning_names_the_extension_and_both_institutions() {
        let warning =
            cross_affiliation_warning(bound("ucsf"), "AtlantisAgent", &allowlist(&["stanford"]))
                .expect("a mismatch must warn");
        assert!(warning.contains("AtlantisAgent"), "{warning}");
        assert!(warning.contains("stanford"), "{warning}");
        assert!(warning.contains("ucsf"), "{warning}");
        assert!(
            warning.contains("UCSF"),
            "the published display name: {warning}"
        );
    }

    /// An institution the registry does not publish has no display name, and the
    /// raw id is surfaced instead. Task 47's Step 2: failing open on a typo is
    /// how a real constraint disappears, so the unknown id must reach the user's
    /// eyes rather than being dropped for being unrenderable.
    #[test]
    fn an_institution_with_no_display_name_is_surfaced_raw() {
        assert_eq!(institution_display_name(inst("ucsf")), Some("UCSF"));
        assert_eq!(institution_display_name(inst("atlantis")), None);

        let warning =
            cross_affiliation_warning(bound("ucsf"), "AtlantisAgent", &allowlist(&["atlantis"]))
                .expect("a mismatch must warn");
        assert!(warning.contains("atlantis"), "{warning}");
    }

    /// Every owner is named, not just the first. An allowlist that omits the
    /// bound institution may still name several, and a user deciding whether a
    /// DUA covers the flow needs all of them.
    #[test]
    fn the_warning_names_every_owner_of_a_multi_institution_allowlist() {
        let warning = cross_affiliation_warning(
            bound("ucsf"),
            "SharedAgent",
            &allowlist(&["broad", "stanford"]),
        )
        .expect("a mismatch must warn");
        assert!(warning.contains("broad"), "{warning}");
        assert!(warning.contains("stanford"), "{warning}");
    }

    /// A model covered by two institutions must have BOTH named — in the warning
    /// and in the mark — or the user is asked to accept a flow that was described
    /// to them as half of itself.
    ///
    /// ⚠ **And each institution keeps its own possessive.** "UCSF and Stanford's
    /// agreements" reads as one joint arrangement; the pair is covered by two
    /// separate ones, and that separateness is exactly why it is a mismatch for
    /// either institution's connector alone.
    #[test]
    fn a_model_covered_by_two_institutions_names_both_of_them() {
        let finding = cross_affiliation(
            spanning(&["ucsf", "stanford"]),
            "AtlantisAgent",
            &allowlist(&["atlantis"]),
        )
        .expect("a pair covered by ucsf and stanford may not reach atlantis's connector");

        assert!(
            finding.warning.contains("UCSF (ucsf)"),
            "{}",
            finding.warning
        );
        assert!(finding.warning.contains("stanford"), "{}", finding.warning);
        assert!(finding.warning.contains("atlantis"), "{}", finding.warning);
        assert!(
            finding.warning.contains("'s and "),
            "each institution keeps its own possessive: {}",
            finding.warning
        );

        // The mark names both too, and still fits the budget it rides in.
        assert!(finding.mark.contains("ucsf"), "{}", finding.mark);
        assert!(finding.mark.contains("stanford"), "{}", finding.mark);
        assert!(
            finding.mark.len() <= MARK_BUDGET,
            "{} bytes: {}",
            finding.mark.len(),
            finding.mark
        );
    }

    /// The model side of the mark is capped exactly as the extension side is, so
    /// a composite spanning more institutions than anyone would build still
    /// cannot make a tool description unsendable. The full list stays in the
    /// warning, which is what the user reads.
    #[test]
    fn a_model_spanning_many_institutions_cannot_blow_the_marks_budget() {
        let many: Vec<String> = (0..64).map(|i| format!("institution-number-{i}")).collect();
        let model = ModelAffiliation::institutions(many.iter().map(|n| inst(n)))
            .expect("64 institutions is not empty");
        let finding = cross_affiliation(model, "someagent", &allowlist(&["atlantis"]))
            .expect("none of the 64 is atlantis");
        assert!(
            finding.mark.len() <= MARK_BUDGET,
            "a 64-institution model produced a {}-byte mark: {}",
            finding.mark.len(),
            finding.mark
        );
        assert!(
            finding.warning.contains("institution-number-63"),
            "the full statement must not be truncated: {}",
            finding.warning
        );
    }

    /// An empty allowlist permits nothing, and says so in words rather than
    /// producing a sentence with a blank where the institution should be.
    #[test]
    fn an_empty_allowlist_warns_without_naming_an_institution_it_does_not_have() {
        let empty = ExtensionAffiliation::Institutions(BTreeSet::new());
        let warning = cross_affiliation_warning(bound("ucsf"), "EmptyAgent", &empty)
            .expect("an empty allowlist admits nothing, so it must warn");
        assert!(warning.contains("EmptyAgent"), "{warning}");
        assert!(warning.contains("no institution"), "{warning}");
    }

    // ------------------------------------------------- Task 48: the gate's own
    // narrowing, and the arm for a model that states nothing.

    /// The unstated arm names the extension's owners exactly as the stated arm
    /// does — the two copies share [`owners_label`] so they cannot drift into
    /// naming one connector's institution by display name in one warning and by
    /// raw id in the other.
    #[test]
    fn the_unstated_model_warning_names_the_extensions_owners_the_same_way() {
        let ucsf = allowlist(&["ucsf"]);
        let stated =
            cross_affiliation_warning(bound("stanford"), "ucsfomopagent", &ucsf).expect("mismatch");
        let unstated = unstated_model_warning("ucsfomopagent", &ucsf).expect("mismatch");
        assert!(stated.contains("UCSF (ucsf)"), "{stated}");
        assert!(unstated.contains("UCSF (ucsf)"), "{unstated}");

        // …and it must NOT invent an institution for the model half. Naming one
        // would be a lie, and a user can only accept a risk that was stated
        // truthfully.
        assert!(!unstated.contains("stanford"), "{unstated}");

        let empty = ExtensionAffiliation::Institutions(BTreeSet::new());
        assert!(unstated_model_warning("EmptyAgent", &empty)
            .expect("an empty allowlist admits nothing, so it must warn")
            .contains("no institution"),);
        // `Any` is the one extension shape an unstated model still clears: no
        // institution claims its data, so there is no boundary to cross.
        assert_eq!(
            unstated_model_warning("developer", &ExtensionAffiliation::Any),
            None
        );
    }

    /// The gate's three guards, asserted on the free function so they hold for
    /// the surfaces that call it without a `CallCapability`
    /// (`POST /agent/add_extension`).
    #[test]
    fn the_gate_asks_affiliation_only_of_two_private_endpoints_with_the_feature_on() {
        use super::super::{ExtensionClassification, ProviderTier};
        let ucsf_private = ExtensionClassification {
            tier: ProviderTier::Private,
            affiliation: allowlist(&["ucsf"]),
        };
        let stanford = Some(bound("stanford"));

        assert!(gate_cross_affiliation_warning(
            true,
            ProviderTier::Private,
            stanford,
            "ucsfomopagent",
            &ucsf_private
        )
        .is_some());
        // The master opt-out.
        assert_eq!(
            gate_cross_affiliation_warning(
                false,
                ProviderTier::Private,
                stanford,
                "ucsfomopagent",
                &ucsf_private
            ),
            None
        );
        // A public caller — already refused on the tier axis, so this must not
        // state a second, different reason for one crossing.
        assert_eq!(
            gate_cross_affiliation_warning(
                true,
                ProviderTier::Public,
                stanford,
                "ucsfomopagent",
                &ucsf_private
            ),
            None
        );
        // A public extension holds no institution's data, whatever a stale
        // allowlist beside it says.
        assert_eq!(
            gate_cross_affiliation_warning(
                true,
                ProviderTier::Private,
                stanford,
                "developer",
                &ExtensionClassification {
                    tier: ProviderTier::Public,
                    affiliation: allowlist(&["ucsf"]),
                }
            ),
            None
        );
    }

    // ------------------------------------------------- The mark (Gate E, DR-26)

    /// ⚠ **The mark rides in a tool DESCRIPTION, which is a bounded field.**
    /// Gate E prepends it to every tool of a mismatched extension, and the
    /// providers that carry a private tier here include Versa Azure — an Azure
    /// OpenAI deployment, whose function `description` is capped at 1024
    /// characters. Prepending the full ~460-character compliance paragraph to a
    /// tool that already describes itself in a few hundred characters turns
    /// "mark this tool" into "this chat can make no request at all", which is
    /// a strictly worse failure than the one DR-26 is preventing.
    ///
    /// So the mark is budgeted, and the budget is asserted here rather than at
    /// the surface, because the composer is what can grow.
    #[test]
    fn the_mark_is_bounded_far_below_a_tool_descriptions_ceiling() {
        let finding = cross_affiliation(bound("stanford"), "ucsfomopagent", &allowlist(&["ucsf"]))
            .expect("a Stanford model reaching a UCSF connector is a mismatch");
        assert!(
            finding.mark.len() <= MARK_BUDGET,
            "the mark is {} bytes, over its {MARK_BUDGET}-byte budget: {}",
            finding.mark.len(),
            finding.mark
        );
        // The full statement is NOT what goes in a description. If these ever
        // become the same string this test still passes on length alone, so the
        // relationship is asserted too.
        assert!(
            finding.mark.len() < finding.warning.len(),
            "the mark must be the SHORT rendering: mark {} vs warning {}",
            finding.mark.len(),
            finding.warning.len()
        );
    }

    /// The budget must survive an allowlist longer than anyone would write, or
    /// it is not a bound — it is a bound on today's registry. The owners are
    /// capped in the MARK only; the full list stays in the warning, which is
    /// what the user and the refusal read.
    #[test]
    fn a_long_allowlist_cannot_blow_the_marks_budget() {
        let many: Vec<String> = (0..64).map(|i| format!("institution-number-{i}")).collect();
        let ext = ExtensionAffiliation::institutions(many.iter().map(|n| inst(n)));
        let finding = cross_affiliation(bound("stanford"), "someagent", &ext)
            .expect("stanford is not on a 64-institution allowlist that omits it");
        assert!(
            finding.mark.len() <= MARK_BUDGET,
            "a 64-institution allowlist produced a {}-byte mark: {}",
            finding.mark.len(),
            finding.mark
        );
        // …and the warning is where the complete list still lives.
        assert!(
            finding.warning.contains("institution-number-63"),
            "the full statement must not be truncated: {}",
            finding.warning
        );
    }

    /// DR-26 requires a warning specific enough to act on, and the mark is the
    /// only form the MODEL reads before a call exists. Shortening it must not
    /// cost the two institution names or the fact that the call will be refused.
    #[test]
    fn the_mark_still_names_both_endpoints_and_the_refusal() {
        let finding = cross_affiliation(bound("stanford"), "ucsfomopagent", &allowlist(&["ucsf"]))
            .expect("a mismatch");
        assert!(finding.mark.contains("ucsfomopagent"), "{}", finding.mark);
        assert!(finding.mark.contains("ucsf"), "{}", finding.mark);
        assert!(finding.mark.contains("stanford"), "{}", finding.mark);
        assert!(finding.mark.contains("refused"), "{}", finding.mark);

        // The unstated-model arm has no institution to name for the model side,
        // and must say so rather than invent one.
        let unstated = unstated_model("ucsfomopagent", &allowlist(&["ucsf"]))
            .expect("a private model that states nothing may not reach a claimed extension");
        assert!(unstated.mark.contains("ucsfomopagent"), "{}", unstated.mark);
        assert!(unstated.mark.contains("ucsf"), "{}", unstated.mark);
        assert!(unstated.mark.contains("refused"), "{}", unstated.mark);
        assert!(
            unstated.mark.len() <= MARK_BUDGET,
            "{} bytes: {}",
            unstated.mark.len(),
            unstated.mark
        );
    }

    /// ⚠ **The mark is read at DISCOVERY, where no session is in hand — so it
    /// must be true both before and after the user accepts the flow.**
    ///
    /// It said *"A call to it will be refused"*, flatly. Task 49 made that false
    /// for a granted triple: Gate C consults `privacy::grant::is_granted` and
    /// lets the dispatch through, while `extension_reach` — which has no session
    /// id to look a grant up with — keeps marking every tool of that extension
    /// with a promise of refusal. The mark is the ONLY thing the model reads
    /// before a call exists, so a model that believes the call cannot succeed may
    /// never make it, and the user's approval silently accomplishes nothing.
    /// That fails closed, which is why it is a product defect rather than a hole,
    /// and DR-26's product IS the accepted flow.
    ///
    /// So the sentence is conditional. It still tells the model the two things
    /// it needs — that this is refused by default, and that the human is the one
    /// who clears it — and it stays true once they have.
    #[test]
    fn the_mark_does_not_promise_a_refusal_a_grant_may_already_have_cleared() {
        for finding in [
            cross_affiliation(bound("stanford"), "ucsfomopagent", &allowlist(&["ucsf"]))
                .expect("a mismatch"),
            unstated_model("ucsfomopagent", &allowlist(&["ucsf"])).expect("a mismatch"),
        ] {
            assert!(
                finding.mark.contains("unless the user has approved"),
                "the mark promises an unconditional refusal, which is false for a granted \
                 triple: {}",
                finding.mark
            );
            // …and it still says what happens by DEFAULT, which is the state
            // every mismatch starts in.
            assert!(finding.mark.contains("refused"), "{}", finding.mark);
            assert!(
                finding.mark.len() <= MARK_BUDGET,
                "{} bytes: {}",
                finding.mark.len(),
                finding.mark
            );
        }
    }

    /// ⚠ **The two renderings must never disagree about whether there IS a
    /// mismatch.** They are born from one decision for exactly this reason; the
    /// test drives the whole table so a future arm cannot produce one without
    /// the other.
    #[test]
    fn the_mark_and_the_warning_are_one_decision_in_two_renderings() {
        let models = [
            ModelAffiliation::Local,
            bound("ucsf"),
            bound("stanford"),
            bound(""),
        ];
        let exts = [
            ExtensionAffiliation::Any,
            allowlist(&[]),
            allowlist(&["ucsf"]),
            allowlist(&["ucsf", "stanford"]),
        ];
        for model in &models {
            for ext in &exts {
                let finding = cross_affiliation(*model, "someagent", ext);
                assert_eq!(
                    finding.is_some(),
                    !compatible(model, ext),
                    "the pair disagreed with the decision for {model:?} × {ext:?}"
                );
                assert_eq!(
                    finding.as_ref().map(|f| f.warning.clone()),
                    cross_affiliation_warning(*model, "someagent", ext),
                    "the warning-only spelling drifted from the pair for {model:?} × {ext:?}"
                );
                if let Some(f) = finding {
                    assert!(!f.mark.is_empty(), "a mismatch with an empty mark");
                    assert!(
                        f.mark.len() <= MARK_BUDGET,
                        "{model:?} × {ext:?} produced a {}-byte mark",
                        f.mark.len()
                    );
                }
            }
        }
    }

    // ------------------------------------- Task 50 Step 0: the crate boundary.
    //
    // `biorouter-mcp` cannot see this module, so DR-26's third axis exists twice
    // — once here as the vocabulary, once there as what a `_meta` key can carry.
    // These two tests are what keep the copies from becoming two different
    // rulings. They live on this side because only this side can import both.

    /// `InstitutionId::new` normalises with `name_to_key`; the reader's crate
    /// re-implements that transform because it cannot import it. Two normalisers
    /// is how `UCSF` and `ucsf` become two institutions and an institution
    /// mismatches **itself** — which fails open in the worst way, by training
    /// users to click through the warning.
    #[test]
    fn the_two_crates_normalise_an_institution_id_identically() {
        use biorouter_mcp::knowledge::affiliation::normalize_institution;
        for name in fuzz_corpus() {
            assert_eq!(
                inst(&name).as_str(),
                normalize_institution(&name),
                "the two crates disagree about the id of {name:?}"
            );
        }
    }

    /// ⚠ **A knowledge base's set is a union of OWNERS, not an allowlist** — a
    /// model matching only one of two owners is a mismatch, where on the
    /// extension side it would be compatible. That is DR-26 Task 50 Step 1, not
    /// a bug, and it is why the two crates hold two functions.
    ///
    /// What must not drift is the *relationship*: a base owned by `S` is
    /// reachable exactly when the model clears `Institution(o)` for **every**
    /// `o` in `S`, which is this module's own table conjoined rather than a
    /// second ruling. Driven over the whole cross product so a future arm on
    /// either side cannot quietly disagree.
    #[test]
    fn the_knowledge_base_rule_is_dr_26s_table_conjoined_over_owners() {
        use biorouter_mcp::knowledge::affiliation::{reachable, KbAffiliation};

        let models = [
            None,
            Some(ModelAffiliation::Local),
            Some(bound("ucsf")),
            Some(bound("stanford")),
            // ⚠ The shape the wire cannot spell. It crosses as `Unstated`, and
            // this is what proves that costs no reachability answer: the
            // conjoined table below is computed from the REAL model, so any
            // divergence between it and what the far side concludes fails here.
            Some(spanning(&["ucsf", "stanford"])),
        ];
        let owner_sets: [&[&str]; 5] = [
            &[],
            &["ucsf"],
            &["stanford"],
            &["ucsf", "stanford"],
            &["ucsf", "stanford", "broad"],
        ];

        for model in models {
            let caller = caller_affiliation(model);
            for owners in owner_sets {
                let kb = KbAffiliation::Owners(owners.iter().map(|o| o.to_string()).collect());

                // This module's table, asked once per owner and conjoined. An
                // unstated model has no `ModelAffiliation`, so its arm is
                // `unstated_model`, which is a mismatch for every claimed owner.
                let expected = owners.iter().all(|owner| {
                    let ext = ExtensionAffiliation::institution(inst(owner));
                    match model {
                        Some(m) => compatible(&m, &ext),
                        None => unstated_model("kb", &ext).is_none(),
                    }
                });

                assert_eq!(
                    reachable(&caller, &kb),
                    expected,
                    "the crates disagree for model {model:?} over owners {owners:?}"
                );
            }
        }
    }

    /// The union-of-owners rule is the allowlist table conjoined — including
    /// the row that separates them, where a model matching ONE of two owners is
    /// compatible with the allowlist and a mismatch with the union.
    #[test]
    fn owners_are_a_union_not_an_allowlist() {
        let both: BTreeSet<InstitutionId> = [inst("ucsf"), inst("stanford")].into_iter().collect();
        let one: BTreeSet<InstitutionId> = [inst("ucsf")].into_iter().collect();
        let none: BTreeSet<InstitutionId> = BTreeSet::new();

        // The separating row. The allowlist says yes; the union says no.
        assert!(compatible(
            &bound("ucsf"),
            &allowlist(&["ucsf", "stanford"])
        ));
        assert!(!owners_compatible(Some(bound("ucsf")), &both));
        assert!(!owners_compatible(Some(bound("stanford")), &both));

        assert!(owners_compatible(Some(bound("ucsf")), &one));
        assert!(!owners_compatible(Some(bound("stanford")), &one));

        // An empty set is `all` over nothing — compatible with everything,
        // which is the opposite of an empty allowlist.
        for model in [None, Some(ModelAffiliation::Local), Some(bound("ucsf"))] {
            assert!(owners_compatible(model, &none), "{model:?}");
        }
        // `Local` reaches everything; an unstated model reaches nothing claimed.
        assert!(owners_compatible(Some(ModelAffiliation::Local), &both));
        assert!(!owners_compatible(None, &one));
    }

    /// Every mismatch the union rule finds has copy, **including** the case the
    /// allowlist composers call compatible — a model matching one of two owners.
    /// A mismatch with no words is a refusal the user cannot act on, and DR-26
    /// makes the warning the product.
    ///
    /// Hand-picked rows, so a failure reads as the row it is. The exhaustive
    /// statement of the same invariant is
    /// [`the_words_for_a_union_mismatch_exist_exactly_when_the_rule_finds_one`]
    /// below; these rows are the ones worth naming.
    #[test]
    fn every_union_mismatch_names_the_institutions_the_model_is_not_covered_by() {
        let both: BTreeSet<InstitutionId> = [inst("ucsf"), inst("stanford")].into_iter().collect();

        let partial = cross_affiliation_owners(Some(bound("ucsf")), "this chat", &both)
            .expect("a model matching one of two owners is a union mismatch");
        assert!(partial.warning.contains("stanford"), "{}", partial.warning);
        assert!(partial.warning.contains("this chat"), "{}", partial.warning);
        assert!(
            partial.mark.len() <= MARK_BUDGET,
            "{} bytes",
            partial.mark.len()
        );

        // The ordinary rows still work, and a compatible pair still has no copy.
        assert!(
            cross_affiliation_owners(Some(bound("mayo")), "this chat", &both)
                .expect("a foreign model mismatches")
                .warning
                .contains("ucsf")
        );
        assert_eq!(
            cross_affiliation_owners(Some(ModelAffiliation::Local), "this chat", &both),
            None
        );
        assert_eq!(
            cross_affiliation_owners(None, "this chat", &BTreeSet::new()),
            None
        );
        assert!(cross_affiliation_owners(None, "this chat", &both).is_some());
    }

    /// The row where the rule and its words used to disagree, named so a
    /// regression reads as itself rather than as one line of a cross product.
    ///
    /// A lead/worker pair covered by `{ucsf, stanford}`, asked for a chat whose
    /// transcript holds **both** institutions' data. [`owners_compatible`]
    /// refuses it — the chat is owned by ucsf, and `{ucsf, stanford} ⊄ {ucsf}` —
    /// and that refusal is right: recalling it would put UCSF's transcript
    /// through the Stanford endpoint and Stanford's through the UCSF one.
    ///
    /// ⚠ **The composer answered `None`, and at chat recall `None` is
    /// "proceed".** It re-asked the ALLOWLIST question over the whole owner set
    /// (`{ucsf, stanford} ⊆ {ucsf, stanford}`, true), which is a different
    /// question from the one the rule conjoins. So the ingest gate refused the
    /// write, the search clause hid the chat, and the recall gate handed it
    /// over.
    #[test]
    fn a_model_spanning_a_chats_owners_is_refused_in_words_not_in_silence() {
        let both: BTreeSet<InstitutionId> = [inst("ucsf"), inst("stanford")].into_iter().collect();
        let model = Some(spanning(&["ucsf", "stanford"]));

        assert!(
            !owners_compatible(model, &both),
            "fixture is only meaningful while the union rule refuses this pair"
        );

        let finding = cross_affiliation_owners(model, "this chat history", &both).expect(
            "a mismatch the union rule found must have copy: at chat recall `None` is not \
             a missing sentence, it is the recall proceeding",
        );
        for named in ["ucsf", "stanford"] {
            assert!(
                finding.warning.contains(named),
                "the warning must name every institution the chat holds data for: {}",
                finding.warning
            );
        }
        assert!(
            finding.mark.len() <= MARK_BUDGET,
            "{} bytes",
            finding.mark.len()
        );
    }

    /// The union rule and the words for it are the **same question**, over every
    /// shape this vocabulary can produce.
    ///
    /// ⚠ **At chat recall the copy IS the decision**, so this is not a coverage
    /// nicety. `chatrecall_extension` refuses exactly when
    /// [`cross_affiliation_owners`] returns `Some`, while the ingest gate reads
    /// [`owners_compatible`] and the search clause is a third spelling of the
    /// same rule — three surfaces, one answer required. A row where the two
    /// disagree is a chat one gate withholds and another hands over, and nothing
    /// but a cross product finds the row nobody thought about.
    #[test]
    fn the_words_for_a_union_mismatch_exist_exactly_when_the_rule_finds_one() {
        let models = [
            None,
            Some(ModelAffiliation::Local),
            Some(bound("ucsf")),
            Some(bound("stanford")),
            Some(bound("mayo")),
            Some(spanning(&["ucsf", "stanford"])),
            Some(spanning(&["ucsf", "stanford", "broad"])),
        ];
        let owner_sets: [&[&str]; 6] = [
            &[],
            &["ucsf"],
            &["stanford"],
            &["mayo"],
            &["ucsf", "stanford"],
            &["ucsf", "stanford", "broad"],
        ];

        for model in models {
            for names in owner_sets {
                let owners: BTreeSet<InstitutionId> = names.iter().map(|n| inst(n)).collect();
                let found = cross_affiliation_owners(model, "this chat history", &owners);
                assert_eq!(
                    found.is_none(),
                    owners_compatible(model, &owners),
                    "the copy and the rule disagree for model {model:?} over owners {names:?}"
                );

                let Some(finding) = found else { continue };
                assert!(
                    finding.warning.contains("this chat history"),
                    "{}",
                    finding.warning
                );
                assert!(
                    finding.mark.len() <= MARK_BUDGET,
                    "{} bytes for {model:?} over {names:?}",
                    finding.mark.len()
                );
                // Every owner this model is NOT covered by is named, so the
                // sentence is one a user can act on rather than a bare refusal.
                // Only that direction is asserted: the model half of the same
                // sentence legitimately names the institutions that DO cover it.
                for owner in &owners {
                    let covered = match model {
                        Some(m) => compatible(&m, &ExtensionAffiliation::institution(*owner)),
                        None => false,
                    };
                    if covered {
                        continue;
                    }
                    assert!(
                        finding.warning.contains(owner.as_str()),
                        "the warning omits {owner}, which {model:?} is not covered by, over \
                         owners {names:?}: {}",
                        finding.warning
                    );
                }
            }
        }
    }

    /// The wire is the only channel, so every value this side can produce must
    /// survive it — including the two that are encoded by the key's absence.
    #[test]
    fn every_model_affiliation_survives_the_crate_boundary() {
        use biorouter_mcp::knowledge::affiliation::{
            caller_affiliation as parse, CallerAffiliation,
        };

        for model in [
            None,
            Some(ModelAffiliation::Local),
            Some(bound("ucsf")),
            Some(spanning(&["ucsf", "stanford"])),
        ] {
            let mut meta = rmcp::model::Meta::default();
            if let Some(wire) = caller_affiliation_meta_value(model) {
                meta.0.insert(
                    biorouter_mcp::knowledge::affiliation::CAPABILITY_AFFILIATION_META_KEY
                        .to_string(),
                    serde_json::Value::String(wire),
                );
            }
            assert_eq!(
                parse(&meta),
                caller_affiliation(model),
                "the wire lost {model:?}"
            );
        }
        // …and a public model, whose affiliation never applies, arrives as the
        // restrictive value rather than as a permit.
        assert_eq!(caller_affiliation_meta_value(None), None);
        assert_eq!(caller_affiliation(None), CallerAffiliation::Unstated);

        // ⚠ The spanning model's encoding, asserted rather than implied. It
        // must be the key's ABSENCE — not a joined id like
        // `institution:stanford+ucsf`, which every reader would take for a real
        // institution and stamp onto a knowledge base as an owner no model can
        // ever match, leaving that base permanently unreachable.
        assert_eq!(
            caller_affiliation_meta_value(Some(spanning(&["ucsf", "stanford"]))),
            None
        );
        assert_eq!(
            caller_affiliation(Some(spanning(&["ucsf", "stanford"]))),
            CallerAffiliation::Unstated
        );
        // ...and a ONE-institution model still crosses as itself, so the arm
        // above narrowed only what it had to.
        assert_eq!(
            caller_affiliation_meta_value(Some(bound("ucsf"))),
            Some("institution:ucsf".to_string())
        );
    }

    // ---------------------------------------------------------------- Fixtures

    /// Inputs chosen to break a normaliser: mixed case, interior and edge
    /// whitespace, non-ASCII with a multi-character lowercasing (`İ`), width
    /// variants, and a long one.
    fn fuzz_corpus() -> Vec<String> {
        let mut corpus: Vec<String> = [
            "",
            " ",
            "\t\n",
            "ucsf",
            "UCSF",
            " ucsf ",
            "UcSf",
            "u c s f",
            "stanford",
            "STANFORD",
            "ucsf-health",
            "ucsf_health",
            "ücsf",
            "ÜCSF",
            "ＵＣＳＦ",
            "İ",
            "i",
            "ß",
            "0",
            "-",
            "local",
            "Local",
            "any",
            "institution",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        corpus.push("u".repeat(4096));
        corpus.push("U".repeat(4096));
        corpus
    }

    struct Xorshift(u64);

    impl Xorshift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn pick<'a>(&mut self, xs: &'a [String]) -> &'a str {
            &xs[(self.next() % xs.len() as u64) as usize]
        }
    }

    /// The stamp table of `dictionary_row_institutions` (P-16, O-35): every model
    /// against every touched set gives the union, `null` when it is empty, and `[]`
    /// when either side cannot be read or the union names more than 8 institutions.
    #[test]
    fn dictionary_rows_record_the_union_of_model_and_connectors() {
        let set =
            |ids: &[&str]| -> BTreeSet<InstitutionId> { ids.iter().map(|s| inst(s)).collect() };
        let owners = |ids: &[&str]| RowInstitutions::Owners(set(ids));
        let pair = ModelAffiliation::institutions([inst("stanford"), inst("ucsf")]).unwrap();
        let models = [
            ("{ucsf}", Some(bound("ucsf"))),
            ("Local", Some(ModelAffiliation::Local)),
            ("unstated", None),
            ("{stanford, ucsf}", Some(pair)),
        ];
        let touched: [&[&str]; 3] = [&[], &["ucsf"], &["stanford"]];
        let want: [[RowInstitutions; 3]; 4] = [
            [
                owners(&["ucsf"]),
                owners(&["ucsf"]),
                owners(&["stanford", "ucsf"]),
            ],
            [
                RowInstitutions::Unclaimed,
                owners(&["ucsf"]),
                owners(&["stanford"]),
            ],
            [
                RowInstitutions::Unclaimed,
                owners(&["ucsf"]),
                owners(&["stanford"]),
            ],
            [
                owners(&["stanford", "ucsf"]),
                owners(&["stanford", "ucsf"]),
                owners(&["stanford", "ucsf"]),
            ],
        ];
        for (row, (label, model)) in models.iter().enumerate() {
            for (column, ids) in touched.iter().enumerate() {
                assert_eq!(
                    dictionary_row_institutions(WriterModel::Known(*model), Ok(set(ids))),
                    want[row][column],
                    "model {label}, touched {ids:?}"
                );
            }
        }

        for model in [
            WriterModel::Unknown,
            WriterModel::Known(Some(bound("ucsf"))),
        ] {
            let unknown = dictionary_row_institutions(model, Err(anyhow::anyhow!("not-json")));
            assert_eq!(
                unknown,
                RowInstitutions::Unknown,
                "a record that cannot be read"
            );
        }
        assert_eq!(
            dictionary_row_institutions(WriterModel::Unknown, Ok(BTreeSet::new())),
            RowInstitutions::Unknown,
            "a model that cannot be read"
        );
        let nine: Vec<String> = (0..9).map(|i| format!("inst{i}")).collect();
        let nine: Vec<&str> = nine.iter().map(String::as_str).collect();
        assert_eq!(
            dictionary_row_institutions(WriterModel::Known(None), Ok(set(&nine))),
            RowInstitutions::Unknown,
            "a union of nine ids"
        );
        assert_eq!(
            dictionary_row_institutions(WriterModel::Known(None), Ok(set(&nine[..8]))),
            owners(&nine[..8])
        );

        assert_eq!(RowInstitutions::Unclaimed.stored_ids(), None);
        assert_eq!(RowInstitutions::Unknown.stored_ids(), Some(vec![]));
        assert_eq!(
            owners(&["UCSF", "stanford"]).stored_ids(),
            Some(vec!["stanford".to_string(), "ucsf".to_string()])
        );
    }
}
