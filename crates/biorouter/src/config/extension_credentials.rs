use super::ExtensionEntry;
use crate::agents::ExtensionConfig;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExtensionCredential {
    pub key: String,
    pub stored: bool,
    pub used_by: Vec<String>,
}

fn credential_keys(config: &ExtensionConfig) -> &[String] {
    match config {
        ExtensionConfig::Stdio { env_keys, .. }
        | ExtensionConfig::StreamableHttp { env_keys, .. } => env_keys,
        _ => &[],
    }
}

pub(super) fn review(
    name: &str,
    config: &HashMap<String, Value>,
    secrets: &HashMap<String, Value>,
    provider_keys: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<ExtensionCredential>> {
    // Unlike the extension inventory, deletion must not skip malformed entries:
    // an unreadable entry may be another consumer of the same credential.
    let extensions: BTreeMap<String, ExtensionEntry> = serde_json::from_value(
        config.get("extensions").cloned().unwrap_or(Value::Null),
    )
    .map_err(|_| {
        anyhow::anyhow!("Cannot verify credential references: invalid extension configuration")
    })?;
    let matches: Vec<_> = extensions
        .values()
        .filter(|e| e.config.name() == name)
        .collect();
    if matches.len() != 1 {
        bail!("Extension is missing or its name is ambiguous; reload Settings");
    }
    let target = matches[0];
    let mut keys = credential_keys(&target.config).to_vec();
    keys.sort();
    keys.dedup();
    Ok(keys
        .into_iter()
        .map(|key| {
            let mut used_by = provider_keys
                .get(&key.to_uppercase())
                .cloned()
                .unwrap_or_default();
            for entry in extensions
                .values()
                .filter(|entry| entry.config.name() != name)
            {
                if credential_keys(&entry.config)
                    .iter()
                    .any(|other| other.eq_ignore_ascii_case(&key))
                {
                    used_by.push(format!("Extension: {}", entry.config.name()));
                }
            }
            if crate::privacy::is_capability_key(&key)
                || crate::privacy::is_privacy_tiers_key(&key)
                || crate::privacy::mixing::is_mixing_policy_key(&key)
            {
                used_by.push("Protected application setting".to_string());
            }
            used_by.sort();
            used_by.dedup();
            ExtensionCredential {
                stored: secrets.contains_key(&key),
                key,
                used_by,
            }
        })
        .collect())
}
