//! Issue #63 at the tool-inspection boundary: global (machine-wide) memory
//! access routed through the user's consent, and — the property the whole fix
//! rests on — a `GlobalMemoryInspector` verdict surviving every way the
//! permission layer can say "allowed".
//!
//! The wiring mirrors `Agent::create_tool_inspection_manager`: the gate is
//! registered *before* the permission inspector, so the escalation-only merge in
//! `tool_inspection` applies it as an override on top of the permission
//! baseline. Ordering matters — see `the_gate_survives_*` below.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::Arc;

use biorouter::config::permission::PermissionLevel;
use biorouter::config::{BioRouterMode, PermissionManager};
use biorouter::conversation::message::{Message, ToolRequest};
use biorouter::managed::{ManagedPolicy, ManagedPolicyFile};
use biorouter::permission::{
    ManagedPolicyInspector, PermissionInspector, SmartApproveConfig, ToolRiskRegistry,
};
use biorouter::security::global_memory::GlobalMemoryInspector;
use biorouter::session::Session;
use biorouter::tool_inspection::ToolInspectionManager;
use rmcp::model::CallToolRequestParams;
use serde_json::{json, Value};
use tempfile::TempDir;

fn tool_request(id: &str, name: &str, arguments: Value) -> ToolRequest {
    Message::assistant()
        .with_tool_request(
            id,
            Ok(CallToolRequestParams {
                task: None,
                meta: None,
                name: name.to_string().into(),
                arguments: Some(arguments.as_object().expect("object args").clone()),
            }),
        )
        .content
        .first()
        .and_then(|content| content.as_tool_request())
        .expect("message contains a tool request")
        .clone()
}

/// The agent's real registration order for the two inspectors under test.
fn build_manager() -> (ToolInspectionManager, Arc<PermissionManager>, TempDir) {
    let temp = TempDir::new().unwrap();
    let permission_manager = Arc::new(PermissionManager::new(temp.path().to_path_buf()));
    let managed = Arc::new(ManagedPolicy::empty());
    let mut manager = ToolInspectionManager::new();
    manager.add_inspector(Box::new(GlobalMemoryInspector));
    manager.add_inspector(Box::new(PermissionInspector::with_smart_config(
        Arc::new(ToolRiskRegistry::new()),
        Arc::clone(&permission_manager),
        managed,
        Arc::new(tokio::sync::Mutex::new(None)),
        SmartApproveConfig::default(),
    )));
    (manager, permission_manager, temp)
}

async fn decide(
    manager: &ToolInspectionManager,
    requests: &[ToolRequest],
    mode: BioRouterMode,
) -> biorouter::permission::permission_judge::PermissionCheckResult {
    let session = Session::default();
    let results = manager
        .inspect_tools(requests, &[], mode, &session)
        .await
        .expect("inspection succeeds");
    manager
        .process_inspection_results_with_permission_inspector(requests, &results)
        .expect("permission inspector present")
}

fn global_read(id: &str, category: &str) -> ToolRequest {
    tool_request(
        id,
        "memory__retrieve_memories",
        json!({"category": category, "is_global": true}),
    )
}

// --- the issue's own scenario ---------------------------------------------

/// The report, verbatim in substance: a session in Auto mode reading a global
/// memory another session wrote, "without the user being asked or told".
#[tokio::test]
async fn auto_mode_no_longer_reads_global_memory_unasked() {
    let (manager, _pm, _tmp) = build_manager();
    let requests = vec![global_read("r1", "clinical")];
    let result = decide(&manager, &requests, BioRouterMode::Auto).await;

    assert!(
        result.approved.is_empty(),
        "Auto mode's blanket allow must not reach a machine-wide memory read"
    );
    assert_eq!(result.needs_approval.len(), 1);
    assert_eq!(result.needs_approval[0].id, "r1");
}

/// `retrieve_memories(category="*", is_global=true)` returned the entire
/// machine-wide store in one call. It is refused outright.
#[tokio::test]
async fn auto_mode_refuses_the_whole_store_read() {
    let (manager, _pm, _tmp) = build_manager();
    let requests = vec![global_read("r1", "*")];
    let result = decide(&manager, &requests, BioRouterMode::Auto).await;

    assert_eq!(
        result.denied.len(),
        1,
        "the bulk global read must be denied"
    );
    assert_eq!(result.denied[0].id, "r1");
    assert!(result.approved.is_empty() && result.needs_approval.is_empty());
}

/// The write side, which was never gated at all (CLAUDE.md: "a real
/// confirmation needs the permission path in `biorouter::permission`").
#[tokio::test]
async fn auto_mode_asks_before_a_machine_wide_write() {
    let (manager, _pm, _tmp) = build_manager();
    let requests = vec![tool_request(
        "r1",
        "memory__remember_memory",
        json!({"category": "clinical", "data": "cohort 4217", "is_global": true}),
    )];
    let result = decide(&manager, &requests, BioRouterMode::Auto).await;

    assert_eq!(result.needs_approval.len(), 1);
    assert!(result.approved.is_empty());
}

// --- the property the fix rests on ----------------------------------------

/// A user `AlwaysAllow` on the memory tool is a *tool-name* grant. It must not
/// buy a machine-wide read, or the gate is one settings toggle from useless.
/// (This is also the precedent check the brief asks for: a security-inspector
/// override already beats an `always_allow`, and it still does.)
#[tokio::test]
async fn the_gate_survives_an_always_allow_grant() {
    let (manager, pm, _tmp) = build_manager();
    pm.update_user_permission("memory__retrieve_memories", PermissionLevel::AlwaysAllow);

    let requests = vec![global_read("r1", "clinical")];
    let result = decide(&manager, &requests, BioRouterMode::Approve).await;

    assert!(
        result.approved.is_empty(),
        "an AlwaysAllow on the tool name must not cover a global read"
    );
    assert_eq!(result.needs_approval.len(), 1);
}

/// The same grant on the *local* store keeps working. The gate narrows exactly
/// one axis; it does not quietly revoke a permission the user granted.
#[tokio::test]
async fn an_always_allow_grant_still_covers_local_recall() {
    let (manager, pm, _tmp) = build_manager();
    pm.update_user_permission("memory__retrieve_memories", PermissionLevel::AlwaysAllow);

    let requests = vec![tool_request(
        "r1",
        "memory__retrieve_memories",
        json!({"category": "*", "is_global": false}),
    )];
    let result = decide(&manager, &requests, BioRouterMode::Approve).await;

    assert_eq!(
        result.approved.len(),
        1,
        "local recall is untouched by this gate"
    );
}

/// SmartApprove grades tools from their MCP annotations and auto-approves what
/// reads as read-only. A retrieval is read-only *of a store the user cannot
/// see*, so the grade must not carry it.
#[tokio::test]
async fn the_gate_survives_a_smart_approve_read_only_grade() {
    let (manager, pm, _tmp) = build_manager();
    // The cached judge verdict SmartApprove would otherwise honour.
    pm.update_smart_approve_permission("memory__retrieve_memories", PermissionLevel::AlwaysAllow);

    let requests = vec![global_read("r1", "clinical")];
    let result = decide(&manager, &requests, BioRouterMode::SmartApprove).await;

    assert!(result.approved.is_empty());
    assert_eq!(result.needs_approval.len(), 1);
}

// --- the feature still works ----------------------------------------------

/// The whole reason the audit left this unfixed was that a blanket refusal kills
/// global memory. One approved category at a time is the point — nothing here
/// denies a named global read.
#[tokio::test]
async fn a_named_global_read_is_approvable_not_refused() {
    let (manager, _pm, _tmp) = build_manager();
    let requests = vec![global_read("r1", "clinical")];
    let result = decide(&manager, &requests, BioRouterMode::Auto).await;

    assert!(
        result.denied.is_empty(),
        "a named global read must remain reachable with the user's consent"
    );
    assert_eq!(result.needs_approval.len(), 1);
}

/// Local memory is not a cross-session channel and keeps running with no prompt
/// in Auto mode — including the bulk read.
#[tokio::test]
async fn local_memory_is_untouched_in_auto_mode() {
    let (manager, _pm, _tmp) = build_manager();
    let requests = vec![
        tool_request(
            "r1",
            "memory__retrieve_memories",
            json!({"category": "*", "is_global": false}),
        ),
        tool_request(
            "r2",
            "memory__remember_memory",
            json!({"category": "development", "data": "black", "is_global": false}),
        ),
    ];
    let result = decide(&manager, &requests, BioRouterMode::Auto).await;

    assert_eq!(
        result.approved.len(),
        2,
        "local memory must not start prompting: {result:?}",
        result = (
            result.approved.len(),
            result.needs_approval.len(),
            result.denied.len()
        )
    );
}

/// A tool call that has nothing to do with memory is unaffected in Auto mode.
#[tokio::test]
async fn unrelated_tools_are_unaffected() {
    let (manager, _pm, _tmp) = build_manager();
    let requests = vec![tool_request(
        "r1",
        "developer__shell",
        json!({"command": "ls -la"}),
    )];
    let result = decide(&manager, &requests, BioRouterMode::Auto).await;
    assert_eq!(result.approved.len(), 1);
}

/// …but "unaffected" is about the *tool*, not the tool's arguments, and the
/// store is a directory of text files. `cat <store>/clinical.txt` is the same
/// disclosure the consent card exists for and `rm -rf <store>` the same
/// destruction, taken with a tool the gate does not recognise by name.
///
/// The unit tests pin the classifier; this pins it through the real inspection
/// manager in Auto mode, where the permission baseline approves everything — the
/// exact configuration the issue was reported against, and the one where a
/// classifier that quietly stopped running would go unnoticed.
#[tokio::test]
async fn a_shell_command_that_reads_the_global_store_is_refused() {
    let store = biorouter_mcp::global_memory_dir();
    let store = store.display().to_string();

    for (id, command) in [
        ("r1", format!("cat {store}/clinical.txt")),
        ("r2", format!("rm -rf {store}")),
        ("r3", format!("tar czf /tmp/x.tgz {store}")),
    ] {
        let (manager, _pm, _tmp) = build_manager();
        let requests = vec![tool_request(
            id,
            "developer__shell",
            json!({"command": command}),
        )];
        let result = decide(&manager, &requests, BioRouterMode::Auto).await;

        assert_partition(&result);
        assert!(
            result.approved.is_empty(),
            "{command:?} reached the machine-wide memory store with nobody asked"
        );
        assert_eq!(
            result.denied.len(),
            1,
            "{command:?} must be refused outright; approving \"run this shell \
             command\" is not consent to disclose a memory category, and there is \
             a call that asks by name"
        );
        assert!(
            result.needs_approval.is_empty(),
            "{command:?} must not become a card that reads as a shell prompt"
        );
    }
}

/// Denying by path must not deny by resemblance, or ordinary work in the config
/// directory stops. Through the real manager, in the same mode.
#[tokio::test]
async fn a_shell_command_near_the_global_store_is_still_approved() {
    let store = biorouter_mcp::global_memory_dir();
    let parent = store.parent().unwrap().display().to_string();
    let store = store.display().to_string();

    for (id, command) in [
        // A sibling whose name merely starts with the store's.
        ("r1", format!("cat {store}-archive.txt")),
        // Backing up ~/.config is an ordinary thing to ask for.
        ("r2", format!("tar czf /tmp/config.tgz {parent}")),
        // Project-local memory: under the directory the user opened.
        ("r3", "cat .biorouter/memory/development.txt".to_string()),
    ] {
        let (manager, _pm, _tmp) = build_manager();
        let requests = vec![tool_request(
            id,
            "developer__shell",
            json!({"command": command}),
        )];
        let result = decide(&manager, &requests, BioRouterMode::Auto).await;

        assert_partition(&result);
        assert_eq!(
            result.approved.len(),
            1,
            "{command:?} is not the machine-wide store and must not be refused"
        );
    }
}

// --- ordering -------------------------------------------------------------

/// The merge is escalation-only, so the *registration order* of these two
/// inspectors must not matter — a mode-based blanket allow cannot arrive "ahead
/// of" the override and win. Pinned because a future reordering that broke it
/// would leave every assertion above passing in the order they happen to use.
#[tokio::test]
async fn registration_order_cannot_defeat_the_gate() {
    let temp = TempDir::new().unwrap();
    let permission_manager = Arc::new(PermissionManager::new(temp.path().to_path_buf()));
    let mut manager = ToolInspectionManager::new();
    // Deliberately inverted: permission first, gate last.
    manager.add_inspector(Box::new(PermissionInspector::with_smart_config(
        Arc::new(ToolRiskRegistry::new()),
        Arc::clone(&permission_manager),
        Arc::new(ManagedPolicy::empty()),
        Arc::new(tokio::sync::Mutex::new(None)),
        SmartApproveConfig::default(),
    )));
    manager.add_inspector(Box::new(GlobalMemoryInspector));

    let requests = vec![global_read("r1", "clinical")];
    let result = decide(&manager, &requests, BioRouterMode::Auto).await;
    assert_eq!(
        result.needs_approval.len(),
        1,
        "the gate must win from either position in the inspector list"
    );
}

// --- the gate must not *weaken* anything either ----------------------------
//
// The merge is escalation-only, which is what makes this gate survive Auto and
// `AlwaysAllow` above. The mirror-image obligation is that its `RequireApproval`
// never *lowers* a denial: a call in both `denied` and `needs_approval` has its
// denial written first and an approval card raised second, and "Allow once" on
// that card dispatches the tool and overwrites the denial. An approval card is
// then a way to run something that was refused.

/// Every request the merge produces must be in exactly one of the three sets.
fn assert_partition(result: &biorouter::permission::permission_judge::PermissionCheckResult) {
    for id in result
        .approved
        .iter()
        .chain(&result.needs_approval)
        .chain(&result.denied)
        .map(|r| r.id.clone())
    {
        let sets = [&result.approved, &result.needs_approval, &result.denied]
            .iter()
            .filter(|set| set.iter().any(|r| r.id == id))
            .count();
        assert_eq!(
            sets,
            1,
            "{id} is in {sets} result sets; approved/needs_approval/denied must partition \
             the batch: {:?}",
            (
                result.approved.len(),
                result.needs_approval.len(),
                result.denied.len()
            )
        );
    }
}

/// The user's own `NeverAllow` on the memory tool is the strongest thing they can
/// say about it. The gate's ask must not convert that standing "never" into a
/// per-call "allow once" card.
#[tokio::test]
async fn a_user_never_allow_is_not_reopened_by_the_memory_card() {
    for mode in [BioRouterMode::Approve, BioRouterMode::SmartApprove] {
        let (manager, pm, _tmp) = build_manager();
        pm.update_user_permission("memory__retrieve_memories", PermissionLevel::NeverAllow);

        let requests = vec![global_read("r1", "clinical")];
        let result = decide(&manager, &requests, mode).await;

        assert_partition(&result);
        assert_eq!(
            result.denied.len(),
            1,
            "{mode:?}: the NeverAllow must stand"
        );
        assert!(
            result.needs_approval.is_empty(),
            "{mode:?}: a tool the user said never to run must not come back as a card"
        );
    }
}

/// A managed (trusted admin) `deny` is non-bypassable by construction — that is
/// the whole point of the tier. It reaches the merge as a *non-permission* Deny,
/// so it collides with the memory ask inside `apply_inspection_results_to_permissions`
/// rather than in the baseline, and the collision must resolve the same way.
///
/// Both registration orders are exercised: the agent registers managed first, but
/// the merge is a lattice and must not depend on that.
#[tokio::test]
async fn a_managed_deny_is_not_reopened_by_the_memory_card() {
    let policy: ManagedPolicyFile =
        serde_yaml::from_str("permissions:\n  deny: [\"memory__retrieve_memories\"]\n")
            .expect("managed yaml parses");
    let policy = Arc::new(ManagedPolicy::from_file(policy));

    for gate_first in [false, true] {
        let temp = TempDir::new().unwrap();
        let permission_manager = Arc::new(PermissionManager::new(temp.path().to_path_buf()));
        let mut manager = ToolInspectionManager::new();
        if gate_first {
            manager.add_inspector(Box::new(GlobalMemoryInspector));
        }
        manager.add_inspector(Box::new(ManagedPolicyInspector::new(Arc::clone(&policy))));
        if !gate_first {
            manager.add_inspector(Box::new(GlobalMemoryInspector));
        }
        manager.add_inspector(Box::new(PermissionInspector::with_smart_config(
            Arc::new(ToolRiskRegistry::new()),
            permission_manager,
            Arc::clone(&policy),
            Arc::new(tokio::sync::Mutex::new(None)),
            SmartApproveConfig::default(),
        )));

        let requests = vec![global_read("r1", "clinical")];
        // Auto is the sharp case: the permission baseline allows everything, so
        // the managed Deny and the memory Ask are both merge-time overrides and
        // the *order* they were registered in is all that separates them.
        let result = decide(&manager, &requests, BioRouterMode::Auto).await;

        assert_partition(&result);
        assert_eq!(
            result.denied.len(),
            1,
            "gate_first={gate_first}: the managed deny must stand"
        );
        assert!(
            result.needs_approval.is_empty(),
            "gate_first={gate_first}: an organization's deny must not be reopened as a \
             consent card the user can click through"
        );
    }
}

/// The gate's own `Deny` (the whole-store read) and its own `Ask` landing on the
/// same batch must not contaminate each other — a Deny on one request may not
/// leak into another, and the refused call may not also acquire a card.
#[tokio::test]
async fn a_refused_bulk_read_and_an_asked_named_read_stay_separate() {
    let (manager, _pm, _tmp) = build_manager();
    let requests = vec![global_read("r1", "*"), global_read("r2", "clinical")];
    let result = decide(&manager, &requests, BioRouterMode::Auto).await;

    assert_partition(&result);
    assert_eq!(result.denied.len(), 1);
    assert_eq!(result.denied[0].id, "r1");
    assert_eq!(result.needs_approval.len(), 1);
    assert_eq!(result.needs_approval[0].id, "r2");
    assert!(result.approved.is_empty());
}
