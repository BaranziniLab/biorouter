//! Which config keys decide where a provider sends its requests, and so where
//! the credentials those requests carry end up.
//!
//! A provider reads its saved key (or this computer's own cloud sign-in) and
//! sends it to the host its configuration names. Every chat on that provider,
//! a model listing and a live credential check all do this, so whoever can
//! change the host decides who receives the key. Over HTTP that has to be the
//! user: `/config/upsert` and `/config/remove` ask for the user-action proof
//! before they change one of these keys (`routes/config_management.rs` in
//! `biorouter-server`), the same proof the capability keys in
//! [`crate::privacy::CAPABILITY_CONFIG_KEYS`] need.
//!
//! The classification is checked, not remembered. A test scans every
//! `get_param`, `get_secret`, `get_secrets` and `get` a file under
//! `providers/` makes, and every `AWS_*` key [`super::aws_stored_settings`]
//! hands the AWS SDK, and fails until a new key is placed in exactly one of the
//! two lists below.
//! Amazon Bedrock and SageMaker read their stored settings as one bulk read of
//! every `AWS_*` key rather than through `get_param`, and a scan of `get_param`
//! alone is how `AWS_ENDPOINT_URL_BEDROCK_RUNTIME` stayed ungated after the
//! first version of this list (W2-PRV-2, round 4). The same test now refuses a
//! bulk read anywhere else under `providers/`.

/// Keys whose value decides where a provider's requests go, which server they
/// trust, which client identity they present, or which program receives them.
/// Changing one of these over HTTP needs the user-action proof.
pub const DESTINATION_CONFIG_KEYS: &[(&str, &str)] = &[
    (
        "ANTHROPIC_HOST",
        "the host requests and the saved key go to",
    ),
    (
        "AZURE_OPENAI_ENDPOINT",
        "the host requests and the key or Entra sign-in go to",
    ),
    (
        "DATABRICKS_HOST",
        "the host requests and the token or browser sign-in go to",
    ),
    ("GOOGLE_HOST", "the host requests and the saved key go to"),
    ("LITELLM_HOST", "the host requests and the saved key go to"),
    ("OPENAI_HOST", "the host requests and the saved key go to"),
    (
        "OPENROUTER_HOST",
        "the host requests and the saved key go to",
    ),
    (
        "SNOWFLAKE_HOST",
        "the host requests and the saved token go to",
    ),
    ("TETRATE_HOST", "the host requests and the saved key go to"),
    ("VENICE_HOST", "the host requests and the saved key go to"),
    (
        "VERSA_AZURE_ENDPOINT",
        "the host requests and the saved key go to",
    ),
    (
        "VERSA_BEDROCK_ENDPOINT",
        "the host requests and the saved keys go to",
    ),
    ("XAI_HOST", "the host requests and the saved key go to"),
    (
        "XIAOMI_MIMO_HOST",
        "the host requests and the saved key go to",
    ),
    ("ZAI_HOST", "the host requests and the saved key go to"),
    // A path is joined onto the host, and a join with an absolute URL replaces
    // the host. Classified with the hosts rather than trusting every join site.
    (
        "OPENAI_BASE_PATH",
        "joined onto the host; an absolute URL replaces it",
    ),
    (
        "LITELLM_BASE_PATH",
        "joined onto the host; an absolute URL replaces it",
    ),
    (
        "VENICE_BASE_PATH",
        "joined onto the host; an absolute URL replaces it",
    ),
    (
        "VENICE_MODELS_PATH",
        "joined onto the host; an absolute URL replaces it",
    ),
    // Vertex AI builds its host from the location (`{location}-aiplatform…`).
    (
        "GCP_LOCATION",
        "becomes part of the host the Google sign-in goes to",
    ),
    (
        "AWS_REGION",
        "selects the regional host the AWS sign-in goes to",
    ),
    // Handed to the AWS SDK as its endpoint by `aws_stored_settings`, from
    // `config.yaml` or the secret store. Every Bedrock or SageMaker request goes
    // there, signed with the AWS credentials or carrying a stored Bedrock API
    // key as a bearer token. Any other `AWS_ENDPOINT_URL_*` key is matched by
    // `DESTINATION_KEY_PREFIXES`.
    (
        "AWS_ENDPOINT_URL",
        "the host every AWS request and its signature or bearer token go to",
    ),
    (
        "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
        "the host Bedrock requests and their signature or bearer token go to",
    ),
    (
        "AWS_ENDPOINT_URL_SAGEMAKER_RUNTIME",
        "the host SageMaker requests and their signature go to",
    ),
    (
        "AWS_PROFILE",
        "selects which AWS credentials sign requests, and their settings",
    ),
    (
        "BIOROUTER_CA_CERT_PATH",
        "decides which servers a request trusts",
    ),
    // A coding-agent CLI keeps its own sign-in, but the daemon hands the program
    // it runs the chat's tool-bridge capability URL, which lets its holder
    // call the chat's tools.
    (
        "CLAUDE_CODE_COMMAND",
        "the program that receives every request and the tool-bridge capability",
    ),
    (
        "CODEX_COMMAND",
        "the program that receives every request and the tool-bridge capability",
    ),
    (
        "BIOROUTER_CLIENT_CERT_PATH",
        "the client certificate every request presents",
    ),
    (
        "BIOROUTER_CLIENT_KEY_PATH",
        "the client key every request presents",
    ),
];

/// Name prefixes that make a key a destination key whatever follows. The AWS
/// SDK honours one endpoint override per service (`AWS_ENDPOINT_URL_<SERVICE>`),
/// so a prefix covers a service BioRouter starts to call later without a row
/// being remembered here.
pub const DESTINATION_KEY_PREFIXES: &[(&str, &str)] = &[(
    "AWS_ENDPOINT_URL_",
    "an AWS service's own endpoint override",
)];

/// Every other key a provider reads, with the reason it does not move where a
/// request goes.
pub const NOT_DESTINATION_CONFIG_KEYS: &[(&str, &str)] = &[
    // Credentials. What a request carries, not where it goes; a destination key
    // above decides that.
    ("ANTHROPIC_API_KEY", "a credential, not where it goes"),
    ("AZURE_OPENAI_API_KEY", "a credential, not where it goes"),
    ("DATABRICKS_TOKEN", "a credential, not where it goes"),
    ("GITHUB_COPILOT_TOKEN", "a credential, not where it goes"),
    ("GOOGLE_API_KEY", "a credential, not where it goes"),
    ("LITELLM_API_KEY", "a credential, not where it goes"),
    ("OPENAI_API_KEY", "a credential, not where it goes"),
    ("OPENROUTER_API_KEY", "a credential, not where it goes"),
    ("TETRATE_API_KEY", "a credential, not where it goes"),
    ("VENICE_API_KEY", "a credential, not where it goes"),
    ("VERSA_AZURE_API_KEY", "a credential, not where it goes"),
    (
        "VERSA_BEDROCK_ACCESS_KEY_ID",
        "a credential, not where it goes",
    ),
    (
        "VERSA_BEDROCK_SECRET_ACCESS_KEY",
        "a credential, not where it goes",
    ),
    ("XAI_API_KEY", "a credential, not where it goes"),
    ("XIAOMI_MIMO_API_KEY", "a credential, not where it goes"),
    ("ZAI_API_KEY", "a credential, not where it goes"),
    ("AWS_ACCESS_KEY_ID", "a credential, not where it goes"),
    ("AWS_SECRET_ACCESS_KEY", "a credential, not where it goes"),
    ("AWS_SESSION_TOKEN", "a credential, not where it goes"),
    (
        "AWS_BEARER_TOKEN_BEDROCK",
        "a credential, not where it goes",
    ),
    (
        "LITELLM_CUSTOM_HEADERS",
        "headers sent to the same host; they can carry a token",
    ),
    (
        "OPENAI_CUSTOM_HEADERS",
        "headers sent to the same host; they can carry a token",
    ),
    // Ollama and the bundled llama.cpp server send no credential, so moving
    // them moves conversations, not a key. That is a privacy tier question,
    // and these hosts are already capability keys (`CAPABILITY_CONFIG_KEYS`),
    // gated with the privacy master switch on purpose (DR-15). Gating them here
    // too would override that switch.
    (
        "OLLAMA_HOST",
        "sends no credential; a privacy capability key",
    ),
    (
        "LLAMACPP_EXTERNAL_HOST",
        "sends no credential; a privacy capability key",
    ),
    (
        "LLAMACPP_PORT",
        "the bundled server's loopback port; it sends no credential",
    ),
    (
        "BIOROUTER_LLAMACPP_BIN",
        "the bundled server's binary; it sends no credential",
    ),
    (
        "AZURE_OPENAI_API_VERSION",
        "a query parameter after the host",
    ),
    (
        "AZURE_OPENAI_DEPLOYMENT_NAME",
        "a path segment after the host",
    ),
    (
        "VERSA_AZURE_API_VERSION",
        "a query parameter after the host",
    ),
    (
        "VERSA_AZURE_DEPLOYMENT_NAME",
        "a path segment after the host",
    ),
    (
        "VERSA_BEDROCK_REGION",
        "SigV4 signing region; the endpoint decides the host",
    ),
    (
        "GCP_PROJECT_ID",
        "a path segment after the host the location decides",
    ),
    (
        "SAGEMAKER_ENDPOINT_NAME",
        "a resource in the user's own AWS account",
    ),
    ("OPENAI_ORGANIZATION", "a header sent to the same host"),
    ("OPENAI_PROJECT", "a header sent to the same host"),
    (
        "SNOWFLAKE_TOKEN",
        "the credential itself, not where it goes",
    ),
    (
        "BIOROUTER_LEAD_MODEL",
        "which model runs; a privacy capability key",
    ),
    (
        "BIOROUTER_LEAD_PROVIDER",
        "which configured provider runs; a capability key",
    ),
    ("BIOROUTER_CONTEXT_LIMIT", "token budget"),
    ("BIOROUTER_WORKER_CONTEXT_LIMIT", "token budget"),
    ("BIOROUTER_LEAD_TURNS", "handoff policy"),
    ("BIOROUTER_LEAD_FAILURE_THRESHOLD", "handoff policy"),
    ("BIOROUTER_LEAD_FALLBACK_TURNS", "handoff policy"),
    (
        "BIOROUTER_CODING_AGENT_TOOL_TIMEOUT_SECS",
        "transport timeout",
    ),
    (
        "BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS",
        "transport timeout",
    ),
    ("BEDROCK_MAX_RETRIES", "retry policy"),
    ("BEDROCK_INITIAL_RETRY_INTERVAL_MS", "retry policy"),
    ("BEDROCK_BACKOFF_MULTIPLIER", "retry policy"),
    ("BEDROCK_MAX_RETRY_INTERVAL_MS", "retry policy"),
    ("BEDROCK_OPERATION_TIMEOUT_SECS", "transport timeout"),
    ("DATABRICKS_MAX_RETRIES", "retry policy"),
    ("DATABRICKS_INITIAL_RETRY_INTERVAL_MS", "retry policy"),
    ("DATABRICKS_BACKOFF_MULTIPLIER", "retry policy"),
    ("DATABRICKS_MAX_RETRY_INTERVAL_MS", "retry policy"),
    ("GCP_MAX_RETRIES", "retry policy"),
    ("GCP_INITIAL_RETRY_INTERVAL_MS", "retry policy"),
    ("GCP_BACKOFF_MULTIPLIER", "retry policy"),
    ("GCP_MAX_RETRY_INTERVAL_MS", "retry policy"),
    ("LITELLM_TIMEOUT", "transport timeout"),
    ("OLLAMA_TIMEOUT", "transport timeout"),
    ("OPENAI_TIMEOUT", "transport timeout"),
    ("LLAMACPP_TIMEOUT", "transport timeout"),
    ("LLAMACPP_STARTUP_TIMEOUT", "sidecar readiness deadline"),
    ("LLAMACPP_CONTEXT_SIZE", "token budget"),
    ("LLAMACPP_ENABLE_THINKING", "sampling option"),
    ("LLAMACPP_SPEC_TYPE", "decoding option"),
    (
        "LLAMACPP_EXTRA_ARGS",
        "arguments to the bundled server, which keeps its port",
    ),
];

/// Changing this key over HTTP needs the user-action proof. Compared the way
/// `Config::get_param` looks a key up in the environment (upper case), so a
/// lower-case spelling is refused too rather than slipping past.
pub fn is_destination_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    DESTINATION_CONFIG_KEYS
        .iter()
        .any(|(name, _why)| *name == upper)
        || DESTINATION_KEY_PREFIXES
            .iter()
            .any(|(prefix, _why)| upper.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::aws_stored_settings::{AwsService, CREDENTIAL_KEYS, ROUTING_KEYS};
    use std::collections::BTreeSet;
    use std::path::Path;

    fn provider_sources() -> Vec<(String, String)> {
        fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
            for entry in std::fs::read_dir(dir).expect("providers/ is readable") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let source = std::fs::read_to_string(&path).expect("a provider source file");
                    // Forward slashes on every platform, so a suffix below that
                    // names a subdirectory matches on Windows too.
                    out.push((path.display().to_string().replace('\\', "/"), source));
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/providers");
        let mut out = Vec::new();
        walk(&root, &mut out);
        assert!(out.len() > 40, "the scan found too few provider files");
        out
    }

    fn classified(key: &str) -> (bool, bool) {
        (
            DESTINATION_CONFIG_KEYS.iter().any(|(k, _)| *k == key)
                || DESTINATION_KEY_PREFIXES
                    .iter()
                    .any(|(prefix, _)| key.starts_with(prefix)),
            NOT_DESTINATION_CONFIG_KEYS.iter().any(|(k, _)| *k == key),
        )
    }

    /// The `AWS_*` keys `aws_stored_settings` hands the SDK, which no
    /// `get_param` names: the routing and endpoint keys, then the credentials.
    fn stored_aws_keys() -> (Vec<&'static str>, Vec<&'static str>) {
        let routing = ROUTING_KEYS
            .into_iter()
            .chain(
                AwsService::ALL
                    .into_iter()
                    .flat_map(AwsService::endpoint_keys),
            )
            .collect();
        (routing, CREDENTIAL_KEYS.to_vec())
    }

    /// Every key a provider reads through `get_param`, `get_secret` or
    /// `get_secrets`, or hands the AWS SDK from the stores, is in exactly one
    /// list, and every listed key is still read. Adding a host setting to a
    /// provider fails this test until someone decides whether it moves where
    /// requests go.
    #[test]
    fn every_key_a_provider_reads_is_classified() {
        let read_call = r#"\.get_(?:param|secrets?)(?:::<[^>]*>)?\s*\(\s*"#;
        let literal = regex::Regex::new(&format!(r#"{read_call}"([^"]+)""#)).unwrap();
        // `Config::get(key, is_secret)`, on a receiver named the way every
        // provider names its `Config`. A `serde_json` or `HashMap` `get` takes
        // no second argument, but a receiver name is what tells them apart.
        let plain_get =
            regex::Regex::new(r#"(?:\bconfig|Config::global\(\))\s*\.get\s*\(\s*"([^"]+)"\s*,"#)
                .unwrap();
        // The extra keys `get_secrets` reads beside its first.
        let extra =
            regex::Regex::new(r#"\.get_secrets\s*\(\s*"[^"]+"\s*,\s*&\[([^\]]*)\]"#).unwrap();
        let quoted = regex::Regex::new(r#""([^"]+)""#).unwrap();
        // Everything after the opening parenthesis up to the end of the line,
        // compared by prefix: a key expression can itself contain a call.
        let computed = regex::Regex::new(&format!(r#"{read_call}([^"\s)].*)"#)).unwrap();
        // A read of every key a store holds, which no pattern above sees.
        let bulk = regex::Regex::new(r"\.all_(?:values|secrets)\s*\(").unwrap();
        // The reads whose key is an expression rather than a literal, each in
        // the one file it may appear in. Their keys are seeded into `read`
        // below, or collected by `closure_arg`.
        let known_computed = [
            ("coding_agent/discovery.rs", "kind.command_config_key()"),
            (
                "coding_agent/bridge.rs",
                "CHILD_TOOL_CALL_TIMEOUT_CONFIG_KEY",
            ),
            ("coding_agent/mod.rs", "TURN_TIMEOUT_CONFIG_KEY"),
            // Versa Bedrock's `read_key` closure; its calls are collected below.
            ("versa_bedrock.rs", "name"),
            // A custom provider's own key. Where it goes is the provider's
            // URL, which `PUT /config/custom-providers/{id}` gates.
            ("anthropic.rs", "&config.api_key_env"),
            ("openai.rs", "&config.api_key_env"),
        ];
        let closure_arg = regex::Regex::new(r#"\bread_key\(\s*"([^"]+)"\s*\)"#).unwrap();
        let (aws_routing, aws_credentials) = stored_aws_keys();
        let mut read: BTreeSet<String> = [
            "CLAUDE_CODE_COMMAND",
            "CODEX_COMMAND",
            "BIOROUTER_CODING_AGENT_TOOL_TIMEOUT_SECS",
            "BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS",
        ]
        .into_iter()
        .chain(aws_routing.iter().copied())
        .chain(aws_credentials.iter().copied())
        .map(str::to_string)
        .collect();
        for (path, source) in provider_sources() {
            if path.ends_with("destination_keys.rs") {
                continue;
            }
            let code: String = source
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            read.extend(
                literal
                    .captures_iter(&code)
                    .chain(plain_get.captures_iter(&code))
                    .map(|caps| caps[1].to_string()),
            );
            for caps in extra.captures_iter(&code) {
                read.extend(quoted.captures_iter(&caps[1]).map(|key| key[1].to_string()));
            }
            for caps in computed.captures_iter(&code) {
                let expr = caps[1].trim();
                assert!(
                    known_computed
                        .iter()
                        .any(|(file, known)| path.ends_with(file)
                            && expr.starts_with(&format!("{known})"))),
                    "{path} reads a config key the scan cannot see: {expr}. Name it with a \
                     literal, or add it here and classify its value."
                );
            }
            if path.ends_with("versa_bedrock.rs") {
                read.extend(
                    closure_arg
                        .captures_iter(&code)
                        .map(|caps| caps[1].to_string()),
                );
            }
            if !path.ends_with("aws_stored_settings.rs") {
                assert!(
                    !bulk.is_match(&code),
                    "{path} reads every key a store holds, so this scan cannot see which it \
                     uses. Read them by name, or through `aws_stored_settings`, whose keys are \
                     listed."
                );
            }
        }
        for key in &read {
            let (destination, not) = classified(key);
            assert!(
                destination ^ not,
                "{key} is in neither list, or in both; decide whether it moves where a \
                 provider's requests go"
            );
        }
        for (key, _why) in DESTINATION_CONFIG_KEYS
            .iter()
            .chain(NOT_DESTINATION_CONFIG_KEYS)
        {
            assert!(
                read.contains(*key),
                "{key} is classified but no provider reads it; delete its row"
            );
        }
    }

    /// What the stored AWS settings decide: the endpoint, the region and the
    /// profile move where requests go, the credentials do not. A Bedrock chat
    /// sends a stored bearer token to whatever endpoint the store names, so an
    /// unclassified endpoint key was a way to receive it (W2-PRV-2, round 4).
    #[test]
    fn the_stored_aws_endpoint_and_routing_keys_are_destination_keys() {
        let (routing, credentials) = stored_aws_keys();
        for key in routing {
            assert!(
                is_destination_key(key),
                "{key} decides where AWS requests go"
            );
        }
        for key in credentials {
            assert!(!is_destination_key(key), "{key} is a credential");
            assert!(classified(key).1, "{key} is not classified");
        }
        for service in AwsService::ALL {
            let [own, generic] = service.endpoint_keys();
            assert!(own.starts_with("AWS_ENDPOINT_URL_"), "{own}");
            assert_eq!(generic, "AWS_ENDPOINT_URL");
        }
    }

    /// Every setting a built-in provider declares is classified, including one
    /// it reads by some other route than the ones the scan knows.
    #[test]
    fn every_declared_setting_is_classified() {
        for metadata in crate::providers::builtin_provider_metadata() {
            for key in &metadata.config_keys {
                let (destination, not) = classified(&key.name);
                assert!(
                    destination ^ not,
                    "{}'s {} is in neither list, or in both",
                    metadata.name,
                    key.name
                );
            }
        }
    }

    #[test]
    fn a_destination_key_is_matched_in_any_case() {
        assert!(is_destination_key("OPENAI_HOST"));
        assert!(is_destination_key("openai_host"));
        assert!(is_destination_key("VERSA_AZURE_ENDPOINT"));
        assert!(is_destination_key("AWS_ENDPOINT_URL"));
        assert!(is_destination_key("aws_endpoint_url_bedrock_runtime"));
        // A service BioRouter does not call yet is covered by the prefix.
        assert!(is_destination_key("AWS_ENDPOINT_URL_STS"));
        assert!(!is_destination_key("AWS_ENDPOINT_URLS"));
        assert!(!is_destination_key("OPENAI_API_KEY"));
        assert!(!is_destination_key("OPENAI_TIMEOUT"));
        assert!(!is_destination_key("BIOROUTER_MODEL"));
        assert!(!is_destination_key("AWS_BEARER_TOKEN_BEDROCK"));
    }

    /// The desktop mirrors this list so Settings can say, in a browser served
    /// by `biorouter serve`, which settings are the host computer's before a
    /// Save meets the 409. A mirror drifts, so this compares the two.
    #[test]
    fn the_desktop_mirror_names_exactly_these_keys() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/desktop/src/components/settings/destinationConfigKeys.ts");
        let source = std::fs::read_to_string(&path).expect("the desktop mirror exists");
        let array = |name: &str| -> BTreeSet<String> {
            let (_, declared) = source
                .split_once(&format!("export const {name}"))
                .unwrap_or_else(|| panic!("the mirror declares {name}"));
            let (_, opened) = declared.split_once("= [").expect("an array literal");
            let (array, _) = opened.split_once("];").expect("the array's end");
            regex::Regex::new(r"'([^']+)'")
                .unwrap()
                .captures_iter(array)
                .map(|caps| caps[1].to_string())
                .collect()
        };
        let rust = |rows: &[(&str, &str)]| -> BTreeSet<String> {
            rows.iter().map(|(key, _)| (*key).to_string()).collect()
        };
        assert_eq!(
            array("DESTINATION_CONFIG_KEYS"),
            rust(DESTINATION_CONFIG_KEYS),
            "ui/desktop/src/components/settings/destinationConfigKeys.ts has drifted"
        );
        assert_eq!(
            array("DESTINATION_CONFIG_KEY_PREFIXES"),
            rust(DESTINATION_KEY_PREFIXES),
            "ui/desktop/src/components/settings/destinationConfigKeys.ts has drifted"
        );
    }
}
