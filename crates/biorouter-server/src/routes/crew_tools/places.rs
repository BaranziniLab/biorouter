//! What the person tools' channel, message, file and task actions share (AO-12): the
//! workspace a change is for, a content read's privacy scope and consent, the channels a read
//! may name, how a write from a Private chat is marked, and the crossing line its card shows.

use anyhow::Result;
use biorouter::crew::{ClusterMode, Connection};
use biorouter::privacy::{ProviderTier, SessionClassification};
use serde_json::{json, Value};

use super::{
    idempotency_key, naming, public_snapshot, said, Card, ContentScope, Ctx, Resolved, Said,
};
use crate::routes::crew::names::SelectorKind;

/// A change's workspace, read before its card is parked: the connection, the person's own
/// snapshot, and the workspace's and server's names as a person reads them.
pub(super) struct Place {
    pub(super) connection: Connection,
    pub(super) snapshot: Value,
    pub(super) workspace: String,
    pub(super) server: String,
    /// The scope a refusal may name things from this workspace under ([`naming_scope`]);
    /// `None` when it may name nothing from it.
    pub(super) names: Option<ContentScope>,
}

pub(super) async fn place(ctx: &Ctx) -> Result<Place> {
    let connection = ctx.connection().await?;
    let snapshot = ctx.snapshot(&connection).await?;
    let workspace = Ctx::workspace_label(&connection, Some(&snapshot));
    let server = Ctx::server_label(&connection).await;
    let names = naming_scope(ctx, &connection, &snapshot).await;
    Ok(Place {
        connection,
        snapshot,
        workspace,
        server,
        names,
    })
}

/// The scope under which this call may be shown names from `connection`'s workspace outside a
/// read of it (a resolver's candidates, a channel's team, a grant's labels): the call may read
/// the workspace ([`Ctx::content_scope`]) and the person let this chat read it as them, as a
/// read itself needs (AO-12, "Read consent"). `None` otherwise, and then nothing from the
/// workspace is named.
pub(super) async fn naming_scope(
    ctx: &Ctx,
    connection: &Connection,
    snapshot: &Value,
) -> Option<ContentScope> {
    let scope = ctx.content_scope(connection, snapshot).await.ok()?;
    ctx.crew()
        .ok()?
        .has_person_read(&ctx.call.session_id, &connection.id)
        .await
        .then_some(scope)
}

/// Send step `step` of one of the person's changes to the workspace, with its own idempotency
/// key, as `biorouter crew` sends it through `POST /crew/connections/{id}/request`.
pub(super) async fn send(
    ctx: &Ctx,
    connection: &Connection,
    method: &str,
    mut params: Value,
    step: usize,
) -> Result<Value> {
    let key = idempotency_key(ctx, step);
    params["idempotency_key"] = json!(key);
    ctx.crew()?
        .human_request(&connection.id, method, params, Some(key))
        .await
}

/// The scope a refusal may list names from `place`'s workspace under: only a private model's
/// that may read the workspace and was let read it ([`Place::names`]). A public model, even one
/// allowed the public-safe part, is never shown a protected channel's name in a list of
/// candidates. A refusal that lists names goes through [`naming`] with it, so the chat is
/// raised as a read would raise it.
pub(super) fn names_readable<'a>(ctx: &Ctx, place: &'a Place) -> Option<&'a ContentScope> {
    if ctx.call.capability.tier() == ProviderTier::Public {
        return None;
    }
    place.names.as_ref()
}

/// A name resolved in `place`'s workspace for a change. To a call that may not be shown names
/// from it ([`names_readable`]), a name that does not resolve is refused without the
/// resolver's candidates.
pub(super) fn resolve(
    ctx: &Ctx,
    place: &Place,
    kind: SelectorKind,
    text: &str,
) -> Result<Resolved> {
    Ctx::resolve(&place.snapshot, kind, text).map_err(|error| {
        if let Some(scope) = names_readable(ctx, place) {
            return naming(error, scope);
        }
        let code = error
            .downcast_ref::<Said>()
            .and_then(|said| said.code)
            .unwrap_or("unknown_name");
        said(
            Some(code),
            format!(
                "{text} doesn't name exactly one thing you can see in this workspace. Name it exactly."
            ),
        )
    })
}

/// The entry with `id` in `snapshot`'s `key` list (`channels`, `teams`, `joinable_teams`...).
pub(super) fn item<'a>(snapshot: &'a Value, key: &str, id: &str) -> Option<&'a Value> {
    snapshot[key]
        .as_array()?
        .iter()
        .find(|item| item["id"].as_str() == Some(id))
}

/// Refuse a channel the person is not in, when the snapshot lists every channel of theirs: a
/// channel given by ID resolves as that ID whether or not they are in it.
pub(super) fn yours(snapshot: &Value, channel: &Resolved, text: &str) -> Result<()> {
    let Some(listed) = snapshot["channels"].as_array() else {
        return Ok(());
    };
    let complete = snapshot["totals"]["channels"]
        .as_u64()
        .is_none_or(|total| usize::try_from(total).is_ok_and(|total| total <= listed.len()));
    if complete && item(snapshot, "channels", &channel.id).is_none() {
        return Err(said(
            Some("unknown_name"),
            format!("No channel you're in is named {text}."),
        ));
    }
    Ok(())
}

/// The name of the team `channel` belongs to in `snapshot`, when it is known.
fn channel_team<'a>(snapshot: &'a Value, channel: &Resolved) -> Option<&'a str> {
    item(snapshot, "channels", &channel.id)
        .and_then(|entry| entry["team_id"].as_str())
        .and_then(|team| item(snapshot, "teams", team))
        .and_then(|team| {
            team["display_name"]
                .as_str()
                .or_else(|| team["name"].as_str())
        })
}

/// `#methods in Analysis Lab`, or the channel's label alone when its team is not known.
pub(super) fn channel_phrase(snapshot: &Value, channel: &Resolved) -> String {
    match channel_team(snapshot, channel) {
        Some(team) => format!("{} in {team}", channel.label),
        None => channel.label.clone(),
    }
}

/// `#methods (Analysis Lab)`: a channel with its team, as a change card names it before the
/// workspace and server (AO-12, "Approvals"), or the channel's label alone when its team is not
/// known.
pub(super) fn card_channel(snapshot: &Value, channel: &Resolved) -> String {
    match channel_team(snapshot, channel) {
        Some(team) => format!("{} ({team})", channel.label),
        None => channel.label.clone(),
    }
}

impl Place {
    /// A card for a change to this workspace, its summary written around the workspace and
    /// its server ([`Card::at`]).
    pub(super) fn card(&self, summary: impl FnOnce(&str) -> String) -> Card {
        Card::at(&self.workspace, &self.server, summary)
    }

    /// One of this workspace's channels with its team, as a change card names it
    /// ([`card_channel`]).
    pub(super) fn channel(&self, channel: &Resolved) -> String {
        card_channel(&self.snapshot, channel)
    }
}

/// Whether the connection's privacy in force is Private, as the daemon judges a write's
/// required privacy (its own mode, or the workspace's as its last signed `hello` said).
async fn private_in_force(ctx: &Ctx, place: &Place) -> Result<bool> {
    Ok(place.connection.mode == ClusterMode::Private
        || ctx.crew()?.signed_workspace_mode(&place.connection.id) == Some(ClusterMode::Private))
}

/// Whether a write from this chat goes marked Private (AO-12, "Writes"): from a chat classified
/// Private, it does. Such a write can't be kept private where the connection's privacy in force
/// is Public, so it is refused there before any card, as the workspace would refuse it.
pub(super) async fn private_write(ctx: &Ctx, place: &Place) -> Result<bool> {
    if ctx.chat_classification().await != SessionClassification::Private {
        return Ok(false);
    }
    if private_in_force(ctx, place).await? {
        return Ok(true);
    }
    Err(said(
        Some(biorouter::crew::refusal::MODE_MISMATCH),
        format!(
            "This chat is Private, and your connection to {} is Public, so what it writes there couldn't be kept private. Write it from a chat that isn't Private, or make the connection Private with crew__change action privacy_set_personal.",
            place.workspace
        ),
    ))
}

/// Whether a write from this chat is marked Private where that can be kept: for a post whose
/// content the workspace restricts anyway (a reference), a Public connection is no reason to
/// refuse it.
pub(super) async fn private_where_kept(ctx: &Ctx, place: &Place) -> Result<bool> {
    Ok(
        ctx.chat_classification().await == SessionClassification::Private
            && private_in_force(ctx, place).await?,
    )
}

/// Whether the channel `id` of `place`'s workspace is public-safe, as the person's snapshot
/// classifies it.
pub(super) fn public_safe(place: &Place, id: &str) -> bool {
    item(&place.snapshot, "channels", id)
        .is_some_and(|entry| entry["classification"] == "public_safe")
}

/// The card's crossing line for a write from this chat (AO-12, "Writes"), when it is one: a
/// chat classified Private writing into a Public workspace, through a connection whose own
/// privacy is Public, or into a public-safe channel (`public_safe_channel`).
pub(super) async fn crossing(
    ctx: &Ctx,
    place: &Place,
    public_safe_channel: bool,
) -> Option<String> {
    let reason = if place.snapshot["workspace"]["mode"] == "public" {
        "a Public workspace"
    } else if place.connection.mode == ClusterMode::Public {
        "a connection that is Public"
    } else if public_safe_channel {
        "a public-safe channel"
    } else {
        return None;
    };
    (ctx.chat_classification().await == SessionClassification::Private).then(|| {
        format!("This chat is Private, and this goes to {reason}: a private-to-public crossing. Everything it sends is shown here.")
    })
}

/// A content read's workspace (AO-12, "Privacy" and "Read consent"): the call's privacy scope,
/// the person's consent, and the snapshot as the call may see it. Nothing read through it
/// reaches the model until [`Ctx::release`].
pub(super) struct Reading {
    pub(super) connection: Connection,
    pub(super) scope: ContentScope,
    /// The snapshot narrowed to what the call may see ([`public_snapshot`]), its `totals`
    /// lowered by what was withheld.
    pub(super) visible: Value,
    pub(super) workspace: String,
}

pub(super) async fn reading(ctx: &Ctx) -> Result<Reading> {
    let connection = ctx.connection().await?;
    // Judged again after a consent card that had to wait for the person ([`Ctx::consented`]).
    let (connection, snapshot, scope) = ctx.consented(connection).await?;
    let workspace = Ctx::workspace_label(&connection, Some(&snapshot));
    // `public_snapshot` also lowers `totals` by what it withheld, so the resolver reads the
    // narrowed view as whole exactly when the person's own list was whole.
    let visible = public_snapshot(&snapshot, &scope);
    Ok(Reading {
        connection,
        scope,
        visible,
        workspace,
    })
}

impl Reading {
    /// One of the person's channels the call may read, by name or ID, resolved in what the call
    /// may see, so an unknown name lists nothing it may not see, and what it lists returns only
    /// as content does ([`naming`]). To a public call, a channel given by an ID the snapshot does
    /// not list is unknown: whether it is protected can't be read.
    pub(super) fn channel(&self, text: &str) -> Result<Resolved> {
        let channel = Ctx::resolve(&self.visible, SelectorKind::Channel, text)
            .map_err(|error| naming(error, &self.scope))?;
        if !self.scope.channel_allowed(&channel.id) {
            return Err(said(
                Some("unknown_name"),
                format!("Nothing you can see is named {text}."),
            ));
        }
        yours(&self.visible, &channel, text)?;
        Ok(channel)
    }

    /// Whether a message, file or reference may reach this call: one the workspace does not
    /// restrict, or a restricted one to a private model the read already raises the chat for.
    /// A restricted item in a read that would raise nothing is withheld, so nothing protected
    /// returns without the raise.
    pub(super) fn keeps(&self, item: &Value) -> bool {
        item["restricted"].as_bool() != Some(true) || (!self.scope.public_only && self.scope.raise)
    }

    /// `#methods in Analysis Lab` as the call may see it.
    pub(super) fn phrase(&self, channel: &Resolved) -> String {
        channel_phrase(&self.visible, channel)
    }
}

/// A local path as the person typed it, with a leading `~` expanded; it must then be absolute.
pub(super) fn local_path(typed: &str) -> Result<std::path::PathBuf> {
    let home = || {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from)
    };
    let path = if typed == "~" {
        home()
    } else if let Some(rest) = typed
        .strip_prefix("~/")
        .or_else(|| typed.strip_prefix("~\\"))
    {
        home().map(|home| home.join(rest))
    } else {
        Some(std::path::PathBuf::from(typed))
    };
    match path {
        Some(path) if path.is_absolute() => Ok(path),
        _ => Err(said(
            None,
            "Give the file's full path on this computer, like ~/results/plot.png.",
        )),
    }
}

/// `1.2 MB`, `640 KB`, `12 bytes`.
pub(super) fn size_words(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let size = bytes as f64;
    if size >= KB * KB * KB {
        format!("{:.1} GB", size / (KB * KB * KB))
    } else if size >= KB * KB {
        format!("{:.1} MB", size / (KB * KB))
    } else if size >= KB {
        format!("{:.0} KB", size / KB)
    } else if bytes == 1 {
        "1 byte".to_owned()
    } else {
        format!("{bytes} bytes")
    }
}

/// A message the workspace would refuse for its size is refused here first: at most 64 KB of
/// text, and at most twice that once JSON-escaped (`message.post`).
pub(super) fn check_message_size(body: &str) -> Result<()> {
    const MAX: usize = biorouter::crew::source_line::MAX_POSTED_BYTES;
    let escaped = serde_json::to_string(body)?.len().saturating_sub(2);
    if body.len() > MAX || escaped > 2 * MAX {
        return Err(said(
            Some("crew_message_too_long"),
            "That message is longer than a workspace takes (64 KB). Shorten it, or share it as a file with crew__change action files_share.",
        ));
    }
    Ok(())
}

/// A message to post: the text, attachments and references, marked Private when `private`.
pub(super) fn post_params(
    channel_id: &str,
    body: &str,
    attachments: &[String],
    references: &[String],
    private: bool,
) -> Value {
    let mut params = json!({
        "channel_id": channel_id,
        "body": body,
        "attachments": attachments,
        "references": references,
    });
    if private {
        params["personal_mode"] = json!("private");
    }
    params
}

/// What a post answers the model: its IDs, never anything the workspace added.
pub(super) fn posted(result: &Value) -> Value {
    json!({
        "posted": true,
        "message_id": result["id"],
        "sequence": result["sequence"],
        "restricted": result["restricted"],
    })
}
