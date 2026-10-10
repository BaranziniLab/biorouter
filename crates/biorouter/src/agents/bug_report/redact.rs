//! The scrubber and the validator: the harness that stands between what the
//! agent wrote and what becomes a public GitHub issue.
//!
//! ## Why this exists as its own layer
//!
//! The bug-report tool never posts a raw transcript — it posts a *distilled*
//! report the model wrote. That is the primary defence and it is not
//! sufficient, because the model writes that report **from** the transcript.
//! It will quote the error it is reporting, and a real error message is exactly
//! where a home path, a username, a bearer token or a signed URL lives:
//!
//! ```text
//! Error: failed to read /Users/jsmith/Documents/IRB-2019-441/cohort.csv
//! Error: 401 from https://api.example.org?token=sk-live-9f2a…
//! ```
//!
//! So every byte that leaves for GitHub passes through [`scrub`], and then
//! through [`validate_issue`], which **fails the report** rather than trusting
//! the scrub to have been complete. The two are deliberately different shapes
//! of check: `scrub` rewrites what it recognises, `validate_issue` refuses what
//! it still recognises afterwards. A single pass that did both would report
//! success for every pattern it forgot.
//!
//! ⚠ Nothing here is a *guarantee*. A secret that looks like prose survives any
//! pattern set. This raises the floor and makes the residue visible on the
//! approval card, where a person reads the exact body before it is posted;
//! the person is the last check and the design assumes it.

use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

/// What the model wrote about `/Users/jsmith/...`, after scrubbing.
pub const HOME_PLACEHOLDER: &str = "~";
/// What replaces anything credential-shaped.
pub const SECRET_PLACEHOLDER: &str = "[redacted]";
/// What replaces a username lifted out of a path.
pub const USER_PLACEHOLDER: &str = "<user>";

/// GitHub rejects an issue body over 65,536 characters. The margin is for the
/// receipt lines the filer appends after validation.
pub const MAX_ISSUE_BODY_CHARS: usize = 60_000;

/// The practical ceiling on a prefilled `…/issues/new?body=` URL.
///
/// GitHub answers 414 well before any browser's own limit, and the body is
/// percent-encoded on the way in — a body of mostly punctuation and newlines
/// triples. So the cap is applied to the ENCODED url, not the raw body, and the
/// raw budget below is the conservative pre-image of it.
pub const MAX_COMPOSE_URL_CHARS: usize = 8_000;

/// The ceiling on the **Suspected cause** section, in characters.
///
/// Well inside [`MAX_COMPOSE_URL_CHARS`]: prose and links grow about 1.4× when
/// percent-encoded, and a typical report without the section already encodes to
/// about 2,000 characters. A cause past this would push an ordinary report off
/// the prefilled page — which, from a private chat, is the only path that does
/// not hand the text back to be pasted — so it is refused for the model to
/// shorten rather than silently costing the user the link.
pub const MAX_SUSPECTED_CAUSE_CHARS: usize = 4_000;

/// One thing the scrubber changed, for the receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// What kind of thing it was. Never the value.
    pub kind: &'static str,
    /// How many were replaced.
    pub count: usize,
}

/// The result of a scrub: the rewritten text and what was rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scrubbed {
    pub text: String,
    pub findings: Vec<Finding>,
}

impl Scrubbed {
    pub fn changed(&self) -> bool {
        !self.findings.is_empty()
    }

    /// A line for the tool's receipt. Names kinds and counts, never a value —
    /// this string reaches the model, and a "redacted" value quoted back into
    /// the conversation is not redacted.
    pub fn summary(&self) -> String {
        if self.findings.is_empty() {
            return "nothing needed redacting".to_string();
        }
        self.findings
            .iter()
            .map(|f| format!("{}×{}", f.count, f.kind))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Credential shapes, most specific first.
///
/// Ordering matters: `GENERIC_ASSIGNMENT` would swallow the tail of a
/// `token=ghp_…` before `VENDOR_TOKEN` ever saw it, and the report would then
/// say "an assignment" where it should say "a GitHub token". Both redact, but
/// the receipt is what tells a user how bad the near-miss was.
static PATTERNS: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    let compile = |source: &str| Regex::new(source).expect("bug-report scrub pattern is valid");
    vec![
        // A JWT: three base64url segments. Whole thing, because the payload is
        // the identifying half.
        (
            "jwt",
            compile(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"),
        ),
        // GitHub's own prefixes, AWS access key ids, OpenAI/Anthropic style keys,
        // Slack, Google.
        (
            "vendor token",
            compile(
                r"\b(?:gh[pousr]_[A-Za-z0-9]{16,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}|sk-[A-Za-z0-9_-]{16,}|sk_live_[A-Za-z0-9]{16,}|xox[baprs]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{30,}|glpat-[A-Za-z0-9_-]{16,})",
            ),
        ),
        // `Authorization: Bearer …`, and the bare `Bearer …` an error message
        // quotes back.
        (
            "bearer token",
            compile(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{12,}"),
        ),
        // `Authorization: Basic dXNlcjpwYXNzd29yZA==` — HTTP's other credential
        // header, and the one shape that slipped through BOTH halves of this
        // harness. The `bearer token` rule above does not match it, and the
        // generic assignment rule below cannot: its value group needs six
        // characters and
        // `Basic` is five, so the match dies on the scheme word and never
        // reaches the base64 behind it. Scrub and validator agreed, and were
        // both wrong — which is precisely the failure the two-shape design
        // exists to prevent and cannot, when the gap is a shape neither knows.
        //
        // Anchored on the header name rather than on a bare `Basic`, on
        // purpose: `(?i)basic\s+\w{8,}` also matches "basic understanding",
        // and a false positive here is not cosmetic — `validate_issue` REFUSES
        // a report on anything the rescrub still finds, so it would reject a
        // bug report for containing ordinary English.
        (
            "basic auth",
            compile(r"(?i)\b((?:proxy-)?authorization\s*[=:]\s*basic)\s+[A-Za-z0-9+/=_-]{8,}"),
        ),
        // `curl -u alice:hunter2` / `--user=alice:hunter2` — the same credential
        // one layer earlier, in the command a user pastes into a bug report to
        // show what they ran. Not covered by `url credential`, which needs a
        // `scheme://`, and not by the assignment rule, which needs a keyword.
        //
        // The `\bcurl\b[^\n]*?` prefix is load-bearing and stays on ONE line:
        // a bare `-u user:pass` rule would redact `docker run -u 1000:1000` and
        // `id -u`, and a false positive is a refused report (see above).
        (
            "command-line credential",
            compile(r#"(?i)(\bcurl\b[^\n]*?\s--?u(?:ser)?[=\s])[^\s'"]+:[^\s'"]+"#),
        ),
        // A credential-shaped assignment in prose, a URL query, an env dump or a
        // YAML line. The needle set matches `diagnostics::is_secret_key` on
        // purpose: two redactors disagreeing about what a credential is means
        // one of them is wrong.
        //
        // ⚠ A Rust path is not an assignment. `crate::oauth::oauth_flow` and
        // `secret_guard::resolve` are what a suspected cause names, and a bare
        // `[=:]` read the first `:` of `::` as the separator and the rest of the
        // path as the value — `crate::oauth=[redacted]`, closing backtick gone.
        // So a `:` separator may not be followed by another `:` (`oauth::x`
        // matches nothing at the key, and a `password=…` later in the same text
        // still matches from its own `\b`). There is no lookahead in `regex`,
        // hence the three arms: `=` and `:` + whitespace may still be followed
        // by a value that starts with `:` (consumed, so `password=:hunter2`
        // loses `hunter2`); a bare `:` may not.
        (
            "credential assignment",
            compile(
                r#"(?i)\b([A-Za-z0-9_.-]*(?:api[_-]?key|secret|password|passwd|passcode|token|credential|private[_-]?key|access[_-]?key|auth)[A-Za-z0-9_.-]*)\s*(?:=\s*["']?:?|:\s+["']?:?|:["']?)([^\s"'&,;)}\]:][^\s"'&,;)}\]]{5,})"#,
            ),
        ),
        // `https://user:password@host` — the password is the whole point.
        (
            "url credential",
            compile(r"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/:@]+:[^\s/@]+@"),
        ),
        (
            "email address",
            compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"),
        ),
    ]
});

/// A `credential assignment` value that is a TYPE, not a secret: the
/// `api_key: String` and `token: Option<String>` a report quotes when its
/// suspected cause points at code.
///
/// ⚠ Anchored and closed on purpose. The value class stops at `)`, `}`, `]`,
/// `,`, quotes and whitespace, so `HashMap<String, String>` reaches here as
/// `HashMap<String` and `Option<Box<dyn Error>>` as `Option<Box<dyn` — hence a
/// generic is matched from its head and `<` alone. What is NOT here is "any
/// identifier": a capitalised alphanumeric value can be a real secret, and a
/// false negative in this list publishes it.
///
/// ⚠ That holds INSIDE the angle brackets too. A generic's arguments are
/// [`TYPE_NAME`]s — a known primitive, or a capitalised name with no digits,
/// behind optional lowercase `seg::` path segments — and nothing may follow
/// the closing `>`s. Any alphanumeric argument let `password=Vec<hunter2xyz>`
/// through whole, and a single `:` let `auth:Vec<u8>:secret:S3cr3t…` swallow
/// the next assignment into an exempt match.
static TYPE_EXPRESSION: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"^(?:String|string|boolean|number|undefined|SecretString|(?:Option|Vec|Result|Box|Arc|Rc|Cow|HashMap|BTreeMap|SecretBox|Promise|Array|Record)<(?:{TYPE_NAME}<|\[)*(?:{TYPE_NAME})?>*)$"
    ))
    .expect("type expression pattern is valid")
});

/// One type argument inside a [`TYPE_EXPRESSION`] generic: `String`,
/// `std::path::PathBuf`, `u8`, `string`, `dyn`. A capitalised name carries no
/// digits, because a digit is what most generated secrets carry.
const TYPE_NAME: &str = r"(?:[a-z][a-z_]*::)*(?:[A-Z][A-Za-z]*|str|u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|f32|f64|bool|char|string|number|boolean|unknown|any|dyn)";

/// A `credential assignment` that is a source location, not a secret:
/// `token_counter.rs:100-200`, `auth.ts:12:5`. The "key" is a file name a
/// suspected cause cites, and the "value" a line, a line range or a
/// line:column — digits only, so nothing a secret could hide in.
static SOURCE_LOCATION_KEY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\.(?:rs|ts|tsx|js|jsx|mjs|py|md|toml|ya?ml|json|sh)$")
        .expect("source file pattern is valid")
});
static LINE_REFERENCE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\d+(?:[-:]\d+)*$").expect("line reference pattern is valid"));

/// Is this `credential assignment` value one the scrub must leave alone?
///
/// Two shapes, each of which used to make a report UNFILEABLE rather than
/// merely ugly:
///
/// * The scrub's own placeholder. `KEY=[redacted]` re-matched with the value
///   `[redacted` (the class stops at `]`), so `validate_issue`'s second scrub
///   "found" a credential in text it had just cleaned and refused the report.
///   When the match sat in the auto-added failure list the model could not
///   rewrite it, so no retry could ever succeed.
/// * A type annotation (see [`TYPE_EXPRESSION`]), which the first pass
///   rewrote to `api_key=[redacted]` — garbling the code a suspected cause
///   quotes, and then tripping the placeholder case above.
///
/// And one that only garbled: a source location (`token_counter.rs:100-200`,
/// see [`SOURCE_LOCATION_KEY`]), which a suspected cause cites constantly.
///
/// A closing backtick is not part of the value: the class does not stop at
/// one, so a code span (`` `token: Option<String>` ``) reaches here with it.
fn assignment_value_is_not_a_secret(key: &str, value: &str) -> bool {
    let placeholder_stem = SECRET_PLACEHOLDER.trim_end_matches(']');
    let value = value.trim_end_matches('`');
    value.starts_with(placeholder_stem)
        || TYPE_EXPRESSION.is_match(value)
        || (SOURCE_LOCATION_KEY.is_match(key) && LINE_REFERENCE.is_match(value))
}

/// `/Users/<name>/…`, `/home/<name>/…`, `C:\Users\<name>\…`.
///
/// Matched independently of whose home it is: another account's name on a
/// shared machine identifies a person just as well as the reporter's does, and
/// a bundle read on a lab workstation routinely contains both.
static USER_HOME_PATH: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(/Users/|/home/|[A-Z]:\\Users\\)([A-Za-z0-9._-]+)")
        .expect("home path pattern is valid")
});

/// Rewrite everything recognisably private in `text`.
///
/// `home` is the current user's home directory when one is known. It is
/// replaced FIRST and with `~`, so the common case reads naturally
/// (`~/Desktop/BioRouter`) instead of collapsing to `<user>`; every other
/// account's home falls through to the generic rule below it.
pub fn scrub(text: &str, home: Option<&Path>) -> Scrubbed {
    let mut out = text.to_string();
    let mut findings: Vec<Finding> = Vec::new();

    if let Some(home) = home.map(|h| h.to_string_lossy().into_owned()) {
        // A home of `/` (or empty) would rewrite every absolute path in the
        // report into nonsense. `dirs::home_dir` can return odd values in a
        // container, so this is a real guard, not defensiveness.
        if home.len() > 1 {
            let count = out.matches(home.as_str()).count();
            if count > 0 {
                out = out.replace(home.as_str(), HOME_PLACEHOLDER);
                findings.push(Finding {
                    kind: "home path",
                    count,
                });
            }
        }
    }

    let mut user_paths = 0usize;
    out = USER_HOME_PATH
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            user_paths += 1;
            format!("{}{USER_PLACEHOLDER}", &caps[1])
        })
        .into_owned();
    if user_paths > 0 {
        findings.push(Finding {
            kind: "username in path",
            count: user_paths,
        });
    }

    for (kind, pattern) in PATTERNS.iter() {
        let mut count = 0usize;
        out = pattern
            .replace_all(&out, |caps: &regex::Captures<'_>| {
                // Left exactly as found, and NOT counted: a finding here would
                // be a `disclosure` violation in `validate_issue`, which
                // refuses the report.
                if *kind == "credential assignment"
                    && assignment_value_is_not_a_secret(&caps[1], &caps[2])
                {
                    return caps[0].to_string();
                }
                count += 1;
                match *kind {
                    // Keep the KEY, drop the value: "GITHUB_TOKEN was empty" is
                    // the whole content of some bug reports, and a report that
                    // cannot name the setting it is about is useless.
                    "credential assignment" => format!("{}={SECRET_PLACEHOLDER}", &caps[1]),
                    "url credential" => format!("{}{SECRET_PLACEHOLDER}@", &caps[1]),
                    // Keep the header/flag, drop the value — same reason as
                    // `credential assignment`: a report that cannot say WHICH
                    // request was rejected is not a report. Both replacements
                    // are also idempotent, which matters more than it looks:
                    // `validate_issue` re-runs this scrub and refuses anything
                    // it still finds, so a rewrite that re-matched its own
                    // output would refuse every report it had just cleaned.
                    "basic auth" => format!("{} {SECRET_PLACEHOLDER}", &caps[1]),
                    "command-line credential" => format!("{}{SECRET_PLACEHOLDER}", &caps[1]),
                    _ => SECRET_PLACEHOLDER.to_string(),
                }
            })
            .into_owned();
        if count > 0 {
            findings.push(Finding { kind, count });
        }
    }

    Scrubbed {
        text: out,
        findings,
    }
}

/// A reason the report must not be posted as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub rule: &'static str,
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.rule, self.detail)
    }
}

/// The sections a body must carry, matched exactly (case-sensitively, as a
/// plain substring) on the heading text the repository's own `bug_report.md`
/// uses. `render_body` writes each one verbatim, so an exact match is all a
/// rendered body ever needs.
///
/// An optional section such as **Suspected cause** is deliberately absent: a
/// report without one is complete.
///
/// Derived from that file at build time would be better and is not possible:
/// the template is markdown prose with no machine-readable section list. It is
/// instead asserted against the real file by a test, so a template edit that
/// renames a section fails here rather than silently producing reports that no
/// longer match it.
pub const REQUIRED_SECTIONS: &[&str] = &[
    "Describe the bug",
    "To Reproduce",
    "Expected behavior",
    "Please provide the following information",
];

/// The last gate before anything is posted.
///
/// Every rule is a refusal, not a warning. The tool has already asked a person
/// to approve a specific body; a check that "notices" a token and posts anyway
/// would make the approval a formality.
pub fn validate_issue(title: &str, body: &str, home: Option<&Path>) -> Vec<Violation> {
    let mut violations = Vec::new();

    let trimmed_title = title.trim();
    if trimmed_title.len() < 8 {
        violations.push(Violation {
            rule: "title",
            detail: format!(
                "the title is {} character(s); a bug report needs a title someone can \
                 recognise in a list",
                trimmed_title.chars().count()
            ),
        });
    }
    if trimmed_title.len() > 200 {
        violations.push(Violation {
            rule: "title",
            detail: "the title is over 200 characters; put the detail in the body".to_string(),
        });
    }
    if trimmed_title.contains('\n') {
        violations.push(Violation {
            rule: "title",
            detail: "the title spans more than one line".to_string(),
        });
    }

    for section in REQUIRED_SECTIONS {
        if !body.contains(section) {
            violations.push(Violation {
                rule: "template",
                detail: format!("the body has no `{section}` section"),
            });
        }
    }

    if body.chars().count() > MAX_ISSUE_BODY_CHARS {
        violations.push(Violation {
            rule: "size",
            detail: format!(
                "the body is {} characters; GitHub's limit is 65,536 and this tool's is {}",
                body.chars().count(),
                MAX_ISSUE_BODY_CHARS
            ),
        });
    }

    // The scrub already ran. Anything it still finds is a pattern the scrub
    // missed on its first pass — an overlap, a value reconstructed by
    // formatting — and the honest answer is to stop.
    let rescrub = scrub(body, home);
    for finding in &rescrub.findings {
        violations.push(Violation {
            rule: "disclosure",
            detail: format!(
                "the body still contains {} {}(s) after redaction",
                finding.count, finding.kind
            ),
        });
    }
    let title_scrub = scrub(title, home);
    for finding in &title_scrub.findings {
        violations.push(Violation {
            rule: "disclosure",
            detail: format!(
                "the title still contains {} {}(s) after redaction",
                finding.count, finding.kind
            ),
        });
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_home_path_becomes_a_tilde_and_other_accounts_lose_their_name() {
        let scrubbed = scrub(
            "failed to read /Users/jsmith/Desktop/cohort.csv and /Users/klee/tmp/x",
            Some(Path::new("/Users/jsmith")),
        );
        assert!(
            scrubbed.text.contains("~/Desktop/cohort.csv"),
            "{scrubbed:?}"
        );
        assert!(
            scrubbed.text.contains("/Users/<user>/tmp/x"),
            "another account's name identifies a person too: {scrubbed:?}"
        );
        assert!(!scrubbed.text.contains("jsmith"), "{scrubbed:?}");
        assert!(!scrubbed.text.contains("klee"), "{scrubbed:?}");
    }

    #[test]
    fn a_linux_and_a_windows_home_are_both_recognised() {
        let scrubbed = scrub(
            r"/home/wgu/.config/biorouter and C:\Users\WGu\AppData\Roaming",
            None,
        );
        assert!(
            scrubbed.text.contains("/home/<user>/.config"),
            "{scrubbed:?}"
        );
        assert!(
            scrubbed.text.contains(r"C:\Users\<user>\AppData"),
            "{scrubbed:?}"
        );
    }

    #[test]
    fn vendor_tokens_are_redacted_whole() {
        for secret in [
            "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
            "github_pat_11ABCDEFG0abcdefghijklmnop",
            "AKIAIOSFODNN7EXAMPLE",
            "sk-proj-abcdefghijklmnopqrstuvwxyz0123",
            "xoxb-1234567890-abcdefghij",
            "glpat-abcdefghijklmnopqrst",
        ] {
            let scrubbed = scrub(&format!("the error was: {secret} rejected"), None);
            assert!(
                !scrubbed.text.contains(secret),
                "`{secret}` survived: {scrubbed:?}"
            );
            assert!(scrubbed.changed());
        }
    }

    #[test]
    fn a_jwt_is_redacted_including_its_payload() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let scrubbed = scrub(&format!("Authorization failed for {jwt}"), None);
        assert!(!scrubbed.text.contains("eyJzdWIi"), "{scrubbed:?}");
        assert_eq!(scrubbed.findings[0].kind, "jwt");
    }

    /// The key survives, the value does not. A report that cannot say WHICH
    /// setting was empty is not a report.
    #[test]
    fn a_credential_assignment_keeps_its_key_and_loses_its_value() {
        let scrubbed = scrub(
            "config had GITHUB_TOKEN=ghs_supersecretvalue123456 set",
            None,
        );
        assert!(scrubbed.text.contains("GITHUB_TOKEN"), "{scrubbed:?}");
        assert!(!scrubbed.text.contains("supersecret"), "{scrubbed:?}");
    }

    /// The shape that slipped through BOTH halves: neither the bearer rule nor
    /// the assignment rule can reach the base64 behind `Basic`.
    #[test]
    fn basic_auth_credentials_are_redacted_and_the_header_survives() {
        let secret = "dXNlcjpodW50ZXIyc3VwZXJzZWNyZXQ=";
        for line in [
            format!("Authorization: Basic {secret}"),
            format!("authorization:basic {secret}"),
            format!("Proxy-Authorization: Basic {secret}"),
            format!("-H 'Authorization: Basic {secret}'"),
        ] {
            let scrubbed = scrub(&line, None);
            assert!(
                !scrubbed.text.contains(secret),
                "`{line}` survived: {scrubbed:?}"
            );
            assert!(
                scrubbed.text.to_ascii_lowercase().contains("basic"),
                "the header names WHICH request was rejected: {scrubbed:?}"
            );
        }
    }

    /// A false positive here is not cosmetic: `validate_issue` refuses a report
    /// on anything the rescrub still finds, so an over-eager `Basic` rule would
    /// reject a bug report for containing ordinary English.
    #[test]
    fn ordinary_prose_about_basics_is_left_alone() {
        for line in [
            "a basic understanding of the graph schema",
            "Basic authentication is not configured",
            "the basic dashboard renders blank",
        ] {
            let scrubbed = scrub(line, None);
            assert_eq!(scrubbed.text, line, "prose was rewritten: {scrubbed:?}");
        }
    }

    /// `curl -u user:pass` — the same credential one layer earlier, in the
    /// command a user pastes in to show what they ran.
    #[test]
    fn a_curl_user_flag_loses_its_credential_and_keeps_its_flag() {
        for line in [
            "curl -u alice:hunter2 https://api.example.org/v1",
            "curl --user=alice:hunter2 https://api.example.org/v1",
            "curl -sS -H 'Accept: application/json' -u alice:hunter2 https://x.example",
        ] {
            let scrubbed = scrub(line, None);
            assert!(
                !scrubbed.text.contains("hunter2"),
                "`{line}` survived: {scrubbed:?}"
            );
            assert!(
                !scrubbed.text.contains("alice"),
                "the username identifies a person too: {scrubbed:?}"
            );
            assert!(scrubbed.text.contains("curl"), "{scrubbed:?}");
        }
    }

    /// The `\bcurl\b` prefix is what keeps this rule off every other `-u`.
    #[test]
    fn a_uid_gid_pair_is_not_a_credential() {
        for line in [
            "docker run -u 1000:1000 biorouter/ci",
            "id -u",
            "sort -u results.tsv",
        ] {
            let scrubbed = scrub(line, None);
            assert_eq!(
                scrubbed.text, line,
                "a non-credential was rewritten: {scrubbed:?}"
            );
        }
    }

    /// `validate_issue` re-runs the scrub and refuses anything it still finds,
    /// so a rewrite that re-matched its own output would refuse every report it
    /// had just cleaned.
    #[test]
    fn the_new_replacements_do_not_match_their_own_output() {
        let once = scrub(
            "Authorization: Basic dXNlcjpodW50ZXIy and curl -u alice:hunter2 https://x.example",
            None,
        );
        assert!(once.changed());
        let twice = scrub(&once.text, None);
        assert!(
            twice.findings.is_empty(),
            "the second pass found something, so validate_issue would refuse: {twice:?}"
        );
    }

    /// ⚠ EVERY placeholder shape, not only the two above: the credential
    /// assignment rule used to re-match its own `KEY=[redacted]` (the value
    /// class stops at `]`, leaving `[redacted`, which is long enough), so the
    /// rescrub refused the report it had just cleaned. When the match was in
    /// the auto-added failure list the model could not rewrite it, and the
    /// report could never be filed.
    #[test]
    fn every_replacement_is_a_fixed_point_of_the_scrub() {
        let home = Some(Path::new("/Users/jsmith"));
        for input in [
            "config had GITHUB_TOKEN=ghs_supersecretvalue123456 set",
            "token=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
            "password: hunter2xyz was rejected",
            "api_key = \"sk-live-abcdefghij\"",
            "Authorization: Bearer abcdefghijklmnopqrstuv",
            "Authorization: Bearer abc123",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            "Authorization: Basic dXNlcjpodW50ZXIy",
            "curl -u alice:hunter2 https://x.example",
            "in `oauth::password=hunter2xyz` the value leaked",
            "password=:hunter2xyz",
            "cloned https://wgu:hunter2@git.example.org/x",
            "mailed a@b.example.com",
            "failed to read /Users/jsmith/Desktop/cohort.csv and /home/klee/x",
        ] {
            let once = scrub(input, home);
            assert!(once.changed(), "`{input}` should have been rewritten: {once:?}");
            let twice = scrub(&once.text, home);
            assert_eq!(
                twice.text, once.text,
                "the scrub is not idempotent on `{input}`"
            );
            assert!(
                twice.findings.is_empty(),
                "the second pass found something in `{}`, so validate_issue would refuse \
                 it: {twice:?}",
                once.text
            );
        }
    }

    /// A report that already carries the placeholder — quoted from a failure
    /// the scrub cleaned on its way into the evidence — is fileable.
    #[test]
    fn a_body_quoting_the_placeholder_passes_the_validator() {
        let violations = validate_issue(
            "Shell tool loses the GitHub token on restart",
            &full_body("The log line was `GITHUB_TOKEN=[redacted]` and then 401."),
            None,
        );
        assert!(violations.is_empty(), "{violations:?}");
    }

    /// Type annotations are not secrets. A suspected cause quotes code, and
    /// `fn f(api_key: String)` used to be rewritten to `api_key=[redacted])`.
    #[test]
    fn a_type_annotation_is_left_alone_and_does_not_block_filing() {
        for line in [
            "fn f(api_key: String)",
            "pub token: Option<String>,",
            "let auth_token: Vec<u8> = read();",
            "struct C { secret: SecretString }",
            "fn g(password: SecretBox<str>) -> Result<(), Error>",
            "credentials: HashMap<String, String>",
            "access_key: Option<Box<dyn Error>>",
            "interface P { apiKey: string; token: undefined }",
            "const auth: Promise<string> = load();",
            "type T = { passcode: number, flag: boolean }",
            "token: Arc<Mutex<String>>",
            "secret: Option<std::path::PathBuf>",
            "let token: Option<Vec<[u8; 32]>> = None;",
            "the field is `pub token: Option<String>` today",
        ] {
            let scrubbed = scrub(line, None);
            assert_eq!(scrubbed.text, line, "code was rewritten: {scrubbed:?}");
            assert!(!scrubbed.changed(), "{scrubbed:?}");
        }
        let violations = validate_issue(
            "Provider loses its key when the config is reloaded",
            &full_body(
                "`fn connect(api_key: String)` in `providers/base.rs` drops it, and the log \
                 shows `GITHUB_TOKEN=[redacted]`.",
            ),
            None,
        );
        assert!(violations.is_empty(), "{violations:?}");
    }

    /// The exemption is a closed list, not "anything that looks like a type":
    /// a capitalised alphanumeric value can be a real secret.
    #[test]
    fn real_secrets_are_still_redacted_beside_the_exemptions() {
        for (line, secret) in [
            ("password=hunter2xyz", "hunter2xyz"),
            ("token=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345", "ABCDEFGH"),
            ("api_key: Strings3cr3tValue9", "Strings3cr3tValue9"),
            ("secret: SecretStringX7f9a2", "SecretStringX7f9a2"),
            ("auth_token: K8sV3ctorT0ken", "K8sV3ctorT0ken"),
            // A generic head is not a licence for its arguments: anything
            // inside the brackets used to be exempt, and a single `:` let the
            // exempt match swallow the NEXT assignment.
            ("password=Vec<hunter2xyz>", "hunter2xyz"),
            ("DB_PASSWORD=Box<Tr0ub4dor3>", "Tr0ub4dor3"),
            ("auth:Vec<u8>:secret:S3cr3tV4lue99", "S3cr3tV4lue99"),
            ("token=Option<String>Tr0ub4dor3", "Tr0ub4dor3"),
            // A source-file key is exempt only for a line reference.
            ("auth.rs:hunter2xyz", "hunter2xyz"),
            ("token.json:12ab34cd56", "12ab34cd56"),
        ] {
            let scrubbed = scrub(line, None);
            assert!(
                !scrubbed.text.contains(secret),
                "`{line}` survived: {scrubbed:?}"
            );
            assert!(scrubbed.changed(), "{scrubbed:?}");
        }
    }

    /// A Rust path or a source location is what a suspected cause names, and
    /// the assignment rule used to read `oauth::oauth_flow` as `oauth` =
    /// `:oauth_flow`: `crate::oauth=[redacted]`, closing backtick gone, and a
    /// "credential assignment" counted in the receipt.
    #[test]
    fn a_rust_path_or_a_source_location_is_not_an_assignment() {
        for line in [
            "crate::oauth::open",
            "The bug is in `crate::oauth::oauth_flow`.",
            "secret_guard::resolve::expand",
            "routes::auth::check_token(req)",
            "token_counter::count",
            "token_counter.rs:100-200",
            "see `crates/biorouter/src/token_counter.rs:100-200` and auth.ts:12:5",
        ] {
            let scrubbed = scrub(line, None);
            assert_eq!(scrubbed.text, line, "code was rewritten: {scrubbed:?}");
            assert!(!scrubbed.changed(), "{scrubbed:?}");
            let twice = scrub(&scrubbed.text, None);
            assert_eq!(twice.text, line);
            assert!(twice.findings.is_empty(), "{twice:?}");
        }

        // ⚠ And a path in front of a real assignment does not hide it: the
        // path matches nothing, and `password` matches from its own `\b`.
        let scrubbed = scrub("oauth::password=hunter2xyz", None);
        assert!(!scrubbed.text.contains("hunter2xyz"), "{scrubbed:?}");
        assert_eq!(scrubbed.text, "oauth::password=[redacted]");
    }

    /// Ordering: the vendor pattern must win, so the receipt names the right
    /// severity.
    #[test]
    fn a_vendor_token_inside_an_assignment_is_reported_as_a_vendor_token() {
        let scrubbed = scrub("token=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345", None);
        assert!(
            scrubbed.findings.iter().any(|f| f.kind == "vendor token"),
            "{scrubbed:?}"
        );
        assert!(!scrubbed.text.contains("ABCDEFGH"), "{scrubbed:?}");
    }

    #[test]
    fn a_url_password_and_an_email_are_removed() {
        let scrubbed = scrub(
            "cloned https://wgu:hunter2@git.example.org/x and mailed a@b.example.com",
            None,
        );
        assert!(!scrubbed.text.contains("hunter2"), "{scrubbed:?}");
        assert!(!scrubbed.text.contains("a@b.example.com"), "{scrubbed:?}");
        assert!(
            scrubbed.text.contains("https://[redacted]@git.example.org"),
            "the host is diagnostic and stays: {scrubbed:?}"
        );
    }

    /// A degenerate home must not rewrite every path in the report.
    #[test]
    fn a_root_home_is_ignored() {
        let scrubbed = scrub("/etc/hosts is unreadable", Some(Path::new("/")));
        assert_eq!(scrubbed.text, "/etc/hosts is unreadable");
    }

    #[test]
    fn ordinary_prose_is_left_alone() {
        let text = "The chart renders blank when the dataset has one row. \
                    Reproduced on macOS 15.4 with the Chart.js panel.";
        let scrubbed = scrub(text, Some(Path::new("/Users/jsmith")));
        assert_eq!(scrubbed.text, text);
        assert!(!scrubbed.changed());
        assert_eq!(scrubbed.summary(), "nothing needed redacting");
    }

    fn full_body(extra: &str) -> String {
        format!(
            "**Describe the bug**\n{extra}\n\n**To Reproduce**\n1. x\n\n\
             **Expected behavior**\ny\n\n**Please provide the following information**\n- OS: mac\n"
        )
    }

    #[test]
    fn a_complete_scrubbed_report_passes() {
        assert!(validate_issue(
            "Chart panel renders blank for a single-row dataset",
            &full_body("It renders blank."),
            Some(Path::new("/Users/jsmith")),
        )
        .is_empty());
    }

    #[test]
    fn a_missing_section_is_a_violation_naming_the_section() {
        let violations = validate_issue(
            "Chart panel renders blank for a single-row dataset",
            "**Describe the bug**\nblank\n",
            None,
        );
        assert!(
            violations
                .iter()
                .any(|v| v.rule == "template" && v.detail.contains("To Reproduce")),
            "{violations:?}"
        );
    }

    /// ⚠ The load-bearing test. The scrub is not trusted: validation re-runs it
    /// and refuses anything still recognisable, so a body assembled AFTER the
    /// scrub (a receipt line, a template splice) cannot smuggle a path through.
    #[test]
    fn a_body_that_skipped_the_scrub_is_refused_rather_than_posted() {
        let violations = validate_issue(
            "Ingest fails on a PDF with no text layer",
            &full_body("failed at /Users/jsmith/IRB-2019-441/notes.pdf"),
            Some(Path::new("/Users/jsmith")),
        );
        assert!(
            violations.iter().any(|v| v.rule == "disclosure"),
            "{violations:?}"
        );
    }

    #[test]
    fn a_secret_in_the_title_is_refused_too() {
        let violations = validate_issue(
            "Install fails with ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
            &full_body("x"),
            None,
        );
        assert!(
            violations
                .iter()
                .any(|v| v.rule == "disclosure" && v.detail.contains("title")),
            "{violations:?}"
        );
    }

    #[test]
    fn an_empty_or_enormous_title_is_refused() {
        assert!(validate_issue("bug", &full_body("x"), None)
            .iter()
            .any(|v| v.rule == "title"));
        assert!(validate_issue(&"x".repeat(300), &full_body("x"), None)
            .iter()
            .any(|v| v.rule == "title"));
    }

    #[test]
    fn an_over_long_body_is_refused() {
        let body = format!("{}{}", full_body("x"), "y".repeat(MAX_ISSUE_BODY_CHARS));
        assert!(validate_issue("A perfectly ordinary title", &body, None)
            .iter()
            .any(|v| v.rule == "size"));
    }

    /// The section list this module enforces is the one the repository's own
    /// template actually uses. They live in different files and neither refers
    /// to the other, so a template rename would otherwise produce reports that
    /// silently stop matching it.
    #[test]
    fn the_required_sections_are_the_ones_the_repository_template_declares() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("the crate sits two levels under the repository root")
            .join(".github/ISSUE_TEMPLATE/bug_report.md");
        let Ok(template) = std::fs::read_to_string(&path) else {
            // A consumer vendoring this crate has no `.github/`. Skipping is
            // right; passing vacuously in the REPOSITORY is not, and the
            // repository always has the file.
            return;
        };
        for section in REQUIRED_SECTIONS {
            assert!(
                template.contains(&format!("**{section}**")),
                "`{section}` is not a heading in {}",
                path.display()
            );
        }
    }
}
