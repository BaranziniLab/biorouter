//! An action has an effect, and the turn knows whether it ran.
//!
//! `ActionDecl` used to carry only name/description/params — no declared effect,
//! no declared writes. So the platform could not tell "apply an intervention" from
//! "read a value", and therefore could not require that one was ever called. The
//! agent could simply *simulate* the effect: `ui_patch_state` will happily write
//! `/params/lion_vision` directly, and `ui_render` will draw the narrative, without
//! the app's handler ever running.
//!
//! Specs 011, 013 and 014 did exactly that — the page showed an intervention being
//! applied that was never applied. The prompt already said "call the action before
//! you narrate it". It was ignored, because prose is not an enforcement mechanism.
//!
//! Two things now make it structurally impossible:
//!
//!   1. **pointer ownership** — a value a `mutate` action declares it writes can
//!      only be changed by calling that action's handler; a direct state write is
//!      refused; and
//!   2. **readback** — `app_call` re-reads those pointers afterwards and tells the
//!      model what actually moved, or that nothing did.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::agent_drafter::control::{AppControlServer, UiBridge};
use biorouter_mcp::agent_drafter::manifest::{ActionDecl, ActionEffect, SurfaceDecl, UiCapability};

/// The Serengeti case (spec 014): an action that applies an intervention by
/// writing `/params/lion_vision`.
fn intervention_surface() -> SurfaceDecl {
    SurfaceDecl {
        actions: vec![
            ActionDecl {
                name: "apply_intervention".into(),
                description: "Apply a parameter intervention to the simulation.".into(),
                params: serde_json::json!({ "type": "object" }),
                effect: ActionEffect::Mutate,
                writes: vec!["/params/lion_vision".into()],
                ..Default::default()
            },
            ActionDecl {
                name: "describe_scene".into(),
                description: "Read the current scene.".into(),
                params: serde_json::json!({}),
                effect: ActionEffect::Read,
                writes: vec![],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn bridge_with(surface: SurfaceDecl) -> UiBridge {
    let bridge = UiBridge::new();
    let _server = AppControlServer::new(bridge.clone(), UiCapability::default(), surface);
    bridge
}

/// THE test: the agent cannot write the number itself.
#[test]
fn the_agent_cannot_directly_write_a_pointer_an_action_owns() {
    let bridge = bridge_with(intervention_surface());

    let err = bridge
        .check_write_allowed("/params/lion_vision")
        .unwrap_err();

    assert!(
        err.contains("apply_intervention"),
        "the refusal must name the action that owns it: {err}"
    );
    assert!(
        err.contains("app_call"),
        "and must say how to legitimately make the change: {err}"
    );
    assert!(
        err.contains("never computed"),
        "and must say why this matters: {err}"
    );
}

/// Ownership extends *under* the pointer — otherwise the guard is trivially
/// sidestepped by patching one level deeper.
#[test]
fn ownership_covers_paths_beneath_an_owned_pointer() {
    let bridge = bridge_with(intervention_surface());

    assert!(
        bridge
            .check_write_allowed("/params/lion_vision/min")
            .is_err(),
        "a nested path under an owned pointer must also be refused"
    );
    assert_eq!(
        bridge.owner_of_path("/params/lion_vision/min").as_deref(),
        Some("apply_intervention")
    );
}

/// The guard must be narrow. Unowned state is still the agent's to write — the
/// `ui_state` / `ui_patch_state` tools remain useful for everything else.
#[test]
fn unowned_state_is_still_freely_writable() {
    let bridge = bridge_with(intervention_surface());

    assert!(bridge.check_write_allowed("/notes").is_ok());
    assert!(bridge.check_write_allowed("/params/other_knob").is_ok());
    // A prefix collision is not ownership.
    assert!(bridge
        .check_write_allowed("/params/lion_vision_backup")
        .is_ok());
}

/// A `read` action owns nothing, even if someone lists `writes` on it.
#[test]
fn a_read_action_owns_nothing() {
    let surface = SurfaceDecl {
        actions: vec![ActionDecl {
            name: "describe_scene".into(),
            description: String::new(),
            params: serde_json::json!({}),
            effect: ActionEffect::Read,
            // Nonsensical, but must not lock anything: only `mutate` owns.
            writes: vec!["/params/lion_vision".into()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let bridge = bridge_with(surface);

    assert!(bridge.check_write_allowed("/params/lion_vision").is_ok());
    assert!(bridge.owned_pointers().is_empty());
}

/// Back-compat: every v1 action defaults to `read` with no `writes`, so nothing
/// an existing app does becomes forbidden.
#[test]
fn a_v1_action_locks_nothing() {
    let surface = SurfaceDecl {
        actions: vec![ActionDecl {
            name: "focus_node".into(),
            description: "Center a node.".into(),
            params: serde_json::json!({}),
            ..Default::default()
        }],
        ..Default::default()
    };
    let bridge = bridge_with(surface);

    assert_eq!(
        ActionEffect::default(),
        ActionEffect::Read,
        "an undeclared effect must be the harmless one"
    );
    assert!(bridge.owned_pointers().is_empty());
    assert!(bridge.check_write_allowed("/anything").is_ok());
}

/// A v1 manifest must not gain `effect` / `writes` keys on re-serialize.
#[test]
fn a_v1_action_round_trips_byte_identically() {
    let decl = ActionDecl {
        name: "focus_node".into(),
        description: "Center a node.".into(),
        params: serde_json::json!({ "type": "object" }),
        ..Default::default()
    };

    let raw = serde_json::to_value(&decl).unwrap();
    assert!(
        raw.get("effect").is_none(),
        "a read action must not serialize an `effect` key: {raw}"
    );
    assert!(
        raw.get("writes").is_none(),
        "nor an empty `writes` array: {raw}"
    );
}

/// A mutate action DOES serialize its contract — that is the point.
#[test]
fn a_mutate_action_serializes_its_ownership() {
    let decl = ActionDecl {
        name: "apply_intervention".into(),
        description: String::new(),
        params: serde_json::json!({}),
        effect: ActionEffect::Mutate,
        writes: vec!["/params/lion_vision".into()],
        ..Default::default()
    };

    let raw = serde_json::to_value(&decl).unwrap();
    assert_eq!(raw["effect"], "mutate");
    assert_eq!(raw["writes"][0], "/params/lion_vision");

    let back: ActionDecl = serde_json::from_value(raw).unwrap();
    assert!(back.effect.is_mutate());
    assert_eq!(back.writes, vec!["/params/lion_vision".to_string()]);
}
