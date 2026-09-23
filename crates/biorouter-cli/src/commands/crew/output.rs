use super::args::OutputFormat;
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::Path;

const MAX_INPUT: usize = 1_048_576;

pub fn stream_format(format: OutputFormat) -> OutputFormat {
    match format {
        OutputFormat::Text => OutputFormat::Text,
        OutputFormat::Json | OutputFormat::StreamJson => OutputFormat::StreamJson,
    }
}

fn terminal_control(ch: char) -> bool {
    ch.is_control() || biorouter::utils::is_invisible_formatting(ch)
}

pub fn safe_text(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| {
            if terminal_control(ch) {
                ch.escape_default().collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect()
}

pub fn emit(value: &Value, format: OutputFormat) -> Result<()> {
    let output = match format {
        OutputFormat::Json => json_terminal_safe(serde_json::to_string_pretty(value)?),
        OutputFormat::StreamJson => json_terminal_safe(serde_json::to_string(value)?),
        OutputFormat::Text => human(value),
    };
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{output}")?;
    stdout.flush()?;
    Ok(())
}

fn json_terminal_safe(value: String) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if terminal_control(ch) && ch != '\n' && ch != '\r' && ch != '\t' {
            use std::fmt::Write;
            let mut units = [0; 2];
            for unit in ch.encode_utf16(&mut units) {
                let _ = write!(out, "\\u{unit:04x}");
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn human(value: &Value) -> String {
    match value {
        Value::Array(items) if items.is_empty() => "No items.".into(),
        Value::Array(items) => items.iter().map(human).collect::<Vec<_>>().join("\n"),
        Value::Object(fields) => {
            if let Some(Value::Array(messages)) = fields.get("messages") {
                let mut out = messages.iter().map(human).collect::<Vec<_>>().join("\n");
                if let Some(cursor) = fields.get("cursor").and_then(Value::as_str) {
                    out.push_str(&format!("\nCursor: {}", safe_text(cursor)));
                }
                return out;
            }
            if let Some(body) = fields.get("body").and_then(Value::as_str) {
                let actor = fields
                    .get("actor_id")
                    .and_then(Value::as_str)
                    .unwrap_or("message");
                let id = fields.get("id").and_then(Value::as_str).unwrap_or("");
                let mut out = format!(
                    "{} [{}]: {}",
                    safe_text(actor),
                    safe_text(id),
                    safe_text(body)
                );
                for field in ["attachments", "references"] {
                    if let Some(Value::Array(items)) = fields.get(field) {
                        if !items.is_empty() {
                            out.push_str(&format!(
                                "\n  {field}: {}",
                                items.iter().map(human).collect::<Vec<_>>().join(", ")
                            ));
                        }
                    }
                }
                for field in ["run_id", "status"] {
                    if let Some(value) = fields.get(field).and_then(Value::as_str) {
                        out.push_str(&format!("\n  {field}: {}", safe_text(value)));
                    }
                }
                if fields.get("restricted").and_then(Value::as_bool) == Some(true) {
                    out.push_str("\n  Restricted context");
                }
                return out;
            }
            fields
                .iter()
                .map(|(key, value)| {
                    let rendered = match value {
                        Value::String(value) => safe_text(value),
                        Value::Array(items) => {
                            items.iter().map(human).collect::<Vec<_>>().join("\n  ")
                        }
                        Value::Object(_) => {
                            json_terminal_safe(serde_json::to_string(value).unwrap_or_default())
                        }
                        other => other.to_string(),
                    };
                    format!("{}: {}", safe_text(key), rendered)
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        Value::String(value) => safe_text(value),
        other => other.to_string(),
    }
}

pub fn read_input(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    if path == Path::new("-") {
        std::io::stdin()
            .lock()
            .take((MAX_INPUT + 1) as u64)
            .read_to_end(&mut bytes)?;
    } else {
        std::fs::File::open(path)
            .context("Could not open Crew input file")?
            .take((MAX_INPUT + 1) as u64)
            .read_to_end(&mut bytes)?;
    }
    ensure!(bytes.len() <= MAX_INPUT, "Crew input exceeds one MiB");
    String::from_utf8(bytes).context("Crew input must be UTF-8")
}

pub fn component(value: &str) -> Result<&str> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "Crew IDs must contain 1–128 letters, digits, underscores or hyphens"
    );
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn terminal_controls_are_escaped_without_changing_json_shape() {
        let value = json!({"request_id":"req-1\u{202e}tail", "ok":true});
        let encoded = serde_json::to_string(&value).unwrap();
        let safe = json_terminal_safe(encoded);
        assert!(safe.contains("\\u202e"));
        assert!(!safe.contains('\u{202e}'));
    }

    #[test]
    fn terminal_controls_escape_without_losing_emoji_or_non_ascii_text() {
        let input = "safe🙂 café\u{202e}bidi\u{200b}zero\u{feff}bom\u{e0041}tag\n\x1b";
        let escaped = safe_text(input);
        assert!(escaped.contains("safe🙂 café"));
        assert!(escaped.contains("\\u{202e}"));
        assert!(escaped.contains("\\u{200b}"));
        assert!(escaped.contains("\\u{feff}"));
        assert!(escaped.contains("\\u{e0041}"));
        assert!(escaped.contains("\\n"));
        assert!(escaped.contains("\\u{1b}"));
        assert!(escaped.chars().all(|ch| !terminal_control(ch)));
    }

    #[test]
    fn json_terminal_safe_round_trips_invisible_non_bmp_tags() {
        let value = json!({
            "text": "emoji🙂 café\u{202e}bidi\u{200b}zero\u{feff}bom\u{e0041}tag\n",
            "plain": "東京"
        });
        let encoded = serde_json::to_string(&value).unwrap();
        let safe = json_terminal_safe(encoded);
        let reparsed: serde_json::Value = serde_json::from_str(&safe).unwrap();
        assert_eq!(reparsed, value);
        assert!(safe.contains("emoji🙂 café"));
        assert!(safe.contains("東京"));
        assert!(safe.chars().all(|ch| !terminal_control(ch)));
        for ch in ['\u{202e}', '\u{200b}', '\u{feff}', '\u{e0041}'] {
            assert!(
                !safe.contains(ch),
                "raw invisible character {ch:?} survived"
            );
        }
    }

    #[test]
    fn component_accepts_stable_ids_and_refuses_terminal_or_empty_values() {
        assert_eq!(component("request_01-abc").unwrap(), "request_01-abc");
        let oversized = "x".repeat(129);
        for invalid in ["", "bad/id", "bad\nline", oversized.as_str()] {
            assert!(component(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn read_input_is_bounded_and_requires_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let valid = dir.path().join("valid");
        std::fs::write(&valid, b"hello").unwrap();
        assert_eq!(read_input(&valid).unwrap(), "hello");
        let invalid = dir.path().join("invalid");
        std::fs::write(&invalid, [0xff]).unwrap();
        assert!(read_input(&invalid).is_err());
        let oversized = dir.path().join("oversized");
        let mut file = std::fs::File::create(&oversized).unwrap();
        file.write_all(&vec![b'x'; MAX_INPUT + 1]).unwrap();
        assert!(read_input(&oversized).is_err());
    }
}
