use std::fmt::Write;

/// Normalizes how `serde_yaml` outputs multi-line strings.
/// It uses internal heuristics to decide between `|` and quoted text with escaped
/// `\n` and `\"`, and the quoted form breaks MiniJinja parsing.
/// Example before:
///   prompt: "Hello \\\"World\\\"\\n{% if user == \\\"admin\\\" %}Welcome{% endif %}"
/// After fix:
///   prompt: |
///     Hello "World"
///     {% if user == "admin" %}Welcome{% endif %}
///
/// Only top-level keys are rewritten. The same names occur deeper in a
/// workflow: a monitor's `question.instructions`, a frontend extension's
/// `instructions`, a gate question or sub-workflow value someone named
/// `prompt`, and ordinary text inside another field's block scalar. There a
/// literal `\n` is the author's text, and splitting at it would change what
/// they wrote. A line with no indentation is always a top-level key here,
/// because a block scalar's text is indented under its key and `serde_yaml`
/// never wraps a scalar onto a second line.
pub fn reformat_fields_with_multiline_values(yaml: &str, multiline_fields: &[&str]) -> String {
    let mut result = String::new();

    for line in yaml.lines() {
        if line.trim_start().is_empty() {
            writeln!(result).unwrap();
            continue;
        }

        // `line`, not its trimmed form: an indented key is not a top-level one.
        let matched_field = multiline_fields
            .iter()
            .find(|&f| line.starts_with(&format!("{f}: ")));

        if let Some(field) = matched_field {
            if let Some((_, raw_val)) = line.split_once(": ") {
                if raw_val.contains("\\n") {
                    // Clean escaped content and unescape quotes
                    let mut value = raw_val.trim_matches('"').to_string();

                    // Unescape quotes and double backslashes (MiniJinja + newlines)
                    value = value.replace("\\\"", "\"").replace("\\\\n", "\\n");

                    writeln!(result, "{field}: |").unwrap();
                    for l in value.split("\\n") {
                        writeln!(result, "  {l}").unwrap();
                    }
                    continue;
                }
            }
        }

        writeln!(result, "{line}").unwrap();
    }

    let mut output = result.trim_end_matches('\n').to_string();
    output.push('\n');
    output
}

#[cfg(test)]
mod tests {
    use super::reformat_fields_with_multiline_values;

    #[test]
    fn keeps_simple_fields_unchanged() {
        let yaml = "version: \"1.0\"\ntitle: \"Simple\"\nprompt: \"Hello\"";
        let expected = "version: \"1.0\"\ntitle: \"Simple\"\nprompt: \"Hello\"\n";

        let result = reformat_fields_with_multiline_values(yaml, &["prompt"]);
        assert_eq!(result, expected);
    }

    #[test]
    fn converts_multiline_prompt_to_literal_block() {
        let yaml = "version: \"1.0\"\nprompt: \"line1\\\\nline2\"";
        let expected = "version: \"1.0\"\nprompt: |\n  line1\n  line2\n";

        let result = reformat_fields_with_multiline_values(yaml, &["prompt"]);
        assert_eq!(result, expected);
    }

    #[test]
    fn unescapes_quotes_inside_block() {
        let yaml = "prompt: \"Hello \\\"World\\\"\\nHow are you?\"";
        let expected = "prompt: |\n  Hello \"World\"\n  How are you?\n";

        let result = reformat_fields_with_multiline_values(yaml, &["prompt"]);
        assert_eq!(result, expected);
    }

    #[test]
    fn preserves_unlisted_fields() {
        let yaml = "version: \"1.0\"\nprompt: \"line1\\\\nline2\"\nnotes: \"note1\\\\nnote2\"";
        let expected =
            "version: \"1.0\"\nprompt: |\n  line1\n  line2\nnotes: \"note1\\\\nnote2\"\n";

        let result = reformat_fields_with_multiline_values(yaml, &["prompt"]);
        assert_eq!(result, expected);
    }

    #[test]
    fn leaves_indented_nested_fields_alone() {
        let yaml = "settings:\n  prompt: \"line1\\\\nline2\"\n  question:\n    instructions: Is there a literal '\\n'?";
        let expected = format!("{yaml}\n");

        let result = reformat_fields_with_multiline_values(yaml, &["prompt", "instructions"]);
        assert_eq!(result, expected);
    }

    #[test]
    fn leaves_field_like_text_inside_a_block_scalar_alone() {
        let yaml = "prompt: |-\n  Use printf.\n  instructions: keep the literal \\n in the format";
        let expected = format!("{yaml}\n");

        let result = reformat_fields_with_multiline_values(yaml, &["prompt", "instructions"]);
        assert_eq!(result, expected);
    }

    /// A monitor's question, a gate question named `prompt` and a line of a
    /// multi-line prompt that reads like a key all keep a literal `\n`
    /// through `Workflow::to_yaml`, which capture, `/workflows/save` and the
    /// user-values PUT write with.
    #[test]
    fn workflow_to_yaml_keeps_a_literal_backslash_n_in_nested_text() {
        use crate::workflow::Workflow;

        let declared = Workflow::from_content(
            r#"
title: Watched
description: Watched work
prompt: |-
  Print the numbers with printf.
  instructions: keep the literal \n in the format string
decision_settings:
  defaultModel: {provider: test_decision, model: cloud}
  monitors:
    - id: literal
      name: Literal
      question:
        instructions: Does the output contain a literal '\n' sequence?
        kind: {binary: {}}
      threshold: 0.7
      action: stop
      onError: pause
orchestration_plan:
  title: Printing
  phases:
    - {id: print, title: Print}
    - id: check
      title: Check
      kind: verification
      depends_on: [print]
      gate:
        questions: {prompt: Was every line ended with a literal \n?}
        threshold: 0.9
        max_attempts: 2
        model: test_decision/cloud
"#,
        )
        .unwrap();

        let yaml = declared.to_yaml().unwrap();
        let again = Workflow::from_content(&yaml).unwrap();

        assert_eq!(
            again.decision_settings.as_ref().unwrap().monitors[0]
                .question
                .instructions,
            r"Does the output contain a literal '\n' sequence?",
            "{yaml}"
        );
        assert_eq!(
            again.decision_settings, declared.decision_settings,
            "{yaml}"
        );
        assert_eq!(
            again.orchestration_plan, declared.orchestration_plan,
            "{yaml}"
        );
        assert_eq!(again.prompt, declared.prompt, "{yaml}");
        // R2-8: a question's kind is written as the schema reference writes
        // it, never as a YAML tag, and reads back the same.
        assert!(!yaml.contains("!binary"), "no YAML tag: {yaml}");
        assert!(yaml.contains("binary: {}"), "{yaml}");
    }

    #[test]
    fn ignores_existing_literal_blocks() {
        let yaml = "prompt: |\n  already good\n  block";
        let expected = "prompt: |\n  already good\n  block\n";

        let result = reformat_fields_with_multiline_values(yaml, &["prompt"]);
        assert_eq!(result, expected);
    }

    #[test]
    fn ignores_fields_without_newlines() {
        let yaml = "prompt: \"single line text\"";
        let expected = "prompt: \"single line text\"\n";

        let result = reformat_fields_with_multiline_values(yaml, &["prompt"]);
        assert_eq!(result, expected);
    }
}
