//! Selectors and the resolver (naming design D7, slice S1b), and the person labels an
//! observation `state` frame carries (the display rule's collision rule).
//!
//! Everything here is a pure function of data the caller already holds: the person's own
//! `workspace.snapshot`, which the broker filters to what they may see, or this device's saved
//! connections. A candidate can therefore never name a team, channel or person the caller could
//! not already see. A resolution is a lookup, not a permission: the broker still authorizes
//! every mutation that uses its answer. People are matched only by `@username`, never by a
//! display name, because anyone can set a display name.

use biorouter::crew::observation::PersonLabel;
use biorouter::crew::Connection;
use biorouter_crew::names::{
    clean, is_uuid_shaped, name_key, sanitize_channel_name, sanitize_display_name,
    sanitize_team_name, skeleton_key, strip_ignorable,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The most selectors one request may carry.
pub const MAX_SELECTORS: usize = 64;
/// The longest selector (or connection) text, in bytes.
pub const MAX_SELECTOR_BYTES: usize = 512;

/// What a selector names.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SelectorKind {
    /// An active member, by `@username` or ID.
    Person,
    /// A former member the caller's teams, channels or invitations still name.
    FormerPerson,
    /// A team the caller belongs to, by name, handle or ID.
    Team,
    /// A channel the caller belongs to: `methods`, `#methods`, `analysis-lab/methods` or an ID.
    Channel,
    /// A connection saved on this device, by name, SSH target or ID.
    Connection,
    /// Not resolved by name here: attachments are chosen from a channel's files.
    Attachment,
}

/// One name to look up.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectorInput {
    /// Omit to let the grammar decide: `@bob` is a person; anything else is a channel, unless it
    /// is UUID-shaped, which is always an ID.
    #[serde(default)]
    #[schema(inline)]
    pub kind: Option<SelectorKind>,
    pub text: String,
}

/// `POST /crew/resolve`.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequest {
    /// The saved connection whose workspace the selectors are resolved in: its ID, its name
    /// (case-insensitive) or its SSH target. May be omitted when only one connection is saved.
    #[serde(default)]
    pub connection: Option<String>,
    #[serde(default)]
    #[schema(inline)]
    pub selectors: Vec<SelectorInput>,
}

/// The answer to one selector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Resolution {
    /// Exactly one object the caller can see has this name, or the text is an ID.
    Resolved {
        #[schema(inline)]
        kind: SelectorKind,
        /// The selector's text, as sent.
        text: String,
        /// Send this to the broker. Never show it to a person.
        id: String,
        /// What to confirm the choice with: `Bob Lee (@bob)`, `Analysis Lab`, `#methods`.
        /// Absent for an ID this workspace's snapshot does not show.
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// A person's canonical username, to send as `expected_username`.
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<String>,
    },
    /// Nothing the caller can see has this name. It lists no candidates, so it cannot be used to
    /// learn what exists.
    UnknownName {
        #[schema(inline)]
        kind: SelectorKind,
        text: String,
        /// The one member whose username differs from the text only in letter case. It is never
        /// resolved silently; the person must type it as shown.
        #[serde(skip_serializing_if = "Option::is_none")]
        did_you_mean: Option<String>,
    },
    /// More than one visible object matches, or the daemon's name rules and the broker's
    /// disagree about one. Nothing is guessed.
    AmbiguousName {
        #[schema(inline)]
        kind: SelectorKind,
        text: String,
        /// Labels to choose between, never IDs.
        candidates: Vec<String>,
    },
}

/// The answer to `POST /crew/resolve`.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ResolveResponse {
    /// The saved connection, when the request named one.
    #[schema(inline)]
    pub connection: Option<Resolution>,
    /// One resolution per selector, in the order they were sent.
    #[schema(inline)]
    pub results: Vec<Resolution>,
}

/// Why a request cannot be answered at all.
#[derive(Debug, PartialEq, Eq)]
pub enum ResolveRefusal {
    /// The request itself is malformed.
    Invalid(String),
    /// The named connection is unknown or matches several saved connections.
    Connection(Resolution),
    /// No connection was named and none can be chosen for the caller.
    ConnectionRequired(String),
}

/// Check sizes and kinds before anything is looked up.
pub fn validate_request(request: &ResolveRequest) -> Result<(), ResolveRefusal> {
    if request.selectors.len() > MAX_SELECTORS {
        return Err(ResolveRefusal::Invalid(format!(
            "Look up at most {MAX_SELECTORS} names at a time."
        )));
    }
    let texts = request
        .connection
        .iter()
        .chain(request.selectors.iter().map(|selector| &selector.text));
    for text in texts {
        if unquote(text).is_empty() {
            return Err(ResolveRefusal::Invalid("A name can't be empty.".into()));
        }
        if text.len() > MAX_SELECTOR_BYTES {
            return Err(ResolveRefusal::Invalid(format!(
                "A name can be at most {MAX_SELECTOR_BYTES} bytes long."
            )));
        }
        if text.chars().any(char::is_control) {
            return Err(ResolveRefusal::Invalid(
                "A name can't contain control characters.".into(),
            ));
        }
    }
    if request
        .selectors
        .iter()
        .any(|selector| selector.kind == Some(SelectorKind::Attachment))
    {
        return Err(ResolveRefusal::Invalid(
            "Attachments aren't looked up by name here. Choose one from the channel's files."
                .into(),
        ));
    }
    Ok(())
}

/// Whether any selector needs the workspace's snapshot (everything but saved connections does).
pub fn needs_snapshot(selectors: &[SelectorInput]) -> bool {
    selectors
        .iter()
        .any(|selector| selector.kind != Some(SelectorKind::Connection))
}

/// The connection to resolve in: `(its resolution when it was named, its ID when one is needed)`.
///
/// A named connection must resolve. With none named, the only saved connection is used when the
/// selectors need a workspace; with several saved, the caller must say which.
pub fn choose_connection(
    connections: &[Connection],
    named: Option<&str>,
    needed: bool,
) -> Result<(Option<Resolution>, Option<String>), ResolveRefusal> {
    if let Some(text) = named {
        let resolution = resolve_connection(connections, text);
        return match &resolution {
            Resolution::Resolved { id, .. } => {
                let id = id.clone();
                Ok((Some(resolution), Some(id)))
            }
            _ => Err(ResolveRefusal::Connection(resolution)),
        };
    }
    if !needed {
        return Ok((None, None));
    }
    match connections {
        [] => Err(ResolveRefusal::ConnectionRequired(
            "No Crew connection is saved on this computer. Add one first.".into(),
        )),
        [only] => Ok((None, Some(only.id.clone()))),
        _ => Err(ResolveRefusal::ConnectionRequired(
            "More than one Crew connection is saved. Name the one to look these names up in."
                .into(),
        )),
    }
}

/// Resolve a saved connection: its ID (UUID-shaped text is always an ID), its name compared
/// case-insensitively, or its exact SSH target. Two saved connections sharing a name are
/// ambiguous. A leading `@` always selects a person, so it never names a connection.
pub fn resolve_connection(connections: &[Connection], raw: &str) -> Resolution {
    let kind = SelectorKind::Connection;
    let text = unquote(raw);
    if is_uuid_shaped(text) {
        return match connections
            .iter()
            .find(|connection| connection.id.eq_ignore_ascii_case(text))
        {
            Some(connection) => resolved(
                kind,
                raw,
                &connection.id,
                Some(connection_label(connection, connections)),
                None,
            ),
            None => unknown(kind, raw, None),
        };
    }
    if text.starts_with('@') {
        return unknown(kind, raw, None);
    }
    let lowered = text.to_lowercase();
    let found: Vec<&Connection> = connections
        .iter()
        .filter(|connection| {
            connection.name.to_lowercase() == lowered || connection.ssh_target == text
        })
        .collect();
    match found.as_slice() {
        [] => unknown(kind, raw, None),
        [one] => resolved(
            kind,
            raw,
            &one.id,
            Some(connection_label(one, connections)),
            None,
        ),
        many => ambiguous(
            kind,
            raw,
            many.iter()
                .map(|connection| {
                    format!(
                        "{} — {}",
                        plain(&connection.name),
                        plain(&connection.ssh_target)
                    )
                })
                .collect(),
        ),
    }
}

/// Resolve every selector, in order: saved connections from `connections`, everything else from
/// the caller's own `snapshot`. A selector that needs a snapshot when there is none is unknown.
pub fn resolve_all(
    snapshot: Option<&Value>,
    connections: &[Connection],
    selectors: &[SelectorInput],
) -> Vec<Resolution> {
    let directory = snapshot.map(Directory::new);
    selectors
        .iter()
        .map(|selector| match (selector.kind, &directory) {
            (Some(SelectorKind::Connection), _) => resolve_connection(connections, &selector.text),
            (_, Some(directory)) => directory.resolve(selector),
            (kind, None) => unknown(kind.unwrap_or(SelectorKind::Channel), &selector.text, None),
        })
        .collect()
}

/// How each person the snapshot names is shown, keyed by principal ID: the active members
/// (the host and the caller among them) and the former members it projects.
///
/// Two people collide when their display names share a [`name_key`] or a [`skeleton_key`] (so
/// `Sam Park` and `Sam Pаrk` with a Cyrillic `а` do); colliding people are shown with their
/// `@username` everywhere, chips included.
pub fn project_labels(snapshot: &Value) -> BTreeMap<String, PersonLabel> {
    let people = people(snapshot);
    let mut name_keys: BTreeMap<String, usize> = BTreeMap::new();
    let mut skeleton_keys: BTreeMap<String, usize> = BTreeMap::new();
    for person in &people {
        *name_keys.entry(name_key(&person.display)).or_default() += 1;
        *skeleton_keys
            .entry(skeleton_key(&person.display))
            .or_default() += 1;
    }
    let shared = |keys: &BTreeMap<String, usize>, key: String| {
        !key.is_empty() && keys.get(&key).copied().unwrap_or(0) > 1
    };
    people
        .iter()
        .map(|person| {
            let collides = shared(&name_keys, name_key(&person.display))
                || shared(&skeleton_keys, skeleton_key(&person.display));
            let username = plain(person.username);
            let both = format!("{} (@{username})", person.display);
            let handle = format!("@{username}");
            let equal = person.display.to_lowercase() == person.username.to_lowercase();
            let label = if collides {
                PersonLabel {
                    full: both.clone(),
                    short: both,
                    collides,
                }
            } else if equal {
                PersonLabel {
                    full: handle.clone(),
                    short: handle,
                    collides,
                }
            } else {
                PersonLabel {
                    full: both,
                    short: person.display.clone(),
                    collides,
                }
            };
            (person.id.to_owned(), label)
        })
        .collect()
}

/// A person the snapshot names.
struct Person<'a> {
    id: &'a str,
    /// Exactly as the broker holds it; the resolver matches against this.
    username: &'a str,
    /// The sanitized display name (the username when none was set or it is unusable).
    display: String,
    active: bool,
    /// The host's snapshot flags a principal whose account no longer matches its username.
    stale: bool,
}

impl Person<'_> {
    /// The authority form: always `Display name (@username)`, even when the two are equal.
    fn label(&self) -> String {
        let mut label = format!("{} (@{})", self.display, plain(self.username));
        if !self.active {
            label.push_str(" · former member");
        }
        if self.stale {
            label.push_str(" · account no longer valid");
        }
        label
    }
}

/// Active members first (their list, then the caller), then former members; the first
/// appearance of an ID wins.
fn people(snapshot: &Value) -> Vec<Person<'_>> {
    let principals = snapshot["principals"].as_array().into_iter().flatten();
    let actor = std::iter::once(&snapshot["actor"]);
    let former = snapshot["former_principals"]
        .as_array()
        .into_iter()
        .flatten();
    let entries: Vec<(&Value, bool)> = principals
        .chain(actor)
        .map(|entry| (entry, entry["active"].as_bool().unwrap_or(true)))
        .chain(former.map(|entry| (entry, false)))
        .collect();
    let usernames: Vec<&str> = entries
        .iter()
        .filter_map(|(entry, _)| entry["username"].as_str())
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut people = Vec::new();
    for (entry, active) in entries {
        let (Some(id), Some(username)) = (entry["id"].as_str(), entry["username"].as_str()) else {
            continue;
        };
        if id.is_empty() || username.is_empty() || !seen.insert(id) {
            continue;
        }
        let nickname = entry["display_name"]
            .as_str()
            .or_else(|| entry["nickname"].as_str())
            .unwrap_or(username);
        let display = sanitize_display_name(
            nickname,
            username,
            usernames.iter().copied().filter(|other| *other != username),
        );
        people.push(Person {
            id,
            username,
            display: plain(&display),
            active,
            stale: entry["account_stale"].as_bool() == Some(true),
        });
    }
    people
}

/// A team or channel the snapshot shows.
struct Named<'a> {
    id: &'a str,
    /// `Analysis Lab` for a team, `#methods` for a channel.
    label: String,
    /// The broker's projected `handle`, else the daemon's own [`name_key`].
    handle: String,
    /// The daemon's own [`name_key`] of the stored name.
    local: String,
    /// The broker projected a handle the daemon's own name rules disagree with (their Unicode
    /// tables differ), so a match cannot be trusted.
    uncertain: bool,
    team_id: Option<&'a str>,
    archived: bool,
}

impl Named<'_> {
    fn matches(&self, key: &str) -> bool {
        !key.is_empty() && (self.handle == key || self.local == key)
    }
}

fn named<'a>(entry: &'a Value, label: impl Fn(&str) -> String) -> Option<Named<'a>> {
    let id = entry["id"].as_str().filter(|id| !id.is_empty())?;
    let name = entry["name"].as_str().unwrap_or_default();
    let local = name_key(name);
    let projected = entry["handle"].as_str().map(str::to_owned);
    Some(Named {
        id,
        label: label(entry["display_name"].as_str().unwrap_or(name)),
        uncertain: projected.as_ref().is_some_and(|handle| *handle != local),
        handle: projected.unwrap_or_else(|| local.clone()),
        local,
        team_id: entry["team_id"].as_str(),
        archived: entry["archived"].as_bool() == Some(true),
    })
}

/// What the resolver consults: the caller's own snapshot, read once per request.
struct Directory<'a> {
    people: Vec<Person<'a>>,
    teams: Vec<Named<'a>>,
    channels: Vec<Named<'a>>,
}

impl<'a> Directory<'a> {
    fn new(snapshot: &'a Value) -> Self {
        let list = |key: &str| snapshot[key].as_array().into_iter().flatten();
        Self {
            people: people(snapshot),
            teams: list("teams")
                .filter_map(|team| named(team, |name| plain(&sanitize_team_name(name))))
                .collect(),
            channels: list("channels")
                .filter_map(|channel| {
                    named(channel, |name| {
                        format!("#{}", plain(&sanitize_channel_name(name)))
                    })
                })
                .collect(),
        }
    }

    fn resolve(&self, selector: &SelectorInput) -> Resolution {
        let raw = selector.text.as_str();
        let text = unquote(raw);
        let Some(kind) = selector.kind else {
            if is_uuid_shaped(text) {
                return self.resolve_any_id(raw, text);
            }
            let kind = if text.starts_with('@') {
                SelectorKind::Person
            } else {
                SelectorKind::Channel
            };
            return self.resolve_kind(kind, raw, text);
        };
        self.resolve_kind(kind, raw, text)
    }

    fn resolve_kind(&self, kind: SelectorKind, raw: &str, text: &str) -> Resolution {
        match kind {
            SelectorKind::Person | SelectorKind::FormerPerson => {
                // A leading `@` is stripped exactly once, so `@bob@ad.ucsf.edu` keeps its own.
                let username = text.strip_prefix('@').unwrap_or(text);
                if is_uuid_shaped(username) {
                    return self.resolve_id(kind, raw, username);
                }
                self.resolve_person(kind, raw, username)
            }
            SelectorKind::Team => {
                if is_uuid_shaped(text) {
                    return self.resolve_id(kind, raw, text);
                }
                let key = name_key(text);
                let found = self
                    .teams
                    .iter()
                    .filter(|team| team.matches(&key))
                    .collect();
                self.decide(kind, raw, found, false)
            }
            SelectorKind::Channel => {
                let text = text.strip_prefix('#').unwrap_or(text);
                if is_uuid_shaped(text) {
                    return self.resolve_id(kind, raw, text);
                }
                self.resolve_channel(raw, text)
            }
            // Handled before a directory is consulted, and refused by validation.
            SelectorKind::Connection | SelectorKind::Attachment => unknown(kind, raw, None),
        }
    }

    /// Exact canonical username only. A match that differs only in letter case is offered as a
    /// suggestion and never resolved; a display name is never matched at all.
    fn resolve_person(&self, kind: SelectorKind, raw: &str, username: &str) -> Resolution {
        let active = kind == SelectorKind::Person;
        let pool: Vec<&Person> = self
            .people
            .iter()
            .filter(|person| person.active == active)
            .collect();
        let exact: Vec<&&Person> = pool
            .iter()
            .filter(|person| person.username == username)
            .collect();
        match exact.as_slice() {
            [one] => resolved(
                kind,
                raw,
                one.id,
                Some(one.label()),
                Some(one.username.to_owned()),
            ),
            [] => {
                let lowered = username.to_lowercase();
                let near: Vec<&&Person> = pool
                    .iter()
                    .filter(|person| person.username.to_lowercase() == lowered)
                    .collect();
                match near.as_slice() {
                    [] => unknown(kind, raw, None),
                    [one] => unknown(kind, raw, Some(format!("@{}", plain(one.username)))),
                    many => ambiguous(
                        kind,
                        raw,
                        many.iter().map(|person| person.label()).collect(),
                    ),
                }
            }
            many => ambiguous(
                kind,
                raw,
                many.iter().map(|person| person.label()).collect(),
            ),
        }
    }

    /// `methods` among all the caller's channels, or `analysis-lab/methods` among one team's.
    fn resolve_channel(&self, raw: &str, text: &str) -> Resolution {
        let kind = SelectorKind::Channel;
        let (teams, channel) = match text.split_once('/') {
            Some((team, channel)) => {
                let key = name_key(unquote(team));
                let teams: Vec<&Named> = self.teams.iter().filter(|t| t.matches(&key)).collect();
                if teams.is_empty() {
                    return unknown(kind, raw, None);
                }
                let channel = unquote(channel);
                (Some(teams), channel.strip_prefix('#').unwrap_or(channel))
            }
            None => (None, text),
        };
        let key = name_key(channel);
        let found = self
            .channels
            .iter()
            .filter(|candidate| {
                candidate.matches(&key)
                    && teams.as_ref().is_none_or(|teams| {
                        teams.iter().any(|team| Some(team.id) == candidate.team_id)
                    })
            })
            .collect();
        let team_uncertain = teams
            .as_ref()
            .is_some_and(|teams| teams.iter().any(|team| team.uncertain));
        self.decide(kind, raw, found, team_uncertain)
    }

    /// One trustworthy match resolves; none is unknown; anything else is ambiguous.
    fn decide(
        &self,
        kind: SelectorKind,
        raw: &str,
        found: Vec<&Named>,
        uncertain: bool,
    ) -> Resolution {
        match found.as_slice() {
            [] => unknown(kind, raw, None),
            [one] if !one.uncertain && !uncertain => {
                resolved(kind, raw, one.id, Some(self.label(kind, one)), None)
            }
            many => ambiguous(
                kind,
                raw,
                many.iter()
                    .map(|entry| self.qualified_label(kind, entry))
                    .collect(),
            ),
        }
    }

    /// UUID-shaped text with a kind: the object's ID, labelled when the snapshot shows it. An ID
    /// the snapshot does not show (a team the caller was invited to, say) still passes through;
    /// the broker authorizes it.
    fn resolve_id(&self, kind: SelectorKind, raw: &str, id: &str) -> Resolution {
        self.find_id(kind, raw, id)
            .unwrap_or_else(|| resolved(kind, raw, id, None, None))
    }

    /// UUID-shaped text without a kind: whatever the snapshot shows with that ID.
    fn resolve_any_id(&self, raw: &str, id: &str) -> Resolution {
        [
            SelectorKind::Person,
            SelectorKind::FormerPerson,
            SelectorKind::Team,
            SelectorKind::Channel,
        ]
        .into_iter()
        .find_map(|kind| self.find_id(kind, raw, id))
        .unwrap_or_else(|| unknown(SelectorKind::Channel, raw, None))
    }

    fn find_id(&self, kind: SelectorKind, raw: &str, id: &str) -> Option<Resolution> {
        match kind {
            SelectorKind::Person | SelectorKind::FormerPerson => {
                let active = kind == SelectorKind::Person;
                self.people
                    .iter()
                    .find(|person| person.active == active && person.id.eq_ignore_ascii_case(id))
                    .map(|person| {
                        resolved(
                            kind,
                            raw,
                            person.id,
                            Some(person.label()),
                            Some(person.username.to_owned()),
                        )
                    })
            }
            SelectorKind::Team | SelectorKind::Channel => {
                let pool = if kind == SelectorKind::Team {
                    &self.teams
                } else {
                    &self.channels
                };
                pool.iter()
                    .find(|entry| entry.id.eq_ignore_ascii_case(id))
                    .map(|entry| resolved(kind, raw, entry.id, Some(self.label(kind, entry)), None))
            }
            SelectorKind::Connection | SelectorKind::Attachment => None,
        }
    }

    /// A team's name; a channel's `#slug`, qualified by its team when another visible team has a
    /// channel of the same name.
    fn label(&self, kind: SelectorKind, entry: &Named) -> String {
        if kind == SelectorKind::Channel {
            let shared = self.channels.iter().any(|other| {
                other.id != entry.id && other.team_id != entry.team_id && other.local == entry.local
            });
            if shared {
                return self.qualified_label(kind, entry);
            }
        }
        with_archived(entry.label.clone(), entry.archived)
    }

    /// Always qualified: `Analysis Lab / #methods`.
    fn qualified_label(&self, kind: SelectorKind, entry: &Named) -> String {
        if kind != SelectorKind::Channel {
            return with_archived(entry.label.clone(), entry.archived);
        }
        let team = entry
            .team_id
            .and_then(|team_id| self.teams.iter().find(|team| team.id == team_id))
            .map_or_else(|| "Another team".to_owned(), |team| team.label.clone());
        with_archived(format!("{team} / {}", entry.label), entry.archived)
    }
}

fn with_archived(mut label: String, archived: bool) -> String {
    if archived {
        label.push_str(" · archived");
    }
    label
}

/// A saved connection's name, or `name — server` when another saved connection shares it.
fn connection_label(connection: &Connection, connections: &[Connection]) -> String {
    let lowered = connection.name.to_lowercase();
    let shared = connections
        .iter()
        .any(|other| other.id != connection.id && other.name.to_lowercase() == lowered);
    if shared {
        format!(
            "{} — {}",
            plain(&connection.name),
            plain(&connection.ssh_target)
        )
    } else {
        plain(&connection.name)
    }
}

/// Text with one pair of surrounding quotes removed (`"Analysis Lab"`, `'Analysis Lab'` or
/// typographic quotes), trimmed.
fn unquote(text: &str) -> &str {
    let text = text.trim();
    for (open, close) in [
        ('"', '"'),
        ('\'', '\''),
        ('\u{201C}', '\u{201D}'),
        ('\u{2018}', '\u{2019}'),
    ] {
        if let Some(inner) = text
            .strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
        {
            return inner.trim();
        }
    }
    text
}

/// Display text with invisible and control characters removed, so a label can never reorder
/// or hide the text around it.
fn plain(text: &str) -> String {
    clean(
        &strip_ignorable(text)
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>(),
    )
}

/// Several labels can be identical (two legacy accounts with one name); number those so each
/// candidate is a distinct line. Sorted for a stable answer.
fn distinct(mut labels: Vec<String>) -> Vec<String> {
    labels.sort();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for label in &labels {
        *counts.entry(label.clone()).or_default() += 1;
    }
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    labels
        .into_iter()
        .map(|label| {
            let total = counts[&label];
            if total == 1 {
                return label;
            }
            let index = seen.entry(label.clone()).or_default();
            *index += 1;
            format!("{label} ({index} of {total})")
        })
        .collect()
}

fn resolved(
    kind: SelectorKind,
    raw: &str,
    id: &str,
    label: Option<String>,
    username: Option<String>,
) -> Resolution {
    Resolution::Resolved {
        kind,
        text: raw.to_owned(),
        id: id.to_owned(),
        label,
        username,
    }
}

fn unknown(kind: SelectorKind, raw: &str, did_you_mean: Option<String>) -> Resolution {
    Resolution::UnknownName {
        kind,
        text: raw.to_owned(),
        did_you_mean,
    }
}

fn ambiguous(kind: SelectorKind, raw: &str, candidates: Vec<String>) -> Resolution {
    Resolution::AmbiguousName {
        kind,
        text: raw.to_owned(),
        candidates: distinct(candidates),
    }
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
