//! Being told about new devices (account ownership AO-10).
//!
//! Under the account model a process signed in as the person's account can add a device to
//! it by itself, with no other computer's approval (AO-1, AO-2). What limits the cost is that
//! the person sees it: every computer of theirs keeps, per connection, the device fingerprints
//! it last saw in the workspace snapshot's `actor.devices`, and reports each device added since
//! then (how and when it was added) until the person acknowledges it. Its own device is never
//! reported to itself.
//!
//! The first snapshot a connection is seen with sets what was seen, and reports nothing: there
//! is nothing earlier to compare with. Kept in the saved registry, so every process sharing the
//! profile reports the same devices and one acknowledgement clears them everywhere. Once a
//! connection is watched, a person's connect reads one snapshot and a keepalive re-dial's
//! membership read is the snapshot, so a computer that was offline is told at its next connect.
//! An acknowledgement names the devices the person was shown, and clears only those.

use super::authentication::grouped_fingerprint_shape;
use super::{CrewManager, Registry};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// At most this many devices are read from one snapshot. The workspace keeps at most 32 per
/// account (AO-2); this bounds what a misbehaving server can make this computer keep.
const MAX_LISTED_DEVICES: usize = 64;
/// The longest `added_via` word kept.
const MAX_ADDED_VIA_BYTES: usize = 32;

/// A device of the person's account this computer has not told them about yet: one added
/// since this computer last saw the account's devices, and not yet acknowledged.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct NewDevice {
    /// The grouped 16-hex fingerprint the snapshot shows (`3F2A 9C1E 77B0 D4E1`).
    pub fingerprint: String,
    /// How it was added, as the workspace says (`account`, `invitation`, `bootstrap`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_via: Option<String>,
    /// When it was added, in seconds since the Unix epoch, as the workspace says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_at: Option<u64>,
}

/// What this computer last saw of the account's devices on one connection.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct DeviceWatch {
    /// The fingerprints the person already knows of: the devices the first snapshot listed,
    /// this computer's own, and every device acknowledged since. Only devices still listed are
    /// kept.
    known: BTreeSet<String>,
    /// The devices listed now that are not known, until the person acknowledges them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    new: Vec<NewDevice>,
}

impl DeviceWatch {
    /// The watch after a snapshot that listed `listed`, from `previous` (`None`: the first
    /// snapshot seen for the connection, which reports nothing). `own` is this computer's own
    /// device, which is never reported to it.
    fn observe(previous: Option<&Self>, listed: &[Listed], own: &str) -> Self {
        let present: BTreeSet<String> = listed
            .iter()
            .map(|device| device.device.fingerprint.clone())
            .collect();
        let mine = |device: &Listed| device.this_device || device.device.fingerprint == own;
        let mut known: BTreeSet<String> = match previous {
            None => present.clone(),
            Some(previous) => previous.known.intersection(&present).cloned().collect(),
        };
        known.extend(
            listed
                .iter()
                .filter(|device| mine(device))
                .map(|device| device.device.fingerprint.clone()),
        );
        let new = listed
            .iter()
            .filter(|device| !known.contains(&device.device.fingerprint))
            .map(|device| device.device.clone())
            .collect();
        Self { known, new }
    }

    /// The watch once the person has seen the devices whose fingerprints are `shown`: those
    /// become known, and a device reported since that is not among them stays reported, so a
    /// device added while a notice was on screen is never cleared unseen.
    fn acknowledged(mut self, shown: &[String]) -> Self {
        let (seen, unseen): (Vec<NewDevice>, Vec<NewDevice>) = self
            .new
            .drain(..)
            .partition(|device| shown.contains(&device.fingerprint));
        self.known
            .extend(seen.into_iter().map(|device| device.fingerprint));
        self.new = unseen;
        self
    }
}

/// One device a snapshot listed.
struct Listed {
    device: NewDevice,
    /// The snapshot says it is the key that signed the snapshot request (AO-5).
    this_device: bool,
}

/// The account's devices a snapshot lists in `actor.devices`, each with a fingerprint of the
/// grouped shape; `None` when the snapshot lists none at all (an older server, or no actor),
/// which tells nothing about the devices.
fn listed_devices(snapshot: &Value) -> Option<Vec<Listed>> {
    let devices = snapshot["actor"]["devices"].as_array()?;
    let mut listed: Vec<Listed> = Vec::new();
    for device in devices.iter().take(MAX_LISTED_DEVICES) {
        let Some(fingerprint) = device["fingerprint"]
            .as_str()
            .filter(|fingerprint| grouped_fingerprint_shape(fingerprint))
        else {
            continue;
        };
        if listed
            .iter()
            .any(|seen| seen.device.fingerprint == fingerprint)
        {
            continue;
        }
        let added_via = device["added_via"]
            .as_str()
            .filter(|word| {
                !word.is_empty()
                    && word.len() <= MAX_ADDED_VIA_BYTES
                    && word
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
            .map(str::to_owned);
        listed.push(Listed {
            device: NewDevice {
                fingerprint: fingerprint.to_owned(),
                added_via,
                added_at: device["added_at"].as_u64(),
            },
            this_device: device["this_device"].as_bool() == Some(true),
        });
    }
    Some(listed)
}

impl CrewManager {
    /// Note the account's devices a signed `workspace.snapshot` of connection `id` listed
    /// (AO-10). Writes the saved registry only when what is reported changes, so a snapshot
    /// read again and again costs nothing more. Best effort: a failed save is logged, and the
    /// next snapshot tries again.
    pub(super) async fn observe_devices(&self, id: &str, snapshot: &Value) {
        let Some(listed) = listed_devices(snapshot) else {
            return;
        };
        let unchanged = {
            let registry = self.registry.lock().await;
            let Some(own) = own_fingerprint(&registry, id) else {
                return;
            };
            let previous = registry.seen_devices.get(id);
            previous.is_some_and(|previous| {
                &DeviceWatch::observe(Some(previous), &listed, &own) == previous
            })
        };
        if unchanged {
            return;
        }
        let saved = self
            .update_registry(|r| {
                let Some(own) = own_fingerprint(r, id) else {
                    return Ok(());
                };
                let watch = DeviceWatch::observe(r.seen_devices.get(id), &listed, &own);
                r.seen_devices.insert(id.to_owned(), watch);
                Ok(())
            })
            .await;
        if let Err(error) = saved {
            tracing::debug!(connection = id, %error, "Couldn't save the Crew devices this computer saw");
        }
    }

    /// The devices of the person's account added since this computer last saw them on the
    /// connection `id`, with how and when each was added, until the person acknowledges them
    /// ([`Self::acknowledge_devices`]). Empty before the first snapshot. Refused for a
    /// connection that is not saved.
    pub async fn new_devices(&self, id: &str) -> Result<Vec<NewDevice>> {
        self.refresh_registry().await;
        let registry = self.registry.lock().await;
        registry
            .connections
            .iter()
            .any(|c| c.id == id)
            .then_some(())
            .context("Unknown Crew connection")?;
        Ok(registry
            .seen_devices
            .get(id)
            .map(|watch| watch.new.clone())
            .unwrap_or_default())
    }

    /// The person has seen the new devices of connection `id` whose fingerprints are `shown`
    /// (the grouped form [`Self::new_devices`] reports): stop reporting those, on every
    /// computer process that shares this profile. A device reported after the person was
    /// shown the list, and so not in `shown`, stays reported; a fingerprint that is not
    /// reported changes nothing. Refused for a connection that is not saved.
    pub async fn acknowledge_devices(&self, id: &str, shown: &[String]) -> Result<()> {
        self.update_registry(|r| {
            r.connections
                .iter()
                .any(|c| c.id == id)
                .then_some(())
                .context("Unknown Crew connection")?;
            if let Some(watch) = r.seen_devices.remove(id) {
                r.seen_devices
                    .insert(id.to_owned(), watch.acknowledged(shown));
            }
            Ok(())
        })
        .await
    }

    /// Whether this computer keeps what it last saw of the account's devices on connection
    /// `id`: a snapshot was read there before, so one read now can report what was added
    /// since (AO-10). Before that there is nothing to compare with.
    pub(super) async fn watches_devices(&self, id: &str) -> bool {
        self.refresh_registry().await;
        self.registry.lock().await.seen_devices.contains_key(id)
    }

    /// AO-10 at a person's connect: one signed `workspace.snapshot`, read the way every
    /// snapshot is ([`Self::observe_devices`]), so a device added while this computer was
    /// offline is reported now rather than at the next read someone happens to make. Only for
    /// a connection whose devices this computer watches ([`Self::watches_devices`]), and best
    /// effort: a refusal is handled where every signed request's is, and the connect stands.
    /// The caller holds no lifecycle guard, which a request whose bridge breaks takes.
    pub(super) async fn observe_devices_at_connect(&self, id: &str) {
        if !self.watches_devices(id).await {
            return;
        }
        if let Err(error) = self
            .signed_request(id, "workspace.snapshot", serde_json::json!({}), None)
            .await
        {
            tracing::debug!(connection = id, %error, "Couldn't read the Crew account's devices after connecting");
        }
    }
}

/// This computer's own device on connection `id`, as the snapshot shows fingerprints; `None`
/// for a connection the registry does not hold.
fn own_fingerprint(registry: &Registry, id: &str) -> Option<String> {
    registry
        .connections
        .iter()
        .find(|c| c.id == id)
        .map(|c| biorouter_crew::invitation::grouped_fingerprint(&c.device_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot(devices: Value) -> Value {
        json!({"actor": {"id": "p-1", "username": "bob", "devices": devices}})
    }

    fn device(fingerprint: &str, via: &str, at: u64) -> Value {
        json!({"fingerprint": fingerprint, "added_via": via, "added_at": at})
    }

    const OWN: &str = "AAAA AAAA AAAA AAAA";
    const LAPTOP: &str = "BBBB BBBB BBBB BBBB";
    const DESKTOP: &str = "CCCC CCCC CCCC CCCC";

    fn observe(previous: Option<&DeviceWatch>, snapshot: &Value) -> DeviceWatch {
        DeviceWatch::observe(previous, &listed_devices(snapshot).unwrap(), OWN)
    }

    /// AO-10: the first snapshot sets what was seen; a device added after it is reported with
    /// how and when it was added until it is acknowledged; this computer's own device never is.
    #[test]
    fn a_device_added_since_the_last_snapshot_is_reported_until_acknowledged() {
        let first = observe(
            None,
            &snapshot(json!([
                device(OWN, "invitation", 1),
                device(LAPTOP, "invitation", 2)
            ])),
        );
        assert!(first.new.is_empty(), "the first snapshot reports nothing");

        let added = snapshot(json!([
            device(OWN, "invitation", 1),
            device(LAPTOP, "invitation", 2),
            device(DESKTOP, "account", 3)
        ]));
        let second = observe(Some(&first), &added);
        assert_eq!(
            second.new,
            vec![NewDevice {
                fingerprint: DESKTOP.into(),
                added_via: Some("account".into()),
                added_at: Some(3),
            }]
        );
        // Seen again, it is still new until acknowledged; then never again.
        assert_eq!(observe(Some(&second), &added), second);
        let acknowledged = second.acknowledged(&[DESKTOP.to_owned()]);
        assert!(acknowledged.new.is_empty());
        assert!(observe(Some(&acknowledged), &added).new.is_empty());
    }

    /// Only the devices the person was shown are acknowledged: one reported after the notice
    /// was drawn stays reported, and a fingerprint nobody reported changes nothing.
    #[test]
    fn acknowledging_clears_only_the_devices_the_person_was_shown() {
        const PHONE: &str = "DDDD DDDD DDDD DDDD";
        let first = observe(None, &snapshot(json!([device(OWN, "invitation", 1)])));
        let shown = observe(
            Some(&first),
            &snapshot(json!([
                device(OWN, "invitation", 1),
                device(DESKTOP, "account", 3)
            ])),
        );
        // A snapshot taken while the notice was on screen reports one more.
        let both = snapshot(json!([
            device(OWN, "invitation", 1),
            device(DESKTOP, "account", 3),
            device(PHONE, "account", 4)
        ]));
        let later = observe(Some(&shown), &both);
        assert_eq!(later.new.len(), 2);

        let acknowledged = later.acknowledged(&[DESKTOP.to_owned(), LAPTOP.to_owned()]);
        assert_eq!(
            acknowledged
                .new
                .iter()
                .map(|device| device.fingerprint.as_str())
                .collect::<Vec<_>>(),
            [PHONE],
            "the device the person never saw is still reported"
        );
        assert!(acknowledged.known.contains(DESKTOP));
        assert!(!acknowledged.known.contains(LAPTOP));
        assert_eq!(observe(Some(&acknowledged), &both).new.len(), 1);
        assert_eq!(acknowledged.clone().acknowledged(&[]), acknowledged);
    }

    /// A new key of this computer (a rejoin) is not news to it, and a revoked device stops
    /// being reported.
    #[test]
    fn this_computers_own_device_is_never_reported_and_a_revoked_one_goes() {
        let first = observe(None, &snapshot(json!([device(LAPTOP, "invitation", 2)])));
        let mine = snapshot(json!([
            device(LAPTOP, "invitation", 2),
            device(OWN, "account", 4),
            {"fingerprint": DESKTOP, "added_via": "account", "added_at": 5, "this_device": true}
        ]));
        assert!(observe(Some(&first), &mine).new.is_empty());

        let with_desktop = observe(
            Some(&first),
            &snapshot(json!([
                device(LAPTOP, "invitation", 2),
                device(DESKTOP, "account", 5)
            ])),
        );
        assert_eq!(with_desktop.new.len(), 1);
        let revoked = observe(
            Some(&with_desktop),
            &snapshot(json!([device(LAPTOP, "invitation", 2)])),
        );
        assert!(revoked.new.is_empty());
        assert!(!revoked.known.contains(DESKTOP));
    }

    /// A snapshot without `actor.devices` tells nothing; malformed entries are left out and
    /// words a server could use to inject text are not kept.
    #[test]
    fn only_well_formed_devices_are_read() {
        assert!(listed_devices(&json!({"actor": {"id": "p-1"}})).is_none());
        let listed = listed_devices(&snapshot(json!([
            {"fingerprint": "not a fingerprint"},
            {"fingerprint": LAPTOP, "added_via": "via\nanything", "added_at": "soon"},
            {"fingerprint": LAPTOP, "added_via": "account"}
        ])))
        .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].device.fingerprint, LAPTOP);
        assert_eq!(listed[0].device.added_via, None);
        assert_eq!(listed[0].device.added_at, None);
    }
}
