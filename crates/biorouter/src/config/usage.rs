use super::base::Config;
use serde::{Deserialize, Serialize};

/// The config key the [`UsageLimits`] live under. Writing or deleting it over
/// HTTP needs the user-action proof (`routes::config_management`), the same
/// proof `PUT /usage/limits` asks for, so a model holding only the daemon
/// secret cannot raise or lift a limit through the generic config routes.
pub const USAGE_LIMITS_CONFIG_KEY: &str = "usage";

/// Is `key` the usage-limits section? One predicate for every verb.
pub fn is_usage_limits_key(key: &str) -> bool {
    key == USAGE_LIMITS_CONFIG_KEY
}

/// The `usage` section of `config.yaml`: the monthly limits and the default
/// per-chat limits. Every field is optional and independent.
///
/// The monthly limits are ENFORCED: once the month's billed tokens (or priced
/// dollars) reach one, no chat makes another model request until the person
/// raises or lifts it or the next LOCAL month begins. The default per-chat
/// limits apply to every chat that has no limit of its own (a chat's own
/// limit lives on its row, see `agents::usage_limit`). Enforcement is in the
/// reply loop; see `docs/agent-loop/usage-limits.md`.
///
/// A missing, zero or negative value means unlimited, so a stray `0` never
/// stops every chat.
///
/// ```yaml
/// usage:
///   monthly_token_limit: 66000000
///   monthly_dollar_limit: 250.0
///   default_session_token_limit: 5000000
///   default_session_dollar_limit: 20.0
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageLimits {
    /// Billed tokens allowed per local calendar month, over every chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_token_limit: Option<i64>,
    /// Priced dollars allowed per local calendar month, over every chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_dollar_limit: Option<f64>,
    /// Billed tokens allowed per chat, for chats without their own limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_session_token_limit: Option<i64>,
    /// Priced dollars allowed per chat, for chats without their own limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_session_dollar_limit: Option<f64>,
}

/// One field of a [`UsageLimitsUpdate`]: leave it as stored, clear it
/// (unlimited), or set it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LimitChange<T> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

/// A partial write of the `usage` section. Fields left at
/// [`LimitChange::Keep`] are untouched, and so is every key of the section
/// this struct does not name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageLimitsUpdate {
    pub monthly_token_limit: LimitChange<i64>,
    pub monthly_dollar_limit: LimitChange<f64>,
    pub default_session_token_limit: LimitChange<i64>,
    pub default_session_dollar_limit: LimitChange<f64>,
}

fn positive_tokens(value: Option<i64>) -> Option<u64> {
    value.filter(|v| *v > 0).map(|v| v as u64)
}

fn positive_usd(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite() && *v > 0.0)
}

impl UsageLimits {
    /// Load the configured limits, defaulting to "no limits" when the section is
    /// absent or malformed — a bad budget knob must never break a usage report.
    pub fn load() -> Self {
        Self::load_from(Config::global())
    }

    /// [`Self::load`] against a given config.
    pub fn load_from(config: &Config) -> Self {
        config
            .get_param::<UsageLimits>(USAGE_LIMITS_CONFIG_KEY)
            .unwrap_or_default()
    }

    /// The monthly token limit in force, `None` when unlimited.
    pub fn monthly_tokens(&self) -> Option<u64> {
        positive_tokens(self.monthly_token_limit)
    }

    /// The monthly dollar limit in force, `None` when unlimited.
    pub fn monthly_usd(&self) -> Option<f64> {
        positive_usd(self.monthly_dollar_limit)
    }

    /// The default per-chat token limit in force, `None` when unlimited.
    pub fn default_session_tokens(&self) -> Option<u64> {
        positive_tokens(self.default_session_token_limit)
    }

    /// The default per-chat dollar limit in force, `None` when unlimited.
    pub fn default_session_usd(&self) -> Option<f64> {
        positive_usd(self.default_session_dollar_limit)
    }

    /// Merge `update` into the `usage` section of `config` in one locked
    /// read-modify-write, and return the limits as stored afterwards. Keys of
    /// the section this type does not know are kept.
    pub fn apply_update(
        config: &Config,
        update: &UsageLimitsUpdate,
    ) -> Result<Self, super::base::ConfigError> {
        config.update_param::<serde_yaml::Value, _, _>(USAGE_LIMITS_CONFIG_KEY, |section| {
            if !section.is_mapping() {
                *section = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
            }
            let map = section.as_mapping_mut().expect("just made a mapping");
            apply_change(map, "monthly_token_limit", update.monthly_token_limit);
            apply_change(map, "monthly_dollar_limit", update.monthly_dollar_limit);
            apply_change(
                map,
                "default_session_token_limit",
                update.default_session_token_limit,
            );
            apply_change(
                map,
                "default_session_dollar_limit",
                update.default_session_dollar_limit,
            );
        })?;
        Ok(Self::load_from(config))
    }
}

fn apply_change<T: Serialize>(map: &mut serde_yaml::Mapping, key: &str, change: LimitChange<T>) {
    let key = serde_yaml::Value::String(key.to_string());
    match change {
        LimitChange::Keep => {}
        LimitChange::Clear => {
            map.remove(&key);
        }
        LimitChange::Set(value) => {
            if let Ok(value) = serde_yaml::to_value(value) {
                map.insert(key, value);
            }
        }
    }
}

/// Percent of a limit consumed, or `None` when the limit is unset. Returns
/// `None` for a non-positive limit rather than dividing by zero.
pub fn percent_of(used: f64, limit: Option<f64>) -> Option<f64> {
    match limit {
        Some(limit) if limit > 0.0 => Some(used / limit * 100.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_limits() {
        let limits: UsageLimits =
            serde_yaml::from_str("monthly_token_limit: 66000000\nmonthly_dollar_limit: 250.0")
                .unwrap();
        assert_eq!(limits.monthly_token_limit, Some(66_000_000));
        assert_eq!(limits.monthly_dollar_limit, Some(250.0));
    }

    #[test]
    fn parses_partial_and_empty() {
        let only_tokens: UsageLimits = serde_yaml::from_str("monthly_token_limit: 1000").unwrap();
        assert_eq!(only_tokens.monthly_token_limit, Some(1000));
        assert_eq!(only_tokens.monthly_dollar_limit, None);

        let empty: UsageLimits = serde_yaml::from_str("{}").unwrap();
        assert_eq!(empty, UsageLimits::default());
        assert_eq!(empty.monthly_token_limit, None);
        assert_eq!(empty.monthly_dollar_limit, None);
    }

    #[test]
    fn parses_default_session_limits() {
        let limits: UsageLimits = serde_yaml::from_str(
            "default_session_token_limit: 5000\ndefault_session_dollar_limit: 2.5",
        )
        .unwrap();
        assert_eq!(limits.default_session_tokens(), Some(5000));
        assert_eq!(limits.default_session_usd(), Some(2.5));
        assert_eq!(limits.monthly_tokens(), None);
    }

    #[test]
    fn non_positive_values_mean_unlimited() {
        let limits = UsageLimits {
            monthly_token_limit: Some(0),
            monthly_dollar_limit: Some(-1.0),
            default_session_token_limit: Some(-5),
            default_session_dollar_limit: Some(f64::NAN),
        };
        assert_eq!(limits.monthly_tokens(), None);
        assert_eq!(limits.monthly_usd(), None);
        assert_eq!(limits.default_session_tokens(), None);
        assert_eq!(limits.default_session_usd(), None);
    }

    #[test]
    fn an_update_merges_into_the_section_and_keeps_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        std::fs::write(
            &config_path,
            "usage:\n  monthly_token_limit: 100\n  monthly_dollar_limit: 5.0\n  note: kept\n",
        )
        .unwrap();
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        let stored = UsageLimits::apply_update(
            &config,
            &UsageLimitsUpdate {
                monthly_token_limit: LimitChange::Set(200),
                monthly_dollar_limit: LimitChange::Clear,
                default_session_token_limit: LimitChange::Set(50),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stored.monthly_token_limit, Some(200));
        assert_eq!(stored.monthly_dollar_limit, None);
        assert_eq!(stored.default_session_token_limit, Some(50));
        assert_eq!(stored.default_session_dollar_limit, None);

        let text = std::fs::read_to_string(&config_path).unwrap();
        assert!(text.contains("note: kept"), "unknown key dropped: {text}");
    }

    #[test]
    fn percent_math_and_guards() {
        // 33M of a 66M allowance = 50%.
        assert_eq!(percent_of(33_000_000.0, Some(66_000_000.0)), Some(50.0));
        // No limit configured → no percent.
        assert_eq!(percent_of(10.0, None), None);
        // A zero/negative limit never divides by zero.
        assert_eq!(percent_of(10.0, Some(0.0)), None);
        assert_eq!(percent_of(10.0, Some(-5.0)), None);
    }
}
