use biorouter::elicitation_answer::{answer_problem, field_choices, field_problem};
use console::style;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, IsTerminal, Write};

/// How many answers a field may refuse before the request is given up, so piped input that
/// keeps failing cannot loop forever.
const MAX_ATTEMPTS: usize = 3;

/// Collect the requested fields from the user at the terminal.
///
/// ALL prompt UI renders on **stderr** (matching cliclack, whose prompts
/// also interact via `Term::stderr()`): stdout may be a structured
/// `--output-format json`/`stream-json` document, and prompt text printed
/// there would corrupt it (#40). Only the answers are read from stdin.
pub fn collect_elicitation_input(
    message: &str,
    schema: &Value,
) -> io::Result<Option<HashMap<String, Value>>> {
    if !message.is_empty() {
        eprintln!("\n{}", style(message).cyan());
    }

    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(props) => props,
        None => return Ok(Some(HashMap::new())),
    };

    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut data: HashMap<String, Value> = HashMap::new();

    for (name, field_schema) in properties {
        let is_required = required.contains(&name.as_str());
        let field_type = field_schema
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("string");
        let description = field_schema.get("description").and_then(|d| d.as_str());
        let default = field_schema.get("default");

        // makes a little true/false toggle
        if field_type == "boolean" {
            let label = match description {
                Some(desc) => format!("{} ({})", name, desc),
                None => name.clone(),
            };
            let default_bool = default.and_then(|v| v.as_bool()).unwrap_or(false);

            match cliclack::confirm(&label)
                .initial_value(default_bool)
                .interact()
            {
                Ok(v) => {
                    data.insert(name.clone(), Value::Bool(v));
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => return Ok(None),
                Err(e) => return Err(e),
            }
            continue;
        }

        let choices = field_choices(field_schema);
        if let Some(choices) = &choices {
            let opts: Vec<String> = choices
                .iter()
                .enumerate()
                .map(|(index, (_, shown))| format!("{}) {shown}", index + 1))
                .collect();
            eprintln!("  {}: {}", style("Options").dim(), opts.join(", "));
        }

        let mut attempts = 0;
        let value = loop {
            eprint!("{}", style(name).yellow());
            if let Some(desc) = description {
                eprint!(" {}", style(format!("({})", desc)).dim());
            }
            if is_required {
                eprint!("{}", style("*").red());
            }
            if let Some(def) = default {
                eprint!(" {}", style(format!("[{}]", format_default(def))).dim());
            }
            eprint!(": ");
            io::stderr().flush()?;

            // Ctrl+C / EOF cancels.
            let Some(input) = read_line()? else {
                return Ok(None);
            };
            match interpret_input(name, field_schema, &input) {
                Ok(None) => break default.cloned(),
                Ok(Some(value)) => break Some(value),
                Err(problem) => {
                    attempts += 1;
                    eprintln!("{}", style(&problem).red());
                    if attempts >= MAX_ATTEMPTS {
                        return Ok(None);
                    }
                }
            }
        };

        if let Some(v) = value {
            if !v.is_null() {
                data.insert(name.clone(), v);
            }
        }

        if is_required && !data.contains_key(name) {
            eprintln!(
                "{}",
                style(format!("Required field '{}' is missing", name)).red()
            );
            return Ok(None);
        }
    }

    // The whole answer against the whole form, as the agent will check it on delivery: a
    // form whose rules span fields (or a default that does not fit) is caught here, at the
    // terminal, rather than as an answer the agent refuses after this prompt has closed.
    let answer = Value::Object(data.clone().into_iter().collect());
    if let Some(problem) = answer_problem(schema, &answer) {
        eprintln!("{}", style(&problem).red());
        return Ok(None);
    }

    eprintln!();
    Ok(Some(data))
}

/// What one typed line means for one field: `Ok(None)` for an empty line (the field's default,
/// or nothing), `Ok(Some(value))` for a value that fits the field, and `Err` with what to type
/// instead. A choice field takes the choice's value, its label or its number.
fn interpret_input(name: &str, field: &Value, input: &str) -> Result<Option<Value>, String> {
    if input.is_empty() {
        return Ok(None);
    }
    let field_type = field
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("string");
    let value = parse_value(input, field_type, field_choices(field).as_deref())?;
    match field_problem(name, field, &value) {
        Some(problem) => Err(problem),
        None => Ok(Some(value)),
    }
}

/// One question of an extension's request, as the TUI shows it: a card of fixed choices.
#[derive(Debug, Clone, PartialEq)]
pub struct ElicitationCard {
    /// What the card says above its choices: the request's message and the field.
    pub prompt: String,
    /// Each choice's label and dim hint, in order; the last one declines the request.
    pub options: Vec<(String, String)>,
    /// The choice the card starts on: the field's default, else the first.
    pub selected: usize,
    /// What each option answers, parallel to `options`.
    answers: Vec<CardAnswer>,
    /// The field's key in the answer (`None` for a form with no fields).
    field: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum CardAnswer {
    Value(Value),
    /// Leave an optional field out of the answer.
    Skip,
    /// Continue a form that asks for nothing.
    Continue,
    Decline,
}

impl ElicitationCard {
    /// Whether choosing `option` declines the whole request.
    pub fn declines(&self, option: usize) -> bool {
        matches!(self.answers.get(option), None | Some(CardAnswer::Decline))
    }
}

/// The cards for an elicitation whose every field is a fixed choice or a yes or no question,
/// one per field in the form's order, or `None` when a field needs typed text or a number,
/// which the TUI cannot ask. A form with no fields is one card that continues or declines.
pub fn elicitation_cards(message: &str, schema: &Value) -> Option<Vec<ElicitationCard>> {
    let decline = || {
        (
            "Decline".to_string(),
            "decline this request; the turn continues".to_string(),
        )
    };
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Some(vec![ElicitationCard {
            prompt: message.to_string(),
            options: vec![
                ("Continue".to_string(), "answer the request".to_string()),
                decline(),
            ],
            selected: 0,
            answers: vec![CardAnswer::Continue, CardAnswer::Decline],
            field: None,
        }]);
    };
    if properties.is_empty() {
        return elicitation_cards(message, &Value::Null);
    }
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let count = properties.len();
    let mut cards = Vec::with_capacity(count);
    for (index, (name, field)) in properties.iter().enumerate() {
        let is_required = required.contains(&name.as_str());
        let is_boolean = field.get("type").and_then(Value::as_str) == Some("boolean");
        let choices = match field_choices(field) {
            Some(choices) if !choices.is_empty() => choices,
            _ if is_boolean => vec![
                (Value::Bool(true), "Yes".to_string()),
                (Value::Bool(false), "No".to_string()),
            ],
            _ => return None,
        };
        let default = field.get("default");
        let selected = default
            .and_then(|default| choices.iter().position(|(value, _)| value == default))
            .unwrap_or(0);
        let mut options: Vec<(String, String)> = choices
            .iter()
            .map(|(value, shown)| {
                let hint = match value {
                    Value::String(text) if text != shown => text.clone(),
                    _ => String::new(),
                };
                (shown.clone(), hint)
            })
            .collect();
        let mut answers: Vec<CardAnswer> = choices
            .into_iter()
            .map(|(value, _)| CardAnswer::Value(value))
            .collect();
        if !is_required {
            options.push(("Skip".to_string(), "leave this field empty".to_string()));
            answers.push(CardAnswer::Skip);
        }
        options.push(decline());
        answers.push(CardAnswer::Decline);

        let label = biorouter::elicitation_answer::label(name, field);
        let mut prompt = message.to_string();
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        if count > 1 {
            prompt.push_str(&format!("Question {} of {count}. ", index + 1));
        }
        prompt.push_str(&label);
        if is_required {
            prompt.push_str(" (required)");
        }
        if let Some(description) = field.get("description").and_then(Value::as_str) {
            prompt.push_str(": ");
            prompt.push_str(description);
        }
        cards.push(ElicitationCard {
            prompt,
            options,
            selected,
            answers,
            field: Some(name.clone()),
        });
    }
    Some(cards)
}

/// The answer the chosen options make, `chosen[i]` being card `i`'s option, checked against
/// the form as the agent will check it. `Err` says why it does not fit (then the request is
/// declined rather than sent).
pub fn answer_from_cards(
    cards: &[ElicitationCard],
    chosen: &[usize],
    schema: &Value,
) -> Result<Value, String> {
    let mut answer = serde_json::Map::new();
    for (card, option) in cards.iter().zip(chosen) {
        match (card.answers.get(*option), &card.field) {
            (Some(CardAnswer::Value(value)), Some(field)) => {
                answer.insert(field.clone(), value.clone());
            }
            (Some(CardAnswer::Skip | CardAnswer::Continue), _) => {}
            _ => return Err("The request was declined.".to_string()),
        }
    }
    let answer = Value::Object(answer);
    match answer_problem(schema, &answer) {
        Some(problem) => Err(problem),
        None => Ok(answer),
    }
}

fn read_line() -> io::Result<Option<String>> {
    if !std::io::stdin().is_terminal() {
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        return Ok(Some(line.trim().to_string()));
    }

    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) => Ok(None), // EOF
        Ok(_) => Ok(Some(line.trim().to_string())),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(None),
        Err(e) => Err(e),
    }
}

fn format_default(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => value.to_string(),
    }
}

fn parse_value(
    input: &str,
    field_type: &str,
    choices: Option<&[(Value, String)]>,
) -> Result<Value, String> {
    if let Some(choices) = choices {
        if let Some((value, _)) = choices
            .iter()
            .find(|(value, _)| format_default(value) == input)
            .or_else(|| {
                choices
                    .iter()
                    .find(|(_, shown)| shown.eq_ignore_ascii_case(input))
            })
        {
            return Ok(value.clone());
        }
        if let Ok(index) = input.parse::<usize>() {
            if let Some((value, _)) = index.checked_sub(1).and_then(|i| choices.get(i)) {
                return Ok(value.clone());
            }
        }
        let names: Vec<&str> = choices.iter().map(|(_, shown)| shown.as_str()).collect();
        return Err(format!(
            "Choose one of: {} (or its number).",
            names.join(", ")
        ));
    }

    match field_type {
        "boolean" => match input.to_lowercase().as_str() {
            "true" | "yes" | "y" | "1" => Ok(Value::Bool(true)),
            "false" | "no" | "n" | "0" => Ok(Value::Bool(false)),
            _ => Err("Answer yes or no.".to_string()),
        },
        "integer" => input
            .parse::<i64>()
            .map(|n| Value::Number(n.into()))
            .map_err(|_| "Enter a whole number.".to_string()),
        "number" => input
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| "Enter a number.".to_string()),
        _ => Ok(Value::String(input.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cohort() -> Value {
        json!({
            "type": "string",
            "oneOf": [
                { "const": "discovery", "title": "Discovery" },
                { "const": "replication", "title": "Replication" }
            ]
        })
    }

    /// A typed value outside a field's choices used to be passed through as text and reach
    /// the extension as `accept`. It is now refused with what to type instead.
    #[test]
    fn a_choice_takes_its_value_its_label_or_its_number_and_nothing_else() {
        for typed in ["discovery", "Discovery", "1"] {
            assert_eq!(
                interpret_input("cohort", &cohort(), typed),
                Ok(Some(json!("discovery"))),
                "{typed}"
            );
        }
        assert_eq!(
            interpret_input("cohort", &cohort(), "everyone"),
            Err("Choose one of: Discovery, Replication (or its number).".into())
        );
        assert!(interpret_input("cohort", &cohort(), "3").is_err());
        let legacy = json!({ "type": "string", "enum": ["red", "green"] });
        assert_eq!(
            interpret_input("color", &legacy, "2"),
            Ok(Some(json!("green")))
        );
        assert!(interpret_input("color", &legacy, "blue").is_err());
    }

    #[test]
    fn numbers_and_bounds_are_checked_as_typed() {
        let samples = json!({ "type": "integer", "minimum": 1, "maximum": 10 });
        assert_eq!(
            interpret_input("samples", &samples, "4"),
            Ok(Some(json!(4)))
        );
        assert_eq!(
            interpret_input("samples", &samples, "four"),
            Err("Enter a whole number.".into())
        );
        assert_eq!(
            interpret_input("samples", &samples, "40"),
            Err("'samples' must be at most 10.".into())
        );
        let ratio = json!({ "type": "number" });
        assert_eq!(
            interpret_input("ratio", &ratio, "abc"),
            Err("Enter a number.".into())
        );
        let name = json!({ "type": "string", "title": "Name", "minLength": 3 });
        assert_eq!(
            interpret_input("name", &name, "ab"),
            Err("'Name' must be at least 3 characters long.".into())
        );
    }

    fn form() -> Value {
        json!({
            "type": "object",
            "properties": {
                "cohort": cohort(),
                "confirm": { "type": "boolean", "description": "Run it now?", "default": true },
                "color": { "type": "string", "enum": ["red", "green"], "enumNames": ["Red", "Green"] }
            },
            "required": ["cohort", "confirm"]
        })
    }

    /// The TUI's cards: one per field, each choice by its title, Skip on an optional field,
    /// Decline last, starting on the field's default.
    #[test]
    fn choice_and_yes_or_no_fields_become_cards() {
        let cards = elicitation_cards("Pick a run", &form()).expect("every field is a choice");
        let labels = |card: &ElicitationCard| -> Vec<String> {
            card.options
                .iter()
                .map(|(label, _)| label.clone())
                .collect()
        };
        let by_field: HashMap<_, _> = cards
            .iter()
            .map(|card| (card.field.clone().unwrap(), card.clone()))
            .collect();
        let cohort_card = &by_field["cohort"];
        assert_eq!(labels(cohort_card), ["Discovery", "Replication", "Decline"]);
        assert_eq!(cohort_card.options[0].1, "discovery");
        assert!(cohort_card.prompt.starts_with("Pick a run\n\nQuestion "));
        assert!(cohort_card.prompt.ends_with("cohort (required)"));
        let confirm_card = &by_field["confirm"];
        assert_eq!(labels(confirm_card), ["Yes", "No", "Decline"]);
        assert_eq!(confirm_card.selected, 0, "the default, true");
        assert!(confirm_card.prompt.ends_with(": Run it now?"));
        let color_card = &by_field["color"];
        assert_eq!(labels(color_card), ["Red", "Green", "Skip", "Decline"]);
        assert!(color_card.declines(3) && !color_card.declines(2));
    }

    #[test]
    fn chosen_options_make_an_answer_that_fits_the_form() {
        let schema = form();
        let cards = elicitation_cards("", &schema).unwrap();
        let choose = |pick: &[(&str, usize)]| -> Vec<usize> {
            cards
                .iter()
                .map(|card| {
                    pick.iter()
                        .find(|(field, _)| card.field.as_deref() == Some(*field))
                        .map(|(_, option)| *option)
                        .unwrap()
                })
                .collect()
        };
        let answer = answer_from_cards(
            &cards,
            &choose(&[("cohort", 1), ("confirm", 1), ("color", 2)]),
            &schema,
        )
        .unwrap();
        assert_eq!(answer, json!({ "cohort": "replication", "confirm": false }));
        let answer = answer_from_cards(
            &cards,
            &choose(&[("cohort", 0), ("confirm", 0), ("color", 1)]),
            &schema,
        )
        .unwrap();
        assert_eq!(
            answer,
            json!({ "cohort": "discovery", "confirm": true, "color": "green" })
        );
        assert!(answer_from_cards(
            &cards,
            &choose(&[("cohort", 2), ("confirm", 0), ("color", 0)]),
            &schema
        )
        .is_err());
    }

    /// A field that needs typed text or a number has no card, so the TUI declines that one
    /// request and the turn goes on, rather than cancelling the turn.
    #[test]
    fn a_form_with_a_typed_field_has_no_cards() {
        let schema = json!({
            "type": "object",
            "properties": { "cohort": cohort(), "notes": { "type": "string" } }
        });
        assert_eq!(elicitation_cards("Pick", &schema), None);
        let numeric = json!({ "type": "object", "properties": { "n": { "type": "integer" } } });
        assert_eq!(elicitation_cards("Pick", &numeric), None);
    }

    #[test]
    fn a_form_with_no_fields_continues_or_declines() {
        for schema in [json!({}), json!({ "type": "object", "properties": {} })] {
            let cards = elicitation_cards("Ready?", &schema).unwrap();
            assert_eq!(cards.len(), 1);
            assert_eq!(cards[0].options[0].0, "Continue");
            assert_eq!(answer_from_cards(&cards, &[0], &schema), Ok(json!({})));
            assert!(cards[0].declines(1));
        }
    }

    #[test]
    fn an_empty_line_takes_the_default() {
        assert_eq!(interpret_input("cohort", &cohort(), ""), Ok(None));
    }
}
