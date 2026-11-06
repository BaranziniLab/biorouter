//! The person tools' team, channel and membership actions (AO-12): what `biorouter crew teams`,
//! `channels`, `members add`, `invites`, `ownership` and `remove-member` do.
//!
//! The two reads (`teams_list`, `channels_list`) go through [`Ctx::read_content`]: the call's
//! privacy scope, the person's read consent, the snapshot narrowed to what the call may see,
//! then the raise. Every change resolves its targets in the person's own snapshot, parks one
//! card naming them, and then sends exactly those IDs, with `expected_username` and
//! `expected_name` guards where the broker takes them, through the request `biorouter crew`
//! sends.

use anyhow::Result;
use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::places::{
    channel_phrase, crossing, item, names_readable, place, public_safe, resolve, send, yours, Place,
};
use super::{said, tool_error, Card, Ctx, Resolved};
use crate::routes::crew::names::SelectorKind;

// ------------------------------------------------------------------------------------- reads

/// `biorouter crew teams list`: the person's teams, and in an open workspace the ones they may
/// join.
pub(super) fn teams_list(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        ctx.read_content(|visible, _| {
            let mut answer = json!({"teams": visible.get("teams").cloned().unwrap_or(json!([]))});
            if let Some(joinable) = visible.get("joinable_teams") {
                answer["joinable_teams"] = joinable.clone();
            }
            if let Some(total) = visible["totals"]["teams"].as_u64() {
                answer["total"] = json!(total);
            }
            Ok(answer)
        })
        .await
    })
}

/// `biorouter crew channels list [--team T]`: the person's channels with their unread counts,
/// and in an open workspace the ones they may join. A public model sees no protected channel,
/// and of the channels it could join only the public-safe ones; the rest are counted.
pub(super) fn channels_list(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let team_text = ctx.text("team");
        ctx.read_content(|visible, scope| {
            let team = team_text
                .as_deref()
                .map(|text| Ctx::resolve(visible, SelectorKind::Team, text))
                .transpose()?;
            let in_team = |channel: &Value| {
                team.as_ref()
                    .is_none_or(|team| channel["team_id"].as_str() == Some(team.id.as_str()))
            };
            let channels: Vec<Value> = visible["channels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|channel| in_team(channel))
                .map(|channel| {
                    let mut channel = channel.clone();
                    let unread = channel["id"]
                        .as_str()
                        .and_then(|id| visible["unread"].get(id))
                        .cloned();
                    if let Some(unread) = unread {
                        channel["unread"] = unread;
                    }
                    channel
                })
                .collect();
            let mut answer = json!({"channels": channels});
            // `visible` is already narrowed (`public_snapshot`): to a public model, only the
            // public-safe channels the person may join are left, and the rest are counted.
            if let Some(joinable) = visible["joinable_channels"].as_array() {
                let kept: Vec<&Value> =
                    joinable.iter().filter(|channel| in_team(channel)).collect();
                answer["joinable_channels"] = json!(kept);
            }
            if scope.public_only {
                let withheld = ["channels", "joinable_channels"]
                    .iter()
                    .filter_map(|key| visible["withheld"][*key].as_u64())
                    .sum::<u64>();
                answer["withheld"] = json!({"channels": withheld});
            }
            Ok(answer)
        })
        .await
    })
}

// ----------------------------------------------------------------------------------- helpers

/// A forgiving comparison key: lower case, a leading `#` dropped, and every run of spaces or
/// punctuation one `-` (`Analysis Lab` is `analysis-lab`), as `biorouter crew` matches what a
/// person may join.
fn loose_key(text: &str) -> String {
    let lowered = text.trim().trim_start_matches('#').to_lowercase();
    let mut key = String::with_capacity(lowered.len());
    let mut gap = false;
    for c in lowered.chars() {
        if c.is_alphanumeric() {
            if gap && !key.is_empty() {
                key.push('-');
            }
            gap = false;
            key.push(c);
        } else {
            gap = true;
        }
    }
    key
}

fn names_of(item: &Value) -> Vec<String> {
    ["name", "display_name", "handle"]
        .iter()
        .filter_map(|field| item[*field].as_str())
        .map(loose_key)
        .collect()
}

/// What the person may join in an open workspace (OW-2): the snapshot's `joinable_teams` or
/// `joinable_channels`, matched as `biorouter crew teams join` matches them. A channel may be
/// named `team/channel`.
fn joinable<'a>(ctx: &Ctx, place: &'a Place, team: bool, text: &str) -> Result<&'a Value> {
    let snapshot = &place.snapshot;
    let (key, noun) = if team {
        ("joinable_teams", "team")
    } else {
        ("joinable_channels", "channel")
    };
    let Some(candidates) = snapshot[key].as_array() else {
        return Err(said(
            None,
            if team {
                "Only an open workspace lets members join teams themselves. Ask the team's creator or the host to add you."
            } else {
                "Only an open workspace lets members join channels themselves. Ask the channel's owner or the host to add you."
            },
        ));
    };
    let literal = uuid::Uuid::parse_str(text.trim()).is_ok();
    let matches: Vec<&Value> = if literal {
        candidates
            .iter()
            .filter(|item| item["id"].as_str() == Some(text.trim()))
            .collect()
    } else {
        let (team_part, name) = match (team, text.split_once('/')) {
            (false, Some((team_part, name))) => (Some(team_part), name),
            _ => (None, text),
        };
        candidates
            .iter()
            .filter(|item| names_of(item).contains(&loose_key(name)))
            .filter(|item| {
                team_part.is_none_or(|wanted| {
                    item["team_id"]
                        .as_str()
                        .and_then(|id| {
                            super::places::item(snapshot, "teams", id)
                                .or_else(|| super::places::item(snapshot, "joinable_teams", id))
                        })
                        .is_some_and(|found| names_of(found).contains(&loose_key(wanted)))
                })
            })
            .collect()
    };
    match matches.as_slice() {
        [only] => Ok(*only),
        [] => Err(said(
            Some("unknown_name"),
            format!("No {noun} you can join is named {text}."),
        )),
        several if names_readable(ctx, place) => Err(said(
            Some("ambiguous_name"),
            format!(
                "More than one {noun} you can join matches {text}: {}. Name its team too, like analysis-lab/methods.",
                several
                    .iter()
                    .map(|item| item["name"].as_str().unwrap_or_default().to_owned())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        )),
        _ => Err(said(
            Some("ambiguous_name"),
            format!("More than one {noun} you can join matches {text}. Name it exactly, or by its team too."),
        )),
    }
}

/// The name the workspace stores a team or channel under, which `expected_name` sends so the
/// broker refuses a deletion of something renamed since the card named it (OW-4).
fn stored_name(place: &Place, key: &str, id: &str) -> Option<String> {
    item(&place.snapshot, key, id)
        .and_then(|entry| entry["name"].as_str())
        .map(|name| name.trim_start_matches('#').to_owned())
        .filter(|name| !name.is_empty())
}

/// A person's canonical username, from the resolver or the snapshot, never guessed.
fn username_of(place: &Place, person: &Resolved) -> Result<String> {
    if let Some(username) = &person.username {
        return Ok(username.clone());
    }
    place.snapshot["principals"]
        .as_array()
        .into_iter()
        .chain(place.snapshot["former_principals"].as_array())
        .flatten()
        .find(|principal| principal["id"].as_str() == Some(person.id.as_str()))
        .and_then(|principal| principal["username"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            said(
                None,
                "Name the person as @username; that ID isn't a member of this workspace.",
            )
        })
}

/// `#a`, `#a and #b`, `#a, #b and #c`.
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A channel selector confined to `team` when both are names: `methods` with team `Lab` is
/// `Lab/methods`, so a same-named channel of another team is never chosen.
fn channel_in_team(team: Option<&str>, channel: &str) -> String {
    match team.map(str::trim) {
        Some(team)
            if !team.is_empty()
                && !team.contains('/')
                && !channel.contains('/')
                && uuid::Uuid::parse_str(team).is_err()
                && uuid::Uuid::parse_str(channel.trim()).is_err() =>
        {
            format!("{team}/{}", channel.trim())
        }
        _ => channel.to_owned(),
    }
}

// ----------------------------------------------------------------------------------- teams

/// `biorouter crew teams create NAME`.
pub(super) fn teams_create(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let name = ctx.required("name")?;
        let place = place(ctx).await?;
        let mut card = Card::new(format!(
            "Create the team {name} in {} as you",
            place.workspace
        ))
        .show("name", name.clone());
        if let Some(line) = crossing(ctx, &place, false).await {
            card = card.line(line);
        }
        ctx.approve(card).await?;
        let result = send(
            ctx,
            &place.connection,
            "team.create",
            json!({"name": name}),
            0,
        )
        .await?;
        Ok(json!({
            "created": true,
            "team_id": result["id"],
            "name": result["name"],
            "general_channel_id": result["general_channel_id"],
        }))
    })
}

/// `biorouter crew teams rename TEAM NAME`.
pub(super) fn teams_rename(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("team")?;
        let name = ctx.required("name")?;
        let place = place(ctx).await?;
        let team = resolve(ctx, &place, SelectorKind::Team, &typed)?;
        let mut card = Card::new(format!(
            "Rename the team {} in {} to {name}, for everyone",
            team.label, place.workspace
        ))
        .show("team", team.label.clone())
        .show("name", name.clone());
        if let Some(line) = crossing(ctx, &place, false).await {
            card = card.line(line);
        }
        ctx.approve(card).await?;
        let result = send(
            ctx,
            &place.connection,
            "team.rename",
            json!({"team_id": team.id, "name": name}),
            0,
        )
        .await?;
        Ok(json!({
            "renamed": true,
            "team_id": team.id,
            "name": result["display_name"].as_str().or_else(|| result["name"].as_str()).unwrap_or(&name),
        }))
    })
}

/// `biorouter crew teams join TEAM` (OW-2): join a team, with its `#general`, in an open
/// workspace.
pub(super) fn teams_join(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("team")?;
        let place = place(ctx).await?;
        let found = joinable(ctx, &place, true, &typed)?;
        let id = found["id"].as_str().unwrap_or_default().to_owned();
        let label = found["display_name"]
            .as_str()
            .or_else(|| found["name"].as_str())
            .unwrap_or("the team")
            .to_owned();
        ctx.approve(
            Card::new(format!(
                "Join the team {label} in {}, with its #general, as you",
                place.workspace
            ))
            .show("team", label.clone()),
        )
        .await?;
        send(
            ctx,
            &place.connection,
            "team.join",
            json!({"team_id": id}),
            0,
        )
        .await?;
        Ok(json!({"joined": true, "team_id": id}))
    })
}

/// `biorouter crew teams delete TEAM` (OW-4): every channel of the team goes with it, its
/// `#general` among them. Guarded by the name the card showed.
pub(super) fn teams_delete(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("team")?;
        let place = place(ctx).await?;
        let team = resolve(ctx, &place, SelectorKind::Team, &typed)?;
        let expected = stored_name(&place, "teams", &team.id);
        ctx.approve(
            Card::new(format!(
                "Delete the team {} in {} for everyone; this cannot be undone",
                team.label, place.workspace
            ))
            .line("All its channels, #general among them, and their messages and files are removed from the workspace.")
            .show("team", team.label.clone()),
        )
        .await?;
        let mut params = json!({"team_id": team.id});
        if let Some(name) = expected {
            params["expected_name"] = json!(name);
        }
        let result = send(ctx, &place.connection, "team.delete", params, 0).await?;
        Ok(json!({"deleted": true, "team_id": team.id, "channels": result["channels"]}))
    })
}

// -------------------------------------------------------------------------------- channels

/// `biorouter crew channels create NAME --team T [--classification C]`. A channel is
/// restricted unless the call says public-safe, as the command makes it.
pub(super) fn channels_create(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let name = ctx.required("name")?;
        let classification = ctx
            .text("classification")
            .unwrap_or_else(|| "restricted".to_owned());
        let place = place(ctx).await?;
        let team = match ctx.text("team") {
            Some(typed) => resolve(ctx, &place, SelectorKind::Team, &typed)?,
            None => {
                let teams = place.snapshot["teams"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let [only] = teams.as_slice() else {
                    return Err(said(None, "Name the team to create the channel in."));
                };
                Resolved {
                    id: only["id"].as_str().unwrap_or_default().to_owned(),
                    label: only["display_name"]
                        .as_str()
                        .or_else(|| only["name"].as_str())
                        .unwrap_or("your team")
                        .to_owned(),
                    username: None,
                }
            }
        };
        let shown = format!("#{}", name.trim().trim_start_matches('#'));
        let public = classification == "public_safe";
        let mut card = Card::new(format!(
            "Create {shown} in {} in {} as you",
            team.label, place.workspace
        ))
        .line(if public {
            "It is public-safe: where the workspace allows Public, public models may read it."
        } else {
            "It is restricted: public models never read it."
        })
        .show("name", shown.clone())
        .show("team", team.label.clone())
        .show("classification", classification.clone());
        if let Some(line) = crossing(ctx, &place, public).await {
            card = card.line(line);
        }
        ctx.approve(card).await?;
        let result = send(
            ctx,
            &place.connection,
            "channel.create",
            json!({"name": name, "team_id": team.id, "classification": classification}),
            0,
        )
        .await?;
        Ok(json!({
            "created": true,
            "channel_id": result["id"],
            "team_id": team.id,
            "name": result["name"],
            "classification": result["classification"],
        }))
    })
}

/// `biorouter crew channels rename CHANNEL NAME`.
pub(super) fn channels_rename(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let name = ctx.required("name")?;
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &typed)?;
        let shown = format!("#{}", name.trim().trim_start_matches('#'));
        let mut card = Card::new(format!(
            "Rename {} to {shown}, for everyone",
            channel_phrase(&place.snapshot, &channel)
        ))
        .show("channel", channel.label.clone())
        .show("name", shown);
        if let Some(line) = crossing(ctx, &place, public_safe(&place, &channel.id)).await {
            card = card.line(line);
        }
        ctx.approve(card).await?;
        let result = send(
            ctx,
            &place.connection,
            "channel.rename",
            json!({"channel_id": channel.id, "name": name}),
            0,
        )
        .await?;
        Ok(json!({"renamed": true, "channel_id": channel.id, "name": result["name"]}))
    })
}

/// `biorouter crew channels archive CHANNEL`: nobody can post in it after this.
pub(super) fn channels_archive(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &typed)?;
        ctx.approve(
            Card::new(format!(
                "Archive {} for everyone; this cannot be undone",
                channel_phrase(&place.snapshot, &channel)
            ))
            .line("Nobody can post in it after this.")
            .show("channel", channel.label.clone()),
        )
        .await?;
        send(
            ctx,
            &place.connection,
            "channel.archive",
            json!({"channel_id": channel.id}),
            0,
        )
        .await?;
        Ok(json!({"archived": true, "channel_id": channel.id}))
    })
}

/// `biorouter crew channels join CHANNEL` (OW-2): a channel of one of the person's teams, in
/// an open workspace.
pub(super) fn channels_join(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let place = place(ctx).await?;
        let found = joinable(ctx, &place, false, &typed)?;
        let id = found["id"].as_str().unwrap_or_default().to_owned();
        let mut label = format!("#{}", found["name"].as_str().unwrap_or_default());
        if let Some(team) = found["team_id"]
            .as_str()
            .and_then(|team| item(&place.snapshot, "teams", team))
            .and_then(|team| {
                team["display_name"]
                    .as_str()
                    .or_else(|| team["name"].as_str())
            })
        {
            label = format!("{label} in {team}");
        }
        ctx.approve(Card::new(format!("Join {label} as you")).show("channel", label.clone()))
            .await?;
        send(
            ctx,
            &place.connection,
            "channel.join",
            json!({"channel_id": id}),
            0,
        )
        .await?;
        Ok(json!({"joined": true, "channel_id": id}))
    })
}

/// `biorouter crew channels delete CHANNEL` (OW-4): the channel goes for everyone, with its
/// messages and files. A team's `#general` goes only with its team.
pub(super) fn channels_delete(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &typed)?;
        let phrase = channel_phrase(&place.snapshot, &channel);
        let general_of = place.snapshot["teams"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|team| team["general_channel_id"].as_str() == Some(channel.id.as_str()));
        if general_of {
            return Err(said(
                None,
                format!("{phrase} belongs to its team. Delete the team instead, with crew__change action teams_delete."),
            ));
        }
        let expected = stored_name(&place, "channels", &channel.id);
        ctx.approve(
            Card::new(format!(
                "Delete {phrase} for everyone; this cannot be undone"
            ))
            .line("Its messages and files are removed from the workspace.")
            .show("channel", channel.label.clone()),
        )
        .await?;
        let mut params = json!({"channel_id": channel.id});
        if let Some(name) = expected {
            params["expected_name"] = json!(name);
        }
        send(ctx, &place.connection, "channel.delete", params, 0).await?;
        Ok(json!({"deleted": true, "channel_id": channel.id}))
    })
}

/// `biorouter crew channels mark-read CHANNEL`: read up to the newest message.
pub(super) fn channels_read(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &typed)?;
        yours(&place.snapshot, &channel, &typed)?;
        let newest = ctx
            .crew()?
            .human_request(
                &place.connection.id,
                "messages.history",
                json!({"channel_id": channel.id, "latest": true, "limit": 1}),
                None,
            )
            .await?;
        let Some(cursor) = newest["cursor"].as_str().map(str::to_owned) else {
            return Ok(json!({"read": false, "channel_id": channel.id,
                "note": "It has no messages to mark read."}));
        };
        ctx.approve(
            Card::new(format!(
                "Mark {} read",
                channel_phrase(&place.snapshot, &channel)
            ))
            .show("channel", channel.label.clone()),
        )
        .await?;
        send(
            ctx,
            &place.connection,
            "channel.read",
            json!({"channel_id": channel.id, "sequence": cursor}),
            0,
        )
        .await?;
        Ok(json!({"read": true, "channel_id": channel.id}))
    })
}

// ------------------------------------------------------------------------------ membership

/// `biorouter crew members add @bob --team T [--channel C ...]` or `--channel C ...`: direct
/// add, guarded by the username the card named. Several channels are added one at a time, each
/// with its own idempotency key; a refusal part way says what was already added.
pub(super) fn members_add(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("member")?;
        let team_text = ctx.text("team");
        let channel_texts = ctx.texts("channels");
        if team_text.is_none() && channel_texts.is_empty() {
            return Err(said(None, "Say where to add them: a team, or channels."));
        }
        let place = place(ctx).await?;
        let person = resolve(ctx, &place, SelectorKind::Person, &typed)?;
        let username = username_of(&place, &person)?;
        let team = team_text
            .as_deref()
            .map(|text| resolve(ctx, &place, SelectorKind::Team, text))
            .transpose()?;
        let mut channels = Vec::with_capacity(channel_texts.len());
        for text in &channel_texts {
            channels.push(resolve(
                ctx,
                &place,
                SelectorKind::Channel,
                &channel_in_team(team_text.as_deref(), text),
            )?);
        }
        let channel_names: Vec<String> = channels
            .iter()
            .map(|channel| channel.label.clone())
            .collect();
        let summary = match (&team, channel_names.is_empty()) {
            (Some(team), true) => format!(
                "Add {} to the team {} in {}",
                person.label, team.label, place.workspace
            ),
            (Some(team), false) => format!(
                "Add {} to the team {} and to {} in {}",
                person.label,
                team.label,
                and_list(&channel_names),
                place.workspace
            ),
            (None, _) => format!(
                "Add {} to {} in {}",
                person.label,
                and_list(&channel_names),
                place.workspace
            ),
        };
        let mut card = Card::new(summary)
            .line("They join at once; nothing waits for them to accept.")
            .show("member", person.label.clone());
        if let Some(team) = &team {
            card = card.show("team", team.label.clone());
        }
        if !channel_names.is_empty() {
            card = card.show("channels", json!(channel_names));
        }
        ctx.approve(card).await?;
        if let Some(team) = &team {
            let channel_ids: Vec<&str> =
                channels.iter().map(|channel| channel.id.as_str()).collect();
            let result = send(
                ctx,
                &place.connection,
                "team.add_member",
                json!({
                    "team_id": team.id,
                    "principal_id": person.id,
                    "expected_username": username,
                    "channel_ids": channel_ids,
                }),
                0,
            )
            .await?;
            return Ok(json!({
                "added": true,
                "principal_id": person.id,
                "team_id": team.id,
                "added_channels": result["added_channels"],
            }));
        }
        let mut added = Vec::new();
        for (step, channel) in channels.iter().enumerate() {
            let answer = send(
                ctx,
                &place.connection,
                "channel.add_member",
                json!({
                    "channel_id": channel.id,
                    "principal_id": person.id,
                    "expected_username": username,
                }),
                step,
            )
            .await;
            match answer {
                Ok(result) => {
                    if result["already_member"].as_bool() != Some(true) {
                        added.push(channel.id.clone());
                    }
                }
                Err(error) if step > 0 => {
                    let done: Vec<String> = channels[..step]
                        .iter()
                        .map(|channel| channel.label.clone())
                        .collect();
                    let reason = tool_error(error, ctx.text("connection")).await;
                    return Err(said(
                        None,
                        format!(
                            "Added {} to {}, then stopped at {}: {reason}",
                            person.label,
                            and_list(&done),
                            channel.label
                        ),
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(json!({"added": true, "principal_id": person.id, "added_channels": added}))
    })
}

/// `biorouter crew invites create @bob --team T` or `--channel C`.
pub(super) fn invites_send(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("member")?;
        let (kind, selector, text) = match (ctx.text("team"), ctx.text("channel")) {
            (Some(team), None) => ("team", SelectorKind::Team, team),
            (None, Some(channel)) => ("channel", SelectorKind::Channel, channel),
            _ => return Err(said(None, "Invite them to one place: a team or a channel.")),
        };
        let place = place(ctx).await?;
        let person = resolve(ctx, &place, SelectorKind::Person, &typed)?;
        let target = resolve(ctx, &place, selector, &text)?;
        let label = if kind == "channel" {
            channel_phrase(&place.snapshot, &target)
        } else {
            format!("the team {}", target.label)
        };
        ctx.approve(
            Card::new(format!("Invite {} to {label}", person.label))
                .line("They join when they accept it.")
                .show("member", person.label.clone())
                .show(kind, target.label.clone()),
        )
        .await?;
        let mut params = json!({"kind": kind, "target_id": target.id, "principal_id": person.id});
        if let Some(username) = &person.username {
            params["expected_username"] = json!(username);
        }
        let result = send(ctx, &place.connection, "invitation.create", params, 0).await?;
        Ok(json!({
            "invited": true,
            "invitation_id": result["id"],
            "kind": kind,
            "target_id": target.id,
            "principal_id": person.id,
        }))
    })
}

/// "Analysis Lab", or "#methods in Analysis Lab", as an invitation names its target.
fn invitation_label(invitation: &Value) -> String {
    let target = invitation["target_name"]
        .as_str()
        .filter(|name| !name.is_empty());
    let team = invitation["team_name"]
        .as_str()
        .filter(|name| !name.is_empty());
    match (invitation["kind"].as_str(), target, team) {
        (Some("channel"), Some(channel), Some(team)) => format!("#{channel} in {team}"),
        (Some("channel"), Some(channel), None) => format!("#{channel}"),
        (Some("channel"), None, _) => "a channel".to_owned(),
        (_, Some(team), _) => team.to_owned(),
        (_, None, _) => "a team".to_owned(),
    }
}

fn invitation_matches(invitation: &Value, text: &str) -> bool {
    if invitation["id"].as_str() == Some(text) {
        return true;
    }
    let target = invitation["target_name"].as_str().map(loose_key);
    let team = invitation["team_name"].as_str().map(loose_key);
    match (invitation["kind"].as_str(), text.split_once('/')) {
        (Some("channel"), Some((team_part, channel_part))) => {
            team.as_deref() == Some(loose_key(team_part).as_str())
                && target.as_deref() == Some(loose_key(channel_part).as_str())
        }
        (_, Some(_)) => false,
        (_, None) => target.as_deref() == Some(loose_key(text).as_str()),
    }
}

/// `biorouter crew invites accept [NAME|ID]`: one of the person's own invitations that has not
/// expired; with none named, the only one.
pub(super) fn invites_accept(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.text("invitation");
        let place = place(ctx).await?;
        let actor = place.snapshot["actor"]["id"].as_str();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let pending: Vec<&Value> = place.snapshot["invitations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|invitation| actor.is_some() && invitation["principal_id"].as_str() == actor)
            .filter(|invitation| {
                invitation["expired"].as_bool() != Some(true)
                    && invitation["expires_at"]
                        .as_u64()
                        .is_none_or(|expires| expires > now)
            })
            .collect();
        let matching: Vec<&Value> = match &typed {
            None => pending.clone(),
            Some(text) => pending
                .iter()
                .copied()
                .filter(|invitation| invitation_matches(invitation, text))
                .collect(),
        };
        let chosen = match (matching.as_slice(), &typed) {
            ([one], _) => *one,
            ([], None) => return Err(said(None, "You have no pending invitations.")),
            ([], Some(text)) => {
                return Err(said(
                    Some("unknown_name"),
                    format!("You have no pending invitation to {text}. List them with crew__read action invites_list."),
                ))
            }
            (several, _) if names_readable(ctx, &place) => {
                return Err(said(
                    Some("ambiguous_name"),
                    format!(
                        "Name the invitation to accept: {}.",
                        several
                            .iter()
                            .map(|invitation| invitation_label(invitation))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                ))
            }
            _ => {
                return Err(said(
                    Some("ambiguous_name"),
                    "You have more than one pending invitation. Name the one to accept.",
                ))
            }
        };
        let id = chosen["id"].as_str().unwrap_or_default().to_owned();
        let label = invitation_label(chosen);
        ctx.approve(
            Card::new(format!(
                "Accept the invitation to {label} in {}",
                place.workspace
            ))
            .show("invitation", label.clone()),
        )
        .await?;
        send(
            ctx,
            &place.connection,
            "invitation.accept",
            json!({"invitation_id": id}),
            0,
        )
        .await?;
        Ok(json!({"accepted": true, "invitation_id": id, "kind": chosen["kind"]}))
    })
}

/// `biorouter crew ownership offer CHANNEL @successor`.
pub(super) fn ownership_offer(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let channel_text = ctx.required("channel")?;
        let successor_text = ctx.required("successor")?;
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &channel_text)?;
        let successor = resolve(ctx, &place, SelectorKind::Person, &successor_text)?;
        ctx.approve(
            Card::new(format!(
                "Offer ownership of {} to {}",
                channel_phrase(&place.snapshot, &channel),
                successor.label
            ))
            .line("When they accept, they own it and you leave the channel.")
            .show("channel", channel.label.clone())
            .show("successor", successor.label.clone()),
        )
        .await?;
        let mut params = json!({"channel_id": channel.id, "successor_id": successor.id});
        if let Some(username) = &successor.username {
            params["expected_username"] = json!(username);
        }
        send(ctx, &place.connection, "channel.transfer", params, 0).await?;
        Ok(json!({"offered": true, "channel_id": channel.id, "successor_id": successor.id}))
    })
}

/// `biorouter crew ownership accept CHANNEL`.
pub(super) fn ownership_accept(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &typed)?;
        ctx.approve(
            Card::new(format!(
                "Take ownership of {}",
                channel_phrase(&place.snapshot, &channel)
            ))
            .show("channel", channel.label.clone()),
        )
        .await?;
        send(
            ctx,
            &place.connection,
            "transfer.accept",
            json!({"channel_id": channel.id}),
            0,
        )
        .await?;
        Ok(json!({"owner": true, "channel_id": channel.id}))
    })
}

/// `biorouter crew remove-member CHANNEL @bob [--former]`, guarded by the username the card
/// named.
pub(super) fn remove_member(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let channel_text = ctx.required("channel")?;
        let member_text = ctx.required("member")?;
        let kind = if ctx.flag("former") == Some(true) {
            SelectorKind::FormerPerson
        } else {
            SelectorKind::Person
        };
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &channel_text)?;
        let person = resolve(ctx, &place, kind, &member_text)?;
        ctx.approve(
            Card::new(format!(
                "Remove {} from {} for everyone; this cannot be undone",
                person.label,
                channel_phrase(&place.snapshot, &channel)
            ))
            .line("They lose access to its messages and files.")
            .show("channel", channel.label.clone())
            .show("member", person.label.clone()),
        )
        .await?;
        let mut params = json!({"channel_id": channel.id, "principal_id": person.id});
        if let Some(username) = &person.username {
            params["expected_username"] = json!(username);
        }
        send(ctx, &place.connection, "membership.revoke", params, 0).await?;
        Ok(json!({"removed": true, "channel_id": channel.id, "principal_id": person.id}))
    })
}
