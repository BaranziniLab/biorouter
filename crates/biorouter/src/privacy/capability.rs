use super::affiliation::ModelAffiliation;
use super::{ExtensionClassification, ProviderTier};
use crate::agents::types::SharedProvider;

/// The capability decision for ONE tool call, taken once and carried.
///
/// `Copy` is load-bearing: this has to thread into `async move` blocks that own
/// no `&self` (`extension_manager.rs`'s dispatch future, `agent.rs`'s wrapper).
///
/// It exists because the capability decision has FOUR readers on one call's
/// path — the Agent seam, Gate C, the built-in `_meta` bit, and the Platform
/// extensions — and every one of them used to re-read the provider mutex at a
/// different program point. `Agent::update_provider` takes that mutex, assigns
/// and drops it with no turn lock and no generation counter, and `biorouterd`
/// runs the multi-thread runtime, so the swap and a dispatch genuinely run in
/// parallel. Worse, the last of those reads happens *inside the driven future*,
/// past `tool_dispatch_limits::acquire` — an unbounded wall-clock gap, not
/// microseconds. Any read-then-read of shared mutable state across two program
/// points is a race; so there is one read, and it is carried.
///
/// The master toggle is a second axis with the identical defect, which is why
/// it is captured here too rather than re-read per gate: without that, one call
/// could pass Gate C with tiers **on** and then build an empty path policy with
/// tiers **off**.
///
/// **What this does NOT close** is AR-13: the sample is taken at *permit* time
/// and the tool runs later, so threading makes the call *consistent* — all
/// consumers agree — not *current*. Re-sampling at execution time to fix that
/// is exactly the bug, because it is what lets a Public-admitted call run
/// Private.
///
/// ⚠ **`affiliation` must stay [`Copy`]-compatible.** DR-26's third axis rides
/// here rather than being looked up per gate, and the natural encodings for it
/// — a `BTreeSet<String>` of institutions, or an owned `String` id — would break
/// the derive above and cascade into every gate signature. [`InstitutionId`] is
/// an interned `&'static str` for exactly that reason; see
/// `a_capability_is_still_copy_with_an_affiliation_on_it`.
///
/// [`InstitutionId`]: super::affiliation::InstitutionId
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallCapability {
    tier: ProviderTier,
    enforced: bool,
    affiliation: Option<ModelAffiliation>,
    /// DR-33: the tier was lowered by one of the chat's decision models, read
    /// in the same lock as the tier (`decision_lowered_by().is_some()`), so a
    /// refusal can pick its sentence without a second read of the slot. It
    /// decides nothing: the gates read [`Self::tier`]. Set only by
    /// [`Self::sample`].
    lowered_by_decision: bool,
}

impl CallCapability {
    /// The ONE read of the provider mutex and the ONE read of the master toggle
    /// on this call's path. Both, together, at one instant.
    ///
    /// The tier and the affiliation come out of the **same** `as_ref()`, under
    /// the same lock, for the reason this whole type exists: `update_provider`
    /// can reassign the mutex with no turn lock, so two reads would let one call
    /// gate on one model's tier and another model's institution.
    pub async fn sample(provider: &SharedProvider) -> Self {
        // `None` — legitimately the state before the first bind — resolves to
        // Public, the safe direction for every gate that reads this. Its
        // affiliation is `None` too, which is what a public model's is.
        let (tier, affiliation, lowered_by_decision) = provider
            .lock()
            .await
            .as_ref()
            .map(|p| (p.tier(), p.affiliation(), p.decision_lowered_by().is_some()))
            .unwrap_or((ProviderTier::Public, None, false));
        Self {
            tier,
            affiliation,
            enforced: crate::privacy::privacy_tiers_enabled(),
            lowered_by_decision,
        }
    }

    /// For an entry with **no caller identity** — `POST /agent/call_tool`
    /// arrives outside the agent loop, so there is no admitted turn whose
    /// capability it could inherit. Public + enforced is the most restrictive
    /// pair this type can express.
    pub const fn public_enforced() -> Self {
        Self {
            tier: ProviderTier::Public,
            enforced: true,
            // A public model's affiliation is the absence of one, and the tier
            // gates keep this capability away from every private extension
            // regardless.
            affiliation: None,
            lowered_by_decision: false,
        }
    }

    /// Test-only constructor.
    ///
    /// Production code obtains a capability from exactly one of [`Self::sample`]
    /// or [`Self::public_enforced`], and the whole-tree count of those two
    /// spellings under `crates/*/src/` is what pins the production entries. A
    /// test that spelled either of them would be indistinguishable from an
    /// entry nobody classified, so tests build their capability here instead.
    ///
    /// The two tests that *must* name the real constructors — because those
    /// constructors are what they exercise — live in
    /// `crates/biorouter/tests/privacy_capability.rs`, outside the window the
    /// census walks. That is why this file's own `mod tests` names neither.
    ///
    /// ⚠ **The affiliation defaults to the one this tier implies**, which is
    /// `Local` for Private and nothing for Public. Every affiliated provider in
    /// the tree decides `Some(..)` with the very predicate that decides its
    /// Private tier, so this is the pairing production actually produces — and
    /// it is what keeps the ~40 existing call sites of this constructor
    /// asserting about the TIER axis. Defaulting to `None` instead would enrol
    /// all of them in [`Self::cross_affiliation_warning`]'s unstated-model arm,
    /// silently changing what they test. A test that means to exercise DR-26
    /// says so with [`Self::for_test_affiliated`].
    ///
    /// ⚠ Nothing *enforces* that, and it does not need to: `Local` is DR-26's
    /// identity element, so a future test that meant to exercise a mismatch and
    /// reached for this constructor gets a capability compatible with every
    /// extension — and the `expect`/`expect_err` such a test is built around
    /// fails loudly on the `None` it gets back. The default is chosen so the
    /// wrong reach for it is noisy rather than green; the opposite default
    /// (`None`) is the one that would pass silently, by warning where nothing
    /// should.
    #[cfg(test)]
    pub(crate) const fn for_test(tier: ProviderTier, enforced: bool) -> Self {
        let affiliation = match tier {
            ProviderTier::Private => Some(ModelAffiliation::Local),
            ProviderTier::Public => None,
        };
        Self {
            tier,
            enforced,
            affiliation,
            lowered_by_decision: false,
        }
    }

    /// The three axes stated explicitly, for the tests that exercise DR-26.
    #[cfg(test)]
    pub(crate) const fn for_test_affiliated(
        tier: ProviderTier,
        enforced: bool,
        affiliation: Option<ModelAffiliation>,
    ) -> Self {
        Self {
            tier,
            enforced,
            affiliation,
            lowered_by_decision: false,
        }
    }

    /// A Public capability whose tier a decision model lowered, for the tests of
    /// the sentences that name it.
    #[cfg(test)]
    pub(crate) const fn for_test_lowered_by_decision(enforced: bool) -> Self {
        Self {
            tier: ProviderTier::Public,
            enforced,
            affiliation: None,
            lowered_by_decision: true,
        }
    }

    /// The capability an unrelated test's dispatch or `McpMeta` carries. The
    /// most restrictive pair, so a test that quietly starts depending on extra
    /// reach fails instead of passing.
    #[cfg(test)]
    pub(crate) const fn for_test_restricted() -> Self {
        Self::for_test(ProviderTier::Public, true)
    }

    pub const fn tier(&self) -> ProviderTier {
        self.tier
    }

    pub const fn enforced(&self) -> bool {
        self.enforced
    }

    /// DR-33: whether this call's tier is Public because of a decision model
    /// the chat names rather than because of its language model. A refusal
    /// branches on it to say so instead of telling a Versa chat to "switch to a
    /// private model" (design §3.9). The words are read off the bound
    /// provider's `decision_lowered_by()` on the refusal path only.
    pub const fn lowered_by_decision(&self) -> bool {
        self.lowered_by_decision
    }

    /// Whose agreements cover the model this call was admitted on — DR-26's
    /// third axis. `None` is a public model, for which affiliation never
    /// applies; see [`Self::cross_affiliation_warning`] for the one case where
    /// `None` arrives on a *private* tier and what is done about it.
    pub const fn affiliation(&self) -> Option<ModelAffiliation> {
        self.affiliation
    }

    /// The predicate every barrier in this plan asks. Folded here so the tier
    /// and the toggle can never be read at two different instants.
    pub const fn restricts_private_data(&self) -> bool {
        self.enforced && !self.tier.is_private()
    }

    /// **The** cross-affiliation question, for one (call, extension) pair —
    /// DR-26. `Some(warning)` is a mismatch; the string is the copy every
    /// surface states it in.
    ///
    /// This is the narrowing of [`affiliation::compatible`] every gate asks,
    /// and there must be exactly one of it: a gate that hand-compares
    /// affiliations is a second implementation of DR-26's table, and the two
    /// will disagree on the row nobody thought about. It is a method on the
    /// capability rather than a free function so that the three guards below
    /// cannot be forgotten at a call site — each of them, forgotten, produces a
    /// compliance warning about a flow with no institution in it, which is the
    /// prompt fatigue DR-19 rejects.
    ///
    /// The guards, in order:
    ///
    ///  * **The master opt-out.** DR-15: with tiers off nothing is refused, on
    ///    this axis as on the others, read off the same sample.
    ///  * **A public extension holds no institution's data.** Affiliation is
    ///    asked only after tier has been.
    ///  * **A public caller has already been refused** by
    ///    [`refusal::privacy_refusal`](super::refusal::privacy_refusal). Two
    ///    refusals for one crossing would state two different reasons for it,
    ///    and DR-26's is the wrong one — the model is not being kept away from
    ///    another institution's data, it is being kept away from *private* data.
    ///
    /// ⚠ **`None` on a Private tier is not "affiliation never applies".** See
    /// [`affiliation::unstated_model_warning`]; short-circuiting there fails
    /// open in the one case DR-26 exists to catch.
    ///
    /// ⚠ **Task 52 (DR-27): this spelling is where a mismatch becomes a
    /// REFUSAL, so it is filtered by the mixing policy — through
    /// [`affiliation::refusing_mismatch`], which is the one place that setting
    /// is read.** In `open` this answers `None` while
    /// [`Self::cross_affiliation`] below goes on resolving, so the badges, the
    /// marks and the grant route's subject keep working and `open → standard`
    /// re-tightens what is already in flight. The three refusal sites — Gate C's
    /// dispatch denial, `assert_extension_reachable`'s eight entry points and
    /// `extensionmanager__manage_extensions` — therefore read no mode of their
    /// own.
    ///
    /// [`affiliation::compatible`]: super::affiliation::compatible
    /// [`affiliation::unstated_model_warning`]: super::affiliation::unstated_model_warning
    /// [`affiliation::refusing_mismatch`]: super::affiliation::refusing_mismatch
    pub fn cross_affiliation_warning(
        &self,
        extension: &str,
        class: &ExtensionClassification,
    ) -> Option<String> {
        super::affiliation::refusing_mismatch(self.cross_affiliation(extension, class))
            .map(|f| f.warning)
    }

    /// The **resolution**, carrying both renderings — see [`CrossAffiliation`].
    /// Gate E is the caller that needs the budgeted one, because its output is a
    /// tool `description` rather than a sentence a human reads.
    ///
    /// ⚠ **Not the same answer as [`Self::cross_affiliation_warning`] any more,
    /// and the difference is DR-27.** This one is mode-blind on purpose: it says
    /// whether the flow crosses an institutional boundary, which is a fact about
    /// the two endpoints and not about what the user asked Biorouter to do about
    /// it. The mixing policy narrows only the refusal spelling above. So a
    /// caller that DISPLAYS a mismatch reads this, and a caller that REFUSES one
    /// reads that.
    ///
    /// [`CrossAffiliation`]: super::affiliation::CrossAffiliation
    pub fn cross_affiliation(
        &self,
        extension: &str,
        class: &ExtensionClassification,
    ) -> Option<super::affiliation::CrossAffiliation> {
        super::affiliation::gate_cross_affiliation(
            self.enforced,
            self.tier,
            self.affiliation,
            extension,
            class,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::affiliation::{ExtensionAffiliation, InstitutionId, ModelAffiliation};
    use crate::privacy::ExtensionClassification;

    fn stanford() -> ModelAffiliation {
        ModelAffiliation::institution(InstitutionId::new("stanford"))
    }

    fn ucsf_extension() -> ExtensionClassification {
        ExtensionClassification {
            tier: ProviderTier::Private,
            affiliation: ExtensionAffiliation::institution(InstitutionId::new("ucsf")),
        }
    }

    fn unconstrained_private_extension() -> ExtensionClassification {
        ExtensionClassification {
            tier: ProviderTier::Private,
            affiliation: ExtensionAffiliation::Any,
        }
    }

    /// The whole of DR-26 as one call's question: a Stanford-covered model
    /// reaching a UCSF connector is a mismatch, and the copy names both.
    #[test]
    fn a_bound_institution_mismatches_another_institutions_extension() {
        let cap =
            CallCapability::for_test_affiliated(ProviderTier::Private, true, Some(stanford()));
        let warning = cap
            .cross_affiliation_warning("ucsfomopagent", &ucsf_extension())
            .expect("a Stanford model reaching a UCSF connector is a mismatch");
        assert!(warning.contains("ucsfomopagent"), "{warning}");
        assert!(warning.contains("stanford"), "{warning}");
        assert!(warning.contains("ucsf"), "{warning}");
    }

    /// The inversion DR-26 warns about, asked through the capability rather
    /// than through the bare comparison: a local model reaches everything
    /// private, because no transfer occurs at all.
    #[test]
    fn a_local_model_is_compatible_with_every_private_extension() {
        let cap = CallCapability::for_test_affiliated(
            ProviderTier::Private,
            true,
            Some(ModelAffiliation::Local),
        );
        assert_eq!(
            cap.cross_affiliation_warning("ucsfomopagent", &ucsf_extension()),
            None
        );
        assert_eq!(
            cap.cross_affiliation_warning("anything", &unconstrained_private_extension()),
            None
        );
    }

    /// The same institution on both ends is the arrangement everyone approved
    /// — a warning here is the prompt fatigue DR-19 rejects.
    #[test]
    fn the_same_institution_on_both_ends_never_warns() {
        let cap = CallCapability::for_test_affiliated(
            ProviderTier::Private,
            true,
            Some(ModelAffiliation::institution(InstitutionId::new("ucsf"))),
        );
        assert_eq!(
            cap.cross_affiliation_warning("ucsfomopagent", &ucsf_extension()),
            None
        );
    }

    /// The three guards, each of which would otherwise produce a compliance
    /// warning about a flow that has no institution in it.
    #[test]
    fn affiliation_is_asked_only_of_two_private_endpoints_with_the_feature_on() {
        let mismatched =
            CallCapability::for_test_affiliated(ProviderTier::Private, true, Some(stanford()));
        assert!(mismatched
            .cross_affiliation_warning("ucsfomopagent", &ucsf_extension())
            .is_some());

        // DR-15's master opt-out: with tiers off, nothing is refused and
        // nothing warns.
        assert_eq!(
            CallCapability::for_test_affiliated(ProviderTier::Private, false, Some(stanford()))
                .cross_affiliation_warning("ucsfomopagent", &ucsf_extension()),
            None
        );

        // A PUBLIC extension holds no institution's data.
        assert_eq!(
            mismatched.cross_affiliation_warning(
                "developer",
                &ExtensionClassification {
                    tier: ProviderTier::Public,
                    affiliation: ExtensionAffiliation::institution(InstitutionId::new("ucsf")),
                },
            ),
            None
        );

        // A PUBLIC caller has already been refused by the tier gate. Answering
        // here would invent a second, different reason for one refusal.
        assert_eq!(
            CallCapability::for_test_affiliated(ProviderTier::Public, true, Some(stanford()))
                .cross_affiliation_warning("ucsfomopagent", &ucsf_extension()),
            None
        );
    }

    /// ⚠ The combination DR-26's vocabulary says cannot exist — Private tier,
    /// no stated affiliation — must NOT short-circuit to compatible.
    ///
    /// `LeadWorkerProvider` reached exactly that state before `f212cd48`, and a
    /// gate that treats `None` as "affiliation never applies" fails **open** in
    /// the one case DR-26 exists to catch. The safe direction is to clear an
    /// extension with no institutional claim and warn for every named one.
    #[test]
    fn a_private_model_that_states_no_affiliation_clears_only_an_unconstrained_extension() {
        let cap = CallCapability::for_test_affiliated(ProviderTier::Private, true, None);
        assert_eq!(
            cap.cross_affiliation_warning("anything", &unconstrained_private_extension()),
            None
        );
        let warning = cap
            .cross_affiliation_warning("ucsfomopagent", &ucsf_extension())
            .expect("a private model with no stated affiliation may not reach a claimed one");
        assert!(warning.contains("ucsfomopagent"), "{warning}");
        assert!(warning.contains("ucsf"), "{warning}");
    }

    /// `Copy` is load-bearing (`async move` blocks that own no `&self`), and the
    /// affiliation field is the one most likely to break it — a `BTreeSet` or a
    /// `String` there cascades into every gate signature. Pinned where the cause
    /// is visible.
    #[test]
    fn a_capability_is_still_copy_with_an_affiliation_on_it() {
        fn assert_copy<T: Copy>(_: &T) {}
        let cap =
            CallCapability::for_test_affiliated(ProviderTier::Private, true, Some(stanford()));
        assert_copy(&cap);
        let moved = cap;
        assert_eq!(cap.affiliation(), moved.affiliation());
    }

    /// The test constructor's default is `Local` for a private tier and nothing
    /// for a public one, which is what keeps the ~40 existing `for_test` call
    /// sites asserting about the TIER axis: a `None` default would silently
    /// enrol every one of them in the mismatch arm above.
    #[test]
    fn the_test_constructors_default_to_the_affiliation_their_tier_implies() {
        assert_eq!(
            CallCapability::for_test(ProviderTier::Private, true).affiliation(),
            Some(ModelAffiliation::Local)
        );
        assert_eq!(
            CallCapability::for_test(ProviderTier::Public, true).affiliation(),
            None
        );
        assert_eq!(CallCapability::for_test_restricted().affiliation(), None);
    }

    #[test]
    fn the_predicate_is_the_conjunction_of_both_axes() {
        use ProviderTier::{Private, Public};
        // Enforced + public reach: the only combination a barrier stops.
        assert!(CallCapability::for_test(Public, true).restricts_private_data());
        // Private reach may read private data even with the feature on.
        assert!(!CallCapability::for_test(Private, true).restricts_private_data());
        // With the master toggle off nothing is refused, in either direction.
        assert!(!CallCapability::for_test(Public, false).restricts_private_data());
        assert!(!CallCapability::for_test(Private, false).restricts_private_data());
    }

    // `public_enforced` and `sample` are covered by
    // `crates/biorouter/tests/privacy_capability.rs`. They are tested from
    // there, not from here, so that Task 51's whole-tree census of those two
    // spellings under `crates/*/src/` counts production entries and nothing
    // else — see [`CallCapability::for_test`].

    // -----------------------------------------------------------------------
    // Task 52 (DR-27) — the mixing policy, asked at the one gate that reads it.
    //
    // These live beside the gate rather than in `privacy::mixing` because the
    // claim is about what the GATE does per mode, and because this file is
    // already sanctioned to name `ExtensionAffiliation` by
    // `crates/biorouter/tests/privacy_capability.rs`'s
    // `compatible_is_the_only_function_that_compares_two_affiliations`.
    // -----------------------------------------------------------------------

    // `mixing::pin_for_test` is per THREAD, so none of the three tests below can
    // be seen by the Gate C dispatch tests running beside them and none of them
    // needs `serial_test`. See the pin's own doc.
    use crate::privacy::mixing::{self, MixingPolicy};

    /// Task 52 gate (1) and (4), in one drive because they are the same
    /// sentence read from two ends: `open` does not refuse, and it still
    /// resolves.
    ///
    /// ⚠ **The resolution must survive.** DR-27: *a mode that short-circuits
    /// the resolver would also blind the badges and the audit trail, and would
    /// make `open → standard` fail to re-tighten anything already in flight.*
    /// So [`CallCapability::cross_affiliation`] — the finding, which Gate E's
    /// mark and the bind surface read — answers identically in all three modes,
    /// and only [`CallCapability::cross_affiliation_warning`] — the spelling a
    /// refusal is stated in — goes quiet.
    #[test]
    fn open_suppresses_the_refusal_and_still_resolves_the_mismatch() {
        let cap =
            CallCapability::for_test_affiliated(ProviderTier::Private, true, Some(stanford()));

        let resolved = {
            let _pin = mixing::pin_for_test(MixingPolicy::Standard);
            let finding = cap
                .cross_affiliation("ucsfomopagent", &ucsf_extension())
                .expect("a Stanford model reaching a UCSF connector is a mismatch");
            assert!(
                cap.cross_affiliation_warning("ucsfomopagent", &ucsf_extension())
                    .is_some(),
                "standard is today's behaviour and must be unchanged"
            );
            finding
        };

        let _pin = mixing::pin_for_test(MixingPolicy::Open);
        assert_eq!(
            cap.cross_affiliation_warning("ucsfomopagent", &ucsf_extension()),
            None,
            "`open` still refused a cross-institutional flow"
        );
        let still_resolved = cap
            .cross_affiliation("ucsfomopagent", &ucsf_extension())
            .expect(
                "`open` short-circuited the resolver: the badges and the audit trail read \
                 through this, and re-tightening to `standard` would then have nothing to \
                 re-tighten",
            );
        assert_eq!(still_resolved.warning, resolved.warning);
        assert_eq!(still_resolved.mark, resolved.mark);
    }

    /// Task 52 gate (1), the other two modes. `strict` refuses exactly as
    /// `standard` does at the gate — what it adds is a second proof at the
    /// grant route, which is `routes::agent`'s to assert.
    #[test]
    fn standard_and_strict_both_refuse_and_only_strict_demands_the_password() {
        let cap =
            CallCapability::for_test_affiliated(ProviderTier::Private, true, Some(stanford()));

        {
            let _pin = mixing::pin_for_test(MixingPolicy::Standard);
            assert!(cap
                .cross_affiliation_warning("ucsfomopagent", &ucsf_extension())
                .is_some());
            assert!(
                !mixing::grant_needs_system_authentication(),
                "`standard` is today's behaviour: one in-app confirmation clears the flow"
            );
        }

        let _pin = mixing::pin_for_test(MixingPolicy::Strict);
        assert!(cap
            .cross_affiliation_warning("ucsfomopagent", &ucsf_extension())
            .is_some());
        assert!(
            mixing::grant_needs_system_authentication(),
            "`strict` must cost something real, on top of the in-app confirmation"
        );
    }

    /// Task 52 gate (3) — **the test that fails the most likely wrong
    /// implementation.**
    ///
    /// The tempting shortcut is to make `open` mean "skip the privacy check",
    /// which is the master switch wearing DR-27's clothes. DR-27 forbids it in
    /// as many words: *all three keep the public/private barrier; this setting
    /// governs the affiliation axis only.*
    ///
    /// Three separate things are asserted because a wrong implementation could
    /// reach any one of them: the tier refusal still fires, an enforced public
    /// capability still restricts, and the master toggle has not moved.
    ///
    /// ⚠ **The first assertion is what makes the rest of it mean anything**, and
    /// it was added after review pointed out that this test passed identically
    /// with the pin removed — a regression pin against a wrong implementation
    /// nobody had written. It asserts, on the SAME fixture and inside the SAME
    /// pin, that `open` really is in force: the affiliation warning goes quiet
    /// here. Only then does "and the tier refusal did not" say something.
    ///
    /// The tier half is asked exactly the way Gate C asks it —
    /// `privacy_refusal(name, class.tier, cap.tier())`, the composition at
    /// `ExtensionManager::assert_extension_reachable` — rather than through a
    /// re-derivation that could agree with itself.
    #[test]
    fn open_is_not_the_master_switch() {
        let before = crate::privacy::privacy_tiers_enabled();
        let _pin = mixing::pin_for_test(MixingPolicy::Open);

        // The pin is in force: on the AFFILIATION axis, `open` is silent.
        let stanford_private =
            CallCapability::for_test_affiliated(ProviderTier::Private, true, Some(stanford()));
        assert_eq!(
            stanford_private.cross_affiliation_warning("ucsfomopagent", &ucsf_extension()),
            None,
            "the pin did not take, so nothing below distinguishes `open` from any \
             other mode"
        );

        // …and on the TIER axis it changes nothing, asked as Gate C asks it.
        let public = CallCapability::for_test(ProviderTier::Public, true);
        assert!(
            crate::privacy::refusal::privacy_refusal(
                "ucsfomopagent",
                ucsf_extension().tier,
                public.tier(),
            )
            .is_some(),
            "`open` let a PUBLIC model reach a private extension. It governs the \
             affiliation axis only; turning the tier system off is what the master \
             switch does, and Task 52 must not duplicate it"
        );
        assert!(
            public.restricts_private_data(),
            "`open` relaxed the public/private barrier on the capability itself"
        );
        // …and the capability whose affiliation refusal `open` just silenced is
        // still, on the tier axis, exactly what it was: the two axes are decided
        // by different code, and only one of them has a policy.
        assert!(
            !stanford_private.restricts_private_data(),
            "a PRIVATE caller's tier reach changed when the mixing policy moved"
        );
        assert_eq!(
            crate::privacy::privacy_tiers_enabled(),
            before,
            "moving the mixing policy moved the MASTER SWITCH, which is a different \
             control, with a different disclosure and a different write"
        );
    }
}
