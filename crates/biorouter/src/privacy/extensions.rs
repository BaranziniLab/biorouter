use std::collections::BTreeSet;

use super::affiliation::{ExtensionAffiliation, InstitutionId};
use super::ProviderTier;

/// The compiled marketplace snapshot, as the resolver reads it.
///
/// A struct rather than three direct reads of [`super::registry_private`], so
/// the resolver itself can be exercised against a catalog the real one does not
/// contain. The tree's only two private extensions are both UCSF-tagged, so the
/// rows that decide the most — a private extension with **no** affiliation, and
/// one naming an institution the registry does not publish — would otherwise
/// have no test on the resolver at all, only on a rule extracted beside it.
///
/// ⚠ **It substitutes the two tables the RESOLVER reads, and nothing else.**
/// The institution display-name map is not in here: [`super::affiliation`]
/// reads `registry_private::INSTITUTIONS` directly, so a synthetic snapshot
/// cannot make an institution unpublished. That is why
/// `an_institution_the_registry_does_not_publish_is_a_mismatch_that_names_it`
/// uses `atlantis` — a slug genuinely absent from the real map — rather than
/// relying on this struct to withhold it. Widening the seam to the warning copy
/// would let a test show the user words the shipped build never produces, so
/// the narrower seam is the deliberate one.
struct RegistrySnapshot {
    /// Keys whose extensions must never be admitted to a public session.
    private: &'static [&'static str],
    /// Key → the institutions whose agreements that extension's data is under.
    ///
    /// A key **absent** from here declares nothing, which is
    /// [`ExtensionAffiliation::Any`]. A key present with an **empty** list
    /// permits nothing. The two are opposite answers; see [`affiliation_for`].
    affiliations: &'static [(&'static str, &'static [&'static str])],
}

/// The snapshot the running app resolves against.
const REGISTRY: RegistrySnapshot = RegistrySnapshot {
    private: super::registry_private::PRIVATE_EXTENSIONS,
    affiliations: super::registry_private::EXTENSION_AFFILIATIONS,
};

/// What the registry says about one installed extension: its tier, and — DR-26's
/// third axis — whose agreements its data is under.
///
/// The two travel together because they come from **one** resolution. Two
/// lookups would let them disagree about the same entry, and the disagreement
/// would be silent: a rename is invisible to a name-only affiliation lookup but
/// not to the config-aware tier resolver, so a connector holding UCSF PHI would
/// come back Private *and* unconstrained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionClassification {
    pub tier: ProviderTier,
    pub affiliation: ExtensionAffiliation,
}

/// The single function implementing R11, both halves.
///
/// (i) **Nothing local can grant private.** The tier is resolved from the
///     compiled-in `registry_private::PRIVATE_EXTENSIONS` baseline, never from
///     `config.yaml` and never from the `.brxt` bundle — which self-declares
///     nothing the resolver reads. That baseline is a **generated** file:
///     `landing/scripts/build-registry.mjs` writes it from the `data-privacy` /
///     `data-extension-name` annotations on the BAAM cards, in the same run as
///     `landing/registry.json` and the desktop fallback snapshot, and its
///     `--check` mode fails CI when the three disagree.
/// (ii) **Anything not on BAAM is PUBLIC.** Fail-open, operator ruling. This is
///     the opposite fail direction from `Provider::tier`'s default and the
///     asymmetry is deliberate: an unknown model is a place data might *go*
///     (restrict it); an unknown extension is a place data might *come from*.
///
/// Reversing ruling (ii) later is a one-line change here, by design.
///
/// The daemon-owned marketplace reader supplies the second term of the design's
/// union: `PRIVATE_EXTENSIONS ∪ private(last_good_fetch)`. A valid live,
/// last-good, or embedded catalog may add private identities and tighten their
/// affiliation, while [`super::registry_live`] persists those additions and
/// never accepts a downgrade.
///
/// # Unioned, never replaced — Task 43, DR-23
///
/// DR-23: an extension's tier is **re-derived** from the registry, keyed on a
/// stable identifier, and never written onto the local config entry. That
/// identifier is the BAAM registry `id` the install recorded in
/// [`super::provenance`]. The config entry's own name stays as the fallback for
/// everything installed before that store existed, and for a `.brxt` dropped in
/// by hand, which carries no registry id at all.
///
/// The sources are **unioned**, not tried in order, and that is the
/// load-bearing choice:
///
///  * It closes the bug. Renaming `cdwagent` to `mystuff` in `config.yaml` made
///    this function answer Public, and Gates C, E, F1 and F2 all read this
///    answer — so a rename removed ENFORCEMENT, not a label.
///  * It cannot be turned into a downgrade. A record only ever ADDS a reason to
///    be Private, so forging one, corrupting the store, or deleting it outright
///    leaves the answer at least as restrictive as the config-name join that
///    shipped before. That is why the provenance file needs no gated writer —
///    DR-23's own argument that re-deriving removes the problem instead of
///    guarding it — and it is the same "raises and never lowers" rule Task 37
///    states for registry freshness.
///
/// ⚠ **A stale or absent registry never lowers anything either.** The compiled
/// snapshot remains the baseline, and the daemon's last-good/private-authority
/// records only add reasons to classify an identity as Private. A recorded id
/// no retained source publishes leaves the name-derived answer standing.
///
/// ⚠ **A caller holding an `ExtensionConfig` must use
/// [`classify_extension_entry`] instead.** This name-only form cannot find a
/// record for an entry that was renamed after it was installed, because the
/// name is exactly what the rename changed; the config carries the install
/// directory, which it did not.
pub fn classify_extension(name: &str) -> ProviderTier {
    classify_extension_entry(name, None)
}

/// The resolver, for a caller that has the config as well as the key.
///
/// `key` is the name the caller is asking about — the extension manager's map
/// key, or the name a model named in `manage_extensions`. `config` is the entry
/// behind it when there is one; passing it is what lets a renamed entry still be
/// found, via the install directory in its arguments (see
/// [`super::provenance::registry_ids_for`]).
///
/// Private if **any** of these says so: the key, the config's own declared name,
/// or the registry id of the record found by either of those or by the install
/// directory. Anything else is Public — R11(ii), fail-open.
///
/// ⚠ **The residual, so this is not read as airtight.** The record lives in an
/// ordinary file in the config dir, and DR-17 descoped the filesystem barrier —
/// so renaming the entry AND deleting `extension-provenance.json` returns the
/// answer to the config-name join, which after a rename is Public. Two edits,
/// not one, and not a regression: before this task the rename alone sufficed.
/// It cannot be closed here without storing a tier, which is precisely what
/// DR-23 deleted. `docs/security/privacy-tiers.md` §5.3 states the bar at that
/// height and `the_residual_bar_is_a_rename_plus_removing_the_record` pins it.
pub fn classify_extension_entry(
    key: &str,
    config: Option<&crate::agents::extension::ExtensionConfig>,
) -> ProviderTier {
    resolve_extension(key, config).tier
}

/// **The** registry resolution: tier and affiliation for one config entry, in
/// one call.
///
/// [`classify_extension_entry`] is this function's `tier` field. A caller that
/// needs the affiliation must come here rather than look it up separately —
/// DR-26/Task 47: "affiliation rides the same resolution, in the same call. Two
/// lookups would let tier and affiliation disagree about the same extension."
pub fn resolve_extension(
    key: &str,
    config: Option<&crate::agents::extension::ExtensionConfig>,
) -> ExtensionClassification {
    let identities = identities(key, config);
    let compiled = resolve_identities_in(&REGISTRY, &identities);
    let Some(live_affiliation) = super::registry_live::resolve(&identities) else {
        return compiled;
    };
    ExtensionClassification {
        tier: ProviderTier::Private,
        affiliation: if compiled.tier == ProviderTier::Private {
            restrict_affiliation(compiled.affiliation, live_affiliation)
        } else {
            live_affiliation
        },
    }
}

#[cfg(test)]
fn resolve_in(
    registry: &RegistrySnapshot,
    key: &str,
    config: Option<&crate::agents::extension::ExtensionConfig>,
) -> ExtensionClassification {
    let identities = identities(key, config);
    resolve_identities_in(registry, &identities)
}

fn resolve_identities_in(
    registry: &RegistrySnapshot,
    identities: &[String],
) -> ExtensionClassification {
    let tier = if identities.iter().any(|k| is_private_key(registry, k)) {
        ProviderTier::Private
    } else {
        ProviderTier::Public
    };
    ExtensionClassification {
        tier,
        affiliation: affiliation_for(registry, identities),
    }
}

fn restrict_affiliation(
    compiled: ExtensionAffiliation,
    live: ExtensionAffiliation,
) -> ExtensionAffiliation {
    match (compiled, live) {
        (ExtensionAffiliation::Any, affiliation) | (affiliation, ExtensionAffiliation::Any) => {
            affiliation
        }
        (
            ExtensionAffiliation::Institutions(compiled),
            ExtensionAffiliation::Institutions(live),
        ) => ExtensionAffiliation::Institutions(compiled.intersection(&live).copied().collect()),
    }
}

/// Every key this entry could be known by: the caller's key, the config's own
/// declared name, and the `name_to_key` reduction of every registry id recorded
/// for either of those or for an install directory the config's arguments name.
///
/// ⚠ **The provenance store is consulted unconditionally**, where the tier-only
/// resolver used to skip it once a name matched the private set. That shortcut
/// cannot survive affiliation: it stops at the first identity that proves
/// Private, so the affiliation would be whatever that one identity declared and
/// the answer would depend on which join matched first — the exact
/// first-match-wins bug `a_key_match_does_not_mask_an_install_directory_match`
/// was written to forbid. The cost is one `stat` of a config-dir file (the store
/// itself is stat-cached) for extensions that are already private by name, which
/// is the two the snapshot publishes.
fn identities(
    key: &str,
    config: Option<&crate::agents::extension::ExtensionConfig>,
) -> Vec<String> {
    use crate::config::extensions::name_to_key;
    let mut keys = vec![name_to_key(key)];
    if let Some(config) = config {
        let declared = name_to_key(&config.name());
        if !keys.contains(&declared) {
            keys.push(declared);
        }
    }
    let referenced = config.map(referenced_paths).unwrap_or_default();
    // ⚠ Looked up with the CONFIG keys only. Feeding recorded ids back in would
    // let one record reach another record's row, chaining identities the install
    // never claimed.
    for id in super::provenance::registry_ids_for(&keys, &referenced) {
        let id = name_to_key(&id);
        if !keys.contains(&id) {
            keys.push(id);
        }
    }
    keys
}

/// The union of what every matched identity declares.
///
/// ⚠ **Over all matches, never the first**, for the reason Task 43 landed for
/// the tier: which join matches first is an accident of iteration order, and
/// the losing order is the dangerous one.
///
/// ⚠ **But the union does NOT fail closed here the way it does for the tier,
/// and the two must not be argued for with one sentence.** Union raises a tier
/// because Private is the restrictive end of that lattice. An
/// [`ExtensionAffiliation::Institutions`] set is an *allowlist*, so unioning
/// two of them makes the extension reachable from MORE institutions' models
/// without a warning. An entry whose `args` name another affiliated
/// extension's recorded install directory therefore acquires that extension's
/// institutions — raising its tier and relaxing its affiliation in one step —
/// and `config.yaml` is agent-writable. This is latent rather than live only
/// because every affiliated extension in the compiled snapshot is `ucsf`, so
/// every union that can be formed today is `{ucsf}`. The full statement, and
/// the ruling it still needs before Tasks 48-51 consume this field, is in
/// [`super::provenance`]'s header, where a reader goes looking for the reason a
/// forged record is harmless.
///
/// ⚠ **"Declared nothing" and "declared an empty allowlist" are different, and
/// the difference is the whole of Step 2.** An identity absent from the table
/// contributes no row and no institution; if *no* identity has a row the answer
/// is [`ExtensionAffiliation::Any`] — unconstrained, reachable from every
/// private model — which is the correct default for the many extensions under no
/// institutional agreement. A row that exists with an empty list permits
/// nothing. Collapsing the two (`institutions.is_empty() → Any`) turns a typo
/// into a granted flow, so the presence of a row is tracked separately from what
/// the row holds.
///
/// A public identity has no row, which is why a forged provenance record naming
/// a public registry id cannot erase a private name's institution: it adds an
/// identity that declares nothing, and nothing unions to nothing.
fn affiliation_for(registry: &RegistrySnapshot, identities: &[String]) -> ExtensionAffiliation {
    let mut declared = false;
    let mut institutions = BTreeSet::new();
    for (key, named) in registry.affiliations {
        if identities.iter().any(|identity| identity == key) {
            declared = true;
            // Interning takes a process-wide mutex, so it happens only for a row
            // that actually matched — a public extension pays nothing.
            institutions.extend(named.iter().map(|id| InstitutionId::new(id)));
        }
    }
    if declared {
        ExtensionAffiliation::Institutions(institutions)
    } else {
        ExtensionAffiliation::Any
    }
}

/// The filesystem paths a config names, for matching against a recorded
/// `install_dir`. Only `Stdio` carries any: it is the shape every `.brxt`
/// install writes (`uv run --directory <install_dir> <entry_point>`). The
/// arguments are compared **whole**, never parsed for a `--directory` flag — a
/// flag-parsing heuristic in a security path is right until an install path
/// spells it differently.
fn referenced_paths(config: &crate::agents::extension::ExtensionConfig) -> Vec<String> {
    match config {
        crate::agents::extension::ExtensionConfig::Stdio { args, .. } => args.clone(),
        _ => Vec::new(),
    }
}

/// Membership of the compiled marketplace snapshot, on an already-reduced key.
fn is_private_key(registry: &RegistrySnapshot, key: &str) -> bool {
    registry.private.contains(&key)
}

/// The compiled-in private set, by `name_to_key` key.
///
/// `registry_private` is a private module on purpose — nothing outside this
/// folder may *decide* a tier from the raw list, it must go through
/// [`classify_extension`]. This accessor exists for the one legitimate reader
/// that is not a classifier: the disclosure test in [`super::refusal`], which
/// asserts a refusal never names a member of the set the caller did not ask
/// about. A hand-written list there would stop tracking this one and the
/// assertion would go quietly vacuous.
pub fn private_extension_ids() -> impl Iterator<Item = &'static str> {
    super::registry_private::PRIVATE_EXTENSIONS.iter().copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::ProviderTier;

    /// The set itself, stated once where a human reads it.
    ///
    /// The test below asserts two members and a list of non-members, which an
    /// **extra** private entry passes — a generator bug that swept, say,
    /// `playwrightagent` into the set would go unnoticed there, and so would a
    /// hand edit. The set is small, deliberate and reviewed by name, so its
    /// exact value belongs in an assertion rather than in a comment.
    #[test]
    fn the_generated_set_is_exactly_these_two_keys() {
        assert_eq!(
            private_extension_ids().collect::<Vec<_>>(),
            vec!["cdwagent", "ucsfomopagent"],
            "the compiled-in private set changed. It is generated from the \
             data-privacy annotations in landing/baam.html by \
             landing/scripts/build-registry.mjs. If that change is intended, \
             say so here; if it is not, the generator or the page is wrong"
        );
    }

    /// The key has to survive both reductions on the way in and come out the
    /// same, or the set holds a spelling the running app never produces.
    ///
    /// `classify_extension` applies `name_to_key`; the extension manager applies
    /// `normalize` to the installed config entry's name before storing it. The
    /// two agree only on ASCII letters, digits, `_` and `-`. The generator
    /// refuses a name outside that set — this is the same invariant asserted
    /// from the consuming side, so a hand-edited key cannot reintroduce it.
    #[test]
    fn every_key_survives_both_reductions_unchanged() {
        for key in private_extension_ids() {
            assert_eq!(
                crate::config::extensions::name_to_key(key),
                key,
                "{key} is not already in name_to_key form, so classify_extension can never match it"
            );
            assert_eq!(
                crate::agents::normalize(key),
                key,
                "the extension manager would store {key} under a different name, \
                 so the tier lookup would miss it and the extension would classify Public"
            );
        }
    }

    #[test]
    fn the_private_set_is_exactly_the_two_the_registry_publishes() {
        use crate::privacy::ProviderTier::{Private, Public};
        assert_eq!(resolve_in(&REGISTRY, "ucsfomopagent", None).tier, Private);
        assert_eq!(resolve_in(&REGISTRY, "cdwagent", None).tier, Private);

        // R11(ii): anything not on BAAM is PUBLIC. Fail-open, by operator ruling.
        // `medcp` is enabled on the operator's own machine with CLINICAL_RECORDS_*
        // against a clinical MSSQL backend and stays fully callable — the badge is
        // a statement about provenance, not about the data behind the connector.
        for name in [
            "medcp",
            "msbaseagent",
            "spokeagent",
            "spokeagent-0.4.1",
            "developer",
            "memory",
            "knowledge",
            "autovisualiser",
            "computercontroller",
            "webdocuments",
            "agent_drafter",
            "todo",
            "chatrecall",
            "extensionmanager",
            "skills",
            "code_execution",
            "appcontrol",
            "datasql",
            "files",
            "compute",
            "evidence",
            "something-nobody-has-published",
        ] {
            assert_eq!(resolve_in(&REGISTRY, name, None).tier, Public, "{name}");
        }
    }

    #[test]
    fn live_registry_authority_can_raise_but_never_lower_classification() {
        super::super::registry_live::insert_test_authority(
            "live-private-fixture",
            Some(BTreeSet::from(["ucsf".to_owned()])),
        );
        let classification = resolve_extension("live-private-fixture", None);
        assert_eq!(classification.tier, ProviderTier::Private);
        assert_eq!(classification.affiliation, owned_by(&["ucsf"]));

        super::super::registry_live::insert_test_authority("live-private-fixture", None);
        assert_eq!(
            resolve_extension("live-private-fixture", None),
            classification,
            "an unconstrained or public-looking refresh cannot lower learned authority"
        );
    }

    #[test]
    fn classification_is_case_and_whitespace_insensitive_the_way_the_key_is() {
        // `name_to_key` (config/extensions.rs:23) strips whitespace and lowercases,
        // then `normalize()` (extension_manager.rs) preserves `_`. The tier
        // must be resolved on the SAME key the manager stores, or a config entry
        // named "UCSFOMOPAgent" installs Private under one rule and Public under
        // the other.
        assert_eq!(classify_extension("UCSFOMOPAgent"), ProviderTier::Private);
        assert_eq!(classify_extension(" ucsfomopagent "), ProviderTier::Private);
    }

    // ----------------------------------------------------------------------
    // Task 43 / DR-23: the tier is re-derived from the registry, keyed on the
    // stable id the install recorded — never on the local config name alone.
    //
    // Every test below states its provenance through
    // `provenance::insert_test_record`, which is additive and keyed on a name
    // no other test uses, so these do not have to be serialised against each
    // other or against the file-backed store.
    // ----------------------------------------------------------------------

    /// A `.brxt` bundle whose installed name is not the registry id it came
    /// from — the situation `spokeagent-0.4.1` already puts the real catalogue
    /// in — is classified from the id, not from the name.
    #[test]
    fn an_install_name_that_differs_from_the_registry_id_is_still_private() {
        super::super::provenance::insert_test_record("clinical-0.5.1", "cdwagent");
        assert_eq!(
            classify_extension("clinical-0.5.1"),
            ProviderTier::Private,
            "the installed name is not the registry id, and the id is what decides"
        );
    }

    /// The registry may publish the id in either spelling the bundle uses, so
    /// the recorded id goes through the same `name_to_key` reduction the
    /// compiled set is stated in.
    #[test]
    fn a_recorded_id_is_reduced_the_same_way_the_set_is() {
        super::super::provenance::insert_test_record("clinical-0.5.2", "CDWAgent");
        assert_eq!(classify_extension("clinical-0.5.2"), ProviderTier::Private);
    }

    /// **The bug DR-23 exists to close, at the resolver.**
    ///
    /// ⚠ The rename changes BOTH the map key and the entry's own `name`, so a
    /// record found by key alone is not found at all — which is why the record
    /// carries the install directory and the resolver is handed the config. The
    /// name-only [`classify_extension`] therefore still answers Public here, and
    /// that asymmetry is asserted rather than described: it is the reason every
    /// gate had to move to `classify_extension_entry`.
    #[test]
    fn a_renamed_private_extension_is_still_private_through_its_install_directory() {
        use crate::agents::extension::{Envs, ExtensionConfig};
        // ⚠ A directory no other test records. `provenance::test_records` is
        // process-global and additive, so a shared one lends this fixture's
        // registry id to someone else's and both go green on provenance only one
        // of them stated.
        let install_dir = "/home/researcher/.config/biorouter/extensions/CDWAgentResolverFixture";
        super::super::provenance::insert_test_record_at(
            "cdwagent-before-the-rename",
            "cdwagent",
            Some(install_dir),
        );
        let renamed = ExtensionConfig::Stdio {
            name: "mystuff".to_string(),
            description: "renamed by hand in config.yaml".to_string(),
            cmd: "uv".to_string(),
            args: vec![
                "run".to_string(),
                "--directory".to_string(),
                install_dir.to_string(),
                "server.py".to_string(),
            ],
            envs: Envs::default(),
            env_keys: vec![],
            timeout: Some(300),
            bundled: None,
            available_tools: vec![],
        };

        assert_eq!(
            classify_extension_entry("mystuff", Some(&renamed)),
            ProviderTier::Private,
            "renaming the entry moved the name, not the directory the install unpacked into"
        );
        assert_eq!(
            classify_extension("mystuff"),
            ProviderTier::Public,
            "the name-only form cannot see the install directory, so a caller holding a config \
             must pass it, and this is why"
        );
    }

    /// The install-directory match is EXACT, over whole arguments. A config that
    /// merely mentions a similar path — a sibling directory, or the extensions
    /// root itself — must not inherit a neighbour's registry id.
    #[test]
    fn a_neighbouring_path_does_not_inherit_the_recorded_id() {
        use crate::agents::extension::{Envs, ExtensionConfig};
        let install_dir = "/home/researcher/.config/biorouter/extensions/ClinicalNeighbour";
        super::super::provenance::insert_test_record_at(
            "clinical-neighbour",
            "cdwagent",
            Some(install_dir),
        );
        let unrelated = ExtensionConfig::Stdio {
            name: "somethingelse".to_string(),
            description: "a different extension entirely".to_string(),
            cmd: "uv".to_string(),
            args: vec![
                "run".to_string(),
                "--directory".to_string(),
                format!("{install_dir}-v2"),
                "server.py".to_string(),
            ],
            envs: Envs::default(),
            env_keys: vec![],
            timeout: Some(300),
            bundled: None,
            available_tools: vec![],
        };
        assert_eq!(
            classify_extension_entry("somethingelse", Some(&unrelated)),
            ProviderTier::Public
        );
    }

    /// **Step 2, first direction.** Provenance may only RAISE. A record naming
    /// a public registry id cannot un-private a name the compiled set knows —
    /// otherwise writing one line into the provenance store would be a
    /// downgrade, and the whole point of re-deriving is that there is no stored
    /// value to forge.
    #[test]
    fn a_public_recorded_id_cannot_lower_a_private_name() {
        super::super::provenance::insert_test_record("cdwagent", "playwrightagent");
        assert_eq!(classify_extension("cdwagent"), ProviderTier::Private);
    }

    /// **Step 2, second direction.** A registry with no entry for the recorded
    /// id retains what is already known rather than defaulting to public.
    #[test]
    fn an_unknown_recorded_id_retains_the_name_derived_tier() {
        super::super::provenance::insert_test_record("ucsfomopagent", "an-id-nobody-publishes");
        assert_eq!(classify_extension("ucsfomopagent"), ProviderTier::Private);
    }

    /// **Step 0's admitted gap, asserted rather than described.** An extension
    /// installed before this task recorded no provenance at all, so it can only
    /// be joined the old way — on its config name. Renaming one of those still
    /// loses the tier, and the fallback is a documented fallback rather than a
    /// silent one.
    #[test]
    fn without_a_record_the_join_is_still_the_name_join() {
        assert_eq!(
            classify_extension("renamed-with-no-record"),
            ProviderTier::Public
        );
    }

    // ----------------------------------------------------------------------
    // Task 47 / DR-26: affiliation, resolved from the same registry read as the
    // tier.
    //
    // Two lookups would let tier and affiliation disagree about one extension —
    // the renamed entry below is exactly where they would, since the rename
    // moves the name and not the install directory. So there is one resolver,
    // and these tests read both fields out of one call.
    //
    // Several rows cannot be exercised against the real snapshot, whose only two
    // private extensions are both UCSF-tagged. Those use a synthetic
    // `RegistrySnapshot` through `resolve_in` — the same code path, a different
    // catalog — rather than testing a hand-extracted rule that the resolver
    // might not be the one using.
    // ----------------------------------------------------------------------

    use crate::privacy::affiliation::{
        compatible, cross_affiliation_warning, ExtensionAffiliation, InstitutionId,
        ModelAffiliation,
    };

    fn inst(name: &str) -> InstitutionId {
        InstitutionId::new(name)
    }

    fn owned_by(names: &[&str]) -> ExtensionAffiliation {
        ExtensionAffiliation::institutions(names.iter().map(|n| inst(n)))
    }

    /// A `.brxt`-shaped config: `uv run --directory <install_dir> server.py`.
    fn uv_config(name: &str, install_dir: &str) -> crate::agents::extension::ExtensionConfig {
        use crate::agents::extension::{Envs, ExtensionConfig};
        ExtensionConfig::Stdio {
            name: name.to_string(),
            description: "fixture".to_string(),
            cmd: "uv".to_string(),
            args: vec![
                "run".to_string(),
                "--directory".to_string(),
                install_dir.to_string(),
                "server.py".to_string(),
            ],
            envs: Envs::default(),
            env_keys: vec![],
            timeout: Some(300),
            bundled: None,
            available_tools: vec![],
        }
    }

    /// The generated table, stated once where a human reads it — the same
    /// reasoning as `the_generated_set_is_exactly_these_two_keys`. An assertion
    /// that merely checked `cdwagent` is UCSF would pass while a generator bug
    /// tagged a third extension, or dropped an institution from the map that the
    /// warning copy needs.
    #[test]
    fn the_generated_affiliation_table_is_exactly_these_two_rows() {
        assert_eq!(
            super::super::registry_private::EXTENSION_AFFILIATIONS,
            &[
                ("cdwagent", &["ucsf"] as &[&str]),
                ("ucsfomopagent", &["ucsf"]),
            ],
            "the compiled-in affiliation table changed. It is generated from the \
             data-affiliation annotations in landing/baam.html by \
             landing/scripts/build-registry.mjs"
        );
        assert_eq!(
            super::super::registry_private::INSTITUTIONS,
            &[("ucsf", "UCSF")],
            "the institution display-name map changed; DR-26's warning renders from it"
        );
    }

    /// Affiliation only ever qualifies a PRIVATE extension — "under whose
    /// agreements" is a question that arises once data is private. A row keyed
    /// on something the private set does not publish would be dead weight the
    /// resolver could never reach, and is more likely a mis-keyed row that was
    /// meant to constrain a real connector.
    #[test]
    fn every_affiliated_key_is_a_key_the_private_set_publishes() {
        for (key, _) in super::super::registry_private::EXTENSION_AFFILIATIONS {
            assert!(
                private_extension_ids().any(|k| k == *key),
                "{key} declares an affiliation but is not in the private set"
            );
        }
    }

    /// Every institution the table names has a display name, or DR-26's
    /// requirement that the warning NAME both institutions degrades to a raw
    /// slug for a connector this build ships.
    #[test]
    fn every_institution_the_table_names_has_a_display_name() {
        for (key, institutions) in super::super::registry_private::EXTENSION_AFFILIATIONS {
            for id in *institutions {
                assert!(
                    crate::privacy::affiliation::institution_display_name(inst(id)).is_some(),
                    "{key} names institution {id:?}, which has no display name to warn with"
                );
            }
        }
    }

    /// **Step 3, first assertion.** A UCSF-tagged extension is still `ucsf` when
    /// the registry is taken away.
    ///
    /// ⚠ In Rust "take the registry away" cannot mean a failed fetch: there is
    /// no fetch. The compiled snapshot IS the registry here, linked into the
    /// binary, so "unreachable registry retains" holds by construction. What can
    /// still go missing or lie are the three inputs below, and none of them may
    /// lower the answer.
    #[test]
    fn a_ucsf_extension_keeps_its_affiliation_when_its_provenance_does_not_help() {
        // (a) No provenance record at all — the join is the name join, which is
        //     what every pre-Task-43 install has.
        let resolved = resolve_extension("cdwagent", None);
        assert_eq!(resolved.tier, ProviderTier::Private);
        assert_eq!(resolved.affiliation, owned_by(&["ucsf"]));

        // (b) A recorded id the snapshot does not publish. It contributes no
        //     institution and must not displace the name-derived one.
        //     ⚠ Deliberately the same (key, id) pair as
        //     `an_unknown_recorded_id_retains_the_name_derived_tier`:
        //     `test_records` is process-global, so two records under one key must
        //     agree rather than race.
        super::super::provenance::insert_test_record("ucsfomopagent", "an-id-nobody-publishes");
        let resolved = resolve_extension("ucsfomopagent", None);
        assert_eq!(resolved.tier, ProviderTier::Private);
        assert_eq!(resolved.affiliation, owned_by(&["ucsf"]));

        // (c) A recorded PUBLIC id against a name the snapshot knows as private.
        //     A public extension declares no affiliation, so a resolver that
        //     treated "some identity declares nothing" as unconstrained would
        //     answer `Any` here and silently clear the flow. Same pairing rule
        //     as (b).
        //
        //     ⚠ It does NOT also catch a last-match-wins resolver, which an
        //     earlier version of this comment claimed: `playwrightagent`
        //     contributes no row at all, so it is never a *matching* row and
        //     taking the last one still yields `ucsf`. Iteration order is pinned
        //     by `affiliation_unions_over_every_matched_identity`, where two
        //     identities really do both have rows. One assertion, one mutation.
        super::super::provenance::insert_test_record("cdwagent", "playwrightagent");
        let resolved = resolve_extension("cdwagent", None);
        assert_eq!(resolved.tier, ProviderTier::Private);
        assert_eq!(
            resolved.affiliation,
            owned_by(&["ucsf"]),
            "a public identity declares nothing; it must not erase the private one's institution"
        );
    }

    /// **Why affiliation had to ride the config-aware resolution.** A rename
    /// rewrites both the map key and the entry's own `name`, so a name-only
    /// affiliation lookup answers `Any` — an unconstrained extension every
    /// private model may reach — for a connector holding UCSF PHI. The tier and
    /// the affiliation come out of ONE call, so they cannot disagree about it.
    #[test]
    fn a_renamed_private_extension_keeps_both_its_tier_and_its_affiliation() {
        // ⚠ An install directory no other test records; see `test_records`.
        let install_dir =
            "/home/researcher/.config/biorouter/extensions/CDWAgentAffiliationFixture";
        super::super::provenance::insert_test_record_at(
            "cdwagent-before-the-affiliation-rename",
            "cdwagent",
            Some(install_dir),
        );
        let renamed = uv_config("mystuff-affiliation", install_dir);

        let resolved = resolve_extension("mystuff-affiliation", Some(&renamed));
        assert_eq!(resolved.tier, ProviderTier::Private);
        assert_eq!(
            resolved.affiliation,
            owned_by(&["ucsf"]),
            "the rename moved the name, not the directory the install unpacked into"
        );
    }

    /// A public extension is unconstrained, because affiliation is a question
    /// about private data. This is the row that must NOT become a mismatch.
    ///
    /// The premise is asserted rather than assumed: without the first two
    /// assertions this test says nothing about *public extensions* in
    /// particular, since a name nobody has ever published (`"zzz"`) would pass
    /// it identically. `playwrightagent` is a real BAAM extension the snapshot
    /// classifies Public, which is the case that has to stay unconstrained — a
    /// marketplace connector, installed and enabled, that no institution's
    /// agreements cover.
    #[test]
    fn a_public_extension_declares_no_affiliation() {
        assert!(
            !private_extension_ids().any(|k| k == "playwrightagent"),
            "this says something about a PUBLIC extension only while it is one"
        );
        assert!(
            !super::super::registry_private::EXTENSION_AFFILIATIONS
                .iter()
                .any(|(key, _)| *key == "playwrightagent"),
            "...and about an UNAFFILIATED one only while it has no row"
        );

        let resolved = resolve_extension("playwrightagent", None);
        assert_eq!(resolved.tier, ProviderTier::Public);
        assert_eq!(resolved.affiliation, ExtensionAffiliation::Any);
        // ...so an institution's model reaches it with no warning, which is the
        // consequence that would actually be felt if this regressed.
        assert!(compatible(
            &ModelAffiliation::institution(inst("ucsf")),
            &resolved.affiliation
        ));
    }

    // ---- The rows the real snapshot cannot exercise ------------------------

    /// **Step 3, second assertion.** A private extension with no
    /// `affiliation` is `Any`.
    ///
    /// The tree's two private extensions are both UCSF-tagged, so this row has
    /// no representative in the real catalog — and it is the row that decides
    /// whether every unconstrained connector becomes a cross-institutional
    /// warning on the day this ships. `private-no-affiliation.html` pins the
    /// generator's half (no row emitted); this pins the resolver's.
    #[test]
    fn an_untagged_private_extension_is_unconstrained_not_a_mismatch() {
        const SNAPSHOT: RegistrySnapshot = RegistrySnapshot {
            private: &["untaggedagent"],
            affiliations: &[],
        };
        let resolved = resolve_in(&SNAPSHOT, "UntaggedAgent", None);
        assert_eq!(resolved.tier, ProviderTier::Private);
        assert_eq!(resolved.affiliation, ExtensionAffiliation::Any);
        // ...and therefore reachable from an institution's model without a warning.
        assert!(compatible(
            &ModelAffiliation::institution(inst("ucsf")),
            &resolved.affiliation
        ));
    }

    /// **Step 3, third assertion.** An affiliation naming an institution the
    /// registry does not publish is a **mismatch**, and the warning surfaces the
    /// raw id.
    ///
    /// Failing open on a typo is how a real constraint disappears: `atlantis`
    /// has no display name, and a resolver that dropped what it could not name
    /// would turn a constrained connector into an unconstrained one.
    #[test]
    fn an_institution_the_registry_does_not_publish_is_a_mismatch_that_names_it() {
        const SNAPSHOT: RegistrySnapshot = RegistrySnapshot {
            private: &["atlantisagent"],
            affiliations: &[("atlantisagent", &["atlantis"])],
        };
        let resolved = resolve_in(&SNAPSHOT, "AtlantisAgent", None);
        assert_eq!(resolved.tier, ProviderTier::Private);
        assert_eq!(resolved.affiliation, owned_by(&["atlantis"]));

        let bound = ModelAffiliation::institution(inst("ucsf"));
        assert!(
            !compatible(&bound, &resolved.affiliation),
            "an institution the registry cannot name must not be silently cleared"
        );
        let warning = cross_affiliation_warning(bound, "AtlantisAgent", &resolved.affiliation)
            .expect("a mismatch must produce a warning");
        assert!(
            warning.contains("atlantis"),
            "the warning must surface the raw id: {warning}"
        );
        assert!(
            warning.contains("UCSF"),
            "the warning must also name the institution the model is bound to: {warning}"
        );
    }

    /// An explicitly EMPTY allowlist permits nothing, and is not `Any`.
    ///
    /// The generator refuses `data-affiliation=""`, so this cannot arrive from
    /// `baam.html` — it is the hand-edit direction, and it must fail closed. A
    /// resolver written as "no institutions collected → unconstrained" reads an
    /// empty allowlist as a granted flow, which is the one mistake DR-26 calls
    /// out about conflating the two.
    #[test]
    fn an_explicitly_empty_allowlist_is_not_unconstrained() {
        const SNAPSHOT: RegistrySnapshot = RegistrySnapshot {
            private: &["emptyallowlistagent"],
            affiliations: &[("emptyallowlistagent", &[])],
        };
        let resolved = resolve_in(&SNAPSHOT, "EmptyAllowlistAgent", None);
        assert_ne!(resolved.affiliation, ExtensionAffiliation::Any);
        assert!(!compatible(
            &ModelAffiliation::institution(inst("ucsf")),
            &resolved.affiliation
        ));
    }

    /// **The union is over ALL matching identities, never the first.** Task 43
    /// learned this for the tier (`a_key_match_does_not_mask_an_install_directory_match`);
    /// affiliation inherits it exactly, so a base matching several institutions
    /// carries all of them rather than whichever join happened to match first.
    #[test]
    fn affiliation_unions_over_every_matched_identity() {
        const SNAPSHOT: RegistrySnapshot = RegistrySnapshot {
            private: &["unionfixturea", "unionfixtureb"],
            affiliations: &[("unionfixturea", &["alpha"]), ("unionfixtureb", &["beta"])],
        };
        // ⚠ An install directory no other test records; see `test_records`.
        let install_dir = "/home/researcher/.config/biorouter/extensions/UnionAffiliationFixture";
        super::super::provenance::insert_test_record_at(
            "union-affiliation-fixture",
            "unionfixtureb",
            Some(install_dir),
        );
        // Named for one private extension, launched out of another's directory.
        let config = uv_config("Union Fixture A", install_dir);

        let resolved = resolve_in(&SNAPSHOT, "unionfixturea", Some(&config));
        assert_eq!(resolved.tier, ProviderTier::Private);
        assert_eq!(
            resolved.affiliation,
            owned_by(&["alpha", "beta"]),
            "the key match and the install-directory match are both identities of this entry"
        );
    }
}
