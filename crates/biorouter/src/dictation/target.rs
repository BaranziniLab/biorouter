//! Who a dictation is aimed at (`docs/dictation/dictionary.md`, "The bias set", DD-30,
//! DD-77): whether its target chat is private, and the terminal rule that decides when a
//! CLI transcription may use private entries.
//!
//! [`chat_reach`] answers "is this chat private?" for every dictation door, so the stream
//! target, the observation stamp and the extractor stamp cannot disagree. The two uses fail
//! in opposite directions, each toward the safe side: a stamp treats `Unknown` as Private
//! (a stamp that fails toward private costs the person a publicize click), and a stream
//! target treats `Unknown` as public (a target that failed toward private would let private
//! terms shape text bound for a chat whose tier nobody knows).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::decision::save::{tier_change, AfterSave, LoweringConsent, SaveDoor};
use crate::decision::settings::{validate, DecisionSettings, MAX_MONITORS};
use crate::decision::{DecisionModelRef, DecisionSession};
use crate::model::ModelConfig;
use crate::privacy::config_keys::NOT_CAPABILITY_CONFIG_KEYS;
use crate::privacy::decision_axes;
use crate::privacy::{
    ModelAffiliation, ProviderTier, SessionClassification, CAPABILITY_CONFIG_KEYS,
};
use crate::session::Session;

use super::bias::{BiasTarget, Standing};

/// How long [`new_chat_bind_tier`] waits for a provider to be built before it answers
/// Public. Building one reads configuration and, for some providers, a local file; it
/// sends nothing, so this bound is only a backstop.
const NEW_CHAT_BUILD_BOUND: Duration = Duration::from_secs(5);
/// Answers kept by [`new_chat_bind_tier`]: the configured default and a few per tab picks.
const NEW_CHAT_CACHE_ENTRIES: usize = 32;
/// How long one answer of [`new_chat_bind_tier`] is kept. A tier can move with no
/// configuration value changing: `ollama`'s `tier()` also reads what `/api/show` said
/// about the model (`remote_host` or `remote_model`, a cloud model renamed with
/// `ollama cp`), which arrives only once the model was asked about, so an answer given
/// before then is rebuilt within this long. The dictation service's thread count is
/// reused for the same time.
pub const NEW_CHAT_CACHE_TTL: Duration = Duration::from_secs(10);

/// The answer [`new_chat_bind_tier`] gives: the tier a new chat would bind at and its
/// model's affiliation, from one provider instance.
pub type BindTier = (ProviderTier, Option<ModelAffiliation>);

/// The tier and model affiliation a new chat's bind would produce (DD-30), for a stream
/// whose target chat does not exist yet (Home, a new tab).
///
/// The provider is `pick` when the caller holds the user action proof (the route passes
/// `None` otherwise, so an unproven caller cannot choose the tier its own stream is
/// judged by), else `BIOROUTER_PROVIDER` with `BIOROUTER_MODEL`. A pick that names a
/// model is built with it; a pick of the configured provider with no model takes the
/// configured model; any other pick takes the provider's default model.
///
/// The provider is built exactly as `/agent/start` builds it (`providers::create`, which
/// takes the lead and worker path while `BIOROUTER_LEAD_MODEL` is set), and the answer is
/// the instance's own `tier()` and `affiliation()`, never a declared tier: `ollama`
/// declares Private and is Public for a cloud tag or a remote `OLLAMA_HOST`, `llamacpp`
/// likewise for a remote `LLAMACPP_EXTERNAL_HOST`, and a lead and worker pair is the
/// lesser of its two halves (`ProviderTier::least` inside the composite's `tier()`).
///
/// Answers are cached by the pick and the value of every key a tier input file reads,
/// `privacy::CAPABILITY_CONFIG_KEYS` and `privacy::NOT_CAPABILITY_CONFIG_KEYS` both,
/// because the second list holds tier inputs too (`VERSA_AZURE_ENDPOINT` moves
/// `versa_azure` off the UCSF gateway, which makes it Public with no affiliation); so a
/// configuration change is a new key. An answer is kept for [`NEW_CHAT_CACHE_TTL`], for
/// the inputs no configuration value carries. A failure to build answers Public with an
/// unstated affiliation and is not cached.
pub async fn new_chat_bind_tier(pick: Option<&str>, pick_model: Option<&str>) -> BindTier {
    let config = Config::global();
    let configured_provider = config.get_biorouter_provider().ok();
    let configured_model = config.get_biorouter_model().ok();
    let pick = pick.map(str::trim).filter(|p| !p.is_empty());
    let pick_model = pick_model.map(str::trim).filter(|m| !m.is_empty());
    let (provider, model) = match pick {
        Some(provider) => {
            let model = pick_model.map(str::to_string).or_else(|| {
                (configured_provider.as_deref() == Some(provider))
                    .then(|| configured_model.clone())
                    .flatten()
            });
            (Some(provider.to_string()), model)
        }
        None => (configured_provider, configured_model),
    };
    let Some(provider) = provider else {
        return (ProviderTier::Public, None);
    };
    let key = bind_cache_key(&provider, model.as_deref(), |name| {
        config.get_param::<String>(name).ok()
    });
    if let Some(answer) = cached(&bind_cache(), &key, Instant::now()) {
        return answer;
    }
    let build = async {
        match &model {
            Some(model) => crate::providers::create(&provider, ModelConfig::new(model)?).await,
            None => crate::providers::create_with_default_model(&provider).await,
        }
    };
    match tokio::time::timeout(NEW_CHAT_BUILD_BOUND, build).await {
        Ok(Ok(instance)) => {
            let answer = (instance.tier(), instance.affiliation());
            let mut cache = bind_cache();
            if cache.len() >= NEW_CHAT_CACHE_ENTRIES {
                cache.clear();
            }
            cache.insert(key, (Instant::now(), answer));
            answer
        }
        _ => (ProviderTier::Public, None),
    }
}

/// What the composer holds for a chat that does not exist yet beside its model pick: the
/// decision models its draft names, which its first message will start it with as
/// `/agent/start`'s `decision_settings` (DR-33).
#[derive(Clone, Copy, Debug)]
pub enum NewChatDraft<'a> {
    /// No draft: the chat starts with no decision models.
    None,
    /// Every decision model the draft names: its default and each monitor's, on or off.
    Models(&'a [DecisionModelRef]),
    /// A draft whose effect cannot be told: one that does not parse, or one sent without
    /// the user action proof. The new chat is then judged public.
    Unknown,
}

/// The most decision models a draft can name: its default and one per monitor.
const MAX_DRAFT_MODELS: usize = MAX_MONITORS + 1;

/// The id the draft chat is checked under. It names no stored chat; [`with_draft_decisions`]
/// reads and writes nothing.
const DRAFT_CHAT_ID: &str = "draft";

/// The tier and model affiliation a chat that does not exist yet would start at (DD-30,
/// DR-33): the bind [`new_chat_bind_tier`] would produce, with the draft's decision models
/// folded in by [`with_draft_decisions`]. An [`NewChatDraft::Unknown`] draft answers
/// Public without building anything.
///
/// `enforced` is the privacy master switch as the caller sampled it, the value
/// `/agent/start` passes to the same check.
pub async fn new_chat_target_tier(
    pick: Option<&str>,
    pick_model: Option<&str>,
    draft: NewChatDraft<'_>,
    enforced: bool,
) -> BindTier {
    if matches!(draft, NewChatDraft::Unknown) {
        return (ProviderTier::Public, None);
    }
    let bind = new_chat_bind_tier(pick, pick_model).await;
    with_draft_decisions(bind, draft, enforced)
}

/// `bind` (a new chat's model tier and affiliation) with a draft's decision models folded
/// in, as `POST /agent/start` folds the `decision_settings` it carries.
///
/// The start saves them through `decision::save::save_checked_by`, whose tier check is
/// [`tier_change`], on the chat it just created: classified Public, holding no message,
/// bound to the model it built. This asks that same check of the same chat before it
/// exists: an empty Public chat (`Session::default()`, as `GET /decisions/models` builds a
/// draft's), with `bind`'s tier as the bound model's and the person's lowering taken as
/// accepted, since a start whose lowering the person refuses creates no chat at all. The
/// check is asked once per model, each as the draft's default: a chat is as private as
/// its least private model, so the draft lowers the chat when any one of its models
/// would. A result the check calls `lowered_after_save` is a chat its decision models
/// made public, and the slot of the started chat then folds the same names
/// (`decision_axes::from_row` over the basis the save writes).
///
/// Every model the draft names is asked, a monitor that is off included: the fold leaves
/// such a monitor out, and counting it can only make the answer more public.
///
/// Doubt answers Public. That is the opposite of the fold's own rule (doubt never lowers a
/// chat) and points the same way, toward the safe side: a stream target that failed
/// toward private would let private terms shape text bound for a chat whose tier nobody
/// knows. So a draft that is [`NewChatDraft::Unknown`], one that
/// names more models than a draft can, a model whose provider declares nothing (an
/// unknown provider, an Ollama host that does not resolve), a model the start would
/// refuse as malformed, and a check that refuses, each answer Public with no affiliation.
/// Through a draft, the answer is only ever `bind` or more public than it.
pub fn with_draft_decisions(bind: BindTier, draft: NewChatDraft<'_>, enforced: bool) -> BindTier {
    const PUBLIC: BindTier = (ProviderTier::Public, None);
    let models = match draft {
        NewChatDraft::None => return bind,
        NewChatDraft::Unknown => return PUBLIC,
        NewChatDraft::Models(models) => models,
    };
    if models.len() > MAX_DRAFT_MODELS {
        return PUBLIC;
    }
    let chat = Session {
        id: DRAFT_CHAT_ID.to_string(),
        ..Session::default()
    };
    let checked = DecisionSession::new(DRAFT_CHAT_ID, chat.privacy_tier, enforced)
        .with_bound_model_tier(Some(bind.0).filter(|_| enforced));
    let door = SaveDoor::person("dictation", LoweringConsent::Confirmed);
    for model in models {
        if decision_axes::declared_name(model).is_none() {
            return PUBLIC;
        }
        let named = DecisionSettings {
            default_model: Some(model.clone()),
            monitors: Vec::new(),
        };
        if validate(&named).is_err() {
            return PUBLIC;
        }
        match tier_change(&checked, &chat, AfterSave::Settings(&named), &door) {
            Ok(change) if !change.lowered_after_save => {}
            _ => return PUBLIC,
        }
    }
    bind
}

type BindCache = HashMap<String, (Instant, BindTier)>;

fn bind_cache() -> std::sync::MutexGuard<'static, BindCache> {
    static CACHE: OnceLock<Mutex<BindCache>> = OnceLock::new();
    CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The answer kept for `key`, unless it is older than [`NEW_CHAT_CACHE_TTL`] at `now`.
fn cached(cache: &BindCache, key: &str, now: Instant) -> Option<BindTier> {
    cache
        .get(key)
        .filter(|(at, _)| now.saturating_duration_since(*at) < NEW_CHAT_CACHE_TTL)
        .map(|(_, answer)| *answer)
}

/// The cache key of one bind: the provider, its model, and the value of every key a tier
/// input file reads (both lists of `privacy::config_keys`, which its own test keeps
/// complete). NUL separates the parts, which no configuration value carries.
fn bind_cache_key(
    provider: &str,
    model: Option<&str>,
    read: impl Fn(&str) -> Option<String>,
) -> String {
    let mut key = format!("{provider}\0{}", model.unwrap_or("\u{1}"));
    let names = CAPABILITY_CONFIG_KEYS
        .iter()
        .copied()
        .chain(NOT_CAPABILITY_CONFIG_KEYS.iter().map(|(name, _why)| *name));
    for name in names {
        key.push('\0');
        key.push_str(name);
        key.push('=');
        key.push_str(read(name).as_deref().unwrap_or("\u{1}"));
    }
    key
}

/// Whether a chat is private, as far as its stored row and its live model tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatReach {
    Private,
    Public,
    /// The chat's row could not be read.
    Unknown,
}

/// `stored` is `None` when the chat's row could not be read. `live_provider` is the tier of
/// the provider a live agent has bound, which can be Private before Gate B ratchets the
/// row: without it, a correction made in a chat bound to a private model before its first
/// turn would be stamped public.
pub fn chat_reach(
    stored: Option<SessionClassification>,
    live_provider: Option<ProviderTier>,
) -> ChatReach {
    if stored == Some(SessionClassification::Private)
        || live_provider.is_some_and(|t| t.is_private())
    {
        ChatReach::Private
    } else if stored.is_none() {
        ChatReach::Unknown
    } else {
        ChatReach::Public
    }
}

impl ChatReach {
    /// As a stream's target: only a known private chat is private.
    pub fn target_private(self) -> bool {
        self == ChatReach::Private
    }

    /// As a stamp: anything not known public is private.
    pub fn stamp_private(self) -> bool {
        self != ChatReach::Public
    }
}

/// What DD-77 reads for a CLI transcription, each sampled once by the caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CliTerminal {
    /// The `--session` chat's stored tier, read offline; `None` with no `--session`, or
    /// when the chat's row could not be read.
    pub session: Option<SessionClassification>,
    /// `prompt_can_run()`: stdin and stderr are both terminals.
    pub interactive: bool,
    /// `caller_capability()` is Private: the model this terminal is configured for.
    pub caller_private: bool,
    /// `BIOROUTER_TERMINAL` is set: an agent's shell.
    pub agent_shell: bool,
}

/// DD-77 for `transcribe` and `dictate --session <id>`: private entries only when the chat
/// is private **and** the run is at an interactive terminal **and** the configured model is
/// private **and** `BIOROUTER_TERMINAL` is unset. An agent's shell is public only whatever
/// the flags; without `--session` the transcription is public only.
///
/// The target's model affiliation is unstated: the chat's tier is read offline, with no
/// live agent to read a model from, so a private row that names an institution is left out
/// and only rows with no institution can reach it.
///
/// This is where a terminal's private standing is decided, the CLI half of the two places
/// that build a private [`Standing`].
pub fn cli_bias(terminal: CliTerminal) -> (BiasTarget, Standing) {
    let private = terminal.session == Some(SessionClassification::Private)
        && terminal.interactive
        && terminal.caller_private
        && !terminal.agent_shell;
    (
        BiasTarget {
            private,
            affiliation: None,
            person_included: false,
        },
        Standing::from_checked(private),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_reach_fails_toward_the_safe_side_of_each_use() {
        use ProviderTier as T;
        use SessionClassification as C;
        assert_eq!(chat_reach(Some(C::Private), None), ChatReach::Private);
        assert_eq!(
            chat_reach(Some(C::Public), Some(T::Private)),
            ChatReach::Private
        );
        assert_eq!(
            chat_reach(Some(C::Public), Some(T::Public)),
            ChatReach::Public
        );
        assert_eq!(chat_reach(None, None), ChatReach::Unknown);
        assert_eq!(chat_reach(None, Some(T::Private)), ChatReach::Private);
        assert!(!ChatReach::Unknown.target_private());
        assert!(ChatReach::Unknown.stamp_private());
        assert!(ChatReach::Private.target_private() && ChatReach::Private.stamp_private());
        assert!(!ChatReach::Public.target_private() && !ChatReach::Public.stamp_private());
    }

    /// DD-77: every condition is needed, and an agent's shell is public whatever the
    /// flags, as is a run without `--session`.
    #[test]
    fn the_cli_target_rule() {
        let all = CliTerminal {
            session: Some(SessionClassification::Private),
            interactive: true,
            caller_private: true,
            agent_shell: false,
        };
        let (target, standing) = cli_bias(all);
        assert!(target.private && standing.is_private());
        assert_eq!(
            target.affiliation, None,
            "a chat read offline states no model"
        );
        for broken in [
            CliTerminal {
                session: None,
                ..all
            },
            CliTerminal {
                session: Some(SessionClassification::Public),
                ..all
            },
            CliTerminal {
                interactive: false,
                ..all
            },
            CliTerminal {
                caller_private: false,
                ..all
            },
            CliTerminal {
                agent_shell: true,
                ..all
            },
        ] {
            let (target, standing) = cli_bias(broken);
            assert!(!target.private && !standing.is_private(), "{broken:?}");
        }
    }

    fn overrides(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// DD-30: a new chat is judged by the tier its bind would produce, never by a
    /// declared tier. `ollama` declares Private; a remote `OLLAMA_HOST` or a cloud tag is
    /// Public, and a private worker under a public lead binds the lesser of the two.
    /// Building these providers opens no connection.
    #[tokio::test]
    async fn a_new_chat_is_judged_by_the_tier_its_bind_would_produce() {
        use crate::config::with_config_overrides;

        let local = overrides(&[
            ("BIOROUTER_PROVIDER", "ollama"),
            ("BIOROUTER_MODEL", "qwen3.8:27b"),
            ("OLLAMA_HOST", "127.0.0.1"),
        ]);
        let (tier, affiliation) =
            with_config_overrides(local.clone(), new_chat_bind_tier(None, None)).await;
        assert_eq!(tier, ProviderTier::Private);
        assert_eq!(affiliation, Some(ModelAffiliation::Local));

        let remote = overrides(&[
            ("BIOROUTER_PROVIDER", "ollama"),
            ("BIOROUTER_MODEL", "qwen3.8:27b"),
            ("OLLAMA_HOST", "https://ollama.example"),
        ]);
        let (tier, affiliation) =
            with_config_overrides(remote, new_chat_bind_tier(None, None)).await;
        assert_eq!(tier, ProviderTier::Public, "a remote OLLAMA_HOST");
        assert_eq!(affiliation, None);

        let cloud = overrides(&[
            ("BIOROUTER_PROVIDER", "ollama"),
            ("BIOROUTER_MODEL", "gpt-oss:120b-cloud"),
            ("OLLAMA_HOST", "127.0.0.1"),
        ]);
        let (tier, _) = with_config_overrides(cloud, new_chat_bind_tier(None, None)).await;
        assert_eq!(
            tier,
            ProviderTier::Public,
            "a cloud tag through a loopback host"
        );

        let pair = overrides(&[
            ("BIOROUTER_PROVIDER", "llamacpp"),
            ("BIOROUTER_MODEL", "qwen3.8-27b"),
            ("BIOROUTER_LEAD_PROVIDER", "ollama"),
            ("BIOROUTER_LEAD_MODEL", "qwen3.8:27b"),
            ("OLLAMA_HOST", "https://ollama.example"),
        ]);
        let (tier, _) = with_config_overrides(pair, new_chat_bind_tier(None, None)).await;
        assert_eq!(
            tier,
            ProviderTier::Public,
            "a private worker under a public lead binds the lesser"
        );

        // A pick replaces the configured provider; a pick of a provider this install
        // does not know answers Public.
        let (tier, _) = with_config_overrides(
            local.clone(),
            new_chat_bind_tier(Some("ollama"), Some("gpt-oss:120b-cloud")),
        )
        .await;
        assert_eq!(tier, ProviderTier::Public);
        let (tier, affiliation) =
            with_config_overrides(local, new_chat_bind_tier(Some("no-such-provider"), None)).await;
        assert_eq!((tier, affiliation), (ProviderTier::Public, None));
        // Nothing configured and no pick: Public.
        let (tier, _) = with_config_overrides(HashMap::new(), new_chat_bind_tier(None, None)).await;
        assert_eq!(tier, ProviderTier::Public);
    }

    fn decision_model(provider: &str, model: &str) -> DecisionModelRef {
        DecisionModelRef::new(provider, model)
    }

    /// [`with_draft_decisions`] with `OLLAMA_HOST` on this computer, so an
    /// `ollama_decision` model declares Private whatever the test machine configures.
    async fn fold_on_loopback(bind: BindTier, draft: NewChatDraft<'_>, enforced: bool) -> BindTier {
        let loopback = overrides(&[("OLLAMA_HOST", "127.0.0.1")]);
        crate::config::with_config_overrides(loopback, async move {
            with_draft_decisions(bind, draft, enforced)
        })
        .await
    }

    /// DR-33 for a chat that does not exist yet: the draft's decision models fold in as
    /// `/agent/start` folds them. A Versa bind with a Baseten model in the draft is a
    /// public chat; with a model on this computer, or with no draft, it keeps its tier.
    /// A draft can only make the answer more public.
    #[tokio::test]
    async fn a_draft_decision_model_folds_in_as_the_start_folds_it() {
        use crate::privacy::affiliation::InstitutionId;

        let versa = (
            ProviderTier::Private,
            Some(ModelAffiliation::institution(InstitutionId::new("ucsf"))),
        );
        let public = (ProviderTier::Public, None);
        let joey = [decision_model("baseten_decision", "joey-1")];
        let local = [decision_model("ollama_decision", "tev1:0.8b")];
        let both = [local[0].clone(), joey[0].clone()];
        let fold = fold_on_loopback;

        assert_eq!(fold(versa, NewChatDraft::None, true).await, versa);
        assert_eq!(
            fold(versa, NewChatDraft::Models(&[]), true).await,
            versa,
            "an empty draft names nothing"
        );
        assert_eq!(
            fold(versa, NewChatDraft::Models(&joey), true).await,
            public,
            "Versa plus a Baseten decision model is a public chat"
        );
        assert_eq!(
            fold(versa, NewChatDraft::Models(&local), true).await,
            versa,
            "Versa plus a decision model on this computer stays private"
        );
        assert_eq!(
            fold(versa, NewChatDraft::Models(&both), true).await,
            public,
            "the least private model sets the chat"
        );
        // A public model's chat is public whatever it names.
        assert_eq!(
            fold(public, NewChatDraft::Models(&local), true).await,
            public
        );
        assert_eq!(
            fold(public, NewChatDraft::Models(&joey), true).await,
            public
        );
        // With the master switch off the start lowers nothing, and neither does this.
        assert_eq!(fold(versa, NewChatDraft::Models(&joey), false).await, versa);
    }

    /// Doubt answers Public, the safe side of a stream target: a draft that cannot be
    /// told, a model whose provider declares nothing, a model the start would refuse as
    /// malformed, and more models than a draft can name.
    #[tokio::test]
    async fn a_draft_whose_effect_cannot_be_told_is_public() {
        let versa = (ProviderTier::Private, None);
        let public = (ProviderTier::Public, None);
        let unknown = [decision_model("no_such_decision_provider", "m")];
        let blank = [decision_model("baseten_decision", " ")];
        let many: Vec<DecisionModelRef> = (0..=MAX_MONITORS + 1)
            .map(|i| decision_model("ollama_decision", &format!("tev{i}:0.8b")))
            .collect();
        let fold = |draft| fold_on_loopback(versa, draft, true);
        assert_eq!(fold(NewChatDraft::Unknown).await, public);
        assert_eq!(fold(NewChatDraft::Models(&unknown)).await, public);
        assert_eq!(fold(NewChatDraft::Models(&blank)).await, public);
        assert_eq!(fold(NewChatDraft::Models(&many)).await, public);
        assert_eq!(
            fold(NewChatDraft::Models(&many[..MAX_DRAFT_MODELS])).await,
            versa,
            "a full draft of local models stays private"
        );
    }

    /// The whole new chat target: the bind `/agent/start` would build, then its draft. A
    /// configured Versa chat is private alone and public once its draft names joey-1, and
    /// an unknown draft builds nothing. Building these providers opens no connection.
    #[tokio::test]
    async fn a_new_chat_whose_draft_names_a_public_decision_model_is_public() {
        use crate::config::with_config_overrides;
        use crate::providers::versa_azure::VERSA_AZURE_DEFAULT_MODEL;

        let versa = overrides(&[
            ("BIOROUTER_PROVIDER", "versa_azure"),
            ("BIOROUTER_MODEL", VERSA_AZURE_DEFAULT_MODEL),
            ("VERSA_AZURE_API_KEY", "placeholder-never-sent"),
            ("OLLAMA_HOST", "127.0.0.1"),
        ]);
        let joey = [decision_model("baseten_decision", "joey-1")];
        for (draft, expected) in [
            (NewChatDraft::None, ProviderTier::Private),
            (NewChatDraft::Models(&joey), ProviderTier::Public),
            (NewChatDraft::Unknown, ProviderTier::Public),
        ] {
            let (tier, _) =
                with_config_overrides(versa.clone(), new_chat_target_tier(None, None, draft, true))
                    .await;
            assert_eq!(tier, expected, "{draft:?}");
        }
    }

    /// The cache key moves with every key a tier input file reads, both lists of
    /// `config_keys`, so a configuration change is never answered from an older bind.
    #[test]
    fn the_bind_cache_key_follows_every_tier_input_key() {
        let base = |_: &str| None;
        let reference = bind_cache_key("ollama", Some("m"), base);
        let names = CAPABILITY_CONFIG_KEYS
            .iter()
            .copied()
            .chain(NOT_CAPABILITY_CONFIG_KEYS.iter().map(|(name, _)| *name));
        for name in names {
            let moved = bind_cache_key("ollama", Some("m"), |key: &str| {
                (key == name).then(|| "x".to_string())
            });
            assert_ne!(moved, reference, "{name}");
        }
        assert_ne!(bind_cache_key("ollama", None, base), reference);
        assert_ne!(bind_cache_key("llamacpp", Some("m"), base), reference);
    }

    /// An answer is kept for [`NEW_CHAT_CACHE_TTL`] and rebuilt after it, for the tier
    /// inputs no configuration value carries (what `/api/show` told `ollama`).
    #[test]
    fn a_kept_answer_lasts_its_lifetime_only() {
        let now = Instant::now();
        let mut cache = BindCache::new();
        let answer = (ProviderTier::Private, Some(ModelAffiliation::Local));
        cache.insert("k".into(), (now, answer));
        assert_eq!(cached(&cache, "k", now), Some(answer));
        assert_eq!(
            cached(
                &cache,
                "k",
                now + NEW_CHAT_CACHE_TTL - Duration::from_millis(1)
            ),
            Some(answer)
        );
        assert_eq!(cached(&cache, "k", now + NEW_CHAT_CACHE_TTL), None);
        assert_eq!(cached(&cache, "other", now), None);
    }

    /// `VERSA_AZURE_ENDPOINT` is a tier input outside `CAPABILITY_CONFIG_KEYS`: once
    /// `versa_azure` on its UCSF gateway was cached as Private with UCSF's affiliation,
    /// an endpoint moved off site is answered Public with none, not from the cache.
    #[tokio::test]
    async fn a_moved_versa_endpoint_is_not_answered_from_the_cache() {
        use crate::config::with_config_overrides;
        use crate::privacy::affiliation::InstitutionId;
        use crate::providers::versa_azure::VERSA_AZURE_DEFAULT_MODEL;

        let mut versa = overrides(&[
            ("BIOROUTER_PROVIDER", "versa_azure"),
            ("BIOROUTER_MODEL", VERSA_AZURE_DEFAULT_MODEL),
            ("VERSA_AZURE_API_KEY", "placeholder-never-sent"),
        ]);
        let (tier, affiliation) =
            with_config_overrides(versa.clone(), new_chat_bind_tier(None, None)).await;
        assert_eq!(tier, ProviderTier::Private, "the UCSF gateway");
        assert_eq!(
            affiliation,
            Some(ModelAffiliation::institution(InstitutionId::new("ucsf")))
        );
        versa.insert(
            "VERSA_AZURE_ENDPOINT".to_string(),
            "https://elsewhere.example/general".to_string(),
        );
        let (tier, affiliation) =
            with_config_overrides(versa, new_chat_bind_tier(None, None)).await;
        assert_eq!(tier, ProviderTier::Public, "an endpoint off the gateway");
        assert_eq!(affiliation, None);
    }
}
