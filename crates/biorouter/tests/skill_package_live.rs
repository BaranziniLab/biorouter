//! The acceptance criterion of #115, against the real repository.
//!
//! ```bash
//! cargo test -p biorouter --test skill_package_live -- --ignored --nocapture
//! ```
//!
//! `#[ignore]` because it downloads ~120 MB from GitHub, so it is not part of
//! any default run — but it is the only test that can fail for the reason the
//! fixtures cannot. Two defects survived a complete synthetic fixture matrix
//! and were found here in one run:
//!
//! * `skills-manifest.json`'s `skills` is a **map** keyed by component name,
//!   not the array the fixtures invented. A `Vec` did not merely miss the
//!   names — serde failed the whole document, and the import was refused with
//!   "skills-manifest.json is not valid JSON".
//! * the human-readable name lives in the plugin manifest's `interface` block,
//!   not at the top level.
//!
//! A fixture is a statement about what you *think* the world looks like. Run
//! this after touching `manifest.rs` or `plan.rs`.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::agents::skill_package::{self, Evidence, ImportKind, ImportSource};

/// `https://github.com/heygen-com/hyperframes` — a declared plugin with a
/// mandatory router and twenty component skills, several of which do **not**
/// share the package's name prefix.
const HYPERFRAMES: &str = "https://github.com/heygen-com/hyperframes";

#[tokio::test]
#[ignore = "downloads ~120 MB from github.com"]
async fn hyperframes_main_imports_as_one_package_with_every_declared_skill() {
    let fetched = skill_package::fetch(&ImportSource::Url {
        url: HYPERFRAMES.to_string(),
        reference: Some("main".to_string()),
    })
    .await
    .expect("fetch");

    // The archive's wrapper directory is stripped, so nothing downstream sees
    // `hyperframes-main/`.
    assert!(
        fetched
            .entries
            .iter()
            .any(|entry| entry.name == "skills/hyperframes/SKILL.md"),
        "the repository wrapper directory was not stripped"
    );

    // What the repository itself says its components are — read here rather
    // than hard-coded, so the assertion survives the package gaining a skill.
    let manifest: serde_json::Value = serde_json::from_str(
        &fetched
            .entries
            .iter()
            .find(|entry| entry.name == "skills-manifest.json")
            .expect("skills-manifest.json")
            .text(),
    )
    .expect("skills-manifest.json parses");
    let mut declared: Vec<String> = manifest["skills"]
        .as_object()
        .expect("`skills` is a map keyed by component name")
        .keys()
        .cloned()
        .collect();
    declared.sort();
    assert!(
        declared.len() >= 20,
        "the manifest declared {} skills",
        declared.len()
    );

    // The version the repository itself declares, read here for the same reason
    // the component list above is: a literal asserts upstream's RELEASE CADENCE
    // rather than our importer. It was pinned to "0.8.12", HyperFrames shipped
    // 0.8.16, and this test then failed *before* reaching the package-identity
    // assertions it exists for — so a real change to `plan.rs` went unvalidated
    // while the failure pointed somewhere else entirely.
    //
    // ⚠ Read BEFORE `plan_from_entries`, which takes `fetched.entries` BY VALUE,
    // and own the string — a `&str` borrowed out of `plugin_manifest` would
    // still be live across that move.
    let plugin_manifest: serde_json::Value = serde_json::from_str(
        &fetched
            .entries
            .iter()
            .find(|entry| entry.name == ".codex-plugin/plugin.json")
            .expect(".codex-plugin/plugin.json")
            .text(),
    )
    .expect(".codex-plugin/plugin.json parses");
    let declared_version = plugin_manifest["version"]
        .as_str()
        .expect("the plugin manifest declares a version")
        .to_string();

    let plan = skill_package::plan_from_entries(fetched.entries, &fetched.id_hints, fetched.source)
        .expect("plan");

    // ONE package, not one top-level skill per SKILL.md.
    assert_eq!(plan.kind, ImportKind::Bundle);
    assert_eq!(plan.id, "hyperframes");
    assert_eq!(plan.display_name, "HyperFrames by HeyGen");
    assert_eq!(plan.evidence, Evidence::CodexPlugin);
    assert!(
        plan.ambiguity.is_none(),
        "an explicitly declared package must not ask the user to choose"
    );
    assert_eq!(plan.version.as_deref(), Some(declared_version.as_str()));

    // Every skill the manifest declares, and only those.
    let mut components: Vec<String> = plan.components.iter().map(|c| c.name.clone()).collect();
    components.sort();
    assert_eq!(components, declared);

    // The router is the entry point, and it is the only one.
    assert_eq!(plan.entry_point.as_deref(), Some("hyperframes"));
    let routers: Vec<&str> = plan
        .components
        .iter()
        .filter(|c| c.entry_point)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(routers, vec!["hyperframes"]);

    // Names are preserved exactly, prefix or no prefix. These four are the
    // reason `hyperframes-` cannot be the detector.
    for name in [
        "media-use",
        "slideshow",
        "product-launch-video",
        "faceless-explainer",
    ] {
        assert!(
            plan.components.iter().any(|c| c.name == name),
            "`{name}` is missing — a member without the package's name prefix"
        );
    }
    assert!(
        plan.components
            .iter()
            .all(|c| !c.name.starts_with("hyperframes-") || declared.contains(&c.name)),
        "the importer invented a prefix"
    );

    // A component's support files travel with it: `media-use` ships audio.
    assert!(
        plan.files
            .iter()
            .any(|(path, _)| path.starts_with("media-use/") && path.ends_with(".mp3")),
        "binary support files were dropped"
    );

    // ...and install it, for real, into a temporary root.
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("skills");
    let installed = skill_package::install(&plan, &root).expect("install");
    assert_eq!(installed.skills.len(), declared.len());
    assert!(root.join("hyperframes/hyperframes/SKILL.md").is_file());
    assert!(root.join("hyperframes/media-use/SKILL.md").is_file());
    assert!(root
        .join("hyperframes")
        .join(biorouter::agents::skill_catalog::PACKAGE_RECORD_FILE)
        .is_file());

    // The catalog sees exactly one bundle here, not twenty top-level skills.
    let roots = vec![biorouter::agents::skill_catalog::SkillRoot {
        path: root.clone(),
        source: biorouter::agents::skill_catalog::SkillSource::new(
            biorouter::agents::skill_catalog::SkillSourceKind::Biorouter,
            None,
        ),
    }];
    let view = biorouter::agents::skill_catalog::SkillCatalog::scan(roots, 1)
        .view(&biorouter::agents::session_skills::SessionSkillOverride::default());
    // ⚠ `add_missing_shipped_skills` injects `KNOWLEDGE_BUNDLE` into every
    // scan, so filter to what this test installed before counting or indexing.
    let bundles: Vec<_> = view
        .bundles
        .iter()
        .filter(|b| b.name != biorouter::agents::skills_extension::KNOWLEDGE_BUNDLE)
        .collect();
    assert_eq!(bundles.len(), 1);
    assert_eq!(bundles[0].name, "hyperframes");
    assert_eq!(bundles[0].display_name, "HyperFrames by HeyGen");
    assert_eq!(bundles[0].skills.len(), declared.len());
    assert_eq!(
        bundles[0]
            .package
            .as_ref()
            .and_then(|p| p.entry_point.as_deref()),
        Some("hyperframes")
    );
}

/// A repository that is one skill, not a package — the other half of the
/// contract, and the case a bundle-happy importer would get wrong.
#[tokio::test]
#[ignore = "downloads from github.com"]
async fn a_single_skill_repository_still_imports_as_one_skill() {
    let fetched = skill_package::fetch(&ImportSource::Url {
        url: "https://github.com/heygen-com/hyperframes".to_string(),
        reference: Some("main".to_string()),
    })
    .await
    .expect("fetch");

    // Take just one component's subtree and re-plan it, which is what a user
    // who zipped a single skill folder would hand us.
    let one: Vec<_> = fetched
        .entries
        .into_iter()
        .filter(|entry| entry.name.starts_with("skills/media-use/"))
        .map(|entry| biorouter::agents::skill_package::archive::Entry {
            name: entry.name.trim_start_matches("skills/").to_string(),
            data: entry.data,
        })
        .collect();
    assert!(!one.is_empty());

    let plan = skill_package::plan_from_entries(one, &[], Default::default()).expect("plan");
    assert_eq!(plan.kind, ImportKind::Single);
    assert_eq!(plan.id, "media-use");
    assert!(plan.ambiguity.is_none());
}
