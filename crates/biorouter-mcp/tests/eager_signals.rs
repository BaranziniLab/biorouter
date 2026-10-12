//! Declaring a signal IS subscribing to it.
//!
//! This is the fix for the single worst result in the 100-app test drive:
//! app→agent signals round-tripped **1 time in 12**. The cause was an
//! unwinnable ordering race, not model sloppiness:
//!
//!   1. the bridge's subscription set started **empty** on every connection;
//!   2. the only way to fill it was the agent voluntarily calling `ui_subscribe`;
//!   3. `validate_signal` failed closed on an unsubscribed name; and
//!   4. the server turned that failure into a warning and **dropped the payload**.
//!
//! But the user's first click necessarily happens *before* the agent's first tool
//! call. The gesture that mattered most was always validated against an empty set.
//! No prompt can win a race that is decided before any prompt is evaluated — one
//! probe called `ui_subscribe` five times in a row trying.
//!
//! These tests assert the contract now holds with **zero tool calls**.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::agent_drafter::control::{AppControlServer, UiBridge};
use biorouter_mcp::agent_drafter::manifest::{SignalDecl, SurfaceDecl, UiCapability};
use serde_json::json;

fn surface_with_signals(signals: Vec<SignalDecl>) -> SurfaceDecl {
    SurfaceDecl {
        signals,
        ..Default::default()
    }
}

fn signal(name: &str) -> SignalDecl {
    SignalDecl {
        name: name.to_string(),
        ..Default::default()
    }
}

/// Build a control server the way `apps.rs` does — this is what mirrors the
/// surface onto the bridge, and therefore what seeds the eager set.
fn bridge_with(surface: SurfaceDecl) -> UiBridge {
    let bridge = UiBridge::new();
    let _server = AppControlServer::new(bridge.clone(), UiCapability::default(), surface);
    bridge
}

/// THE test. A declared signal validates before any tool has been called.
#[test]
fn a_declared_signal_is_accepted_before_the_agent_has_called_anything() {
    let bridge = bridge_with(surface_with_signals(vec![signal("criterion_clicked")]));

    assert!(
        bridge
            .validate_signal("criterion_clicked", &json!({"id": "age"}))
            .is_ok(),
        "the user clicks before the agent's first tool call, so a declared signal \
         must already be subscribed, or the gesture is lost exactly when it matters"
    );
}

/// `ui_describe` must SHOW the agent it is already listening, or the model will
/// keep calling `ui_subscribe` to "turn signals on" (which is what it did).
#[test]
fn the_agent_can_see_that_it_is_already_subscribed() {
    let bridge = bridge_with(surface_with_signals(vec![
        signal("criterion_clicked"),
        signal("stratum_changed"),
    ]));

    let subscribed = bridge.subscribed_signals();
    assert_eq!(subscribed, vec!["criterion_clicked", "stratum_changed"]);
    assert_eq!(
        bridge.eager_signals(),
        vec!["criterion_clicked", "stratum_changed"]
    );
}

/// A narrowing `ui_subscribe` must never drop below the declared floor. Without
/// this, one `ui_subscribe(["other"])` would silently re-break the app for the
/// rest of the session — the exact bug, reintroduced by the fix.
#[test]
fn an_explicit_subscribe_cannot_unsubscribe_a_declared_signal() {
    let bridge = bridge_with(surface_with_signals(vec![
        signal("criterion_clicked"),
        signal("stratum_changed"),
    ]));

    // The agent narrows to one signal.
    bridge.replace_subscriptions_for_test(vec!["stratum_changed".into()]);

    assert!(
        bridge
            .validate_signal("criterion_clicked", &json!({}))
            .is_ok(),
        "a declared (eager) signal stays subscribed no matter what ui_subscribe says"
    );
}

/// Opting out is still possible — but you have to say so.
#[test]
fn a_signal_can_opt_out_of_eager_delivery() {
    let lazy = SignalDecl {
        name: "expensive_hover".into(),
        eager: false,
        ..Default::default()
    };
    let bridge = bridge_with(surface_with_signals(vec![lazy]));

    assert!(
        bridge
            .validate_signal("expensive_hover", &json!({}))
            .is_err(),
        "eager: false means the agent must opt in explicitly"
    );
    assert!(bridge.eager_signals().is_empty());
}

/// The contract is a whitelist: a signal the app never declared is still refused.
/// Eager subscription must not become "accept anything".
#[test]
fn an_undeclared_signal_is_still_refused() {
    let bridge = bridge_with(surface_with_signals(vec![signal("criterion_clicked")]));

    let err = bridge
        .validate_signal("something_invented", &json!({}))
        .unwrap_err();
    assert!(
        err.contains("not subscribed") || err.contains("not declared"),
        "got: {err}"
    );
}

/// Back-compat: a v1 app declares no signals at all, so nothing changes for it.
#[test]
fn a_v1_app_with_no_declared_signals_is_unaffected() {
    let bridge = bridge_with(SurfaceDecl::default());
    assert!(bridge.eager_signals().is_empty());
    assert!(bridge.subscribed_signals().is_empty());
    assert!(bridge.validate_signal("anything", &json!({})).is_err());
}

/// A reconnect must restore the floor — the page reloading is not an opt-out.
#[test]
fn the_eager_floor_survives_a_reconnect() {
    let bridge = bridge_with(surface_with_signals(vec![signal("criterion_clicked")]));

    let (_rx, token) = bridge.attach();
    bridge.detach(token);
    let (_rx2, _token2) = bridge.attach();

    assert!(
        bridge
            .validate_signal("criterion_clicked", &json!({}))
            .is_ok(),
        "a browser reload must not silently unsubscribe the agent"
    );
}
