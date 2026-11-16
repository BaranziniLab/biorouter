//! A model's name as a person reads it, for text the daemon writes.
//!
//! The renderer names every model through `modelDisplayName`
//! (`ui/desktop/src/components/bottom_menu/modelLabel.ts`): the same Bedrock
//! model reads "Claude Haiku 5.5" whether its id is
//! `us.anthropic.claude-haiku-5-5`, `claude-haiku-5-5` or
//! `anthropic/claude-haiku-5.5`. A sentence the daemon writes and a surface
//! shows as it is (a phase gate's judge, "so GPT-5.5, the chat's model, judged
//! the gate.") must name the model the same way, or one screen reads "GPT-5.5"
//! beside "gpt-5.5". [`model_display_name`] is a step for step port of
//! `modelDisplayName`, and one case table,
//! `ui/desktop/src/components/bottom_menu/modelDisplayName.cases.json`, holds
//! both to the same answers.
//!
//! Only text for a person goes through it. Data keeps the raw id: a record, a
//! wire field, a request and anything a person may need to type or quote.
//!
//! The CLI's `short_model_label` (`commands/session_subagents.rs`) is a
//! different rule, the short id a terminal column fits; this is the name.

use std::sync::LazyLock;

use regex::Regex;

/// A provider's release date at the end of an id: `-2026-07-09` or `-20241022`.
static TRAILING_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-(?:[0-9]{4}-[0-9]{2}-[0-9]{2}|[0-9]{8})$").unwrap());

/// An inference profile's region (`us.`, `eu.`, `apac.`, `global.`), which
/// says the id is Bedrock's whatever its vendor.
const BEDROCK_REGION: &str = "(?:us|us-gov|eu|apac|jp|au|ca|global)";

/// The vendors Bedrock names in an id without a region (`anthropic.`).
const BEDROCK_VENDOR: &str = "(?:amazon|anthropic|ai21|cohere|deepseek|google|luma|meta|minimax|mistral|moonshot|nvidia|openai|qwen|stability|twelvelabs|writer)";

/// A Bedrock id: the region and vendor (group 1), or the vendor alone
/// (group 2), then the model (group 3).
static BEDROCK_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:{BEDROCK_REGION}\.([a-z][a-z0-9-]*)|({BEDROCK_VENDOR}))\.(.+)$"
    ))
    .unwrap()
});

/// Bedrock's API version (`-v1:0`, `-v1`, `-1:0`), after which a date is no
/// longer trailing.
static BEDROCK_VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-(?:v[0-9]+(?::[0-9]+)?|[0-9]+:[0-9]+)$").unwrap());

/// Vertex AI's date (`@20250514`).
static VERTEX_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@(?:[0-9]{8}|[0-9]{4}-[0-9]{2}-[0-9]{2})$").unwrap());

const CLAUDE_TIER: &str = "(opus|sonnet|haiku|fable|mythos)";

/// `claude-opus-5-5`, `claude-haiku-4.5`.
static CLAUDE_NAMED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^claude-{CLAUDE_TIER}-([0-9]+)(?:[-.]([0-9]+))?$"
    ))
    .unwrap()
});

/// `claude-3-5-sonnet`, `claude-3-haiku`.
static CLAUDE_NUMBERED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^claude-([0-9]+)(?:[-.]([0-9]+))?-{CLAUDE_TIER}$"
    ))
    .unwrap()
});

static GPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^gpt-([0-9]+(?:\.[0-9]+)?[a-z]?)((?:-[a-z]+)*)$").unwrap());

static GEMINI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^gemini-([0-9]+(?:\.[0-9]+)?)((?:-[a-z]+)*)$").unwrap());

/// A local tag with nothing after its size: `qwen3.8:27b`, `qwen3.8-27b`.
static QWEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^qwen([0-9]+(?:\.[0-9]+)?)[:-]([0-9]+(?:\.[0-9]+)?)b$").unwrap());

/// `gemma4:31b`, `gemma4-31b`.
static GEMMA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^gemma([0-9]+(?:\.[0-9]+)?)[:-]([0-9]+(?:\.[0-9]+)?)b$").unwrap()
});

/// Databricks writes a dash where the vendor writes a dot
/// (`databricks-gpt-5-6-sol`). The renderer spells the end as a lookahead
/// (`(?=-|$)`), which this engine has not got, so the dash or the end is
/// matched (group 4) and written back.
static DATABRICKS_VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(gpt|gemini)-([0-9]+)-([0-9]+)(-|$)").unwrap());

/// Words OpenAI writes in lower case after the version (`GPT-5 mini`).
const GPT_LOWER: [&str; 2] = ["mini", "nano"];

/// The predefined model list a deployment sets, as the renderer reads it.
const PREDEFINED_MODELS: &str = "BIOROUTER_PREDEFINED_MODELS";

/// The model's name as a person reads it: "Claude Haiku 5.5" for
/// `us.anthropic.claude-haiku-5-5`, "GPT-5.5" for `gpt-5.5-2026-04-24`,
/// "Qwen3.8 27B" for `qwen3.8:27b`. In order:
///
/// 1. the alias a deployment gives the model (`BIOROUTER_PREDEFINED_MODELS`);
/// 2. the family's own written name, for Claude, GPT, Gemini, Qwen and Gemma;
/// 3. otherwise the short id: no date stamp, no Bedrock profile or API
///    version, no `vendor/` path, no Vertex `@date`.
///
/// A name handed back in (it has a space in it) comes out as it went in, so
/// asking twice changes nothing.
pub fn model_display_name(id: &str) -> String {
    display_name(id, predefined_alias)
}

/// [`model_display_name`] with the alias step asked of `alias`, so the case
/// table can be read with no deployment's list in force.
fn display_name(id: &str, alias: impl Fn(&str) -> Option<String>) -> String {
    let trimmed = js_trim(id);
    if trimmed.is_empty() {
        return id.to_string();
    }
    // No model id has a space in it, and every name does ("Claude Haiku 5.5",
    // "Qwen3.8 27B", an alias), so a name handed back in comes out as it went in.
    if trimmed.chars().any(is_js_whitespace) {
        return trimmed.to_string();
    }
    if let Some(alias) = alias(trimmed).filter(|alias| alias != trimmed) {
        return alias;
    }
    let short = short_model_id(trimmed);
    family_name(&short).unwrap_or(short)
}

/// The deployment's alias for `name`, as the renderer's `getModelDisplayName`
/// reads it: the first entry named exactly so, when its `alias` is not empty.
fn predefined_alias(name: &str) -> Option<String> {
    let list = std::env::var(PREDEFINED_MODELS).ok()?;
    alias_in(&list, name)
}

fn alias_in(list: &str, name: &str) -> Option<String> {
    let models: Vec<serde_json::Value> = serde_json::from_str(list).ok()?;
    let entry = models
        .iter()
        .find(|model| model.get("name").and_then(serde_json::Value::as_str) == Some(name))?;
    entry
        .get("alias")
        .and_then(serde_json::Value::as_str)
        .filter(|alias| !alias.is_empty())
        .map(str::to_string)
}

/// The characters JavaScript's `trim()` and `\s` take as space (WhiteSpace and
/// LineTerminator): Rust's own set also holds U+0085 and lacks U+FEFF, and the
/// two must not name one id two ways.
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

fn js_trim(id: &str) -> &str {
    id.trim_matches(is_js_whitespace)
}

fn without_date(name: &str) -> String {
    let stripped = TRAILING_DATE.replace(name, "");
    if stripped.is_empty() {
        name.to_string()
    } else {
        stripped.into_owned()
    }
}

/// The renderer's `friendlyModelName`: a trailing date comes off, and a
/// Bedrock id's region, vendor and API version too. A name left as one word
/// keeps its vendor (`deepseek.r1-v1:0` reads `deepseek-r1`).
fn friendly_model_name(id: &str) -> String {
    let trimmed = js_trim(id);
    let Some(bedrock) = BEDROCK_ID.captures(trimmed) else {
        return without_date(trimmed);
    };
    let vendor = bedrock
        .get(1)
        .or_else(|| bedrock.get(2))
        .map_or("", |m| m.as_str());
    let model = bedrock.get(3).map_or("", |m| m.as_str());
    let unversioned = BEDROCK_VERSION.replace(model, "");
    let name = without_date(if unversioned.is_empty() {
        model
    } else {
        &unversioned
    });
    if name.contains('-') {
        name
    } else {
        format!("{vendor}-{name}")
    }
}

/// The id without what only says where it is served: the friendly name's last
/// `vendor/` path segment, without Vertex AI's `@date`. An Ollama tag keeps
/// all of itself, `:latest` included.
fn short_model_id(id: &str) -> String {
    let friendly = friendly_model_name(id);
    let last = friendly
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(friendly.as_str());
    let short = VERTEX_DATE.replace(last, "");
    if short.is_empty() {
        last.to_string()
    } else {
        short.into_owned()
    }
}

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Databricks' endpoint prefix comes off, and its dash between a GPT or
/// Gemini version's numbers reads as the vendor's dot. Only there: OpenAI's
/// own `gpt-4-0613` is a dated GPT-4, not a GPT-4.0613.
fn without_databricks_prefix(id: &str) -> String {
    let Some(rest) = id.strip_prefix("databricks-") else {
        return id.to_string();
    };
    DATABRICKS_VERSION
        .replace(rest, |caps: &regex::Captures| {
            format!("{}-{}.{}{}", &caps[1], &caps[2], &caps[3], &caps[4])
        })
        .into_owned()
}

/// `major.minor`, or `major` alone.
fn version(major: &str, minor: Option<regex::Match>) -> String {
    match minor {
        Some(minor) => format!("{major}.{}", minor.as_str()),
        None => major.to_string(),
    }
}

/// The family's own written name, for the families whose naming is regular.
fn family_name(short: &str) -> Option<String> {
    let id = without_databricks_prefix(&short.to_lowercase());
    if let Some(named) = CLAUDE_NAMED.captures(&id) {
        return Some(format!(
            "Claude {} {}",
            capitalise(&named[1]),
            version(&named[2], named.get(3))
        ));
    }
    if let Some(numbered) = CLAUDE_NUMBERED.captures(&id) {
        return Some(format!(
            "Claude {} {}",
            version(&numbered[1], numbered.get(2)),
            capitalise(&numbered[3])
        ));
    }
    if let Some(gpt) = GPT.captures(&id) {
        let version = &gpt[1];
        let words: Vec<&str> = gpt[2].split('-').filter(|w| !w.is_empty()).collect();
        // OpenAI joins the Codex line with hyphens: `GPT-5.3-Codex`.
        if words.first() == Some(&"codex") {
            let joined: Vec<String> = words.iter().copied().map(capitalise).collect();
            return Some(format!("GPT-{version}-{}", joined.join("-")));
        }
        let mut parts = vec![format!("GPT-{version}")];
        parts.extend(words.iter().copied().map(|w| {
            if GPT_LOWER.contains(&w) {
                w.to_string()
            } else {
                capitalise(w)
            }
        }));
        return Some(parts.join(" "));
    }
    if let Some(gemini) = GEMINI.captures(&id) {
        let mut parts = vec![format!("Gemini {}", &gemini[1])];
        parts.extend(
            gemini[2]
                .split('-')
                .filter(|w| !w.is_empty())
                .map(capitalise),
        );
        return Some(parts.join(" "));
    }
    if let Some(qwen) = QWEN.captures(&id) {
        return Some(format!("Qwen{} {}B", &qwen[1], &qwen[2]));
    }
    if let Some(gemma) = GEMMA.captures(&id) {
        return Some(format!("Gemma {} {}B", &gemma[1], &gemma[2]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Table {
        groups: Vec<Group>,
    }

    #[derive(serde::Deserialize)]
    struct Group {
        cases: Vec<(String, String)>,
    }

    fn table() -> Vec<(String, String)> {
        let table: Table = serde_json::from_str(include_str!(
            "../../../../ui/desktop/src/components/bottom_menu/modelDisplayName.cases.json"
        ))
        .expect("the shared case table parses");
        table.groups.into_iter().flat_map(|g| g.cases).collect()
    }

    /// Every case the renderer's `modelDisplayName` passes, this passes too
    /// (`modelLabel.test.ts` reads the same table), with no deployment alias.
    #[test]
    fn model_display_name_matches_the_desktop_case_table() {
        let cases = table();
        assert!(cases.len() > 80, "{} cases", cases.len());
        let wrong: Vec<String> = cases
            .iter()
            .filter_map(|(id, name)| {
                let got = display_name(id, |_| None);
                (&got != name).then(|| format!("{id:?}: expected {name:?}, got {got:?}"))
            })
            .collect();
        assert!(
            wrong.is_empty(),
            "{} of {}:\n{}",
            wrong.len(),
            cases.len(),
            wrong.join("\n")
        );
    }

    #[test]
    fn asking_twice_changes_nothing() {
        for (id, _) in table() {
            let name = display_name(&id, |_| None);
            assert_eq!(display_name(&name, |_| None), name, "{id:?}");
        }
    }

    /// A deployment's alias wins, as the renderer's does; an empty one, or an
    /// entry for another model, is no alias.
    #[test]
    fn a_deployments_alias_wins() {
        let list = r#"[
            {"name": "us.anthropic.claude-haiku-5-5", "alias": "Lab Haiku"},
            {"name": "gpt-5.5", "alias": ""},
            {"name": "gpt-5.5", "alias": "Second"}
        ]"#;
        let alias = |name: &str| alias_in(list, name);
        assert_eq!(
            display_name("us.anthropic.claude-haiku-5-5", alias),
            "Lab Haiku"
        );
        assert_eq!(
            display_name(" us.anthropic.claude-haiku-5-5 ", alias),
            "Lab Haiku"
        );
        // The first entry named so decides, and its alias is empty.
        assert_eq!(display_name("gpt-5.5", alias), "GPT-5.5");
        assert_eq!(display_name("claude-haiku-5-5", alias), "Claude Haiku 5.5");
        assert_eq!(alias_in("not json", "gpt-5.5"), None);
        assert_eq!(alias_in(r#"{"name": "gpt-5.5"}"#, "gpt-5.5"), None);
    }
}
