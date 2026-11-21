//! Does an answer fit the form an elicitation asked for?
//!
//! An MCP elicitation carries a `requested_schema`: a flat object of named fields, each a
//! string, number, integer, boolean or a choice among fixed values. Every surface that
//! answers one (the desktop card, the CLI prompt, the TUI card, the HTTP route) used to hand
//! whatever it was given straight to the extension, so an answer naming a value outside a
//! field's choices, or a field the form never asked for, reached the extension as `accept`.
//! [`answer_problem`] is the one check every resolution door asks before an answer is
//! delivered; a door that finds a problem refuses the answer and leaves the request waiting.
//!
//! What is checked: the field types, `enum` and `const`, `oneOf`/`anyOf` lists of `const`
//! choices (titled choices), string lengths, number bounds, list lengths and their items,
//! `required`, and fields the form did not ask for. Other keywords (`format`, `pattern`,
//! composed schemas that are not choice lists, `$ref`) are not evaluated, so a schema this
//! reads only partly never refuses an answer for a reason it cannot state. Nothing here
//! resolves a reference or reads a file.
//!
//! Two readings are deliberate, and both are about the form rather than general JSON Schema:
//!
//! * **The top level is closed.** An elicitation's schema is a form, and every answering
//!   surface returns only the fields it lists. rmcp's typed `ElicitationSchema` has no
//!   `additionalProperties` field, so a server's `"additionalProperties": false` never
//!   survives to the card; reading the absence as "open" would let any key through. A field
//!   the form did not list is refused unless the schema says `additionalProperties: true` or
//!   gives a schema for them. A schema that lists no `properties` at all, and nested objects
//!   (outside MCP's flat form), keep JSON Schema's default.
//! * **A blank optional text field is left blank.** The desktop form sends `""` for an
//!   optional field the person did not fill, choice fields included, and that has always
//!   meant "not answered". An empty string for a field that is not required, and that accepts
//!   text, is therefore accepted whatever the field's choices or length bounds.

use serde_json::{Map, Value};

/// Why `answer` does not fit `schema`, in words for the person who gave it, or `None` when
/// it fits.
pub fn answer_problem(schema: &Value, answer: &Value) -> Option<String> {
    let form = schema.as_object()?;
    let is_form = form.contains_key("properties")
        || form.get("type").and_then(Value::as_str) == Some("object");
    if !is_form {
        return value_problem("The answer", schema, answer);
    }
    let Some(fields) = answer.as_object() else {
        return Some("The answer must be a set of named fields.".to_string());
    };
    object_problem(form, fields, true)
}

/// Why `value` does not fit one field's schema, for a surface that asks field by field (the
/// CLI and TUI), or `None` when it fits. `name` is the field's key.
pub fn field_problem(name: &str, field: &Value, value: &Value) -> Option<String> {
    value_problem(&label(name, field), field, value)
}

/// The fixed choices a field offers, each as its value and the label to show, or `None` when
/// the field is not a single choice. Reads `enum` (with the legacy `enumNames` labels) and a
/// `oneOf`/`anyOf` list of `{ const, title }`.
pub fn field_choices(field: &Value) -> Option<Vec<(Value, String)>> {
    let field = field.as_object()?;
    if let Some(choices) = const_choices(field) {
        return Some(choices);
    }
    let values = field.get("enum")?.as_array()?;
    let names = field.get("enumNames").and_then(Value::as_array);
    Some(
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let shown = names
                    .and_then(|names| names.get(index))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| display(value));
                (value.clone(), shown)
            })
            .collect(),
    )
}

/// How a field is named to a person: its `title`, else its key.
pub fn label(name: &str, field: &Value) -> String {
    field
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(name)
        .to_string()
}

fn object_problem(
    form: &Map<String, Value>,
    fields: &Map<String, Value>,
    top: bool,
) -> Option<String> {
    let properties = form.get("properties").and_then(Value::as_object);
    let empty = Map::new();
    let properties = properties.unwrap_or(&empty);
    let required: Vec<&str> = form
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for name in &required {
        if !fields.contains_key(*name) {
            let shown = properties
                .get(*name)
                .map(|field| label(name, field))
                .unwrap_or_else(|| (*name).to_string());
            return Some(format!("'{shown}' is required."));
        }
    }
    for (name, value) in fields {
        match properties.get(name) {
            Some(field) => {
                if !required.contains(&name.as_str()) && is_blank_text(field, value) {
                    continue;
                }
                if let Some(problem) = field_problem(name, field, value) {
                    return Some(problem);
                }
            }
            None => match form.get("additionalProperties") {
                Some(Value::Bool(true)) => {}
                Some(extra @ Value::Object(_)) => {
                    if let Some(problem) = field_problem(name, extra, value) {
                        return Some(problem);
                    }
                }
                Some(Value::Bool(false)) => return Some(not_asked_for(name)),
                // A form that lists its fields is closed at the top level (see the module
                // notes); one that lists none, and anything below the top, is open.
                _ if top && form.contains_key("properties") => return Some(not_asked_for(name)),
                _ => {}
            },
        }
    }
    None
}

fn not_asked_for(name: &str) -> String {
    format!("'{name}' is not a field this request asked for.")
}

/// An empty string for a field that takes text: what a form sends for a field left empty.
fn is_blank_text(field: &Value, value: &Value) -> bool {
    value.as_str() == Some("") && types_of(field).is_none_or(|types| types.contains(&"string"))
}

fn types_of(field: &Value) -> Option<Vec<&str>> {
    types_in(field.as_object()?)
}

fn types_in(schema: &Map<String, Value>) -> Option<Vec<&str>> {
    match schema.get("type")? {
        Value::String(one) => Some(vec![one.as_str()]),
        Value::Array(many) => Some(many.iter().filter_map(Value::as_str).collect()),
        _ => None,
    }
}

fn value_problem(shown: &str, schema: &Value, value: &Value) -> Option<String> {
    let schema = schema.as_object()?;
    if let Some(types) = types_in(schema) {
        if !types.is_empty() && !types.iter().any(|kind| is_of_type(kind, value)) {
            return Some(format!("'{shown}' must be {}.", describe_types(&types)));
        }
    }
    if let Some(choices) = const_choices(schema).or_else(|| {
        schema.get("enum").and_then(Value::as_array).map(|values| {
            values
                .iter()
                .map(|value| (value.clone(), display(value)))
                .collect()
        })
    }) {
        if !choices.iter().any(|(choice, _)| same_value(choice, value)) {
            let names: Vec<&str> = choices.iter().map(|(_, shown)| shown.as_str()).collect();
            return Some(format!("'{shown}' must be one of: {}.", names.join(", ")));
        }
    }
    if let Some(expected) = schema.get("const") {
        if !same_value(expected, value) {
            return Some(format!("'{shown}' must be {}.", display(expected)));
        }
    }
    match value {
        Value::String(text) => text_problem(shown, schema, text),
        Value::Number(_) => number_problem(shown, schema, value.as_f64()?),
        Value::Array(items) => list_problem(shown, schema, items),
        Value::Object(fields) => object_problem(schema, fields, false),
        _ => None,
    }
}

fn text_problem(shown: &str, schema: &Map<String, Value>, text: &str) -> Option<String> {
    let length = text.chars().count() as u64;
    if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
        if length < min {
            return Some(format!("'{shown}' must be at least {min} characters long."));
        }
    }
    if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
        if length > max {
            return Some(format!("'{shown}' must be at most {max} characters long."));
        }
    }
    None
}

fn number_problem(shown: &str, schema: &Map<String, Value>, number: f64) -> Option<String> {
    let bound = |key: &str| schema.get(key).and_then(Value::as_f64);
    if let Some(min) = bound("minimum") {
        if number < min {
            return Some(format!("'{shown}' must be at least {}.", trim_number(min)));
        }
    }
    if let Some(max) = bound("maximum") {
        if number > max {
            return Some(format!("'{shown}' must be at most {}.", trim_number(max)));
        }
    }
    if let Some(min) = bound("exclusiveMinimum") {
        if number <= min {
            return Some(format!("'{shown}' must be more than {}.", trim_number(min)));
        }
    }
    if let Some(max) = bound("exclusiveMaximum") {
        if number >= max {
            return Some(format!("'{shown}' must be less than {}.", trim_number(max)));
        }
    }
    None
}

fn list_problem(shown: &str, schema: &Map<String, Value>, items: &[Value]) -> Option<String> {
    let count = items.len() as u64;
    if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
        if count < min {
            return Some(format!("'{shown}' needs at least {min} choices."));
        }
    }
    if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
        if count > max {
            return Some(format!("'{shown}' takes at most {max} choices."));
        }
    }
    let item_schema = schema.get("items")?;
    items
        .iter()
        .find_map(|item| value_problem(shown, item_schema, item))
}

/// A `oneOf`/`anyOf` whose every entry is a `{ const, title? }` choice.
fn const_choices(schema: &Map<String, Value>) -> Option<Vec<(Value, String)>> {
    let entries = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))?
        .as_array()?;
    if entries.is_empty() {
        return None;
    }
    entries
        .iter()
        .map(|entry| {
            let value = entry.get("const")?.clone();
            let shown = entry
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| display(&value));
            Some((value, shown))
        })
        .collect()
}

fn is_of_type(kind: &str, value: &Value) -> bool {
    match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|n| n.fract() == 0.0)
        }
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        // A type this does not know is not one it can refuse for.
        _ => true,
    }
}

fn describe_types(types: &[&str]) -> String {
    let words: Vec<&str> = types
        .iter()
        .map(|kind| match *kind {
            "string" => "text",
            "number" => "a number",
            "integer" => "a whole number",
            "boolean" => "true or false",
            "array" => "a list",
            "object" => "a set of named fields",
            "null" => "empty",
            other => other,
        })
        .collect();
    words.join(" or ")
}

/// JSON equality, with numbers compared by value (`1` and `1.0` are the same choice).
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

fn display(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn trim_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        number.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn form() -> Value {
        json!({
            "type": "object",
            "properties": {
                "cohort": {
                    "type": "string",
                    "title": "Cohort",
                    "oneOf": [
                        { "const": "discovery", "title": "Discovery" },
                        { "const": "replication", "title": "Replication" }
                    ]
                },
                "color": { "type": "string", "enum": ["red", "green"] },
                "samples": { "type": "integer", "minimum": 1, "maximum": 100 },
                "notes": { "type": "string", "maxLength": 5 },
                "confirm": { "type": "boolean" }
            },
            "required": ["cohort", "confirm"]
        })
    }

    #[test]
    fn a_fitting_answer_has_no_problem() {
        let answer = json!({ "cohort": "discovery", "color": "red", "samples": 12, "notes": "ok", "confirm": true });
        assert_eq!(answer_problem(&form(), &answer), None);
        // A whole number written as a float is still a whole number.
        let answer = json!({ "cohort": "replication", "samples": 12.0, "confirm": false });
        assert_eq!(answer_problem(&form(), &answer), None);
    }

    /// The probe's answer: a value outside the choices and a key the form never asked for.
    #[test]
    fn an_out_of_choice_value_and_an_extra_key_are_refused() {
        let outside = json!({ "cohort": "everyone", "confirm": true });
        assert_eq!(
            answer_problem(&form(), &outside).as_deref(),
            Some("'Cohort' must be one of: Discovery, Replication.")
        );
        let legacy = json!({ "cohort": "discovery", "color": "blue", "confirm": true });
        assert_eq!(
            answer_problem(&form(), &legacy).as_deref(),
            Some("'color' must be one of: red, green.")
        );
        let extra = json!({ "cohort": "discovery", "confirm": true, "admin": true });
        assert_eq!(
            answer_problem(&form(), &extra).as_deref(),
            Some("'admin' is not a field this request asked for.")
        );
    }

    #[test]
    fn types_bounds_and_required_fields_are_checked() {
        let cases = [
            (json!({ "cohort": "discovery" }), "'confirm' is required."),
            (
                json!({ "cohort": "discovery", "confirm": "yes" }),
                "'confirm' must be true or false.",
            ),
            (
                json!({ "cohort": "discovery", "confirm": true, "samples": "12" }),
                "'samples' must be a whole number.",
            ),
            (
                json!({ "cohort": "discovery", "confirm": true, "samples": 1.5 }),
                "'samples' must be a whole number.",
            ),
            (
                json!({ "cohort": "discovery", "confirm": true, "samples": 0 }),
                "'samples' must be at least 1.",
            ),
            (
                json!({ "cohort": "discovery", "confirm": true, "notes": "too long" }),
                "'notes' must be at most 5 characters long.",
            ),
            (
                json!(["discovery"]),
                "The answer must be a set of named fields.",
            ),
        ];
        for (answer, expected) in cases {
            assert_eq!(
                answer_problem(&form(), &answer).as_deref(),
                Some(expected),
                "{answer}"
            );
        }
    }

    /// The desktop form sends `""` for an optional field left empty, choice fields included.
    #[test]
    fn a_blank_optional_text_field_is_left_blank() {
        let answer = json!({ "cohort": "discovery", "color": "", "notes": "", "confirm": true });
        assert_eq!(answer_problem(&form(), &answer), None);
        // A blank required choice is not an answer.
        let answer = json!({ "cohort": "", "confirm": true });
        assert!(answer_problem(&form(), &answer).is_some());
        // A blank is text: a number field still refuses it.
        let answer = json!({ "cohort": "discovery", "samples": "", "confirm": true });
        assert!(answer_problem(&form(), &answer).is_some());
    }

    #[test]
    fn additional_properties_opens_or_types_the_extra_fields() {
        let mut open = form();
        open["additionalProperties"] = json!(true);
        let answer = json!({ "cohort": "discovery", "confirm": true, "extra": 1 });
        assert_eq!(answer_problem(&open, &answer), None);
        let mut typed = form();
        typed["additionalProperties"] = json!({ "type": "string" });
        assert_eq!(
            answer_problem(&typed, &answer).as_deref(),
            Some("'extra' must be text.")
        );
    }

    #[test]
    fn multi_select_lists_are_checked_item_by_item() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tissues": {
                    "type": "array",
                    "minItems": 1,
                    "items": { "type": "string", "enum": ["blood", "brain"] }
                }
            }
        });
        assert_eq!(
            answer_problem(&schema, &json!({ "tissues": ["blood"] })),
            None
        );
        assert_eq!(
            answer_problem(&schema, &json!({ "tissues": ["liver"] })).as_deref(),
            Some("'tissues' must be one of: blood, brain.")
        );
        assert_eq!(
            answer_problem(&schema, &json!({ "tissues": [] })).as_deref(),
            Some("'tissues' needs at least 1 choices.")
        );
    }

    /// A schema with no fields (a bare confirmation) accepts an empty answer, and nothing that
    /// is not a schema refuses anything.
    #[test]
    fn schemas_without_fields_or_constraints_refuse_nothing_they_cannot_state() {
        assert_eq!(answer_problem(&json!({}), &json!({})), None);
        assert_eq!(
            answer_problem(&json!({ "type": "object" }), &json!({})),
            None
        );
        // Listing no fields is not listing a closed set of none.
        assert_eq!(
            answer_problem(&json!({ "type": "object" }), &json!({ "cohort": "x" })),
            None
        );
        assert_eq!(answer_problem(&json!(null), &json!({ "x": 1 })), None);
        let unknown = json!({
            "type": "object",
            "properties": { "when": { "type": "string", "format": "date", "pattern": "^[0-9]+$" } }
        });
        assert_eq!(
            answer_problem(&unknown, &json!({ "when": "tomorrow" })),
            None
        );
    }

    #[test]
    fn choices_read_titles_from_every_shape() {
        let titled = json!({ "type": "string", "oneOf": [{ "const": "a", "title": "Alpha" }, { "const": "b" }] });
        assert_eq!(
            field_choices(&titled),
            Some(vec![(json!("a"), "Alpha".into()), (json!("b"), "b".into())])
        );
        let legacy =
            json!({ "type": "string", "enum": ["a", "b"], "enumNames": ["Alpha", "Beta"] });
        assert_eq!(
            field_choices(&legacy),
            Some(vec![
                (json!("a"), "Alpha".into()),
                (json!("b"), "Beta".into())
            ])
        );
        assert_eq!(field_choices(&json!({ "type": "string" })), None);
    }
}
