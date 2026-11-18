//! The acceptance criterion of #115, against the real repository.
//!
//! ```bash
//! cargo test -p biorouter --test skill_package_live -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! `#[ignore]` because it downloads from GitHub (about 146 MB for the default
//! branch, measured 2026-11-17), so it is not part of any default run. Run it
//! with `--test-threads=1` to download one archive at a time. It is the only
//! test that can fail for the reason the fixtures cannot. Two defects survived
//! a complete synthetic fixture matrix and were found here in one run:
//!
//! * `skills-manifest.json`'s `skills` is a **map** keyed by component name,
//!   not the array the fixtures invented. A `Vec` did not merely miss the
//!   names — serde failed the whole document, and the import was refused with
//!   "skills-manifest.json is not valid JSON".
//! * the human-readable name lives in the plugin manifest's `interface` block,
//!   not at the top level.
//!
//! A later run failed for a reason no fixture could show: the default branch
//! grew to 327 MB expanded, about 14 MB of it in skill folders, and inflating
//! every file against the 256 MiB limit refused the import with a raw byte
//! count (rq2-220). Only the files a plan reads are inflated now.
//!
//! A fixture is a statement about what you *think* the world looks like. Run
//! this after touching `archive.rs`, `manifest.rs` or `plan.rs`.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::agents::skill_package::{self, Evidence, ImportKind, ImportSource};

/// `https://github.com/heygen-com/hyperframes`: a declared plugin with a
/// mandatory router and component skills, several of which do **not** share
/// the package's name prefix.
const HYPERFRAMES: &str = "https://github.com/heygen-com/hyperframes";

/// The release this suite was written against: twenty components, among them
/// the four whose names are why a prefix cannot be the detector. Upstream's
/// default branch has since merged them down to nine (0.9.64, measured
/// 2026-11-17), so assertions about those twenty live here, against a ref that
/// does not move. Fetching it as a ref name also exercises the tag fallback in
/// `source.rs`.
const TWENTY_SKILL_RELEASE: &str = "v0.8.12";

/// Fetch, plan and install HyperFrames at `reference` (`None` is the default
/// branch, exactly what a pasted URL fetches), check everything that holds at
/// any ref, and hand back the plan and the names its manifest declares.
///
/// Every expectation below is read from the repository's own manifests rather
/// than written down, so it asserts the importer and not upstream's release
/// cadence. A literal version once failed this test before it reached the
/// identity assertions it exists for.
async fn import_hyperframes(reference: Option<&str>) -> (skill_package::ImportPlan, Vec<String>) {
    let fetched = skill_package::fetch(&ImportSource::Url {
        url: HYPERFRAMES.to_string(),
        reference: reference.map(str::to_string),
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

    // The repository's docs, registry packages and root files belong to no
    // skill, so none of them was inflated. On the default branch they are most
    // of the 327 MB that used to put this import over the limit.
    for outside in ["docs/", "packages/", "README.md"] {
        assert!(
            fetched
                .entries
                .iter()
                .all(|entry| !entry.name.starts_with(outside)),
            "`{outside}` was read although no plan uses it"
        );
    }
    let read: usize = fetched.entries.iter().map(|entry| entry.data.len()).sum();
    assert!(
        (read as u64) < biorouter::agents::skill_package::archive::MAX_ARCHIVE_BYTES,
        "read {read} bytes"
    );

    // What the repository itself says its components are.
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
        declared.len() >= 2,
        "the manifest declared {} skills",
        declared.len()
    );

    // ⚠ Read BEFORE `plan_from_entries`, which takes `fetched.entries` BY VALUE,
    // and own the string: a `&str` borrowed out of `plugin_manifest` would
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

    // Names are preserved exactly, prefix or no prefix, and some members have
    // none: the reason `hyperframes-` cannot be the detector.
    assert!(
        plan.components
            .iter()
            .any(|c| c.name != "hyperframes" && !c.name.starts_with("hyperframes-")),
        "every member carries the package prefix, so this run cannot show it is not the detector"
    );
    assert!(
        plan.components
            .iter()
            .all(|c| !c.name.starts_with("hyperframes-") || declared.contains(&c.name)),
        "the importer invented a prefix"
    );

    // A component's support files travel with it: `media-use` ships audio.
    // Its directory is read from the plan: since 2026-09-03 a manifest's own
    // components folder is kept (`skills/media-use`), because members and
    // shared assets refer to one another by relative path. This assertion
    // spelled `media-use/` and was stale from then on.
    let media_use = &plan
        .components
        .iter()
        .find(|c| c.name == "media-use")
        .expect("media-use is a component")
        .directory;
    assert!(
        plan.files
            .iter()
            .any(|(path, _)| path.starts_with(&format!("{media_use}/")) && path.ends_with(".mp3")),
        "binary support files were dropped"
    );

    // ...and install it, for real, into a temporary root.
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("skills");
    let installed = skill_package::install(&plan, &root).expect("install");
    assert_eq!(installed.skills.len(), declared.len());
    for component in &plan.components {
        assert!(
            root.join("hyperframes")
                .join(&component.directory)
                .join("SKILL.md")
                .is_file(),
            "`{}` was not installed at {}",
            component.name,
            component.directory
        );
    }
    assert!(root
        .join("hyperframes")
        .join(biorouter::agents::skill_catalog::PACKAGE_RECORD_FILE)
        .is_file());

    // The catalog sees exactly one bundle here, not one top-level skill each.
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

    (plan, declared)
}

/// The default branch, as a pasted repository URL fetches it. This is the
/// import that was refused once the repository grew past the size limit with
/// files no skill uses (rq2-220).
#[tokio::test]
#[ignore = "downloads about 146 MB from github.com"]
async fn hyperframes_main_imports_as_one_package_with_every_declared_skill() {
    import_hyperframes(None).await;
}

/// The twenty-skill release, by tag: the package #115 was about, with the
/// members that share no prefix named outright.
#[tokio::test]
#[ignore = "downloads from github.com"]
async fn the_twenty_skill_release_imports_with_its_unprefixed_members() {
    let (plan, declared) = import_hyperframes(Some(TWENTY_SKILL_RELEASE)).await;
    assert!(
        declared.len() >= 20,
        "the manifest declared {} skills",
        declared.len()
    );
    assert_eq!(
        plan.version.as_deref(),
        Some(TWENTY_SKILL_RELEASE.trim_start_matches('v'))
    );
    for name in [
        "media-use",
        "slideshow",
        "product-launch-video",
        "faceless-explainer",
    ] {
        assert!(
            plan.components.iter().any(|c| c.name == name),
            "`{name}` is missing: a member without the package's name prefix"
        );
    }
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
