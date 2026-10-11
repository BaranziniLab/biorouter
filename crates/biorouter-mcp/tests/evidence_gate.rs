//! Invented numbers cannot reach the page dressed as computed ones.
//!
//! The worst failure in the 100-app test drive, reproduced here as a test:
//!
//! > The "Fine Mapper" worker reported that posterior inclusion probabilities were
//! > **not defensible** without summary statistics, an LD reference, and
//! > harmonization. The main agent read that, then invented five PIPs summing to
//! > 1.0 and shaded them onto the page as a credible set. They rendered exactly
//! > like real ones.
//!
//! Nothing could have stopped it. `consult` returns free prose, so the refusal was
//! — to the platform — an ordinary paragraph. `app_call` validated arguments
//! shape-only, and five plausible floats satisfy any schema an author would write.
//! And no field anywhere recorded where a number came from.
//!
//! The model *read* the refusal and proceeded. So the fix cannot be prose. It is:
//! a machine-readable worker verdict (`report_evidence`), a per-turn ledger the
//! main agent cannot write, and a fail-closed check at `app_call` — the one place
//! quantitative output can reach the page.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::agent_drafter::control::{
    AppControlServer, EvidenceEntry, EvidenceStatus, ProvenanceSource, UiBridge,
};
use biorouter_mcp::agent_drafter::manifest::{ActionDecl, ActionEffect, SurfaceDecl, UiCapability};

/// The spec-008 surface: an action that shades a credible set onto a locus plot,
/// and which depends on evidence the Fine Mapper may not have.
fn fine_mapping_surface() -> SurfaceDecl {
    SurfaceDecl {
        actions: vec![ActionDecl {
            name: "shade_credible_set".into(),
            description: "Shade a 95% credible set on the locus plot.".into(),
            params: serde_json::json!({ "type": "object" }),
            effect: ActionEffect::Mutate,
            writes: vec!["/locus/credible_set".into()],
            requires_evidence: vec!["sumstats".into(), "ld_reference".into()],
            provenance_required: true,
        }],
        ..Default::default()
    }
}

fn bridge() -> UiBridge {
    let b = UiBridge::new();
    let _s = AppControlServer::new(b.clone(), UiCapability::default(), fine_mapping_surface());
    b
}

/// The worker's verdict becomes a fact the platform can act on.
#[test]
fn a_worker_verdict_names_the_missing_inputs() {
    let bridge = bridge();

    bridge.record_evidence(EvidenceEntry {
        profile: "fine_mapper".into(),
        status: EvidenceStatus::InsufficientData,
        missing: vec!["sumstats".into(), "ld_reference".into()],
    });

    let missing = bridge.missing_evidence();
    assert_eq!(missing.len(), 2);
    assert!(missing
        .iter()
        .any(|(m, who)| m == "sumstats" && who == "fine_mapper"));
    assert!(missing.iter().any(|(m, _)| m == "ld_reference"));
}

/// A worker that HAD its inputs blocks nothing — the gate must not fire on a
/// healthy turn, or every well-grounded app starts failing.
#[test]
fn an_ok_verdict_blocks_nothing() {
    let bridge = bridge();

    bridge.record_evidence(EvidenceEntry {
        profile: "fine_mapper".into(),
        status: EvidenceStatus::Ok,
        missing: vec![],
    });

    assert!(
        bridge.missing_evidence().is_empty(),
        "a worker that had its data must not block the main agent"
    );
}

/// The gap is scoped to the turn. Data missing last turn must not keep blocking
/// once the user supplies it.
#[test]
fn the_ledger_is_cleared_between_turns() {
    let bridge = bridge();

    bridge.record_evidence(EvidenceEntry {
        profile: "fine_mapper".into(),
        status: EvidenceStatus::InsufficientData,
        missing: vec!["sumstats".into()],
    });
    assert_eq!(bridge.missing_evidence().len(), 1);

    bridge.clear_evidence();
    assert!(
        bridge.missing_evidence().is_empty(),
        "a new turn starts with a clean slate"
    );
}

/// Provenance parsing: the ONE value that means "I made this up" must be
/// recognised, and everything grounded must be distinguishable from it.
#[test]
fn provenance_distinguishes_fabricated_from_grounded() {
    assert_eq!(
        ProvenanceSource::parse("synthetic"),
        Some(ProvenanceSource::Synthetic)
    );
    assert_eq!(
        ProvenanceSource::parse("tool"),
        Some(ProvenanceSource::Grounded)
    );
    assert_eq!(
        ProvenanceSource::parse("consult:fine_mapper"),
        Some(ProvenanceSource::Grounded)
    );
    assert_eq!(
        ProvenanceSource::parse("user"),
        Some(ProvenanceSource::User)
    );

    // An unrecognised source is NOT silently treated as grounded — a typo must not
    // become a free pass past the gate.
    assert_eq!(ProvenanceSource::parse("computed"), None);
    assert_eq!(ProvenanceSource::parse(""), None);
}

/// The declared contract survives a manifest round-trip — the gate is only as good
/// as the declaration that arms it.
#[test]
fn the_evidence_contract_round_trips() {
    let decl = &fine_mapping_surface().actions[0];

    let raw = serde_json::to_value(decl).unwrap();
    assert_eq!(raw["requires_evidence"][0], "sumstats");
    assert_eq!(raw["provenance_required"], true);

    let back: ActionDecl = serde_json::from_value(raw).unwrap();
    assert_eq!(back.requires_evidence.len(), 2);
    assert!(back.provenance_required);
}

/// Back-compat: an action that declares no evidence requirements is untouched, and
/// a v1 manifest gains no new keys.
#[test]
fn a_v1_action_declares_no_evidence_and_serializes_none() {
    let decl = ActionDecl {
        name: "focus_node".into(),
        description: String::new(),
        params: serde_json::json!({}),
        ..Default::default()
    };

    assert!(decl.requires_evidence.is_empty());
    assert!(!decl.provenance_required);

    let raw = serde_json::to_value(&decl).unwrap();
    assert!(raw.get("requires_evidence").is_none(), "{raw}");
    assert!(raw.get("provenance_required").is_none(), "{raw}");
}

/// Status parsing is strict: an unrecognised verdict is rejected rather than
/// silently downgraded to "ok" (which would make the gate a no-op).
#[test]
fn an_unknown_status_is_rejected() {
    assert_eq!(EvidenceStatus::parse("ok"), Some(EvidenceStatus::Ok));
    assert_eq!(
        EvidenceStatus::parse("insufficient_data"),
        Some(EvidenceStatus::InsufficientData)
    );
    assert_eq!(EvidenceStatus::parse("maybe"), None);
    assert_eq!(EvidenceStatus::parse("unsure"), None);
}
