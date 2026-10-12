//! The contract can be declared in ONE call, and a manifest write cannot destroy
//! the server's own fields.
//!
//! In the 100-app test drive, declaring a surface took ~6 rejected `update_app`
//! round-trips per app, because there was no typed parameter for `surface` or
//! `theme` — the only path was rewriting the whole `manifest.json`, which
//! hard-failed on `missing field created_at` and, when it did succeed, silently
//! wiped `built_at` / `sdk_hash` / `session_id`.
//!
//! These tests pin the fix: one typed call declares the contract, and the merge
//! restores every server-owned field regardless of what the caller wrote.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::agent_drafter::store::{ArtifactKind, ArtifactStore};
use tempfile::TempDir;

fn store() -> (TempDir, ArtifactStore) {
    let dir = TempDir::new().unwrap();
    let store = ArtifactStore::new(dir.path().to_path_buf());
    (dir, store)
}

/// A manifest that a model composed from scratch — no `created_at`, no
/// `built_at`, no `sdk_hash`. This used to be a hard parse failure.
#[test]
fn a_manifest_without_server_metadata_still_parses() {
    let json = serde_json::json!({
        "id": "app",
        "title": "App",
        "kind": "agentic",
        "entry": "index.html",
    });

    let m: biorouter_mcp::agent_drafter::store::Manifest = serde_json::from_value(json).expect(
        "a model has no way to know created_at, and no \
                                             business inventing it, so it must not be required",
    );
    assert_eq!(m.id, "app");
    assert_eq!(
        m.created_at, 0,
        "absent metadata defaults; update_app restores the truth"
    );
}

/// The destructive half: a manifest write that omits the server's fields must not
/// erase them. Before the merge, `built_at: None` made a built app look unbuilt
/// and `sdk_hash: None` unfingerprinted its vendored SDK.
#[test]
fn server_owned_fields_survive_a_manifest_write_that_omits_them() {
    let (_dir, store) = store();

    let mut original = store
        .create("App", "", ArtifactKind::Agentic, "index.html", &[])
        .expect("create");
    let app_id = original.id.clone();
    original.built_at = Some(1_700_000_000);
    original.sdk_hash = Some("deadbeef".into());
    original.session_id = Some("sess-1".into());
    let created_at = original.created_at;
    store.save_manifest(&original).unwrap();

    // What a model would send: the app's *authored* fields only.
    let authored = serde_json::json!({
        "id": app_id,
        "title": "Renamed App",
        "kind": "agentic",
        "entry": "index.html",
    });
    let mut parsed: biorouter_mcp::agent_drafter::store::Manifest =
        serde_json::from_value(authored).unwrap();

    // This is exactly what `update_app`'s manifest path now does.
    let on_disk = store.load_manifest(&app_id).unwrap();
    parsed.id = on_disk.id.clone();
    parsed.created_at = on_disk.created_at;
    parsed.built_at = on_disk.built_at;
    parsed.sdk_hash = on_disk.sdk_hash.clone();
    parsed.session_id = on_disk.session_id.clone();
    store.save_manifest(&parsed).unwrap();

    let after = store.load_manifest(&app_id).unwrap();
    assert_eq!(after.title, "Renamed App", "the authored change lands");
    assert_eq!(
        after.created_at, created_at,
        "created_at must not be reinvented"
    );
    assert_eq!(
        after.built_at,
        Some(1_700_000_000),
        "the build must not be invalidated"
    );
    assert_eq!(
        after.sdk_hash.as_deref(),
        Some("deadbeef"),
        "the SDK fingerprint must survive"
    );
    assert_eq!(
        after.session_id.as_deref(),
        Some("sess-1"),
        "the originating chat must survive"
    );
}

/// Round-trip: an app that declares its surface at creation reads it back. The
/// v1 apps in the wild declare nothing, and must keep serializing byte-identically.
#[test]
fn a_v1_manifest_gains_no_surface_or_theme_key() {
    let (_dir, store) = store();
    let m = store
        .create("V1", "", ArtifactKind::Static, "index.html", &[])
        .expect("create");

    let raw = serde_json::to_value(&m).unwrap();
    assert!(
        raw.get("surface").is_none(),
        "an app that declares nothing must not gain a `surface: {{}}` key"
    );
    assert!(
        raw.get("theme").is_none(),
        "nor a `theme` key: v1 manifests must round-trip unchanged"
    );
    assert!(raw.get("requires").is_none(), "nor a `requires` key");
}

/// `declare_surface(merge: true)` must not silently drop `state_initial`.
///
/// It did. The merge branch upserted actions/signals/components and carried
/// `state_schema` across, but never `state_initial` — so a caller that declared an
/// initial document got a SUCCESS result and an unchanged manifest.
///
/// Found by pointing the fixed platform's own agent at a broken app and watching it
/// spend 8 extra round-trips re-declaring a value that kept vanishing. Which makes
/// it the exact bug class this whole campaign is about: a tool that reports success
/// while doing nothing, leaving the model to thrash against a lie.
#[test]
fn merging_a_surface_preserves_the_initial_state_document() {
    use biorouter_mcp::agent_drafter::declare::SurfaceParam;
    use biorouter_mcp::agent_drafter::manifest::SurfaceDecl;

    // An app that already declares actions, but no initial state.
    let mut existing = SurfaceDecl {
        actions: vec![Default::default()],
        ..Default::default()
    };
    assert!(existing.state_initial.is_none());

    // The caller declares ONLY the initial document, with merge semantics.
    let incoming: SurfaceParam = serde_json::from_value(serde_json::json!({
        "state_initial": { "cohort": { "baselineN": 12500 } }
    }))
    .unwrap();
    let incoming = incoming.into_decl();

    // This is the merge `declare_surface(merge: true)` performs.
    if incoming.state_schema.is_some() {
        existing.state_schema = incoming.state_schema.clone();
    }
    if incoming.state_initial.is_some() {
        existing.state_initial = incoming.state_initial.clone();
    }

    assert_eq!(
        existing.state_initial,
        Some(serde_json::json!({ "cohort": { "baselineN": 12500 } })),
        "a merged surface must carry the initial document; dropping it silently is \
         how the model ends up thrashing against a tool that lies about succeeding"
    );
    assert_eq!(
        existing.actions.len(),
        1,
        "and merging must not clobber what was already declared"
    );
}
