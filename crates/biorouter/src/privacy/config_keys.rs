//! Which config keys decide a session's privacy capability, and the scan that
//! keeps that list honest (issue #56, DR-16, open question 24).
//!
//! `restore_provider_from_session` falls back to `config.get_biorouter_provider()`
//! when the session row names nothing usable (`agents/agent.rs`), so a write to
//! `BIOROUTER_PROVIDER` is a tier raise for every session opened afterwards —
//! with no `/agent/update_provider` call at all. DR-14 already makes
//! `config.yaml` a filesystem deny root because *"a master switch a public model
//! can edit is not a switch"*; `/config/upsert`, `/config/remove` and
//! `/config/set_provider` are the HTTP channels to the same file.
//!
//! **Both verbs.** Deleting one of these keys is not the absence of a write, it
//! is a write of the key's default — and for `OLLAMA_HOST` that default is
//! `localhost`, which `self_hosted_tier` maps to Private. So the guard is on the
//! key, not on the operation.
//!
//! The requirement is deliberately scoped to tier-relevant keys. A blanket rule
//! would make every programmatic config write a user act — the GUI writes config
//! on nearly every settings interaction — and a rule that fires constantly is a
//! rule people route around.

/// Keys whose value decides what privacy capability a session gets by default.
/// Writing **or deleting** one of these over HTTP is a user act (DR-16, open
/// question 24).
pub const CAPABILITY_CONFIG_KEYS: &[&str] = &[
    // The default provider itself. Read through `config_value!` (base.rs), so
    // the literal never appears in a `get_param(` call — seeded, not scanned.
    "BIOROUTER_PROVIDER",
    // Its presence alone switches `create()` to the lead/worker path
    // (factory.rs, BEFORE the registry lookup), which changes the tier of every
    // provider name rather than of one.
    "BIOROUTER_LEAD_MODEL",
    // Names the lead half, whose tier is one of the two `least()` takes.
    "BIOROUTER_LEAD_PROVIDER",
    // Task 5's third test: a self-hosted provider is Private only while its base
    // URL is loopback. These two keys ARE that base URL, so writing one moves
    // `ollama`/`llamacpp` across the tier boundary in both directions.
    "OLLAMA_HOST",
    "LLAMACPP_EXTERNAL_HOST",
];

/// Every other key the tier-input files read, each with the reason it does not
/// determine capability. A key must be in exactly one of these two lists.
pub const NOT_CAPABILITY_CONFIG_KEYS: &[(&str, &str)] = &[
    ("BIOROUTER_CONTEXT_LIMIT", "token budget, not a tier input"),
    (
        "BIOROUTER_LEAD_TURNS",
        "handoff policy between two already-tiered halves",
    ),
    ("BIOROUTER_LEAD_FAILURE_THRESHOLD", "handoff policy"),
    ("BIOROUTER_LEAD_FALLBACK_TURNS", "handoff policy"),
    ("BIOROUTER_WORKER_CONTEXT_LIMIT", "token budget"),
    ("OLLAMA_TIMEOUT", "transport timeout"),
    ("LLAMACPP_TIMEOUT", "transport timeout"),
    ("LLAMACPP_STARTUP_TIMEOUT", "sidecar readiness deadline"),
    ("LLAMACPP_CONTEXT_SIZE", "token budget"),
    // ⚠ The two endpoint keys below MOVE where a Private-badged provider sends
    //   traffic, and since `e2e4eb9d` that moves its tier as well: `tier()`
    //   follows the endpoint an instance resolved (`ucsf_gateway_tier`), so an
    //   off-site value demotes it to Public, and deleting that value restores
    //   Private. These rows used to say the keys "cannot RAISE a tier" because
    //   Task 5 name-keyed versa_* Private regardless of endpoint, and that
    //   stopped being true. The classification rests on this instead: the only
    //   value that reads Private is the UCSF gateway's own host, so no write can
    //   make an off-site endpoint look Private, and a raise through one of these
    //   keys is always a return to the institution's gateway. Whether even that
    //   raise should be a user act, as it is for `OLLAMA_HOST`, is an open DR-16
    //   question, recorded here rather than left unstated.
    //
    // Versa Azure's three overrides, in its own namespace. It used to share the
    // public `azure_openai` card's `AZURE_OPENAI_*` keys, which went wrong both
    // ways: onboarding WROTE them on Versa's behalf, so connecting UCSF's
    // PRIVATE Versa made that PUBLIC card report itself Configured (hence this
    // namespace, 2026-09-03); and Versa went on READING them as a fallback, so
    // whatever that card was set up with — a company resource's endpoint,
    // deployment and API version — steered every Versa request (read removed
    // 2026-09-11). No tier-input file reads the `AZURE_OPENAI_*` keys now, so
    // they have no rows here; `azure.rs` still reads them and is not a
    // tier-input file, because `azure_openai` is Public wherever it points.
    (
        "VERSA_AZURE_ENDPOINT",
        "moves a Private provider's endpoint; only the UCSF gateway reads Private (see above)",
    ),
    ("VERSA_AZURE_DEPLOYMENT_NAME", "deployment selection"),
    ("VERSA_AZURE_API_VERSION", "wire version"),
    // Versa Bedrock's two overrides, in its own namespace since 2026-09-11. It
    // used to declare and read the public Amazon Bedrock card's `AWS_REGION` and
    // an `AWS_ENDPOINT_URL_BEDROCK` key, then fall back to the process
    // environment, so the public side's values steered Versa and a Versa setup
    // configured the public card. No tier-input file reads an `AWS_*` key now,
    // so none has a row; `bedrock.rs` still reads them and is not a tier-input
    // file, because `aws_bedrock` is Public wherever it points.
    (
        "VERSA_BEDROCK_ENDPOINT",
        "moves a Private provider's endpoint; only the UCSF gateway reads Private (see above)",
    ),
    (
        "VERSA_BEDROCK_REGION",
        "SigV4 signing region; the endpoint, not the region, decides where a request goes",
    ),
    ("BEDROCK_MAX_RETRIES", "retry policy"),
    ("BEDROCK_INITIAL_RETRY_INTERVAL_MS", "retry policy"),
    ("BEDROCK_BACKOFF_MULTIPLIER", "retry policy"),
    ("BEDROCK_MAX_RETRY_INTERVAL_MS", "retry policy"),
    ("BEDROCK_OPERATION_TIMEOUT_SECS", "transport timeout"),
];

/// The files whose `get_param` reads the scan covers: every provider file Task
/// 5's Files table marks Modify to define `tier()`, plus the factory intercept
/// it marks Reference. A new provider whose tier depends on config must be added
/// here — and Task 5's `the_private_set_is_a_table_of_reviewed_decisions` is what
/// fails if a new private provider is added without being classified at all.
///
/// ⚠ `#[cfg(test)]`, along with the two scans below: these `include_str!`s pull
/// ~97 KB of provider source into the crate, and nothing outside this file's own
/// test module reads them. The shipped `biorouterd`/`biorouter` binaries carry
/// the key lists and [`is_capability_key`]; they have no reason to carry a copy
/// of `ollama.rs`.
#[cfg(test)]
pub const TIER_INPUT_FILES: &[(&str, &str)] = &[
    (
        "providers/factory.rs",
        include_str!("../providers/factory.rs"),
    ),
    (
        "providers/ollama.rs",
        include_str!("../providers/ollama.rs"),
    ),
    (
        "providers/llamacpp.rs",
        include_str!("../providers/llamacpp.rs"),
    ),
    (
        "providers/versa_azure.rs",
        include_str!("../providers/versa_azure.rs"),
    ),
    (
        "providers/versa_bedrock.rs",
        include_str!("../providers/versa_bedrock.rs"),
    ),
];

/// Writing **or deleting** this key over HTTP requires the user-action proof.
pub fn is_capability_key(key: &str) -> bool {
    CAPABILITY_CONFIG_KEYS.contains(&key)
}

/// The `(path, source)` pairs the two scans below read. An accessor rather than
/// the constant itself so a caller cannot accidentally iterate a *different*
/// set than the one the classification test walks.
#[cfg(test)]
pub fn tier_input_sources() -> impl Iterator<Item = (&'static str, &'static str)> {
    TIER_INPUT_FILES.iter().copied()
}

/// Every distinct config key the tier-input files read through a `get_param`
/// **string literal**, sorted.
///
/// Literal-only by construction, which is exactly why
/// [`computed_get_param_re`] exists beside it: a key built at runtime would be
/// invisible here and the classification test would go quietly vacuous.
#[cfg(test)]
pub fn scan_get_param_keys() -> Vec<String> {
    let literal = regex::Regex::new(r#"get_param(?:::<[^>]*>)?\s*\(\s*"([^"]+)""#)
        .expect("the get_param literal scan is a compile-time-constant pattern");
    let mut keys: Vec<String> = tier_input_sources()
        .flat_map(|(_path, src)| {
            literal
                .captures_iter(src)
                .map(|caps| caps[1].to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Matches a `get_param` whose key is **not** a string literal — a
/// `get_param(&format!(..))` or a `get_param(some_var)`. The scan above cannot
/// see those, so the test asserts there are none.
#[cfg(test)]
pub fn computed_get_param_re() -> regex::Regex {
    regex::Regex::new(r#"get_param(?:::<[^>]*>)?\s*\(\s*[^"\s]"#)
        .expect("the computed-key pattern is a compile-time constant")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_config_key_the_tier_resolver_reads_is_classified() {
        // Scans the five files Task 5 touches to define `tier()` plus
        // factory.rs's BIOROUTER_LEAD_MODEL intercept, extracts every
        // `get_param("KEY")` literal, and requires each to appear in EXACTLY ONE
        // of the two lists. Adding a config read to any of them fails this test
        // until someone decides whether it determines capability. That is the
        // checkable list: it does not depend on anyone remembering a rule.
        let scanned = scan_get_param_keys(); // 23 today
        assert_eq!(
            scanned.len(),
            23,
            "the tier-input files' config surface changed: {scanned:?}"
        );
        for key in &scanned {
            let cap = CAPABILITY_CONFIG_KEYS.contains(&key.as_str());
            let not = NOT_CAPABILITY_CONFIG_KEYS
                .iter()
                .any(|(k, _why)| *k == key.as_str());
            assert!(
                cap ^ not,
                "{key} is in neither list, or in both; classify it"
            );
        }
        // BIOROUTER_PROVIDER is read through the `config_value!` macro
        // (base.rs:1147), so the literal never appears in a `get_param(` call
        // and the scan cannot see it. It is seeded, and this asserts the seed
        // survives.
        assert!(CAPABILITY_CONFIG_KEYS.contains(&"BIOROUTER_PROVIDER"));
        assert_eq!(CAPABILITY_CONFIG_KEYS.len(), 5);

        // …and the other way round: every classified key is still READ by a
        // tier-input file. Without this, a read that goes away leaves its row
        // behind — the count above moves, someone edits the number, and the
        // lists quietly start classifying keys nothing reads.
        let classified = CAPABILITY_CONFIG_KEYS
            .iter()
            .copied()
            .chain(NOT_CAPABILITY_CONFIG_KEYS.iter().map(|(key, _why)| *key));
        for key in classified.filter(|key| *key != "BIOROUTER_PROVIDER") {
            assert!(
                scanned.iter().any(|read| read == key),
                "{key} is classified but no tier-input file reads it; delete its row"
            );
        }
    }

    #[test]
    fn the_scan_cannot_be_defeated_by_a_computed_key() {
        // The scan reads string literals. A `get_param(&format!(..))` would be
        // invisible to it, so the scan asserts there are none — measured: today
        // every key in all five files is a literal.
        let computed = computed_get_param_re();
        for (path, src) in tier_input_sources() {
            assert!(
                !computed.is_match(src),
                "{path} builds a config key at runtime; the key scan cannot see it"
            );
        }
    }
}
