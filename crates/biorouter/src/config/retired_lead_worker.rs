//! Lead/worker mode is retired (owner, 2026-11-11); this carries an old setup
//! onto the shape that replaced it, once, at start.
//!
//! Lead/worker mode ran a lead model for a chat's first turns and then handed
//! the chat to a worker model. In the config file the lead was
//! `BIOROUTER_LEAD_PROVIDER` / `BIOROUTER_LEAD_MODEL`, and the worker was the
//! ordinary default, `BIOROUTER_PROVIDER` / `BIOROUTER_MODEL`. The composite
//! reported itself as its lead (its name and model were the lead's), so the
//! lead was the model a chat was bound to.
//!
//! The new shape keeps that reading. A chat runs on one model, the one it is
//! bound to, and subagents run on an optional default worker model
//! (`agents::default_worker`). So [`migrate`] makes the old lead the model new
//! chats start on and the old worker the default worker model:
//!
//! | before                                   | after                                  |
//! |------------------------------------------|----------------------------------------|
//! | `BIOROUTER_LEAD_PROVIDER` (or the default provider) | `BIOROUTER_PROVIDER`        |
//! | `BIOROUTER_LEAD_MODEL`                   | `BIOROUTER_MODEL`                      |
//! | `BIOROUTER_PROVIDER` / `BIOROUTER_MODEL` | `BIOROUTER_WORKER_PROVIDER` / `_MODEL` |
//! | `BIOROUTER_LEAD_TURNS`, `_FAILURE_THRESHOLD`, `_FALLBACK_TURNS` | removed   |
//!
//! One exception: with `BIOROUTER_LEAD_TURNS` at 0 the worker answered every
//! turn and the lead was only a fallback, so the worker stays the model new
//! chats start on and the lead is dropped.
//!
//! **Idempotent.** It reads the config file's own values, never the
//! environment, and does nothing once no lead key is left in the file, so it
//! runs at every start of `biorouterd` and of the command line tool. All of it
//! is one config write ([`Config::edit_values`]): a reader never sees the lead
//! moved and the worker not yet.
//!
//! An existing chat that was saved as a lead/worker pair keeps restoring as
//! one (`providers::factory::create_from_persisted`); only new chats and
//! subagents follow the new shape.
//!
//! ⚠ `BIOROUTER_LEAD_PROVIDER` and `BIOROUTER_LEAD_MODEL` stay capability keys
//! (`privacy::config_keys`) because this migration writes them into
//! `BIOROUTER_PROVIDER` / `BIOROUTER_MODEL`: an unproven write to one of them
//! over HTTP would otherwise become a write to the default provider at the
//! next start.

use serde_yaml::{Mapping, Value};

use crate::agents::default_worker::{DefaultWorker, WORKER_MODEL_KEY, WORKER_PROVIDER_KEY};
use crate::config::{Config, ConfigError};

pub const LEAD_MODEL_KEY: &str = "BIOROUTER_LEAD_MODEL";
pub const LEAD_PROVIDER_KEY: &str = "BIOROUTER_LEAD_PROVIDER";
pub const LEAD_TURNS_KEY: &str = "BIOROUTER_LEAD_TURNS";

/// Every key lead/worker mode read, all removed by [`migrate`].
pub const RETIRED_KEYS: [&str; 6] = [
    LEAD_MODEL_KEY,
    LEAD_PROVIDER_KEY,
    LEAD_TURNS_KEY,
    "BIOROUTER_LEAD_FAILURE_THRESHOLD",
    "BIOROUTER_LEAD_FALLBACK_TURNS",
    "BIOROUTER_LEAD_CONTEXT_LIMIT",
];

const PROVIDER_KEY: &str = "BIOROUTER_PROVIDER";
const MODEL_KEY: &str = "BIOROUTER_MODEL";

/// What [`migrate`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeadWorkerMigration {
    /// A lead model was set. New chats now start on `chat_model` (and on
    /// `chat_provider` when the file named one), and `worker` is the default
    /// worker model when the pair had a distinct one.
    Moved {
        chat_provider: Option<String>,
        chat_model: String,
        worker: Option<DefaultWorker>,
    },
    /// `BIOROUTER_LEAD_TURNS` was 0, so the worker answered every turn: it stays
    /// the chat model and the fallback-only lead is dropped.
    KeptWorker,
    /// Lead keys with no lead model, which did nothing: removed.
    RemovedLeftovers,
}

impl LeadWorkerMigration {
    /// One line for the log.
    pub fn summary(&self) -> String {
        match self {
            Self::Moved {
                chat_provider,
                chat_model,
                worker,
            } => {
                let chat = match chat_provider {
                    Some(provider) => format!("{chat_model} ({provider})"),
                    None => chat_model.clone(),
                };
                match worker {
                    Some(worker) => format!(
                        "lead/worker mode is retired: new chats start on the old lead model, \
                         {chat}, and subagents use the old worker model, {}, as the default \
                         worker model",
                        worker.label()
                    ),
                    None => format!(
                        "lead/worker mode is retired: new chats start on the old lead model, \
                         {chat}"
                    ),
                }
            }
            Self::KeptWorker => "lead/worker mode is retired: the model that answered every \
                                 turn stays the chat model, and the fallback lead model is \
                                 removed"
                .to_string(),
            Self::RemovedLeftovers => {
                "lead/worker mode is retired: removed lead settings that named no lead model"
                    .to_string()
            }
        }
    }
}

fn text(values: &Mapping, key: &str) -> Option<String> {
    values
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// `BIOROUTER_LEAD_TURNS` as the old factory parsed it (a `usize`), whether the
/// file holds a number or a string.
fn lead_turns(values: &Mapping) -> Option<u64> {
    match values.get(LEAD_TURNS_KEY)? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn set(values: &mut Mapping, key: &str, value: &str) {
    values.insert(Value::from(key), Value::from(value));
}

/// The edit itself, on the file's values. Pure, for the tests.
fn migrate_values(values: &mut Mapping) -> Option<LeadWorkerMigration> {
    let present = RETIRED_KEYS.iter().any(|key| values.contains_key(*key));
    if !present {
        return None;
    }
    let lead_model = text(values, LEAD_MODEL_KEY);
    let lead_provider = text(values, LEAD_PROVIDER_KEY);
    let turns = lead_turns(values);
    let old_provider = text(values, PROVIDER_KEY);
    let old_model = text(values, MODEL_KEY);
    for key in RETIRED_KEYS {
        values.shift_remove(key);
    }

    let Some(lead_model) = lead_model else {
        return Some(LeadWorkerMigration::RemovedLeftovers);
    };
    if turns == Some(0) {
        return Some(LeadWorkerMigration::KeptWorker);
    }

    let chat_provider = lead_provider.or_else(|| old_provider.clone());
    if let Some(provider) = &chat_provider {
        set(values, PROVIDER_KEY, provider);
    }
    set(values, MODEL_KEY, &lead_model);

    // The old worker becomes the default worker model, unless one is already
    // set (a person's own choice wins) or the pair named one model twice.
    let worker_already_set =
        text(values, WORKER_PROVIDER_KEY).is_some() || text(values, WORKER_MODEL_KEY).is_some();
    let worker = match (old_provider, old_model) {
        (Some(provider), Some(model))
            if !worker_already_set
                && !(chat_provider.as_deref() == Some(provider.as_str())
                    && model == lead_model) =>
        {
            set(values, WORKER_MODEL_KEY, &model);
            set(values, WORKER_PROVIDER_KEY, &provider);
            Some(DefaultWorker { provider, model })
        }
        _ => None,
    };
    Some(LeadWorkerMigration::Moved {
        chat_provider,
        chat_model: lead_model,
        worker,
    })
}

/// Carry a lead/worker setup in `config`'s file onto the new shape. `Ok(None)`
/// when the file holds no lead key, which is every start after the first.
pub fn migrate(config: &Config) -> Result<Option<LeadWorkerMigration>, ConfigError> {
    config.edit_values(migrate_values)
}

/// [`migrate`] on the global configuration, logged. Called once at the start
/// of `biorouterd agent` and of the command line tool, before anything reads
/// the default provider. A failure is logged and the process carries on: the
/// old keys are then simply ignored, as they are by everything else now.
pub fn migrate_on_start() {
    let config = Config::global();
    match migrate(config) {
        Ok(Some(done)) => tracing::info!("{}", done.summary()),
        Ok(None) => {}
        Err(error) => tracing::warn!(
            %error,
            "could not carry the retired lead/worker settings across; they are ignored"
        ),
    }
    if std::env::var_os(LEAD_MODEL_KEY).is_some() {
        tracing::warn!(
            "{LEAD_MODEL_KEY} is set in the environment, but lead/worker mode is retired and the \
             variable is ignored. Set {WORKER_PROVIDER_KEY} and {WORKER_MODEL_KEY} to choose a \
             default worker model for subagents."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(pairs: &[(&str, Value)]) -> Mapping {
        let mut values = Mapping::new();
        for (key, value) in pairs {
            values.insert(Value::from(*key), value.clone());
        }
        values
    }

    fn s(value: &str) -> Value {
        Value::from(value)
    }

    fn get(values: &Mapping, key: &str) -> Option<String> {
        values.get(key).and_then(Value::as_str).map(str::to_string)
    }

    #[test]
    fn the_lead_becomes_the_chat_model_and_the_worker_the_default_worker() {
        let mut values = file(&[
            (PROVIDER_KEY, s("openai")),
            (MODEL_KEY, s("gpt-4o-mini")),
            (LEAD_PROVIDER_KEY, s("anthropic")),
            (LEAD_MODEL_KEY, s("claude-opus-4")),
            (LEAD_TURNS_KEY, Value::from(3)),
            ("BIOROUTER_LEAD_FAILURE_THRESHOLD", Value::from(2)),
            ("BIOROUTER_LEAD_FALLBACK_TURNS", Value::from(2)),
            ("UNRELATED", s("kept")),
        ]);
        let done = migrate_values(&mut values).expect("a lead/worker setup is moved");
        assert_eq!(
            done,
            LeadWorkerMigration::Moved {
                chat_provider: Some("anthropic".into()),
                chat_model: "claude-opus-4".into(),
                worker: Some(DefaultWorker {
                    provider: "openai".into(),
                    model: "gpt-4o-mini".into()
                }),
            }
        );
        assert_eq!(get(&values, PROVIDER_KEY).as_deref(), Some("anthropic"));
        assert_eq!(get(&values, MODEL_KEY).as_deref(), Some("claude-opus-4"));
        assert_eq!(get(&values, WORKER_PROVIDER_KEY).as_deref(), Some("openai"));
        assert_eq!(
            get(&values, WORKER_MODEL_KEY).as_deref(),
            Some("gpt-4o-mini")
        );
        assert_eq!(get(&values, "UNRELATED").as_deref(), Some("kept"));
        for key in RETIRED_KEYS {
            assert!(!values.contains_key(key), "{key} is left behind");
        }
    }

    #[test]
    fn an_unset_lead_provider_meant_the_default_provider() {
        let mut values = file(&[
            (PROVIDER_KEY, s("openai")),
            (MODEL_KEY, s("gpt-4o-mini")),
            (LEAD_MODEL_KEY, s("gpt-4o")),
        ]);
        let done = migrate_values(&mut values).unwrap();
        assert_eq!(
            done,
            LeadWorkerMigration::Moved {
                chat_provider: Some("openai".into()),
                chat_model: "gpt-4o".into(),
                worker: Some(DefaultWorker {
                    provider: "openai".into(),
                    model: "gpt-4o-mini".into()
                }),
            }
        );
        assert_eq!(get(&values, MODEL_KEY).as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn it_runs_once() {
        let mut values = file(&[
            (PROVIDER_KEY, s("openai")),
            (MODEL_KEY, s("gpt-4o-mini")),
            (LEAD_MODEL_KEY, s("gpt-4o")),
        ]);
        assert!(migrate_values(&mut values).is_some());
        let after_first = values.clone();
        assert_eq!(migrate_values(&mut values), None, "nothing left to move");
        assert_eq!(values, after_first, "a second run changes nothing");
    }

    #[test]
    fn a_config_with_no_lead_keys_is_untouched() {
        let mut values = file(&[(PROVIDER_KEY, s("openai")), (MODEL_KEY, s("gpt-4o"))]);
        let before = values.clone();
        assert_eq!(migrate_values(&mut values), None);
        assert_eq!(values, before);
    }

    #[test]
    fn zero_lead_turns_keeps_the_model_that_answered_every_turn() {
        let mut values = file(&[
            (PROVIDER_KEY, s("openai")),
            (MODEL_KEY, s("gpt-4o-mini")),
            (LEAD_MODEL_KEY, s("gpt-4o")),
            (LEAD_TURNS_KEY, s("0")),
        ]);
        assert_eq!(
            migrate_values(&mut values),
            Some(LeadWorkerMigration::KeptWorker)
        );
        assert_eq!(get(&values, PROVIDER_KEY).as_deref(), Some("openai"));
        assert_eq!(get(&values, MODEL_KEY).as_deref(), Some("gpt-4o-mini"));
        assert!(!values.contains_key(WORKER_PROVIDER_KEY));
        assert!(!values.contains_key(LEAD_MODEL_KEY));
    }

    #[test]
    fn leftovers_without_a_lead_model_are_removed() {
        let mut values = file(&[
            (PROVIDER_KEY, s("openai")),
            (MODEL_KEY, s("gpt-4o")),
            (LEAD_TURNS_KEY, Value::from(3)),
            (LEAD_PROVIDER_KEY, s("anthropic")),
        ]);
        assert_eq!(
            migrate_values(&mut values),
            Some(LeadWorkerMigration::RemovedLeftovers)
        );
        assert_eq!(get(&values, PROVIDER_KEY).as_deref(), Some("openai"));
        assert_eq!(get(&values, MODEL_KEY).as_deref(), Some("gpt-4o"));
        assert!(!values.contains_key(LEAD_PROVIDER_KEY));
        assert!(!values.contains_key(LEAD_TURNS_KEY));
    }

    #[test]
    fn a_default_worker_already_chosen_is_kept() {
        let mut values = file(&[
            (PROVIDER_KEY, s("openai")),
            (MODEL_KEY, s("gpt-4o-mini")),
            (LEAD_MODEL_KEY, s("gpt-4o")),
            (WORKER_PROVIDER_KEY, s("ollama")),
            (WORKER_MODEL_KEY, s("qwen3.8:27b")),
        ]);
        let done = migrate_values(&mut values).unwrap();
        assert_eq!(
            done,
            LeadWorkerMigration::Moved {
                chat_provider: Some("openai".into()),
                chat_model: "gpt-4o".into(),
                worker: None,
            }
        );
        assert_eq!(get(&values, WORKER_PROVIDER_KEY).as_deref(), Some("ollama"));
        assert_eq!(
            get(&values, WORKER_MODEL_KEY).as_deref(),
            Some("qwen3.8:27b")
        );
    }

    #[test]
    fn a_pair_that_named_one_model_twice_sets_no_worker() {
        let mut values = file(&[
            (PROVIDER_KEY, s("openai")),
            (MODEL_KEY, s("gpt-4o")),
            (LEAD_MODEL_KEY, s("gpt-4o")),
        ]);
        let done = migrate_values(&mut values).unwrap();
        assert_eq!(
            done,
            LeadWorkerMigration::Moved {
                chat_provider: Some("openai".into()),
                chat_model: "gpt-4o".into(),
                worker: None,
            }
        );
        assert!(!values.contains_key(WORKER_PROVIDER_KEY));
    }

    #[test]
    fn the_migration_is_one_write_to_the_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new_with_file_secrets(
            dir.path().join("config.yaml"),
            dir.path().join("secrets.yaml"),
        )
        .unwrap();
        config
            .set_params(&[
                (PROVIDER_KEY, "openai"),
                (MODEL_KEY, "gpt-4o-mini"),
                (LEAD_PROVIDER_KEY, "anthropic"),
                (LEAD_MODEL_KEY, "claude-opus-4"),
            ])
            .unwrap();
        let done = migrate(&config).unwrap();
        assert!(matches!(done, Some(LeadWorkerMigration::Moved { .. })));
        let values = config.all_values().unwrap();
        let read = |key: &str| values.get(key).and_then(|v| v.as_str()).map(str::to_string);
        assert_eq!(read(PROVIDER_KEY).as_deref(), Some("anthropic"));
        assert_eq!(read(MODEL_KEY).as_deref(), Some("claude-opus-4"));
        assert_eq!(read(WORKER_PROVIDER_KEY).as_deref(), Some("openai"));
        assert_eq!(read(WORKER_MODEL_KEY).as_deref(), Some("gpt-4o-mini"));
        assert_eq!(read(LEAD_MODEL_KEY), None);
        assert_eq!(
            migrate(&config).unwrap(),
            None,
            "the second start does nothing"
        );
    }
}
