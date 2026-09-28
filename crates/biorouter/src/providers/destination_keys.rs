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
//! The classification is checked, not remembered: a test scans every
//! `get_param` a file under `providers/` makes and fails until a new key is
//! placed in exactly one of the two lists below.

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

/// Every other key a provider reads through `get_param`, with the reason it
/// does not move where a request goes.
pub const NOT_DESTINATION_CONFIG_KEYS: &[(&str, &str)] = &[
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
    DESTINATION_CONFIG_KEYS
        .iter()
        .any(|(name, _why)| name.eq_ignore_ascii_case(key))
}

#[cfg(test)]
mod tests {
    use super::*;
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
                    out.push((path.display().to_string(), source));
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
            DESTINATION_CONFIG_KEYS.iter().any(|(k, _)| *k == key),
            NOT_DESTINATION_CONFIG_KEYS.iter().any(|(k, _)| *k == key),
        )
    }

    /// Every key a provider reads through `get_param` is in exactly one list,
    /// and every listed key is still read. Adding a host setting to a provider
    /// fails this test until someone decides whether it moves where requests go.
    #[test]
    fn every_key_a_provider_reads_is_classified() {
        let literal = regex::Regex::new(r#"\.get_param(?:::<[^>]*>)?\s*\(\s*"([^"]+)""#).unwrap();
        // Everything after the opening parenthesis up to the end of the line,
        // compared by prefix: a key expression can itself contain a call.
        let computed =
            regex::Regex::new(r#"\.get_param(?:::<[^>]*>)?\s*\(\s*([^"\s)].*)"#).unwrap();
        // The three reads whose key is a constant rather than a literal, and
        // the keys those constants hold.
        let known_computed = [
            "kind.command_config_key()",
            "CHILD_TOOL_CALL_TIMEOUT_CONFIG_KEY",
            "TURN_TIMEOUT_CONFIG_KEY",
        ];
        let mut read: BTreeSet<String> = [
            "CLAUDE_CODE_COMMAND",
            "CODEX_COMMAND",
            "BIOROUTER_CODING_AGENT_TOOL_TIMEOUT_SECS",
            "BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        for (path, source) in provider_sources() {
            if path.ends_with("destination_keys.rs") {
                continue;
            }
            read.extend(
                literal
                    .captures_iter(&source)
                    .map(|caps| caps[1].to_string()),
            );
            for caps in computed.captures_iter(&source) {
                let expr = caps[1].trim();
                assert!(
                    known_computed
                        .iter()
                        .any(|known| expr.starts_with(&format!("{known})"))),
                    "{path} reads a config key the scan cannot see: get_param({expr}). \
                     Name it with a literal, or add it here and classify its value."
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

    /// Every non-secret setting a built-in provider declares is classified as
    /// well, including one it reads by some other route than `get_param`.
    #[test]
    fn every_declared_setting_is_classified() {
        for metadata in crate::providers::builtin_provider_metadata() {
            for key in metadata.config_keys.iter().filter(|key| !key.secret) {
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
        assert!(!is_destination_key("OPENAI_API_KEY"));
        assert!(!is_destination_key("OPENAI_TIMEOUT"));
        assert!(!is_destination_key("BIOROUTER_MODEL"));
    }
}
