//! An id that does not exist cannot be saved.
//!
//! This is the regression corpus for the 100-app test drive's worst
//! configuration failures. Across 18 authored apps, Agent Drafter configured 13
//! knowledge-base ids and 7 skill lists that did not exist on the machine it was
//! running on — including the literal string `br.kb`, which is the client API
//! namespace, not an identifier. Every one of those apps then failed its first
//! turn, because the server armed the tools anyway.
//!
//! These tests take the *actual ids from the test drive* and assert each is now
//! rejected at the write boundary, with an error that names what is really
//! installed.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::agent_drafter::catalog::{Catalog, KbEntry, SkillEntry};
use biorouter_mcp::agent_drafter::store::{Requirement, RequirementKind};
use biorouter_mcp::agent_drafter::validate;

/// The knowledge-base ids Agent Drafter invented during the test drive.
const INVENTED_KB_IDS: &[&str] = &[
    "br.kb",
    "clinvar",
    "gwas-catalog",
    "pathway-commons",
    "trial-registry",
    "omop-vocab",
];

/// The skill ids Agent Drafter configured against a runtime with zero installed
/// skills.
const INVENTED_SKILL_IDS: &[&str] = &[
    "pathway-analysis",
    "variant-calling",
    "clinical-biostatistics",
    "single-cell",
    "differential-expression",
];

fn catalog(kbs: &[&str], skills: &[&str]) -> Catalog {
    Catalog {
        knowledge_bases: kbs
            .iter()
            .map(|id| KbEntry {
                id: (*id).to_string(),
                name: (*id).to_string(),
            })
            .collect(),
        skills: skills
            .iter()
            .map(|id| SkillEntry {
                id: (*id).to_string(),
                description: String::new(),
            })
            .collect(),
        extensions: Catalog::discover(&biorouter_mcp::knowledge::caller::KbCaller::restricted())
            .extensions,
    }
}

/// The exact runtime the test drive ran against: seven built-in extensions, zero
/// skills, zero knowledge bases.
fn empty_install() -> Catalog {
    catalog(&[], &[])
}

#[test]
fn every_invented_knowledge_base_from_the_test_drive_is_now_rejected() {
    let install = empty_install();
    for id in INVENTED_KB_IDS {
        let result = validate::check_knowledge_base(id, &install);
        assert!(
            result.is_err(),
            "'{id}' was configured by Agent Drafter during the test drive and must now be rejected"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("requires"),
            "the rejection must point at the honest alternative (`requires`), got: {err}"
        );
    }
}

#[test]
fn every_invented_skill_list_from_the_test_drive_is_now_rejected() {
    let install = empty_install();
    for id in INVENTED_SKILL_IDS {
        let skills = vec![(*id).to_string()];
        let result = validate::check_skills(&skills, &install);
        assert!(
            result.is_err(),
            "skill '{id}' must be rejected on an install with none"
        );
        assert!(
            result.unwrap_err().contains("(none installed)"),
            "an empty catalog must say so in words, not show an empty list"
        );
    }
}

/// The rejection has to be *useful*: a model that is told only "no" will guess
/// again. Naming the installed ids is what makes the retry grounded.
#[test]
fn a_rejection_names_what_is_actually_installed() {
    let install = catalog(&["ms-literature"], &["r-scripting"]);

    let kb_err = validate::check_knowledge_base("clinvar", &install).unwrap_err();
    assert!(kb_err.contains("ms-literature"), "{kb_err}");

    let skill_err =
        validate::check_skills(&["pathway-analysis".to_string()], &install).unwrap_err();
    assert!(skill_err.contains("r-scripting"), "{skill_err}");
}

/// The other half of the fix: wanting something that isn't here must be
/// *expressible*, or the model has no honest option and will invent an id again.
#[test]
fn an_app_may_declare_a_need_it_cannot_satisfy() {
    let install = empty_install();

    // No configured id at all — nothing to reject.
    assert!(validate::check_all(None, &[], &[], &install).is_ok());

    // But the need is recorded, and reported as unmet.
    let requires = vec![Requirement {
        kind: RequirementKind::KnowledgeBase,
        id: "clinvar".into(),
        reason: "variant pathogenicity annotations".into(),
    }];
    let unmet = validate::unmet_requirements(&requires, &install);
    assert_eq!(unmet.len(), 1);
    assert_eq!(unmet[0].id, "clinvar");
    assert_eq!(unmet[0].reason, "variant pathogenicity annotations");
}

/// A requirement that this install *can* satisfy is not reported as a gap.
#[test]
fn a_satisfiable_requirement_is_not_unmet() {
    let install = catalog(&["clinvar"], &["r-scripting"]);
    let requires = vec![
        Requirement {
            kind: RequirementKind::KnowledgeBase,
            id: "clinvar".into(),
            reason: String::new(),
        },
        Requirement {
            kind: RequirementKind::Skill,
            id: "r-scripting".into(),
            reason: String::new(),
        },
    ];
    assert!(validate::unmet_requirements(&requires, &install).is_empty());
}

/// Back-compat guard: the built-in extensions every app names must never be
/// rejected, or all ~110 v1 apps stop configuring.
#[test]
fn the_builtin_extensions_every_app_uses_still_validate() {
    let install = empty_install();
    let exts: Vec<String> = ["developer", "autovisualiser", "knowledge", "memory"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert!(
        validate::check_extensions(&exts, &install).is_ok(),
        "built-ins ship with the daemon and are always available"
    );
}
