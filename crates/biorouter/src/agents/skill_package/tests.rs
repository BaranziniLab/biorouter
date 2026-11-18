//! The fixture matrix #115's acceptance criteria enumerate.
//!
//! Each fixture is built as a real ZIP and run through the real pipeline —
//! `read_zip` → `strip_wrapper` → `plan_from_entries` → `install` — because
//! every defect this module fixes lived in the seam between two of those steps,
//! and a test that hands `plan_from_entries` a hand-built entry list would jump
//! straight over the one that mattered most (the wrapper directory).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use tempfile::TempDir;

use super::archive::{self, WrapperHint};
use super::{install, plan_from_entries, Evidence, ImportKind, ImportPlan, SourceProvenance};

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

fn skill_md(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n\nBody.\n")
}

/// Build a real ZIP from `(path, contents)` pairs.
fn zip_of(files: &[(&str, String)]) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (path, contents) in files {
            writer.start_file(*path, options).unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }
    buffer
}

/// The pipeline as a caller runs it, minus the network.
///
/// ⚠ **Every fixture runs through both readers.** `read_package` inflates only
/// the files a plan reads; `read_zip` + `strip_wrapper` inflates everything.
/// The two must give the same plan, or the same error, for every shape in this
/// matrix, which is what shows that leaving files out changed no plan.
fn plan(
    files: &[(&str, String)],
    hint: WrapperHint<'_>,
    hints: &[&str],
) -> anyhow::Result<ImportPlan> {
    let bytes = zip_of(files);
    let planned = |(entries, wrapper): (Vec<archive::Entry>, Option<String>)| {
        let mut id_hints: Vec<String> = hints.iter().map(|h| h.to_string()).collect();
        if let Some(wrapper) = wrapper {
            id_hints.push(wrapper);
        }
        plan_from_entries(entries, &id_hints, SourceProvenance::default())
    };
    let full = archive::read_zip(&bytes)
        .map(|entries| archive::strip_wrapper(entries, hint))
        .and_then(planned);
    let lean = archive::read_package(&bytes, hint).and_then(planned);
    match (&full, &lean) {
        (Ok(full), Ok(lean)) => assert_eq!(
            format!("{lean:?}"),
            format!("{full:?}"),
            "reading only what a plan uses changed the plan"
        ),
        (Err(full), Err(lean)) => assert_eq!(format!("{lean:#}"), format!("{full:#}")),
        _ => panic!("the two readers disagree: full {full:?}, lean {lean:?}"),
    }
    lean
}

/// HyperFrames as its main branch actually ships: a repository wrapper, a
/// plugin manifest naming the package and its skills path, a component
/// manifest with groups, a router named after the package, and members that do
/// **not** share the package's name prefix.
fn hyperframes(files: usize) -> Vec<(&'static str, String)> {
    let members: &[&str] = &[
        "hyperframes",
        "media-use",
        "slideshow",
        "product-launch-video",
        "faceless-explainer",
    ];
    let mut out: Vec<(&'static str, String)> = vec![
        (
            "hyperframes-main/.codex-plugin/plugin.json",
            r#"{"name":"hyperframes","version":"0.8.12","skills":"./skills/"}"#.to_string(),
        ),
        (
            "hyperframes-main/skills-manifest.json",
            serde_json::json!({
                "source": "heygen-com/hyperframes",
                "entryPoint": "hyperframes",
                "skills": members,
                "groups": {
                    "core": ["hyperframes", "media-use"],
                    "on-demand": ["slideshow", "product-launch-video", "faceless-explainer"],
                },
            })
            .to_string(),
        ),
        (
            "hyperframes-main/README.md",
            "# HyperFrames\n\nRead the router first.\n".to_string(),
        ),
    ];
    // `&'static str` paths, so the member list is spelled out rather than
    // formatted. Kept to five to stay readable; the 20-skill case is asserted
    // separately, by count.
    out.push((
        "hyperframes-main/skills/hyperframes/SKILL.md",
        skill_md("hyperframes", "Mandatory entry point and router"),
    ));
    out.push((
        "hyperframes-main/skills/media-use/SKILL.md",
        skill_md("media-use", "Work with media assets"),
    ));
    out.push((
        "hyperframes-main/skills/slideshow/SKILL.md",
        skill_md("slideshow", "Build a slideshow"),
    ));
    out.push((
        "hyperframes-main/skills/product-launch-video/SKILL.md",
        skill_md("product-launch-video", "Launch video workflow"),
    ));
    out.push((
        "hyperframes-main/skills/faceless-explainer/SKILL.md",
        skill_md("faceless-explainer", "Faceless explainer workflow"),
    ));
    assert_eq!(out.len(), files, "fixture size drifted");
    out
}

fn installed_names(root: &Path) -> Vec<String> {
    let roots = vec![crate::agents::skill_catalog::SkillRoot {
        path: root.to_path_buf(),
        source: crate::agents::skill_catalog::SkillSource::new(
            crate::agents::skill_catalog::SkillSourceKind::Biorouter,
            None,
        ),
    }];
    let catalog = crate::agents::skill_catalog::SkillCatalog::scan(roots, 1);
    let mut names: Vec<String> = catalog
        .skills()
        .values()
        .filter(|skill| skill.directory.starts_with(root))
        .map(|skill| skill.metadata.name.clone())
        .collect();
    names.sort();
    names
}

// ---------------------------------------------------------------------------
// 1. root SKILL.md
// ---------------------------------------------------------------------------

#[test]
fn a_root_skill_md_is_one_skill() {
    let plan = plan(
        &[("SKILL.md", skill_md("solo", "A single skill"))],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    assert_eq!(plan.kind, ImportKind::Single);
    assert_eq!(plan.evidence, Evidence::SingleSkill);
    assert_eq!(plan.id, "solo");
    assert_eq!(plan.components.len(), 1);
    assert!(plan.ambiguity.is_none());
}

// ---------------------------------------------------------------------------
// 2. one folder, one skill
// ---------------------------------------------------------------------------

#[test]
fn one_folder_holding_one_skill_is_still_one_skill_not_a_package_of_one() {
    let plan = plan(
        &[("my-skill/SKILL.md", skill_md("gwas-pipeline", "Run a GWAS"))],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    assert_eq!(plan.kind, ImportKind::Single);
    // Named after the frontmatter, not the folder — the frontmatter is the
    // identity every enablement surface keys on.
    assert_eq!(plan.id, "gwas-pipeline");
    assert_eq!(
        plan.files.iter().filter(|(p, _)| p == "SKILL.md").count(),
        1
    );
}

/// The same shape reached WITHOUT `strip_wrapper` unwrapping it first.
///
/// ⚠ This is the fixture that was missing, and its absence hid a real refusal.
/// `plan()` above hands `<slug>/SKILL.md` to `strip_wrapper`, which unwraps it
/// into the root-SKILL.md case — so every synthetic test jumped over the
/// single-component-under-a-named-folder branch. A zip carrying a `README.md`
/// beside the folder has no single common root, is not unwrapped, and lands
/// squarely on it.
#[test]
fn one_folder_beside_a_readme_is_still_one_skill_and_needs_no_package_name() {
    let plan = plan(
        &[
            ("my-skill/SKILL.md", skill_md("gwas-pipeline", "Run a GWAS")),
            ("my-skill/references/notes.md", "notes".to_string()),
            ("README.md", "# A repository".to_string()),
        ],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    assert_eq!(plan.kind, ImportKind::Single);
    assert_eq!(plan.id, "gwas-pipeline");
    assert!(plan.files.iter().any(|(path, _)| path == "SKILL.md"));
    assert!(plan
        .files
        .iter()
        .any(|(path, _)| path == "references/notes.md"));
}

// ---------------------------------------------------------------------------
// 3. the canonical BioRouter bundle
// ---------------------------------------------------------------------------

#[test]
fn a_two_level_bundle_archive_stays_one_bundle_and_keeps_its_name() {
    let plan = plan(
        &[
            (
                "superpowers/brainstorming/SKILL.md",
                skill_md("brainstorming", "Ideas"),
            ),
            ("superpowers/writing/SKILL.md", skill_md("writing", "Prose")),
        ],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    assert_eq!(plan.kind, ImportKind::Bundle);
    assert_eq!(plan.id, "superpowers");
    assert_eq!(plan.components.len(), 2);
    // ⚠ The old parser and the `Infer` rule agree here for a reason: unwrapping
    // `superpowers/` would produce two unrelated skills, which is the exact
    // flattening this issue is about.
    assert!(
        plan.ambiguity.is_none(),
        "a named bundle folder is not ambiguous"
    );
}

// ---------------------------------------------------------------------------
// 4. repository wrapper + skills/
// ---------------------------------------------------------------------------

#[test]
fn a_github_source_archive_is_unwrapped_and_read_as_one_package() {
    let files = hyperframes(8);
    let plan = plan(&files, WrapperHint::SourceArchive, &["hyperframes"]).unwrap();

    assert_eq!(plan.kind, ImportKind::Bundle);
    assert_eq!(plan.id, "hyperframes");
    assert_eq!(plan.display_name, "hyperframes");
    assert_eq!(plan.version.as_deref(), Some("0.8.12"));
    assert_eq!(plan.entry_point.as_deref(), Some("hyperframes"));
    assert_eq!(plan.evidence, Evidence::CodexPlugin);
    assert!(
        plan.ambiguity.is_none(),
        "an explicit manifest is not ambiguous"
    );

    let mut names: Vec<&str> = plan.components.iter().map(|c| c.name.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "faceless-explainer",
            "hyperframes",
            "media-use",
            "product-launch-video",
            "slideshow"
        ]
    );

    // The router is marked, and only the router.
    let routers: Vec<&str> = plan
        .components
        .iter()
        .filter(|c| c.entry_point)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(routers, vec!["hyperframes"]);
}

/// The prefix trap, stated as a test. `media-use`, `slideshow`,
/// `product-launch-video` and `faceless-explainer` do not begin with
/// `hyperframes-`, so a prefix is neither a usable detector nor something the
/// importer may add.
#[test]
fn component_names_are_preserved_exactly_and_never_prefixed() {
    let files = hyperframes(8);
    let plan = plan(&files, WrapperHint::SourceArchive, &["hyperframes"]).unwrap();
    for component in &plan.components {
        assert!(
            !component.name.starts_with("hyperframes-"),
            "the importer invented a prefix: {}",
            component.name
        );
    }
    assert!(plan.components.iter().any(|c| c.name == "media-use"));
    assert!(plan
        .components
        .iter()
        .any(|c| c.directory == "skills/media-use"));
}

/// Zipping only the `skills/` directory is a thing people do, and the depth
/// parser this replaces would have installed HyperFrames as a bundle named
/// literally "skills", losing the declared package identity.
#[test]
fn a_bare_skills_directory_never_becomes_a_package_called_skills() {
    let files = vec![
        (
            "skills/hyperframes/SKILL.md",
            skill_md("hyperframes", "Router"),
        ),
        ("skills/media-use/SKILL.md", skill_md("media-use", "Media")),
    ];
    let plan = plan(&files, WrapperHint::Infer, &["hyperframes-main"]).unwrap();
    assert_ne!(plan.id, "skills");
    assert_eq!(plan.id, "hyperframes-main");
    assert_eq!(plan.evidence, Evidence::SkillsDirectory);
}

// ---------------------------------------------------------------------------
// 5. plugin manifest with a skills path
// ---------------------------------------------------------------------------

#[test]
fn a_claude_plugin_manifest_is_read_like_a_codex_one() {
    let files = vec![
        (
            ".claude-plugin/plugin.json",
            r#"{"name":"lab-pack","displayName":"Lab Pack","version":"3.1","skills":"./skills"}"#
                .to_string(),
        ),
        ("skills/alpha/SKILL.md", skill_md("alpha", "First")),
        ("skills/beta/SKILL.md", skill_md("beta", "Second")),
    ];
    let plan = plan(&files, WrapperHint::Infer, &[]).unwrap();
    assert_eq!(plan.evidence, Evidence::ClaudePlugin);
    assert_eq!(plan.id, "lab-pack");
    assert_eq!(plan.display_name, "Lab Pack");
    assert_eq!(plan.version.as_deref(), Some("3.1"));
}

// ---------------------------------------------------------------------------
// 6. manifest-defined bundle with a router and groups
// ---------------------------------------------------------------------------

#[test]
fn declared_groups_survive_import_and_a_phantom_member_does_not() {
    let files = vec![
        (
            "skills-manifest.json",
            serde_json::json!({
                "name": "toolkit",
                "router": "toolkit",
                "skills": ["toolkit", "alpha", "never-shipped"],
                "groups": { "core": ["toolkit"], "on-demand": ["alpha", "never-shipped"] },
            })
            .to_string(),
        ),
        ("toolkit/SKILL.md", skill_md("toolkit", "Router")),
        ("alpha/SKILL.md", skill_md("alpha", "First")),
    ];
    let plan = plan(&files, WrapperHint::Infer, &[]).unwrap();
    assert_eq!(plan.evidence, Evidence::SkillsManifest);
    assert_eq!(plan.entry_point.as_deref(), Some("toolkit"));
    assert_eq!(plan.groups["core"], vec!["toolkit"]);
    assert_eq!(
        plan.groups["on-demand"],
        vec!["alpha"],
        "a manifest that over-promises must not put a phantom name in the interface"
    );
    let alpha = plan.components.iter().find(|c| c.name == "alpha").unwrap();
    assert_eq!(alpha.group.as_deref(), Some("on-demand"));
}

// ---------------------------------------------------------------------------
// 7. nested support files
// ---------------------------------------------------------------------------

#[test]
fn a_components_support_files_travel_with_it_and_do_not_become_components() {
    let files = vec![
        (
            ".codex-plugin/plugin.json",
            r#"{"name":"deep","skills":"./skills/"}"#.to_string(),
        ),
        ("skills/alpha/SKILL.md", skill_md("alpha", "First")),
        ("skills/alpha/references/notes.md", "notes".to_string()),
        ("skills/alpha/scripts/run.sh", "echo hi".to_string()),
        // A support file that is itself named SKILL.md, one level too deep to
        // be a component. Matching at any depth would promote it.
        (
            "skills/alpha/references/SKILL.md",
            skill_md("not-a-component", "Should not be imported"),
        ),
        ("skills/beta/SKILL.md", skill_md("beta", "Second")),
    ];
    let plan = plan(&files, WrapperHint::Infer, &[]).unwrap();
    let names: Vec<&str> = plan.components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names.len(), 2, "got {names:?}");
    assert!(!names.contains(&"not-a-component"));
    assert!(plan
        .files
        .iter()
        .any(|(path, _)| path == "skills/alpha/references/notes.md"));
    assert!(plan
        .files
        .iter()
        .any(|(path, _)| path == "skills/alpha/scripts/run.sh"));
}

/// Destiny is a plugin-shaped bundle whose members intentionally reach back to
/// shared package roots (`../../scripts` and `../../references`).  Installing
/// each member at the package root severs that contract even though every
/// `SKILL.md` remains individually loadable.
#[test]
fn a_plugin_bundle_preserves_component_paths_and_shared_package_roots() {
    let files = vec![
        (
            ".claude-plugin/plugin.json",
            r#"{"name":"destiny-skill","skills":"./skills/"}"#.to_string(),
        ),
        (
            "skills/destiny-astrology/SKILL.md",
            format!(
                "{}\nUse ../../references/environment.md and ../../scripts/natal.py.\n",
                skill_md("destiny-astrology", "Compute a chart")
            ),
        ),
        (
            "skills/destiny-mbti/SKILL.md",
            skill_md("destiny-mbti", "Text-only typology"),
        ),
        ("scripts/natal.py", "print('synthetic')".to_string()),
        (
            "references/environment.md",
            "Synthetic environment notes".to_string(),
        ),
        (
            "references/decoy/SKILL.md",
            skill_md("must-not-be-callable", "Shared data, not a component"),
        ),
        ("install.sh", "must not be imported".to_string()),
    ];
    let plan = plan(&files, WrapperHint::SourceArchive, &["destiny-skill"]).unwrap();

    assert!(plan
        .components
        .iter()
        .all(|component| component.directory.starts_with("skills/")));
    for expected in [
        "skills/destiny-astrology/SKILL.md",
        "skills/destiny-mbti/SKILL.md",
        "scripts/natal.py",
        "references/environment.md",
    ] {
        assert!(
            plan.files.iter().any(|(path, _)| path == expected),
            "missing {expected}"
        );
    }
    assert!(!plan.files.iter().any(|(path, _)| path == "install.sh"));

    let temp = TempDir::new().unwrap();
    let root = temp.path().join("skills-root");
    let installed = install::install(&plan, &root).unwrap();
    let package_root = root.join("destiny-skill");
    assert_eq!(installed.directory, package_root);
    assert!(package_root.join("scripts/natal.py").is_file());
    assert!(package_root.join("references/environment.md").is_file());

    let roots = vec![crate::agents::skill_catalog::SkillRoot {
        path: root.clone(),
        source: crate::agents::skill_catalog::SkillSource::new(
            crate::agents::skill_catalog::SkillSourceKind::Biorouter,
            None,
        ),
    }];
    let catalog = crate::agents::skill_catalog::SkillCatalog::scan(roots, 1);
    assert!(!catalog.skills().contains_key("must-not-be-callable"));
    let astrology = catalog.skills().get("destiny-astrology").cloned().unwrap();
    assert_eq!(
        astrology.directory,
        package_root.join("skills/destiny-astrology")
    );
    assert_eq!(
        std::fs::canonicalize(astrology.directory.join("../../scripts/natal.py")).unwrap(),
        std::fs::canonicalize(package_root.join("scripts/natal.py")).unwrap()
    );
    assert_eq!(
        std::fs::canonicalize(astrology.directory.join("../../references/environment.md")).unwrap(),
        std::fs::canonicalize(package_root.join("references/environment.md")).unwrap()
    );
    let view = catalog.view(&Default::default());
    let bundle = view
        .bundles
        .iter()
        .find(|bundle| bundle.name == "destiny-skill")
        .unwrap();
    assert_eq!(bundle.directory, package_root);
    assert!(bundle.package.is_some());
}

// ---------------------------------------------------------------------------
// 8. duplicate component names
// ---------------------------------------------------------------------------

#[test]
fn two_components_declaring_one_name_is_refused_rather_than_silently_shadowed() {
    let files = vec![
        (
            ".codex-plugin/plugin.json",
            r#"{"name":"clashy","skills":"./skills/"}"#.to_string(),
        ),
        ("skills/one/SKILL.md", skill_md("same-name", "First")),
        ("skills/two/SKILL.md", skill_md("same-name", "Second")),
    ];
    let error = plan(&files, WrapperHint::Infer, &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("declares the skill `same-name` twice"),
        "{error}"
    );
    assert!(error.contains("shadow"), "{error}");
}

// ---------------------------------------------------------------------------
// 9. malformed manifests, and partial-write rollback
// ---------------------------------------------------------------------------

#[test]
fn a_malformed_manifest_stops_the_import_rather_than_inferring_a_different_identity() {
    let files = vec![
        (".codex-plugin/plugin.json", "{ not json".to_string()),
        ("skills/alpha/SKILL.md", skill_md("alpha", "First")),
    ];
    let error = plan(&files, WrapperHint::Infer, &[])
        .unwrap_err()
        .to_string();
    assert!(error.contains("plugin.json is not valid JSON"), "{error}");
}

#[test]
fn a_source_with_no_skill_md_anywhere_is_refused() {
    let error = plan(
        &[("README.md", "hello".to_string())],
        WrapperHint::Infer,
        &[],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("no SKILL.md"), "{error}");
}

/// The rollback property, exercised where it actually bites: a plan whose
/// components cannot all be verified from disk after staging.
#[test]
fn a_failed_install_leaves_the_previous_version_exactly_as_it_was() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("skills");

    let good = plan(
        &[
            ("pack/alpha/SKILL.md", skill_md("alpha", "First")),
            ("pack/beta/SKILL.md", skill_md("beta", "Second")),
        ],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    install::install(&good, &root).unwrap();
    assert_eq!(installed_names(&root), vec!["alpha", "beta"]);

    // A plan that claims a component it does not carry: staging writes what it
    // has, verification reads the tree back and finds the claim unmet.
    let mut broken = good.clone();
    broken.components.push(super::PlannedSkill {
        name: "gamma".to_string(),
        description: "Missing".to_string(),
        directory: "gamma".to_string(),
        group: None,
        entry_point: false,
    });
    let error = install::install(&broken, &root).unwrap_err().to_string();
    assert!(error.contains("gamma"), "{error}");

    // The previous install is untouched, and no debris was left behind.
    assert_eq!(installed_names(&root), vec!["alpha", "beta"]);
    let stray: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with(".br-"))
        .collect();
    assert!(stray.is_empty(), "staging debris left behind: {stray:?}");
}

#[test]
fn an_import_that_still_carries_an_unanswered_question_refuses_to_install() {
    let temp = TempDir::new().unwrap();
    let plan = plan(
        &[
            ("alpha/SKILL.md", skill_md("alpha", "First")),
            ("beta/SKILL.md", skill_md("beta", "Second")),
        ],
        WrapperHint::Infer,
        &["mixed-bag"],
    )
    .unwrap();
    assert!(plan.ambiguity.is_some());
    let error = install::install(&plan, temp.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("needs an answer"), "{error}");
}

// ---------------------------------------------------------------------------
// Ambiguity: a question, never a default.
// ---------------------------------------------------------------------------

#[test]
fn sibling_skills_with_no_manifest_ask_instead_of_flattening() {
    let plan = plan(
        &[
            ("alpha/SKILL.md", skill_md("alpha", "First")),
            ("beta/SKILL.md", skill_md("beta", "Second")),
        ],
        WrapperHint::Infer,
        &["mixed-bag"],
    )
    .unwrap();
    let ambiguity = plan
        .ambiguity
        .as_ref()
        .expect("this genuinely is ambiguous");
    assert_eq!(ambiguity.components, vec!["alpha", "beta"]);
    assert!(ambiguity.reason.contains("2 skills"));
}

#[test]
fn answering_the_question_either_way_produces_an_installable_plan() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("skills");
    let base = plan(
        &[
            ("alpha/SKILL.md", skill_md("alpha", "First")),
            ("beta/SKILL.md", skill_md("beta", "Second")),
        ],
        WrapperHint::Infer,
        &["mixed-bag"],
    )
    .unwrap();

    let bundled = base.clone().as_bundle();
    install::install(&bundled, &root).unwrap();
    assert_eq!(installed_names(&root), vec!["alpha", "beta"]);
    assert!(root
        .join("mixed-bag")
        .join(crate::agents::skill_catalog::PACKAGE_RECORD_FILE)
        .is_file());

    // ...and the other answer gives one top-level skill per component, not a
    // package of one, so `alpha` installs exactly where a single skill would.
    let picked = base.into_individual(&["alpha".to_string()]);
    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].kind, ImportKind::Single);
    assert_eq!(picked[0].id, "alpha");
    assert!(picked[0].ambiguity.is_none());

    let solo_root = temp.path().join("solo");
    install::install(&picked[0], &solo_root).unwrap();
    assert!(solo_root.join("alpha/SKILL.md").is_file());
    assert!(
        !solo_root
            .join("alpha")
            .join(crate::agents::skill_catalog::PACKAGE_RECORD_FILE)
            .exists(),
        "a single skill is not a package and gets no package record"
    );
}

// ---------------------------------------------------------------------------
// Install: the record, the layout, and update-in-place.
// ---------------------------------------------------------------------------

#[test]
fn installing_hyperframes_produces_one_expandable_bundle_the_catalog_can_see() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("skills");
    let files = hyperframes(8);
    let mut plan = plan(&files, WrapperHint::SourceArchive, &["hyperframes"]).unwrap();
    plan.source = SourceProvenance {
        url: Some("https://github.com/heygen-com/hyperframes".to_string()),
        reference: Some("main".to_string()),
        resolved_commit: Some("abc1234".to_string()),
        installer: Some("repository".to_string()),
    };

    let installed = install::install(&plan, &root).unwrap();
    assert_eq!(installed.id, "hyperframes");
    assert_eq!(installed.skills.len(), 5);
    assert_eq!(installed.entry_point.as_deref(), Some("hyperframes"));
    assert!(!installed.replaced);

    // The source's component root is preserved because members and shared
    // package assets may refer to one another by relative path.
    assert!(root
        .join("hyperframes/skills/hyperframes/SKILL.md")
        .is_file());
    assert!(root.join("hyperframes/skills/media-use/SKILL.md").is_file());

    let roots = vec![crate::agents::skill_catalog::SkillRoot {
        path: root.clone(),
        source: crate::agents::skill_catalog::SkillSource::new(
            crate::agents::skill_catalog::SkillSourceKind::Biorouter,
            None,
        ),
    }];
    let view = crate::agents::skill_catalog::SkillCatalog::scan(roots, 1)
        .view(&crate::agents::session_skills::SessionSkillOverride::default());
    // ⚠ `KNOWLEDGE_BUNDLE` is injected into every scan by
    // `add_missing_shipped_skills`, so filter to what this test installed.
    let bundles: Vec<_> = view
        .bundles
        .iter()
        .filter(|b| b.name != crate::agents::skills_extension::KNOWLEDGE_BUNDLE)
        .collect();
    assert_eq!(bundles.len(), 1, "one bundle, not five top-level skills");
    let bundle = bundles[0];
    assert_eq!(bundle.name, "hyperframes");
    assert_eq!(bundle.skills.len(), 5);
    let package = bundle.package.as_ref().expect("the record was written");
    assert_eq!(package.entry_point.as_deref(), Some("hyperframes"));
    assert_eq!(package.version.as_deref(), Some("0.8.12"));
    assert_eq!(
        package.source_url.as_deref(),
        Some("https://github.com/heygen-com/hyperframes")
    );
    assert_eq!(package.resolved_commit.as_deref(), Some("abc1234"));
    assert_eq!(package.groups["core"], vec!["hyperframes", "media-use"]);
    // Not one top-level skill per component. Filtered to what came from this
    // root: `SkillCatalog::scan` also injects the shipped skills, which have no
    // bundle and belong to the real config directory.
    let from_here: Vec<&crate::agents::skill_catalog::CatalogSkill> = view
        .skills
        .iter()
        .filter(|s| s.directory.starts_with(&root))
        .collect();
    assert_eq!(from_here.len(), 5);
    assert!(from_here
        .iter()
        .all(|s| s.bundle.as_deref() == Some("hyperframes")));
}

#[test]
fn reinstalling_replaces_the_package_as_one_unit_and_drops_a_removed_component() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("skills");

    let first = plan(
        &[
            ("pack/alpha/SKILL.md", skill_md("alpha", "First")),
            ("pack/beta/SKILL.md", skill_md("beta", "Second")),
        ],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    install::install(&first, &root).unwrap();

    let second = plan(
        &[
            ("pack/alpha/SKILL.md", skill_md("alpha", "First, revised")),
            ("pack/gamma/SKILL.md", skill_md("gamma", "Third")),
        ],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    let installed = install::install(&second, &root).unwrap();
    assert!(installed.replaced);

    assert_eq!(installed_names(&root), vec!["alpha", "gamma"]);
    assert!(
        !root.join("pack/beta").exists(),
        "an update must replace the package, not merge into it"
    );
}

#[test]
fn removing_a_package_takes_every_component_with_it() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("skills");
    let plan = plan(
        &[
            ("pack/alpha/SKILL.md", skill_md("alpha", "First")),
            ("pack/beta/SKILL.md", skill_md("beta", "Second")),
        ],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    install::install(&plan, &root).unwrap();

    let removed = install::remove("pack", &root).unwrap();
    assert_eq!(removed.id, "pack");
    assert!(installed_names(&root).is_empty());
    assert!(!root.join("pack").exists());
    assert!(install::remove("pack", &root).is_err());
}

/// A package this importer wrote can be re-imported: the record is a manifest,
/// and it is the ladder's top rung.
#[test]
fn an_installed_package_round_trips_through_its_own_record() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("skills");
    let files = hyperframes(8);
    let plan_a = plan(&files, WrapperHint::SourceArchive, &["hyperframes"]).unwrap();
    install::install(&plan_a, &root).unwrap();

    // Re-zip what landed on disk, exactly as an export would.
    let mut exported: Vec<(String, String)> = Vec::new();
    for entry in walkdir::WalkDir::new(root.join("hyperframes"))
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
    {
        let relative = entry
            .path()
            .strip_prefix(root.join("hyperframes"))
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        exported.push((relative, std::fs::read_to_string(entry.path()).unwrap()));
    }
    let borrowed: Vec<(&str, String)> = exported
        .iter()
        .map(|(path, body)| (path.as_str(), body.clone()))
        .collect();

    let plan_b = plan(&borrowed, WrapperHint::Infer, &[]).unwrap();
    assert_eq!(plan_b.evidence, Evidence::BiorouterManifest);
    assert_eq!(plan_b.id, "hyperframes");
    assert_eq!(plan_b.entry_point.as_deref(), Some("hyperframes"));
    assert_eq!(plan_b.components.len(), 5);
    assert!(plan_b
        .components
        .iter()
        .all(|component| component.directory.starts_with("skills/")));
    assert_eq!(plan_b.groups["core"], vec!["hyperframes", "media-use"]);
}

// ---------------------------------------------------------------------------
// Archive safety.
// ---------------------------------------------------------------------------

#[test]
fn an_entry_that_would_escape_the_install_directory_is_refused() {
    for evil in [
        "../outside.md",
        "a/../../outside.md",
        "/etc/passwd",
        "..\\windows.md",
    ] {
        assert!(archive::safe_entry_name(evil).is_err(), "accepted {evil}");
    }
    assert_eq!(
        archive::safe_entry_name("a/b/SKILL.md").unwrap(),
        "a/b/SKILL.md"
    );
    assert_eq!(
        archive::safe_entry_name("a\\b\\SKILL.md").unwrap(),
        "a/b/SKILL.md"
    );
    assert_eq!(
        archive::safe_entry_name("./a/SKILL.md").unwrap(),
        "a/SKILL.md"
    );
}

/// Staging re-checks every path, because a plan can be built by any caller and
/// this is the last point before bytes reach the filesystem.
#[test]
fn install_refuses_a_hand_built_plan_carrying_an_escaping_path() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("skills");
    let mut plan = plan(
        &[
            ("pack/alpha/SKILL.md", skill_md("alpha", "First")),
            ("pack/beta/SKILL.md", skill_md("beta", "Second")),
        ],
        WrapperHint::Infer,
        &["pack"],
    )
    .unwrap();
    plan.files
        .push(("../escaped.md".to_string(), b"nope".to_vec()));

    assert!(install::install(&plan, &root).is_err());
    assert!(!temp.path().join("escaped.md").exists());
}

#[test]
fn install_refuses_a_hand_built_plan_with_an_escaping_component_directory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("skills");
    let mut plan = plan(
        &[
            ("pack/alpha/SKILL.md", skill_md("alpha", "First")),
            ("pack/beta/SKILL.md", skill_md("beta", "Second")),
        ],
        WrapperHint::Infer,
        &["pack"],
    )
    .unwrap();
    plan.components[0].directory = "../outside".to_string();

    assert!(install::install(&plan, &root).is_err());
    assert!(!root.join("pack").exists());
}

#[test]
fn a_bundle_that_gains_a_component_keeps_the_bundle_name_it_had() {
    // Not a property of the importer so much as of the shape it installs into:
    // the per-chat override stores the BUNDLE name, so a package that grows is
    // still covered. Asserted here because this is where the shape is decided.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("skills");
    let first = plan(
        &[
            ("pack/alpha/SKILL.md", skill_md("alpha", "First")),
            ("pack/zeta/SKILL.md", skill_md("zeta", "Placeholder")),
        ],
        WrapperHint::Infer,
        &["pack"],
    )
    .unwrap();
    install::install(&first, &root).unwrap();

    let second = plan(
        &[
            ("pack/alpha/SKILL.md", skill_md("alpha", "First")),
            ("pack/beta/SKILL.md", skill_md("beta", "Second")),
        ],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    assert_eq!(second.id, "pack");
    install::install(&second, &root).unwrap();
    assert_eq!(installed_names(&root), vec!["alpha", "beta"]);
}

// ---------------------------------------------------------------------------
// Wrapper stripping, on its own.
// ---------------------------------------------------------------------------

#[test]
fn inference_strips_a_wrapper_only_when_stripping_reveals_structure() {
    let wrapped_repo = archive::read_zip(&zip_of(&[
        ("repo-main/skills-manifest.json", "{}".to_string()),
        ("repo-main/skills/a/SKILL.md", skill_md("a", "A")),
    ]))
    .unwrap();
    let (_, stripped) = archive::strip_wrapper(wrapped_repo, WrapperHint::Infer);
    assert_eq!(stripped.as_deref(), Some("repo-main"));

    let real_bundle = archive::read_zip(&zip_of(&[
        ("pack/alpha/SKILL.md", skill_md("alpha", "A")),
        ("pack/beta/SKILL.md", skill_md("beta", "B")),
    ]))
    .unwrap();
    let (kept, stripped) = archive::strip_wrapper(real_bundle, WrapperHint::Infer);
    assert_eq!(stripped, None, "a bundle folder is not a wrapper");
    assert!(kept.iter().all(|e| e.name.starts_with("pack/")));
}

/// The same archive, one hint apart. A repository download says "this wraps",
/// and that knowledge beats the inference — which is the only way to tell a
/// repository of loose skills from a bundle directory.
#[test]
fn a_source_archive_hint_strips_a_wrapper_inference_would_have_kept() {
    let files = &[
        ("repo-main/alpha/SKILL.md", skill_md("alpha", "A")),
        ("repo-main/beta/SKILL.md", skill_md("beta", "B")),
    ];
    let (_, inferred) = archive::strip_wrapper(
        archive::read_zip(&zip_of(files)).unwrap(),
        WrapperHint::Infer,
    );
    assert_eq!(inferred, None);

    let (entries, from_source) = archive::strip_wrapper(
        archive::read_zip(&zip_of(files)).unwrap(),
        WrapperHint::SourceArchive,
    );
    assert_eq!(from_source.as_deref(), Some("repo-main"));
    assert!(entries.iter().all(|e| !e.name.starts_with("repo-main/")));
}

// ---------------------------------------------------------------------------
// Size: only what a plan reads counts against the limit.
// ---------------------------------------------------------------------------

/// A source archive's `docs/`, `registry/` and `packages/` belong to no skill.
/// HyperFrames' default branch grew to 327 MB that way while its skills stayed
/// near 14 MB, and inflating every file against the 256 MiB budget refused the
/// documented import outright (rq2-220). The same shape, scaled down: a limit
/// the skills fit under and the whole archive does not.
#[test]
fn an_archive_heavy_outside_its_skills_still_imports() {
    let heavy = "x".repeat(64 * 1024);
    let mut files = hyperframes(8);
    files.push(("hyperframes-main/docs/media/demo.mp4", heavy.clone()));
    files.push((
        "hyperframes-main/registry/blocks/catalog.json",
        heavy.clone(),
    ));
    files.push(("hyperframes-main/packages/core/dist/bundle.js", heavy));
    let limit: u64 = 32 * 1024;
    let total: usize = files.iter().map(|(_, body)| body.len()).sum();
    assert!(
        total as u64 > limit,
        "the fixture must exceed the limit as a whole"
    );
    let bytes = zip_of(&files);

    let (entries, wrapper) =
        archive::read_package_limited(&bytes, WrapperHint::SourceArchive, limit)
            .expect("the skills fit under the limit, so the archive imports");
    assert_eq!(wrapper.as_deref(), Some("hyperframes-main"));
    for prefix in ["docs/", "registry/", "packages/"] {
        assert!(
            entries.iter().all(|entry| !entry.name.starts_with(prefix)),
            "`{prefix}` was inflated although no plan reads it"
        );
    }

    // And the plan is exactly the one the whole archive gives.
    let mut hints = vec!["hyperframes".to_string()];
    hints.extend(wrapper);
    let lean = plan_from_entries(entries, &hints, SourceProvenance::default()).unwrap();
    let (all, wrapper) = archive::strip_wrapper(
        archive::read_zip(&bytes).unwrap(),
        WrapperHint::SourceArchive,
    );
    let mut hints = vec!["hyperframes".to_string()];
    hints.extend(wrapper);
    let full = plan_from_entries(all, &hints, SourceProvenance::default()).unwrap();
    assert_eq!(format!("{lean:?}"), format!("{full:?}"));
}

/// The refusal a person reads when the skills themselves are too large: the
/// limit in a unit people use, and what to import instead. It used to be "the
/// archive expands past the 268435456 byte limit", a raw count with no way on.
#[test]
fn the_over_limit_sentence_names_a_tag_or_subfolder() {
    let mut files = hyperframes(8);
    files.push((
        "hyperframes-main/skills/media-use/assets/clip.mp3",
        "x".repeat(64 * 1024),
    ));
    let limit: u64 = 32 * 1024;
    let error = archive::read_package_limited(&zip_of(&files), WrapperHint::SourceArchive, limit)
        .unwrap_err()
        .to_string();
    assert_eq!(error, archive::too_large_message(limit));
    assert!(error.contains("32 KB"), "{error}");
    assert!(!error.contains("byte"), "a raw byte count: {error}");
    assert!(error.contains("/tree/<tag>"), "no tag to import: {error}");
    assert!(
        error.contains("skills folder"),
        "no folder to import: {error}"
    );

    // The real limit reads in MB.
    let real = archive::too_large_message(archive::MAX_ARCHIVE_BYTES);
    assert!(real.contains("256 MB"), "{real}");
}

#[test]
fn the_groups_map_is_empty_rather_than_absent_when_nothing_declares_one() {
    let plan = plan(
        &[
            ("pack/alpha/SKILL.md", skill_md("alpha", "First")),
            ("pack/beta/SKILL.md", skill_md("beta", "Second")),
        ],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    assert_eq!(plan.groups, BTreeMap::new());
    assert!(plan.components.iter().all(|c| c.group.is_none()));
}

/// ⚠ **A seeded skill or bundle cannot be deleted from ANY surface.**
///
/// `biorouter skill remove` and the Skills pane's Trash both land on
/// `install::remove`, and before this guard both *succeeded* on a shipped
/// skill: the folder went, the tick and the toast confirmed it, and the next
/// startup rewrote it. That is regression 1 of #77 — a control that reports
/// success and reverts is worse than no control — and it survived the fix on
/// the desktop side because the fix was a UI gate, not a refusal.
///
/// Driven through `refuse_shipped` rather than `remove` so both halves can be
/// stated without setting `BIOROUTER_PATH_ROOT`, whose process-global reach
/// would make this test depend on what ran before it.
#[test]
fn removing_a_shipped_skill_or_bundle_is_refused() {
    use crate::agents::skills_extension::KNOWLEDGE_BUNDLE;

    let seeded = Path::new("/config/biorouter/skills");
    let elsewhere = Path::new("/home/someone/.claude/skills");

    for name in [
        "about-biorouter",
        "develop-biorouter",
        "knowledge-lint",
        "update-soul",
        KNOWLEDGE_BUNDLE,
    ] {
        let refusal = install::refuse_shipped(seeded, seeded, name)
            .unwrap_or_else(|| panic!("{name} can still be deleted, and the seeder undoes it"));
        // The refusal has to name what it refused and what to do instead, or it
        // reads as a bug rather than as a rule.
        assert!(refusal.contains(name), "{refusal}");
        assert!(refusal.contains("disable"), "{refusal}");

        // ⚠ The same name under someone else's skills root is THEIRS. The
        // seeder never writes there, so refusing it would be this guard
        // reaching past its own reason to exist.
        assert!(
            install::refuse_shipped(seeded, elsewhere, name).is_none(),
            "{name} is refused under a root Biorouter does not seed"
        );
    }

    // A user package is removable, so the assertions above are about shipped
    // entries and not about `remove` having stopped working.
    assert!(install::refuse_shipped(seeded, seeded, "hyperframes").is_none());
    // The trap: a name that merely resembles a shipped one.
    assert!(install::refuse_shipped(seeded, seeded, "about-biorouter-notes").is_none());
}

/// ⚠ **The WIRING, not the predicate.** The test above drives
/// `refuse_shipped` directly, so deleting the `bail!` that calls it from
/// `remove` or `install` leaves it green while restoring the exact regression
/// it describes. This one goes through the function bodies themselves.
///
/// ⚠ **No `BIOROUTER_PATH_ROOT`.** `install_in`/`remove_in` take the seeded
/// root as an argument for exactly this reason — the one-line public wrappers
/// supply `Paths::config_dir()`, and everything below that is pure. A previous
/// draft set the env var under a global lock, which contradicted the rationale
/// written on `refuse_shipped` itself and made this test order-dependent on
/// every other test in the binary.
#[test]
fn remove_and_install_both_refuse_a_shipped_name_through_their_own_bodies() {
    let tmp = TempDir::new().unwrap();
    let seeded = tmp.path().join("biorouter-skills");
    let package = seeded.join("about-biorouter");
    std::fs::create_dir_all(&package).unwrap();
    let shipped = "---\nname: about-biorouter\ndescription: Shipped\n---\nBody\n";
    std::fs::write(package.join("SKILL.md"), shipped).unwrap();

    let error = format!(
        "{:#}",
        install::remove_in(&seeded, "about-biorouter", &seeded).unwrap_err()
    );
    assert!(error.contains("ships with Biorouter"), "{error}");
    assert!(
        package.join("SKILL.md").is_file(),
        "remove deleted a shipped skill; only the refusal text was missing"
    );

    // ⚠ And the write side. Installing over a shipped name renames the seeded
    // directory aside and deletes it; the seeder then writes `SKILL.md` back
    // inside the user's package, and `remove` above refuses to uninstall the
    // result. A guard on one half of the pair is worse than a guard on neither.
    let mine = plan(
        &[(
            "about-biorouter/SKILL.md",
            skill_md("about-biorouter", "Mine"),
        )],
        WrapperHint::Infer,
        &[],
    )
    .unwrap();
    let error = format!(
        "{:#}",
        install::install_in(&seeded, &mine, &seeded).unwrap_err()
    );
    assert!(error.contains("ships with Biorouter"), "{error}");
    assert_eq!(
        std::fs::read_to_string(package.join("SKILL.md")).unwrap(),
        shipped,
        "install overwrote a shipped skill"
    );

    // ⚠ And the same package under a root Biorouter does not seed is THEIRS.
    // Without this the guard would be a blanket ban on the name.
    let elsewhere = tmp.path().join("claude-skills");
    std::fs::create_dir_all(&elsewhere).unwrap();
    install::install_in(&seeded, &mine, &elsewhere)
        .expect("a same-named package under another root must install");
    install::remove_in(&seeded, "about-biorouter", &elsewhere)
        .expect("and must be removable again");
}
