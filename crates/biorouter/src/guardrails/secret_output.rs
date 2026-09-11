//! Credential redaction in tool *output* (H1, defence in depth).
//!
//! `biorouter_mcp::secret_guard::SecretGuard` refuses a tool call whose
//! arguments reach a protected file. It reads text, so a path a program
//! assembles while it runs — `python -c` joining the pieces, base64, a script
//! written a turn earlier — is beyond it, and QA-C's H1 measured how quickly a
//! model finds such a spelling. This module is the second line: it recognises
//! credential *material* in what a tool returns and replaces the value before
//! any model sees it, so an unforeseen spelling still cannot hand over the
//! bytes.
//!
//! # Where it runs
//!
//! Inside the result future of `ExtensionManager::dispatch_tool_call`, the one
//! point every tool result crosses on its way to a model: the agent loop, the
//! coding-agent tool bridge (Claude Code, Codex — whose results never pass the
//! agent loop's own output guardrail), `POST /agent/call_tool`, and code
//! execution's sub-calls. Error results are redacted too: `automation_script`
//! returns a failing script's stdout inside its error.
//!
//! It has no switch and does not read the privacy tier. The secret floor is not
//! a privacy-tier feature; it applies to every chat, and a private model has no
//! more business with the user's AWS key than a public one.
//!
//! # What it recognises
//!
//! A deliberately short list of formats that cannot be much else:
//!   * PEM / OpenSSH / PGP **private-key blocks** (the body is replaced, the
//!     BEGIN/END lines are kept so the reader knows what was there);
//!   * **AWS** access key ids (`AKIA…`, `ASIA…`), a 40-character secret next to
//!     one or under an `aws_secret_access_key`-style name, and session tokens;
//!   * the **provider-key store's own key names** (`OPENAI_API_KEY: …`,
//!     `"VERSA_AZURE_API_KEY":"…"`) and names that say they hold a secret
//!     (`*_API_KEY`, `*_TOKEN`, `*_SECRET`, `*_PASSWORD`, …), in the YAML,
//!     env and JSON shapes `secrets.yaml`, the keyring blob and `.env` use;
//!   * provider token prefixes that are unambiguous (`sk-ant-`, `sk-proj-`,
//!     `AIza…`, `ghp_…`, `github_pat_`, `xox?-`, `hf_`).
//!
//! # What it does not do
//!
//! It does not find a secret it has no format for, and it does not decode —
//! base64 of a key passes. Like the guard in front of it, it is a safety net
//! against mistakes and against a cooperative model being steered, not a
//! boundary against a determined adversary (see the privacy-is-safety ruling in
//! `docs/security/privacy-tiers.md`).

use std::sync::LazyLock;

use regex::{Regex, RegexSet};
use rmcp::model::{CallToolResult, ErrorData, RawContent, ResourceContents};
use serde_json::Value;

/// The `_meta` key a redacted result carries: `{ "count": n, "kinds": [...] }`.
pub const REDACTION_META_KEY: &str = "biorouterSecretRedaction";

/// What was withheld from one tool result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Redaction {
    pub count: usize,
    /// Distinct kinds, in first-seen order (`aws-secret-access-key`, …).
    pub kinds: Vec<&'static str>,
}

impl Redaction {
    fn absorb(&mut self, found: &[&'static str]) {
        self.count += found.len();
        for kind in found {
            if !self.kinds.contains(kind) {
                self.kinds.push(kind);
            }
        }
    }

    /// One line for a log or a guardrail note. Never contains a value.
    pub fn summary(&self) -> String {
        format!(
            "{} credential value(s) withheld ({})",
            self.count,
            self.kinds.join(", ")
        )
    }
}

/// Names the provider-key store is known to hold. Kept as a list, and pinned by
/// `every_provider_secret_key_is_recognised`, which fails when a provider adds a
/// secret key this module would not recognise.
const STORE_KEY_NAMES: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "DATABRICKS_TOKEN",
    "GOOGLE_API_KEY",
    "LITELLM_API_KEY",
    "LITELLM_CUSTOM_HEADERS",
    "OPENAI_API_KEY",
    "OPENAI_CUSTOM_HEADERS",
    "OPENROUTER_API_KEY",
    "SNOWFLAKE_TOKEN",
    "TETRATE_API_KEY",
    "VENICE_API_KEY",
    "VERSA_AZURE_API_KEY",
    "VERSA_BEDROCK_ACCESS_KEY_ID",
    "VERSA_BEDROCK_SECRET_ACCESS_KEY",
    "XAI_API_KEY",
    "XIAOMI_MIMO_API_KEY",
    "ZAI_API_KEY",
];

/// A name that says it holds a secret: exactly one of these, or ending in
/// `_` + one of these. Upper case only, which is how env files and the store
/// spell them and how code usually does not (`api_key = config.get(…)`).
const SECRET_NAME_SUFFIXES: &[&str] = &[
    "API_KEY",
    "APIKEY",
    "SECRET",
    "SECRET_KEY",
    "SECRET_ACCESS_KEY",
    "ACCESS_KEY",
    "ACCESS_KEY_ID",
    "ACCESS_TOKEN",
    "AUTH_TOKEN",
    "REFRESH_TOKEN",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "PASSCODE",
    "PASSPHRASE",
    "PRIVATE_KEY",
    "CLIENT_SECRET",
    "CREDENTIALS",
    "CUSTOM_HEADERS",
];

fn is_secret_name(name: &str) -> bool {
    if STORE_KEY_NAMES.contains(&name) {
        return true;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return false;
    }
    SECRET_NAME_SUFFIXES.iter().any(|suffix| {
        name == *suffix
            || name
                .strip_suffix(suffix)
                .is_some_and(|head| head.ends_with('_'))
    })
}

/// A value worth withholding: long enough to be a credential, and not a
/// reference to one (`${X}`, `$X`, `<your key>`), a placeholder, or a marker
/// this module already wrote.
///
/// A *quoted* value is a literal by construction. A bare one may be code — the
/// model has to read `API_KEY = os.environ["API_KEY"]` or `TOKEN = settings.token`
/// to do its job — so it must look like a credential: no code punctuation, and
/// a digit somewhere or real length.
fn is_credential_value(value: &str, quoted: bool) -> bool {
    let v = value.trim();
    if v.chars().count() < 8
        || v.starts_with('$')
        || v.starts_with('<')
        || v.starts_with("{{")
        || v.starts_with("[REDACTED")
        || v.eq_ignore_ascii_case("changeme")
        || v.chars().all(|c| matches!(c, '*' | 'x' | 'X' | '.'))
    {
        return false;
    }
    if quoted {
        return true;
    }
    !v.chars().any(|c| {
        matches!(
            c,
            '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\'' | '`' | ';' | ','
        )
    }) && (v.chars().any(|c| c.is_ascii_digit()) || v.chars().count() >= 20)
}

struct Detector {
    kind: &'static str,
    regex: Regex,
    /// The capture group holding the value to replace (0 = the whole match).
    group: usize,
    /// Only a match whose `name` group is a secret name counts.
    needs_secret_name: bool,
}

static DETECTORS: LazyLock<Vec<Detector>> = LazyLock::new(|| {
    let d = |kind, pattern: &str, group, needs_secret_name| Detector {
        kind,
        regex: Regex::new(pattern).expect("static regex"),
        group,
        needs_secret_name,
    };
    vec![
        d(
            "aws-secret-access-key",
            r#"(?i)\b(?:aws_?)?secret_?access_?key\b["']?\s*[:=]\s*["']?([A-Za-z0-9/+]{40})(?:[^A-Za-z0-9/+=]|$)"#,
            1,
            false,
        ),
        d(
            "aws-secret-access-key",
            r#"\b(?:AKIA|ASIA)[A-Z0-9]{16}[\s,;:|"'=]+([A-Za-z0-9/+]{40})(?:[^A-Za-z0-9/+=]|$)"#,
            1,
            false,
        ),
        d(
            "aws-session-token",
            r#"(?i)\b(?:aws_?)?(?:session_?token|security_?token)\b["']?\s*[:=]\s*["']?([A-Za-z0-9/+=]{16,})"#,
            1,
            false,
        ),
        d(
            "aws-access-key-id",
            r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b",
            0,
            false,
        ),
        // `NAME: value` / `NAME=value` / `export NAME=value`, one per line.
        d(
            "secret-value",
            r#"(?m)^[ \t]*(?:export[ \t]+)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)[ \t]*(?:=|:[ \t])[ \t]*(?P<value>"[^"\n]*"|'[^'\n]*'|[^\s#"'][^\s#]*)"#,
            2,
            true,
        ),
        // `"NAME": "value"` — the keyring blob, JSON configs.
        d(
            "secret-value",
            r#""(?P<name>[A-Za-z_][A-Za-z0-9_]*)"\s*:\s*"(?P<value>(?:[^"\\\n]|\\.)*)""#,
            2,
            true,
        ),
        d("api-token", r"\bsk-ant-[A-Za-z0-9_\-]{20,}", 0, false),
        d(
            "api-token",
            r"\bsk-(?:proj|svcacct|admin)-[A-Za-z0-9_\-]{20,}",
            0,
            false,
        ),
        d(
            "api-token",
            r"\bsk-[A-Za-z0-9]{20}T3BlbkFJ[A-Za-z0-9]{20}\b",
            0,
            false,
        ),
        d("api-token", r"\bAIza[0-9A-Za-z_\-]{35}\b", 0, false),
        d("api-token", r"\bgh[pousr]_[A-Za-z0-9]{36,}\b", 0, false),
        d("api-token", r"\bgithub_pat_[A-Za-z0-9_]{22,}\b", 0, false),
        d("api-token", r"\bxox[abprs]-[A-Za-z0-9\-]{10,}", 0, false),
        d("api-token", r"\bhf_[A-Za-z0-9]{30,}\b", 0, false),
    ]
});

/// One pass that says which detectors can match at all, so ordinary output
/// costs a single scan.
static PREFILTER: LazyLock<RegexSet> = LazyLock::new(|| {
    let mut patterns: Vec<String> = DETECTORS
        .iter()
        .map(|d| d.regex.as_str().to_string())
        .collect();
    patterns.push(PEM_BEGIN.as_str().to_string());
    RegexSet::new(patterns).expect("static regex set")
});

static PEM_BEGIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----").expect("static regex")
});
static PEM_END: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----END [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----").expect("static regex")
});

/// Characters a PEM body is made of, including the headers of an encrypted key
/// (`Proc-Type: 4,ENCRYPTED`) and the `\n` escapes of a JSON-embedded key.
fn is_pem_body_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '+' | '/' | '=' | '\\' | ':' | ',' | '-' | '.' | ' ' | '\t' | '\r' | '\n'
        )
}

/// Redact credential material in `text`. `None` when there was nothing to
/// withhold; otherwise the new text and one kind per withheld value.
pub fn redact_text(text: &str) -> Option<(String, Vec<&'static str>)> {
    let hits = PREFILTER.matches(text);
    if !hits.matched_any() {
        return None;
    }
    // (start, end, kind) of every value to withhold.
    let mut spans: Vec<(usize, usize, &'static str)> = Vec::new();

    if hits.matched(DETECTORS.len()) {
        let mut from = 0;
        while let Some(begin) = PEM_BEGIN.find_at(text, from) {
            let body_start = begin.end();
            let body_end = match PEM_END.find_at(text, body_start) {
                Some(end) => end.start(),
                // Cut off (`head -3 key.pem`): the run of body characters.
                None => text
                    .get(body_start..)
                    .unwrap_or("")
                    .char_indices()
                    .find(|(_, c)| !is_pem_body_char(*c))
                    .map(|(i, _)| body_start + i)
                    .unwrap_or(text.len()),
            };
            // A body that is already this module's marker was redacted by an
            // earlier pass; reporting it again would double-count.
            let body = text.get(body_start..body_end).unwrap_or("").trim();
            if body != "[REDACTED:private-key]" && body.chars().any(|c| c.is_ascii_alphanumeric()) {
                spans.push((body_start, body_end, "private-key"));
            }
            from = body_end.max(body_start);
            if from >= text.len() {
                break;
            }
        }
    }

    for (index, detector) in DETECTORS.iter().enumerate() {
        if !hits.matched(index) {
            continue;
        }
        for caps in detector.regex.captures_iter(text) {
            if detector.needs_secret_name {
                let (Some(name), Some(value)) = (caps.name("name"), caps.name("value")) else {
                    continue;
                };
                // The JSON shape's value is always inside quotes; the line
                // shape keeps them in the capture when they are there.
                let raw = value.as_str();
                let quoted = raw.starts_with(['"', '\''])
                    || caps.get(0).is_some_and(|m| m.as_str().starts_with('"'));
                let unquoted = raw.trim_matches(|c| c == '"' || c == '\'');
                if !is_secret_name(name.as_str()) || !is_credential_value(unquoted, quoted) {
                    continue;
                }
            }
            if let Some(m) = caps.get(detector.group) {
                if !m.as_str().is_empty() {
                    spans.push((m.start(), m.end(), detector.kind));
                }
            }
        }
    }
    if spans.is_empty() {
        return None;
    }

    // Merge overlaps, keeping the first kind seen for a region.
    spans.sort_by_key(|(start, end, _)| (*start, std::cmp::Reverse(*end)));
    let mut merged: Vec<(usize, usize, &'static str)> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.0 < last.1 => last.1 = last.1.max(span.1),
            _ => merged.push(span),
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut kinds = Vec::with_capacity(merged.len());
    let mut cursor = 0;
    for (start, end, kind) in merged {
        out.push_str(text.get(cursor..start).unwrap_or(""));
        if kind == "private-key" {
            out.push_str("\n[REDACTED:private-key]\n");
        } else {
            out.push_str("[REDACTED:");
            out.push_str(kind);
            out.push(']');
        }
        kinds.push(kind);
        cursor = end;
    }
    out.push_str(text.get(cursor..).unwrap_or(""));
    Some((out, kinds))
}

/// Redact every text-bearing part of a tool result in place, and stamp what was
/// withheld into its `_meta` under [`REDACTION_META_KEY`].
pub fn redact_call_tool_result(result: &mut CallToolResult) -> Option<Redaction> {
    let mut redaction = Redaction::default();
    for content in result.content.iter_mut() {
        match &mut content.raw {
            RawContent::Text(raw) => redact_in_place(&mut raw.text, &mut redaction),
            RawContent::Resource(embedded) => {
                if let ResourceContents::TextResourceContents { text, .. } = &mut embedded.resource
                {
                    redact_in_place(text, &mut redaction);
                }
            }
            _ => {}
        }
    }
    if let Some(structured) = result.structured_content.as_mut() {
        redact_value(structured, &mut redaction);
    }
    if redaction.count == 0 {
        return None;
    }
    let meta = result.meta.get_or_insert_with(rmcp::model::Meta::new);
    meta.0.insert(
        REDACTION_META_KEY.to_string(),
        serde_json::json!({ "count": redaction.count, "kinds": redaction.kinds }),
    );
    Some(redaction)
}

/// The same for a tool's error: its message and any string in its `data`.
pub fn redact_error(error: &mut ErrorData) -> Option<Redaction> {
    let mut redaction = Redaction::default();
    let mut message = error.message.to_string();
    redact_in_place(&mut message, &mut redaction);
    if redaction.count > 0 {
        error.message = message.into();
    }
    if let Some(data) = error.data.as_mut() {
        redact_value(data, &mut redaction);
    }
    (redaction.count > 0).then_some(redaction)
}

/// Whether a result carries this module's stamp, and what it says.
pub fn redaction_of(result: &CallToolResult) -> Option<Redaction> {
    let stamp = result.meta.as_ref()?.0.get(REDACTION_META_KEY)?;
    let count = stamp.get("count")?.as_u64()? as usize;
    let kinds = stamp
        .get("kinds")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|k| {
            [
                "private-key",
                "aws-access-key-id",
                "aws-secret-access-key",
                "aws-session-token",
                "secret-value",
                "api-token",
            ]
            .into_iter()
            .find(|known| *known == k)
        })
        .collect();
    Some(Redaction { count, kinds })
}

fn redact_in_place(text: &mut String, redaction: &mut Redaction) {
    if let Some((redacted, found)) = redact_text(text) {
        *text = redacted;
        redaction.absorb(&found);
    }
}

fn redact_value(value: &mut Value, redaction: &mut Redaction) {
    match value {
        Value::String(s) => redact_in_place(s, redaction),
        Value::Array(items) => {
            for item in items {
                redact_value(item, redaction);
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                // A JSON object whose key names a secret: `{"OPENAI_API_KEY": "…"}`
                // arrives here as a bare string value, without the quotes the
                // text detector keys on.
                if let Value::String(s) = item {
                    if is_secret_name(key) && is_credential_value(s, true) {
                        *s = "[REDACTED:secret-value]".to_string();
                        redaction.absorb(&["secret-value"]);
                        continue;
                    }
                }
                redact_value(item, redaction);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{AnnotateAble, Content, RawEmbeddedResource};

    // Every value below is made up, and assembled at run time so no
    // key-shaped literal sits in the source for a secret scanner to trip on.
    fn fake_key_id() -> String {
        format!("{}{}", "AKIA", "FAKEFAKEFAKE0000")
    }
    fn fake_secret() -> String {
        format!("{}{}", "fakeSecretKeyForTestsOnly", "0".repeat(15))
    }
    fn fake_pem() -> String {
        let kind = ["OPENSSH", "PRIVATE", "KEY"].join(" ");
        format!(
            "-----BEGIN {kind}-----\nb3BlbnNzaC1rZXktdjEAAAAAfakefakefake\nZmFrZSBrZXkgbWF0ZXJpYWw=\n-----END {kind}-----\n"
        )
    }

    fn redacted(text: &str) -> String {
        redact_text(text)
            .map(|(t, _)| t)
            .unwrap_or_else(|| text.to_string())
    }

    #[test]
    fn an_aws_credentials_file_keeps_its_shape_and_loses_its_values() {
        let (id, secret) = (fake_key_id(), fake_secret());
        let file = format!(
            "[default]\naws_access_key_id = {id}\naws_secret_access_key = {secret}\nregion = us-west-2\n"
        );
        let (out, kinds) = redact_text(&file).expect("found");
        assert!(!out.contains(&id) && !out.contains(&secret), "{out}");
        assert!(
            out.contains("[default]") && out.contains("region = us-west-2"),
            "{out}"
        );
        assert!(out.contains("aws_secret_access_key = [REDACTED:aws-secret-access-key]"));
        assert!(kinds.contains(&"aws-access-key-id"));
        assert!(kinds.contains(&"aws-secret-access-key"));
    }

    #[test]
    fn an_aws_pair_in_csv_or_json_is_withheld() {
        let (id, secret) = (fake_key_id(), fake_secret());
        let csv = format!("Access key ID,Secret access key\n{id},{secret}\n");
        let out = redacted(&csv);
        assert!(!out.contains(&id) && !out.contains(&secret), "{out}");
        let json = format!(
            r#"{{"Credentials": {{"AccessKeyId": "{id}", "SecretAccessKey": "{secret}", "SessionToken": "{}"}}}}"#,
            "FwoGZXIvYXdzEFAKEtokenFAKEtoken0123456789abcdef"
        );
        let out = redacted(&json);
        assert!(!out.contains(&id) && !out.contains(&secret), "{out}");
        assert!(!out.contains("FwoGZXIvYXdzEFAKE"), "{out}");
    }

    #[test]
    fn a_private_key_block_keeps_its_armour_and_loses_its_body() {
        let pem = fake_pem();
        let out = redacted(&format!("before\n{pem}after\n"));
        assert!(out.contains("-----BEGIN OPENSSH PRIVATE KEY-----"), "{out}");
        assert!(out.contains("-----END OPENSSH PRIVATE KEY-----"), "{out}");
        assert!(!out.contains("b3BlbnNzaC1rZXktdjEAAAAA"), "{out}");
        assert!(out.contains("[REDACTED:private-key]"));
        assert!(
            out.starts_with("before\n") && out.ends_with("after\n"),
            "{out}"
        );
    }

    #[test]
    fn a_cut_off_or_json_embedded_private_key_is_still_withheld() {
        let pem = fake_pem();
        let head: String = pem.lines().take(2).collect::<Vec<_>>().join("\n");
        let out = redacted(&format!("{head}\n$ next command\n"));
        assert!(!out.contains("b3BlbnNzaC1rZXktdjEAAAAA"), "{out}");
        assert!(
            out.contains("$ next command"),
            "the rest of the output survives: {out}"
        );

        let escaped = serde_json::to_string(&pem).unwrap();
        let out = redacted(&format!(r#"{{"private_key": {escaped}}}"#));
        assert!(!out.contains("b3BlbnNzaC1rZXktdjEAAAAA"), "{out}");
    }

    #[test]
    fn the_provider_key_store_is_withheld_in_every_shape_it_is_stored_in() {
        let yaml = "OPENAI_API_KEY: not-a-real-key-0123456789\nBIOROUTER_PROVIDER: versa_azure\n";
        let out = redacted(yaml);
        assert!(!out.contains("not-a-real-key"), "{out}");
        assert!(out.contains("BIOROUTER_PROVIDER: versa_azure"), "{out}");

        let keyring = r#"{"VERSA_AZURE_API_KEY":"abcdef0123456789abcdef","GOOSE_MODEL":"gpt"}"#;
        let out = redacted(keyring);
        assert!(!out.contains("abcdef0123456789abcdef"), "{out}");
        assert!(out.contains(r#""GOOSE_MODEL":"gpt""#), "{out}");

        let dotenv = "export STRIPE_SECRET_KEY=\"fake_live_value_123456\"\nDEBUG=true\n";
        let out = redacted(dotenv);
        assert!(!out.contains("fake_live_value"), "{out}");
        assert!(out.contains("DEBUG=true"), "{out}");
    }

    #[test]
    fn distinctive_provider_tokens_are_withheld_wherever_they_appear() {
        for token in [
            format!("sk-ant-api03-{}", "x".repeat(40)),
            format!("sk-proj-{}", "Y".repeat(40)),
            format!("AIza{}", "Z".repeat(35)),
            format!("ghp_{}", "a".repeat(36)),
            format!("hf_{}", "b".repeat(34)),
        ] {
            let out = redacted(&format!("the token is {token} ok"));
            assert!(!out.contains(&token), "{out}");
            assert!(out.ends_with(" ok"), "{out}");
        }
    }

    /// The model still has to read code, configs and docs that *mention*
    /// credentials. A reference, a short value, a placeholder or ordinary prose
    /// must pass untouched.
    #[test]
    fn ordinary_text_passes_untouched() {
        for text in [
            "API_KEY = os.environ[\"API_KEY\"]\n",
            "SECRET_KEY = settings.secret_key\n",
            "API_TOKEN = get_token(user)\n",
            "ACCESS_TOKEN: token_from_vault\n",
            "OPENAI_API_KEY=${OPENAI_API_KEY}\n",
            "MAX_TOKEN=4096\n",
            "api_key: <your key here>\n",
            "password: hunter2\n",
            "The aws_secret_access_key setting lives in ~/.aws/credentials.\n",
            "commit 3f2a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e3f2a\n",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFAKE tester@example\n",
            "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE\n-----END PUBLIC KEY-----\n",
            "BIOROUTER_PROVIDER: versa_azure\nGOOSE_MODEL: gpt-5.5\n",
        ] {
            assert_eq!(redact_text(text), None, "redacted ordinary text: {text}");
        }
    }

    #[test]
    fn redaction_is_idempotent() {
        let file = format!(
            "aws_secret_access_key = {}\nOPENAI_API_KEY: {}\n{}",
            fake_secret(),
            "not-a-real-key-0123456789",
            fake_pem()
        );
        let once = redacted(&file);
        assert_eq!(redact_text(&once), None, "a second pass found more: {once}");

        let cut_off: String = fake_pem().lines().take(2).collect::<Vec<_>>().join("\n");
        let once = redacted(&format!("{cut_off}\n$ next\n"));
        assert_eq!(redact_text(&once), None, "a second pass found more: {once}");
    }

    #[test]
    fn a_result_is_redacted_in_every_text_part_and_stamped() {
        let secret = fake_secret();
        let mut result = CallToolResult {
            content: vec![
                Content::text(format!("aws_secret_access_key = {secret}"))
                    .with_audience(vec![rmcp::model::Role::Assistant]),
                RawContent::Resource(RawEmbeddedResource {
                    meta: None,
                    resource: ResourceContents::TextResourceContents {
                        uri: "file:///x".into(),
                        mime_type: None,
                        text: fake_pem(),
                        meta: None,
                    },
                })
                .no_annotation(),
            ],
            structured_content: Some(
                serde_json::json!({ "OPENAI_API_KEY": "not-a-real-key-0123456789" }),
            ),
            is_error: None,
            meta: None,
        };
        let redaction = redact_call_tool_result(&mut result).expect("redacted");
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(!serialized.contains(&secret), "{serialized}");
        assert!(
            !serialized.contains("b3BlbnNzaC1rZXktdjEAAAAA"),
            "{serialized}"
        );
        assert!(!serialized.contains("not-a-real-key"), "{serialized}");
        assert_eq!(redaction.count, 3);
        assert_eq!(redaction_of(&result), Some(redaction));
        // The audience annotation a tool set survives the rewrite.
        assert_eq!(
            result.content[0].audience(),
            Some(&vec![rmcp::model::Role::Assistant])
        );
    }

    #[test]
    fn an_error_is_redacted_too() {
        let secret = fake_secret();
        let mut error = ErrorData::new(
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            format!("Script failed.\nOutput:\naws_secret_access_key = {secret}\n"),
            Some(serde_json::json!({ "stdout": format!("aws_secret_access_key={secret}") })),
        );
        assert!(redact_error(&mut error).is_some());
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&secret), "{serialized}");
    }

    /// Every secret a built-in provider asks the store for is a name this
    /// module recognises — the pin on [`STORE_KEY_NAMES`] and the suffix rule.
    #[test]
    fn every_provider_secret_key_is_recognised() {
        let mut missing = Vec::new();
        for metadata in crate::providers::builtin_provider_metadata() {
            for key in metadata.config_keys.iter().filter(|k| k.secret) {
                let line = format!("{}: abcdefgh0123456789\n", key.name);
                if redact_text(&line).is_none() {
                    missing.push(format!("{} ({})", key.name, metadata.name));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "secret config keys the output redactor would not recognise: {missing:?}"
        );
    }
}
