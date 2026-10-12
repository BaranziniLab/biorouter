//! The *resolved* manifest view — what `read_app` shows the model.
//!
//! A serialized manifest is a **diff against defaults**: every optional block
//! carries `skip_serializing_if`, so an explicitly-chosen default disappears on
//! save. `theme.pack = "biorouter"` is `ThemeConfig::is_default()`, so it is
//! omitted; an empty `surface` is omitted; unset capabilities are omitted. That
//! is correct for the file on disk (a v1 manifest re-serializes byte-identically)
//! and *wrong* for a reader, which sees **absence** where the truth is
//! **default** — and which is never shown the shape of the fields it must fill in.
//!
//! Two real failures came out of that ambiguity:
//!
//!   * a reviewer read an omitted `theme` block as "no pack was set" and filed a
//!     bug against an app that had correctly set the default pack;
//!   * the authoring model, having no skeleton to edit, round-tripped whole
//!     manifests by guessing field shapes from serde errors — six rejected
//!     mutations per app.
//!
//! [`resolved_view`] emits a canonical, fully-populated, editable skeleton:
//! every optional block present, the theme pack resolved through
//! [`ThemeConfig::resolved_pack`], and a `_server_managed` list naming the keys
//! the caller must not invent. `read_app(view: "raw")` still returns the
//! on-disk bytes for anything that needs fidelity.

use serde_json::{json, Value};

use super::store::Manifest;

/// Keys the server owns. A caller that writes these is either inventing metadata
/// (`created_at`), destroying build state (`built_at`, `sdk_hash`) or pointing
/// the app at another app's conversations (`app_uid`) — the manifest merge in
/// `update_app` restores all of them, and naming them here is what tells the
/// model not to try.
pub const SERVER_MANAGED_KEYS: &[&str] = &[
    "id",
    "created_at",
    "updated_at",
    "built_at",
    "sdk_hash",
    "session_id",
    "app_uid",
];

/// A fully-populated view of `m`: no field is absent merely because it holds its
/// default. Safe to hand to a model as an editable skeleton, and safe to feed
/// back into `update_app` (`Manifest` has no `deny_unknown_fields`, and the merge
/// restores the server-managed keys regardless).
pub fn resolved_view(m: &Manifest) -> Value {
    // Start from the real serialization so we never drift from the struct, then
    // fill in what `skip_serializing_if` dropped.
    let mut v = serde_json::to_value(m).unwrap_or_else(|_| json!({}));
    let obj = match v.as_object_mut() {
        Some(o) => o,
        None => return v,
    };

    // Theme: always present, with the pack *resolved* (an unknown pack in the
    // file resolves to the default rather than showing a value the renderer
    // would never use).
    obj.insert(
        "theme".to_string(),
        json!({
            "pack": m.theme.resolved_pack(),
            "accent": m.theme.sanitized_accent(),
            "tokens": m.theme.tokens,
        }),
    );

    // Fill omitted defaults without discarding fields from the real serialization.
    if let Some(surface) = obj
        .entry("surface")
        .or_insert_with(|| json!({}))
        .as_object_mut()
    {
        surface.entry("state_schema").or_insert(Value::Null);
        surface.entry("state_initial").or_insert(Value::Null);
        for key in ["actions", "signals", "components"] {
            surface.entry(key).or_insert_with(|| json!([]));
        }
    }

    // Agent: show the deny-by-default grants explicitly, plus the tokens the
    // client will actually be told about.
    if let Some(agent) = &m.agent {
        let caps = &agent.capabilities;
        if let Some(agent_obj) = obj.get_mut("agent").and_then(|a| a.as_object_mut()) {
            agent_obj.insert(
                "capabilities".to_string(),
                json!({
                    "files": caps.files,
                    "data": caps.data,
                    "compute": caps.compute,
                    "vault": caps.vault,
                    "memory": caps.memory,
                    "tracing": caps.tracing,
                    "ui": caps.ui,
                    "events": caps.events,
                    "_advertised": caps.advertised(),
                }),
            );
            agent_obj.insert(
                "orchestration".to_string(),
                serde_json::to_value(&agent.orchestration).unwrap_or_else(|_| json!({})),
            );
            agent_obj.insert(
                "durable_session".to_string(),
                json!(agent.durable_session()),
            );
        }
    }

    obj.insert("_server_managed".to_string(), json!(SERVER_MANAGED_KEYS));
    v
}

/// Which view `read_app` should return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestView {
    /// Canonical, fully-populated skeleton (the default).
    Resolved,
    /// The bytes as serialized on disk.
    Raw,
    /// The preview card the USER sees, rather than JSON for the model.
    /// Absorbed from the retired `preview_app`, whose whole body was
    /// load_manifest + card_result.
    Card,
}

impl ManifestView {
    /// Parse the `view` param. Unknown values are an error rather than a silent
    /// fallback — a typo'd view must not quietly change what the model reads.
    pub fn parse(s: Option<&str>) -> Result<Self, String> {
        match s.map(str::trim) {
            None | Some("") | Some("resolved") => Ok(Self::Resolved),
            Some("raw") => Ok(Self::Raw),
            Some("card") => Ok(Self::Card),
            Some(other) => Err(format!(
                "unknown view '{other}': expected \"resolved\" (default), \"raw\" or \"card\""
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_drafter::manifest::DEFAULT_THEME_PACK;
    use crate::agent_drafter::store::{ArtifactKind, Manifest};

    fn v1_manifest() -> Manifest {
        Manifest {
            id: "app".into(),
            title: "App".into(),
            description: String::new(),
            kind: ArtifactKind::Agentic,
            entry: "index.html".into(),
            created_at: 1,
            updated_at: 2,
            agent: None,
            width: None,
            height: None,
            built_at: None,
            sdk_hash: None,
            session_id: None,
            app_uid: None,
            surface: Default::default(),
            theme: Default::default(),
        }
    }

    /// The exact ambiguity that produced a false bug report: a v1 manifest
    /// serializes with **no** `theme` key, and a reader concluded "no pack".
    #[test]
    fn raw_serialization_omits_the_default_theme_but_the_resolved_view_shows_it() {
        let m = v1_manifest();

        let raw = serde_json::to_value(&m).unwrap();
        assert!(
            raw.get("theme").is_none(),
            "on-disk bytes must stay byte-identical for a v1 manifest"
        );

        let resolved = resolved_view(&m);
        assert_eq!(resolved["theme"]["pack"], DEFAULT_THEME_PACK);
    }

    /// The surface is the skeleton the model must edit; all fields are present
    /// even when nothing is declared.
    #[test]
    fn resolved_view_always_carries_the_full_surface_shape() {
        let resolved = resolved_view(&v1_manifest());
        let surface = &resolved["surface"];
        for key in [
            "state_schema",
            "state_initial",
            "actions",
            "signals",
            "components",
        ] {
            assert!(surface.get(key).is_some(), "surface.{key} missing");
        }
        assert_eq!(surface["actions"], json!([]));
    }

    #[test]
    fn resolved_view_preserves_initial_state_when_used_as_an_editable_manifest() {
        let mut manifest = v1_manifest();
        manifest.surface.state_schema = Some(json!({"type": "object"}));
        manifest.surface.state_initial = Some(json!({
            "metrics": {"cohorts": {"value": 0, "delta": "Not loaded"}},
            "ready": false
        }));
        let serialized_surface = serde_json::to_value(&manifest.surface).unwrap();
        let resolved = resolved_view(&manifest);
        for (key, value) in serialized_surface.as_object().unwrap() {
            assert_eq!(resolved["surface"][key], *value, "surface.{key} changed");
        }
        let edited: Manifest = serde_json::from_value(resolved).unwrap();
        assert_eq!(edited.surface.state_initial, manifest.surface.state_initial);
    }

    /// An unknown pack on disk must read back as what the renderer will actually
    /// use, not as the bogus value.
    #[test]
    fn an_unknown_pack_resolves_to_the_default() {
        let mut m = v1_manifest();
        m.theme.pack = "not-a-pack".into();
        assert_eq!(resolved_view(&m)["theme"]["pack"], DEFAULT_THEME_PACK);
    }

    #[test]
    fn resolved_view_names_the_server_managed_keys() {
        let resolved = resolved_view(&v1_manifest());
        let managed = resolved["_server_managed"].as_array().unwrap();
        assert!(managed.iter().any(|k| k == "created_at"));
        assert!(managed.iter().any(|k| k == "sdk_hash"));
        // A model that rewrote it could graft one app's conversations onto
        // another; `update_app` restores it from disk.
        assert!(managed.iter().any(|k| k == "app_uid"));
    }

    #[test]
    fn view_parsing() {
        assert_eq!(ManifestView::parse(None), Ok(ManifestView::Resolved));
        assert_eq!(
            ManifestView::parse(Some("resolved")),
            Ok(ManifestView::Resolved)
        );
        assert_eq!(ManifestView::parse(Some("raw")), Ok(ManifestView::Raw));
        assert!(ManifestView::parse(Some("full")).is_err());
    }
}
