//! The person tools' workspace actions (AO-12): what the workspace holds for the person
//! (`workspace_show`, `members`, `enroll_pending`, `invites_list`, `privacy_show`, the profile,
//! the grants), and the changes a member or host makes to it.
//!
//! A read of workspace content goes through [`Ctx::read_content`]: the privacy scope of the
//! call, the person's read consent, then the raise. A change parks its card with the targets
//! already resolved and then sends exactly what the card showed, through the same
//! `CrewManager::human_request` the `biorouter crew` commands reach through
//! `POST /crew/connections/{id}/request`, or the same manager method a route calls.

use anyhow::Result;
use biorouter::crew::{ClusterMode, Connection};
use biorouter::privacy::{ProviderTier, SessionClassification};
use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::places::{naming_scope, place as target, Place as Target};
use super::setup::{hosts, save_over, saved_fields};
use super::{document_diff, idempotency_key, naming, said, Card, ContentScope, Ctx, Resolved};
use crate::routes::crew::names::SelectorKind;

/// The context document's size limit, as `biorouter crew workspace context set` holds it.
const MAX_CONTEXT_BYTES: usize = 64 * 1024;
/// Said on a change's card when a Private chat writes into something Public (AO-12, "Writes").
const CROSSING: &str = "This chat is Private, and this goes to a Public workspace: a private-to-public crossing. Everything it sends is shown here.";

// ------------------------------------------------------------------------------------- reads

pub(super) fn workspace_show(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move { ctx.read_content(|visible, _| Ok(visible.clone())).await })
}

/// The id of the workspace's host, as the snapshot says it.
fn host_id(snapshot: &Value) -> Option<String> {
    snapshot["workspace"]["host_principal_id"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            let uid = snapshot["workspace"]["host_uid"].as_u64()?;
            snapshot["principals"]
                .as_array()?
                .iter()
                .find(|person| {
                    person["uid"].as_u64() == Some(uid) && person["active"].as_bool() != Some(false)
                })
                .and_then(|person| person["id"].as_str())
                .map(str::to_owned)
        })
}

/// `biorouter crew members`: the members, each marked `is_host` and `is_you`, the host first.
pub(super) fn members(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        ctx.read_content(|visible, _| {
            let host = host_id(visible);
            let you = visible["actor"]["id"].as_str().map(str::to_owned);
            let mut people: Vec<Value> = visible["principals"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for person in &mut people {
                let id = person["id"].as_str().map(str::to_owned);
                person["is_host"] = json!(id.is_some() && id == host);
                person["is_you"] = json!(id.is_some() && id == you);
            }
            people.sort_by_key(|person| person["is_host"].as_bool() != Some(true));
            Ok(json!({"members": people}))
        })
        .await
    })
}

/// `biorouter crew enroll pending`: who is waiting to join. The workspace lists them only to
/// its host.
pub(super) fn enroll_pending(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        ctx.read_content(|visible, _| match visible.get("pending_joins") {
            Some(joins) => Ok(json!({"pending_joins": joins})),
            None if hosts(visible) == Some(true) => Err(said(
                Some("crew_join_by_name_unsupported"),
                "This workspace's server doesn't support joining by name.",
            )),
            None => Err(said(
                Some("crew_host_required"),
                "Only the workspace host can see who is waiting to join.",
            )),
        })
        .await
    })
}

pub(super) fn invites_list(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        ctx.read_content(|visible, _| {
            let mut answer =
                json!({"invitations": visible.get("invitations").cloned().unwrap_or(json!([]))});
            // To a public model, an invitation to a channel it is not shown is counted, not
            // named (`public_snapshot`).
            if let Some(withheld) = visible["withheld"].get("invitations") {
                answer["withheld"] = json!({"invitations": withheld});
            }
            Ok(answer)
        })
        .await
    })
}

/// `biorouter crew privacy show`: the connection's own setting, the workspace's, the one in
/// force, and each channel's classification the call may see.
pub(super) fn privacy_show(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let connection = ctx.connection().await?;
        let answer = ctx
            .read_content(|visible, scope: &ContentScope| {
                let workspace = &visible["workspace"];
                let effective = if connection.mode == ClusterMode::Public
                    && workspace["mode"] == "public"
                {
                    "public"
                } else {
                    "private"
                };
                let channels: Vec<Value> = visible["channels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|channel| {
                        let id = channel["id"].as_str().unwrap_or_default();
                        json!({
                            "id": id,
                            "name": channel["name"],
                            "team_id": channel["team_id"],
                            "protected": scope.all_protected || scope.protected_channels.contains(id),
                        })
                    })
                    .collect();
                Ok(json!({
                    "connection_id": connection.id,
                    "personal_mode": connection.mode,
                    "institution_id": connection.institution_id,
                    "effective_mode": effective,
                    "workspace": {
                        "name": workspace["name"],
                        "mode": workspace["mode"],
                        "institution_id": workspace["institution_id"],
                    },
                    "channels": channels,
                    "withheld": visible.get("withheld"),
                }))
            })
            .await?;
        Ok(answer)
    })
}

pub(super) fn profile_show(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        ctx.read_content(|visible, _| {
            let mut actor = visible["actor"].clone();
            if let Some(actor) = actor.as_object_mut() {
                actor.remove("devices");
            }
            Ok(json!({"profile": actor}))
        })
        .await
    })
}

/// `biorouter crew profile devices` (AO-10): the person's own computers, which one is this
/// computer, and which are new. Fingerprints and dates, no workspace content, so no consent.
pub(super) fn profile_devices(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let connection = ctx.connection().await?;
        let snapshot = ctx.snapshot(&connection).await?;
        let new_devices = ctx
            .crew()?
            .new_devices(&connection.id)
            .await
            .unwrap_or_default();
        Ok(json!({
            "connection_id": connection.id,
            "devices": snapshot["actor"]["devices"],
            "new_devices": new_devices,
        }))
    })
}

/// `biorouter crew grants list` (AO-12): the route's own listing, keeping only the chats this
/// chat may list ([`Ctx::lists_chat`]), so a listing never names a chat it could not read.
///
/// A grant's `labels` name the workspace's channels, teams and people, which is content: they
/// are kept only under a scope that may name things from the workspace ([`naming_scope`]: the
/// call may read it and the person let this chat read it as them), narrowed to what a public
/// call may see, and the chat is raised as a read raises it before they return. Otherwise,
/// and whenever the snapshot can't be read, they are left out and counted. A replaced grant's
/// chat ID may name another chat now, so a public call is never shown its labels.
pub(super) fn grants_list(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let connection = ctx.connection().await?;
        let listed = crate::routes::crew_profile::grant_list(&ctx.state, &connection.id)
            .await
            .map_err(profile_refusal)?;
        let mut answer = serde_json::to_value(listed.0)?;
        for key in ["grants", "replaced_grants", "person_reads"] {
            let rows = answer[key].as_array().cloned().unwrap_or_default();
            let mut kept = Vec::with_capacity(rows.len());
            for row in rows {
                let session = row["session_id"].as_str().unwrap_or_default();
                if ctx.lists_chat(session).await {
                    kept.push(row);
                }
            }
            answer[key] = json!(kept);
        }
        let names = match ctx.snapshot(&connection).await {
            Ok(snapshot) => naming_scope(ctx, &connection, &snapshot).await,
            Err(_) => None,
        };
        let public =
            ctx.call.capability.enforced() && ctx.call.capability.tier() == ProviderTier::Public;
        let (mut shown, mut withheld) = (false, 0u64);
        for key in ["grants", "replaced_grants"] {
            let scope = names
                .as_ref()
                .filter(|_| !(public && key == "replaced_grants"));
            for row in answer[key].as_array_mut().into_iter().flatten() {
                if row["labels"].is_null() {
                    continue;
                }
                row["labels"] = grant_labels(&row["labels"], scope);
                if row["labels"].is_null() {
                    withheld += 1;
                } else {
                    shown = true;
                }
            }
        }
        answer["connection_id"] = json!(connection.id);
        if withheld > 0 {
            answer["withheld"] = json!({"labels": withheld});
        }
        match names {
            Some(scope) if shown => ctx.release(&scope, answer).await,
            _ => Ok(answer),
        }
    })
}

/// A grant's `labels` as `scope` lets the call see them: none without a scope; to a public
/// call, none when its channel may not be read, and of the channels it reads only those that
/// may be ([`ContentScope::channel_allowed`]).
pub(super) fn grant_labels(labels: &Value, scope: Option<&ContentScope>) -> Value {
    let Some(scope) = scope else {
        return Value::Null;
    };
    if !scope.public_only {
        return labels.clone();
    }
    let allowed = |label: &Value| {
        label["channel_id"]
            .as_str()
            .is_some_and(|id| scope.channel_allowed(id))
    };
    if !allowed(&labels["destination"]) {
        return Value::Null;
    }
    let mut kept = labels.clone();
    if let Some(sources) = kept["sources"].as_array_mut() {
        sources.retain(allowed);
    }
    kept
}

fn profile_refusal(refusal: crate::routes::crew_profile::Refusal) -> anyhow::Error {
    said(Some(refusal.code), refusal.error)
}

// ----------------------------------------------------------------------------------- changes

// A change's target is the connection it is for, its workspace's snapshot, the workspace's
// name and its server's, read before the card is parked (`places::place`, a `Target` here).

/// Send one of the person's changes to the workspace, with its idempotency key, as
/// `biorouter crew` sends it through `POST /crew/connections/{id}/request`.
async fn send(
    ctx: &Ctx,
    connection: &Connection,
    method: &str,
    mut params: Value,
) -> Result<Value> {
    let key = idempotency_key(ctx, 0);
    params["idempotency_key"] = json!(key);
    ctx.crew()?
        .human_request(&connection.id, method, params, Some(key))
        .await
}

/// The crossing line for a write from this chat (AO-12, "Writes"), when it is one: a chat
/// classified Private writing into a Public workspace or through a Public connection.
async fn crossing(ctx: &Ctx, target: &Target) -> Option<&'static str> {
    let public = target.connection.mode == ClusterMode::Public
        || target.snapshot["workspace"]["mode"] == "public";
    (public && ctx.chat_classification().await == SessionClassification::Private)
        .then_some(CROSSING)
}

/// A name resolved in `target`'s workspace. Unless the call may be shown names from it (it may
/// read the workspace and the person let this chat read it: [`Target::names`]), a name that does
/// not resolve is refused without the resolver's candidates, so no name from a withheld
/// workspace reaches the model; when it may, they return only as content does ([`naming`]).
fn resolve_in(target: &Target, kind: SelectorKind, text: &str) -> Result<Resolved> {
    Ctx::resolve(&target.snapshot, kind, text).map_err(|error| match &target.names {
        Some(scope) => naming(error, scope),
        None => said(
            Some("unknown_name"),
            format!("Nothing you can see in this workspace is named {text}."),
        ),
    })
}

/// A username as the broker takes it: `@` dropped, checked against the server's rules.
fn username(typed: &str) -> Result<String> {
    let name = typed.trim().trim_start_matches('@');
    if biorouter_crew::valid_username(name) {
        Ok(name.to_owned())
    } else {
        Err(said(
            None,
            "Type the person's username on the server, like @bob.",
        ))
    }
}

pub(super) fn workspace_rename(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let name = ctx.required("name")?;
        let target = target(ctx).await?;
        let mut card = target
            .card(|here| format!("Rename {here} to {name}, for everyone"))
            .show("name", name.clone());
        if let Some(line) = crossing(ctx, &target).await {
            card = card.line(line);
        }
        ctx.approve(card).await?;
        let result = send(
            ctx,
            &target.connection,
            "workspace.rename",
            json!({"name": name}),
        )
        .await?;
        Ok(json!({"renamed": true, "result": result}))
    })
}

/// `biorouter crew workspace delete`: the host deletes the workspace on its server, and it is
/// removed here (OW-5). The name the broker compares is the one it stores.
pub(super) fn workspace_delete(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let target = target(ctx).await?;
        if hosts(&target.snapshot) == Some(false) {
            return Err(said(
                Some("crew_host_required"),
                format!(
                    "Only the workspace host can delete {}. To remove it from this computer, use connections_remove.",
                    target.workspace
                ),
            ));
        }
        let workspace_id = target.connection.workspace_id.clone();
        let stored_name = target.snapshot["workspace"]["name"]
            .as_str()
            .map(str::to_owned);
        ctx.approve(
            target
                .card(|here| format!("Delete {here} for everyone; this cannot be undone"))
                .line("Its teams, channels, messages and files are erased from the server, everyone's connection to it ends, and it is removed from this computer.")
                .show("workspace_id", workspace_id.clone()),
        )
        .await?;
        let deletion = ctx
            .crew()?
            .delete_workspace(
                &target.connection.id,
                &workspace_id,
                stored_name.as_deref(),
                Some(idempotency_key(ctx, 0)),
            )
            .await?;
        Ok(json!({
            "deleted": true,
            "removed": true,
            "workspace_id": deletion.workspace_id,
            "erase_after_seconds": deletion.erase_after_seconds,
        }))
    })
}

pub(super) fn workspace_governance(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let governance = ctx.required("governance")?;
        let target = target(ctx).await?;
        if hosts(&target.snapshot) == Some(false) {
            return Err(said(
                Some("crew_host_required"),
                format!(
                    "Only the workspace host can change who manages {}.",
                    target.workspace
                ),
            ));
        }
        let card = target.card(|here| {
            if governance == "open" {
                format!(
                    "Make {here} open: every member can manage teams, channels and people, and let people in"
                )
            } else {
                format!(
                    "Make {here} managed: channel owners and team creators manage them, and only you let people in"
                )
            }
        });
        ctx.approve(card.show("governance", governance.clone()))
            .await?;
        let result = send(
            ctx,
            &target.connection,
            "workspace.set_governance",
            json!({"governance": governance}),
        )
        .await?;
        Ok(json!({"governance": governance, "result": result}))
    })
}

/// `biorouter crew workspace context set`: replace the context document, against the revision
/// read before the card. The card shows the change as a diff of the current document.
pub(super) fn context_set(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let text = ctx.raw_text("text").unwrap_or_default();
        if text.len() > MAX_CONTEXT_BYTES {
            return Err(said(
                None,
                format!(
                    "The workspace context can be up to 64 KB; this is {} KB. Shorten it, then try again.",
                    text.len().div_ceil(1024)
                ),
            ));
        }
        let target = target(ctx).await?;
        let current = ctx
            .crew()?
            .human_request(
                &target.connection.id,
                "workspace.context.get",
                json!({}),
                None,
            )
            .await?;
        let revision = current["context"]["revision"].as_u64().unwrap_or(0);
        let old = current["context"]["text"].as_str().unwrap_or_default();
        let mut card = target
            .card(|here| {
                format!("Replace the context document of {here}; every member's agent reads it")
            })
            .show("text", text.clone())
            .show("revision", revision);
        card.preview = Some(document_diff("workspace context", old, &text));
        if let Some(line) = crossing(ctx, &target).await {
            card = card.line(line);
        }
        ctx.approve(card).await?;
        let saved = send(
            ctx,
            &target.connection,
            "workspace.context.set",
            json!({"text": text, "expected_revision": revision}),
        )
        .await?;
        Ok(json!({"saved": true, "result": saved}))
    })
}

/// `biorouter crew enroll invite @bob` (AO-2): the invitation names the account, and signing in
/// to the server as it is the proof, so no code is read out. The invitation text to send is
/// answered with it.
pub(super) fn enroll_invite(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let name = username(&ctx.required("username")?)?;
        let add_device = ctx.flag("add_device") == Some(true);
        let target = target(ctx).await?;
        let card = target.card(|here| {
            if add_device {
                format!("Invite @{name} to add another computer to {here}")
            } else {
                format!("Invite @{name} to join {here}")
            }
        });
        ctx.approve(
            card.line(format!(
                "Anyone who signs in to {} as {name} can then join as @{name}.",
                target.server
            ))
            .show("username", name.clone()),
        )
        .await?;
        let mut params = json!({"username": name});
        if add_device {
            params["add_device"] = json!(true);
        }
        let mut result = send(ctx, &target.connection, "enrollment.invite", params).await?;
        let canonical = result["username"].as_str().unwrap_or(&name).to_owned();
        if let Ok(invitation) = ctx
            .crew()?
            .invitation_for(&target.connection.id, Some(&canonical))
            .await
        {
            result["invitation"] = serde_json::to_value(invitation)?;
        }
        Ok(result)
    })
}

pub(super) fn enroll_cancel(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let name = username(&ctx.required("username")?)?;
        let target = target(ctx).await?;
        ctx.approve(
            target
                .card(|here| format!("Cancel @{name}'s invitation to join {here}"))
                .show("username", name.clone()),
        )
        .await?;
        send(
            ctx,
            &target.connection,
            "enrollment.cancel",
            json!({"username": name}),
        )
        .await
    })
}

/// `biorouter crew enroll approve @bob CODE`: let in a person joining from an older Biorouter,
/// with the device code they sent.
pub(super) fn enroll_approve(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let name = username(&ctx.required("username")?)?;
        let code = ctx.required("code")?;
        let digits: String = code.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
        if digits.len() != 16 || !digits.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(said(
                None,
                "A device code is 16 letters and digits in four groups, like ABCD-EFGH-JKMN-PQRS.",
            ));
        }
        let replace = ctx.flag("replace") == Some(true);
        let target = target(ctx).await?;
        ctx.approve(
            target
                .card(|here| format!("Let @{name} into {here} with the code {}", code.trim()))
                .line(format!(
                    "Only allow this with a code you got from @{name} yourself."
                ))
                .show("username", name.clone())
                .show("code", code.trim().to_owned()),
        )
        .await?;
        let mut params = json!({"username": name, "code": code.trim()});
        if replace {
            params["replace"] = json!(true);
        }
        send(ctx, &target.connection, "enrollment.approve", params).await
    })
}

/// `biorouter crew enroll revoke @bob`: remove a person from the workspace, guarded by the
/// username the card named.
pub(super) fn enroll_revoke(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let member = ctx.required("member")?;
        let target = target(ctx).await?;
        let person = resolve_in(&target, SelectorKind::Person, &member)?;
        ctx.approve(
            target
                .card(|here| {
                    format!(
                        "Remove {} from {here} for everyone; this cannot be undone",
                        person.label
                    )
                })
                .line(
                    "Their membership, their computers' keys and their agent grants stop working.",
                )
                .show("member", person.label.clone()),
        )
        .await?;
        let mut params = json!({"principal_id": person.id});
        if let Some(username) = &person.username {
            params["expected_username"] = json!(username);
        }
        send(ctx, &target.connection, "enrollment.revoke", params).await
    })
}

pub(super) fn profile_set(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let nickname = ctx.text("nickname");
        let avatar = ctx.text("avatar");
        if nickname.is_none() && avatar.is_none() {
            return Err(said(None, "Say what to change: nickname or avatar."));
        }
        let target = target(ctx).await?;
        let mut card = target.card(|here| format!("Change your profile in {here}"));
        if let Some(nickname) = &nickname {
            card = card.show("nickname", nickname.clone());
        }
        if let Some(avatar) = &avatar {
            card = card.show("avatar", avatar.clone());
        }
        if let Some(line) = crossing(ctx, &target).await {
            card = card.line(line);
        }
        ctx.approve(card).await?;
        send(
            ctx,
            &target.connection,
            "profile.update",
            json!({"nickname": nickname, "avatar": avatar}),
        )
        .await
    })
}

fn fingerprint_digits(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_hexdigit)
        .take(16)
        .map(|digit| digit.to_ascii_uppercase())
        .collect()
}

/// `biorouter crew profile revoke-device` (AO-5): one of the person's own computers, by its
/// fingerprint or as this computer. Taking the computer's SSH access away comes first, and the
/// card says so.
pub(super) fn profile_revoke_device(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let fingerprint = ctx.text("fingerprint");
        let this_device = ctx.flag("this_device") == Some(true);
        if fingerprint.is_some() == this_device {
            return Err(said(
                None,
                "Name one device: its fingerprint, or this_device.",
            ));
        }
        let target = target(ctx).await?;
        let devices = target.snapshot["actor"]["devices"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let wanted = fingerprint.as_deref().map(fingerprint_digits);
        let found: Vec<&Value> = devices
            .iter()
            .filter(|device| match &wanted {
                Some(wanted) => {
                    device["fingerprint"]
                        .as_str()
                        .map(fingerprint_digits)
                        .as_deref()
                        == Some(wanted.as_str())
                }
                None => device["this_device"].as_bool() == Some(true),
            })
            .collect();
        let [device] = found.as_slice() else {
            return Err(said(
                None,
                "None of your computers in this workspace has that fingerprint. List them with crew__read action profile_devices.",
            ));
        };
        let shown = device["fingerprint"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let this_computer = device["this_device"].as_bool() == Some(true);
        let mut card = target
            .card(|here| format!("Remove your computer {shown} from {here}"))
            .line(format!(
                "A lost computer can still sign in as you while it holds an SSH key or password for your account on {}: remove its key from ~/.ssh/authorized_keys there, or change your password, first.",
                target.server
            ))
            .show("fingerprint", shown.clone());
        if this_computer {
            card = card.line(format!(
                "This is this computer: it stops working in {} until you add it again with rejoin.",
                target.workspace
            ));
        }
        ctx.approve(card).await?;
        send(
            ctx,
            &target.connection,
            "device.revoke",
            json!({"fingerprint": shown}),
        )
        .await
    })
}

/// `biorouter crew grants revoke SESSION [--reads]` (AO-12): take back a chat's Crew access
/// through the connection, through the same revocation as the route, or with `reads`, its
/// permission to read the workspace as the person. Only a chat this chat may list can be named.
pub(super) fn grants_revoke(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let session = ctx.required("session")?;
        let connection = ctx.connection().await?;
        let crew = ctx.crew()?;
        if !ctx.lists_chat(&session).await {
            return Err(said(
                Some(crate::routes::crew_profile::NOT_FOUND),
                "No chat this chat can see has that ID.",
            ));
        }
        let name = ctx
            .state
            .session_manager()
            .get_session(&session, false)
            .await
            .ok()
            .map(|found| found.name)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| session.clone());
        let server = Ctx::server_label(&connection).await;
        if ctx.flag("reads") == Some(true) {
            let held =
                crew.person_read_rows().await.iter().any(|row| {
                    row.session_id == session && row.consent.connection_id == connection.id
                });
            if !held {
                return Err(said(
                    Some(crate::routes::crew_profile::NOT_FOUND),
                    crate::routes::crew_profile::NO_READ_CONSENT,
                ));
            }
            ctx.approve(
                Card::at(&connection.name, &server, |here| {
                    format!("Take back chat {name}'s permission to read {here} as you")
                })
                .show("session", session.clone()),
            )
            .await?;
            return match crew.revoke_person_read(&session, &connection.id).await {
                Ok(true) => Ok(
                    json!({"revoked": true, "session_id": session, "connection_id": connection.id}),
                ),
                Ok(false) => Err(said(
                    Some(crate::routes::crew_profile::NOT_FOUND),
                    crate::routes::crew_profile::NO_READ_CONSENT,
                )),
                Err(_) => Err(said(
                    Some(crate::routes::crew_profile::REVOCATION_NOT_SAVED),
                    crate::routes::crew_profile::READ_CONSENT_NOT_SAVED,
                )),
            };
        }
        let held = crew
            .run_metadata(&session)
            .await
            .is_some_and(|metadata| metadata.connection_id == connection.id);
        if !held {
            return Err(said(
                Some(crate::routes::crew_profile::GRANT_NOT_FOUND),
                format!("That chat has no Crew access through {}.", connection.name),
            ));
        }
        ctx.approve(
            Card::at(&connection.name, &server, |here| {
                format!("Take back chat {name}'s Crew access to {here}")
            })
            .line("If it is a task, the task stops.")
            .show("session", session.clone()),
        )
        .await?;
        match crate::routes::crew_profile::revoke_grant(connection.id.clone(), session).await {
            Ok(revoked) => Ok(serde_json::to_value(revoked.0)?),
            Err(refusal) => Err(profile_refusal(refusal)),
        }
    })
}

/// `biorouter crew privacy set-personal`: how this computer treats the workspace.
pub(super) fn privacy_set_personal(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let mode = ctx.required("mode")?;
        let institution = ctx.text("institution_id");
        let connection = ctx.connection().await?;
        let server = Ctx::server_label(&connection).await;
        let mut card = Card::at(&connection.name, &server, |here| {
            format!("Make your connection to {here} {mode}")
        })
        .show("mode", mode.clone());
        if mode == "public" {
            card = card.line("Public models will be able to read public-safe work you can see there. Restricted content stays private.");
        }
        if let Some(institution) = &institution {
            card = card.show("institution_id", institution.clone());
        }
        card = card.line(format!(
            "This ends every chat's and task's access through {}.",
            connection.name
        ));
        ctx.approve(card).await?;
        let mut input = saved_fields(&connection)?;
        input.insert("mode".into(), json!(mode));
        if let Some(institution) = institution {
            input.insert("institution_id".into(), json!(institution));
        }
        save_over(&connection, input).await
    })
}

/// `biorouter crew privacy set-workspace`: the host sets the workspace's privacy for everyone.
pub(super) fn privacy_set_workspace(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let mode = ctx.required("mode")?;
        let institution = ctx.text("institution_id");
        let target = target(ctx).await?;
        let mut card = target
            .card(|here| {
                if mode == "public" {
                    format!("Allow Public in {here} for everyone")
                } else {
                    format!("Make {here} Private for everyone")
                }
            })
            .show("mode", mode.clone());
        if mode == "public" {
            card = card.line("Members will be able to choose Public. Agents with access will need permission again.");
        }
        if let Some(institution) = &institution {
            card = card.show("institution_id", institution.clone());
        }
        ctx.approve(card).await?;
        let mut policy = json!({"mode": mode});
        if let Some(institution) = institution {
            policy["institution_id"] = json!(institution);
        }
        send(ctx, &target.connection, "policy.set", policy).await
    })
}
