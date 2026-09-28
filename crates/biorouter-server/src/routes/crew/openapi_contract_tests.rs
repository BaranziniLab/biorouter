//! The `/crew` routes' OpenAPI contract (CROSSCUT-6).
//!
//! The generated TypeScript client, and the no-drift gate that regenerates it, protect a wire
//! shape only when the spec states it. `/crew` once declared most answers as `{}` and only
//! their 200, while the routes answered typed refusals the CLI and the desktop branch on. These
//! tests hold the spec to what the routes do: every operation names its answer and its
//! refusals, every refusal is a [`CrewError`], every code a Crew source can answer appears in
//! the spec, and every field a refusal can carry is one [`CrewError`] declares.
//!
//! Each detector runs on a bad fixture first, so a check cannot pass by failing to look.
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::wire::CrewError;

const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];

/// Where a schema reference points, when `schema` is one: a `$ref`, or a nullable `allOf` of
/// exactly one.
fn referenced(schema: &Value) -> Option<&str> {
    if let Some(reference) = schema["$ref"].as_str() {
        return Some(reference);
    }
    match schema["allOf"].as_array().map(Vec::as_slice) {
        Some([only]) => only["$ref"].as_str(),
        _ => None,
    }
}

/// The names a path template interpolates: `{id}` and `{run_id}` in
/// `/crew/connections/{id}/runs/{run_id}/cancel`.
fn template_parameters(path: &str) -> BTreeSet<String> {
    path.split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}').map(|(name, _)| name.to_owned()))
        .collect()
}

/// Every way the spec's `/crew` operations fall short of the contract, one line each.
fn crew_contract_violations(spec: &Value) -> Vec<String> {
    let mut violations = Vec::new();
    let Some(paths) = spec["paths"].as_object() else {
        return vec!["the spec has no paths".into()];
    };
    for (path, operations) in paths.iter().filter(|(path, _)| path.starts_with("/crew")) {
        let template = template_parameters(path);
        if template.contains("session") {
            violations.push(format!(
                "{path}: a chat is `{{session_id}}`, never `{{session}}`"
            ));
        }
        for method in METHODS {
            let operation = &operations[method];
            if operation.is_null() {
                continue;
            }
            let at = format!("{} {path}", method.to_uppercase());
            if !operation["operationId"]
                .as_str()
                .is_some_and(|id| id.starts_with("crew_"))
            {
                violations.push(format!("{at}: its operationId does not start with crew_"));
            }
            let declared: BTreeSet<String> = operation["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|parameter| parameter["in"] == "path")
                .filter_map(|parameter| parameter["name"].as_str().map(str::to_owned))
                .collect();
            if declared != template {
                violations.push(format!(
                    "{at}: declares path parameters {declared:?} for {template:?}"
                ));
            }
            if let Some(body) = operation["requestBody"]["content"].as_object() {
                for (media, content) in body {
                    if referenced(&content["schema"]).is_none() {
                        violations.push(format!("{at}: its {media} request body is untyped"));
                    }
                }
            }
            let Some(responses) = operation["responses"].as_object() else {
                violations.push(format!("{at}: declares no responses"));
                continue;
            };
            if !responses.contains_key("403") {
                violations.push(format!("{at}: does not document its person gate's 403"));
            }
            if !responses.keys().any(|status| status.starts_with('4')) {
                violations.push(format!("{at}: documents no refusal"));
            }
            for (status, response) in responses {
                let content = response["content"].as_object();
                if status.starts_with('2') {
                    match content {
                        Some(content) if !content.is_empty() => {
                            for (media, body) in content {
                                if referenced(&body["schema"]).is_none() {
                                    violations
                                        .push(format!("{at}: its {status} {media} is untyped"));
                                }
                            }
                        }
                        _ => violations.push(format!("{at}: its {status} names no body")),
                    }
                } else if status.starts_with('4') || status.starts_with('5') {
                    let refusal = content
                        .and_then(|content| content.get("application/json"))
                        .and_then(|body| referenced(&body["schema"]));
                    if refusal != Some("#/components/schemas/CrewError") {
                        violations.push(format!("{at}: its {status} is not a CrewError"));
                    }
                    if response["description"]
                        .as_str()
                        .is_none_or(|text| text.trim().is_empty())
                    {
                        violations.push(format!("{at}: its {status} does not say why"));
                    }
                }
            }
        }
    }
    violations.extend(dangling_references(spec));
    violations
}

/// Every `$ref` in `spec` that names no schema it declares.
fn dangling_references(spec: &Value) -> Vec<String> {
    fn walk(value: &Value, found: &mut BTreeSet<String>) {
        match value {
            Value::Object(fields) => {
                if let Some(reference) = fields.get("$ref").and_then(Value::as_str) {
                    found.insert(reference.to_owned());
                }
                fields.values().for_each(|value| walk(value, found));
            }
            Value::Array(items) => items.iter().for_each(|value| walk(value, found)),
            _ => {}
        }
    }
    let mut references = BTreeSet::new();
    walk(spec, &mut references);
    references
        .into_iter()
        .filter(|reference| {
            reference
                .strip_prefix("#/components/schemas/")
                .is_none_or(|name| spec["components"]["schemas"][name].is_null())
        })
        .map(|reference| format!("{reference} names no schema"))
        .collect()
}

fn spec() -> Value {
    serde_json::from_str(&crate::openapi::generate_schema()).expect("the spec is JSON")
}

#[test]
fn the_contract_check_finds_what_the_old_crew_spec_did_wrong() {
    // `/crew` as it was declared: a 200 of `{}`, a `{}` body, a second spelling of a chat, a
    // generic operation name, a refusal with no schema, and a `$ref` to nothing.
    let bad = json!({
        "paths": {
            "/crew/connections/{id}/sessions/{session}/revoke": {"post": {
                "operationId": "revoke",
                "parameters": [
                    {"name": "id", "in": "path"},
                    {"name": "session", "in": "path"}
                ],
                "requestBody": {"content": {"application/json": {"schema": {}}}},
                "responses": {"200": {"content": {"application/json": {"schema": {}}}}}
            }},
            "/crew/files/{capability_id}": {"delete": {
                "operationId": "crew_transfer_discard_file",
                "responses": {
                    "200": {"content": {"application/json": {
                        "schema": {"$ref": "#/components/schemas/Gone"}
                    }}},
                    "403": {"description": "", "content": {"application/json": {"schema": {}}}}
                }
            }}
        },
        "components": {"schemas": {}}
    });
    let found = crew_contract_violations(&bad).join("\n");
    for expected in [
        "never `{session}`",
        "operationId does not start with crew_",
        "request body is untyped",
        "200 application/json is untyped",
        "does not document its person gate's 403",
        "documents no refusal",
        "declares path parameters {} for {\"capability_id\"}",
        "403 is not a CrewError",
        "403 does not say why",
        "#/components/schemas/Gone names no schema",
    ] {
        assert!(found.contains(expected), "missed {expected:?} in:\n{found}");
    }
}

#[test]
fn every_crew_operation_names_its_answer_and_its_refusals() {
    let spec = spec();
    let operations = spec["paths"]
        .as_object()
        .expect("paths")
        .iter()
        .filter(|(path, _)| path.starts_with("/crew"))
        .flat_map(|(_, operations)| METHODS.iter().filter(|m| !operations[**m].is_null()))
        .count();
    // A check that reads nothing would agree with one that finds nothing.
    assert!(
        operations >= 42,
        "only {operations} /crew operations were read"
    );
    let violations = crew_contract_violations(&spec);
    assert!(
        violations.is_empty(),
        "the /crew OpenAPI contract is broken:\n{}",
        violations.join("\n")
    );
}

/// The Crew sources a `/crew` answer or refusal is written in.
fn crew_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let rust_files = |dir: PathBuf| -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
            .collect();
        files.sort();
        files
    };
    let mut sources = rust_files(root.join("../biorouter/src/crew"));
    sources.extend(rust_files(root.join("src/crew")));
    // Not this file, whose fixtures are made of the very words it scans for.
    sources.extend(
        rust_files(root.join("src/routes/crew"))
            .into_iter()
            .filter(|path| !path.ends_with("openapi_contract_tests.rs")),
    );
    sources.extend(
        rust_files(root.join("src/routes"))
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("crew"))
            }),
    );
    sources
}

/// Every quoted `crew_*` word in `source`.
fn crew_codes(source: &str) -> BTreeSet<String> {
    source
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|word| {
            word.len() > "crew_".len()
                && word.starts_with("crew_")
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
        .map(str::to_owned)
        .collect()
}

/// Quoted `crew_*` words in the Crew sources that are not `/crew` codes.
const NOT_CREW_CODES: [(&str, &str); 7] = [
    ("crew__", "the prefix of the Crew tools' names"),
    ("crew__request", "a Crew tool's name"),
    (
        "crew_run_stopped",
        "the code a task's own chat reads when the task stops",
    ),
    ("crew_bob", "a test account"),
    ("crew_dave", "a test account"),
    ("crew_gina", "a test account"),
    ("crew_example", "a test refusal's code"),
];

#[test]
fn the_code_scan_reads_quoted_crew_words_only() {
    let source = r#"Self::new("crew_not_sent", "text"); let x = "crew_"; "Crew_Word" // crew_bare"#;
    assert_eq!(
        crew_codes(source),
        BTreeSet::from(["crew_not_sent".to_owned()])
    );
}

/// The spec's `operationId`s, which are quoted `crew_*` words in the sources too.
fn operation_ids(spec: &Value) -> BTreeSet<String> {
    spec["paths"]
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(_, operations)| METHODS.iter().map(move |method| &operations[*method]))
        .filter_map(|operation| operation["operationId"].as_str().map(str::to_owned))
        .collect()
}

/// The values [`CrewErrorCode`](super::wire::CrewErrorCode) declares.
fn declared_codes() -> BTreeSet<String> {
    let (_, schema) = <super::wire::CrewErrorCode as utoipa::ToSchema>::schema();
    serde_json::to_value(schema).expect("its schema")["enum"]
        .as_array()
        .expect("an enum")
        .iter()
        .filter_map(|code| code.as_str().map(str::to_owned))
        .collect()
}

/// The codes a Crew answer carries that are not `crew_*` words, so no source scan can find
/// them: the resolver's two and the observer limit's, which share the workspace's vocabulary.
const UNPREFIXED_CODES: [&str; 3] = [
    "unknown_name",
    "ambiguous_name",
    "observer_capacity_reached",
];

#[test]
fn every_code_a_crew_source_answers_is_a_declared_crew_error_code() {
    let sources = crew_sources();
    let mut codes = BTreeSet::new();
    for source in &sources {
        let text = std::fs::read_to_string(source).expect("a Crew source");
        codes.extend(crew_codes(&text));
    }
    for (word, _) in NOT_CREW_CODES {
        codes.remove(word);
    }
    let spec = spec();
    for id in operation_ids(&spec) {
        codes.remove(&id);
    }
    assert!(
        codes.len() > 60,
        "only {} codes were read from {} sources",
        codes.len(),
        sources.len()
    );
    let declared = declared_codes();
    let undeclared: Vec<&String> = codes.difference(&declared).collect();
    assert!(
        undeclared.is_empty(),
        "a Crew source answers codes CrewErrorCode does not declare: {undeclared:?}"
    );
    codes.extend(UNPREFIXED_CODES.map(str::to_owned));
    let stale: Vec<&String> = declared.difference(&codes).collect();
    assert!(
        stale.is_empty(),
        "CrewErrorCode declares codes no Crew source answers any more: {stale:?}"
    );
}

#[test]
fn every_crew_error_code_is_explained_where_it_is_answered() {
    let text = crate::openapi::generate_schema();
    let unexplained: Vec<String> = declared_codes()
        .into_iter()
        .filter(|code| !text.contains(&format!("`{code}`")))
        .collect();
    assert!(
        unexplained.is_empty(),
        "no route or schema description says when these codes are answered: {unexplained:?}"
    );
}

/// Every field a Crew refusal adds beside `code` and `error`, from its `.with("…"` calls.
fn refusal_fields(source: &str) -> BTreeSet<String> {
    source
        .split(".with(")
        .skip(1)
        .filter_map(|rest| {
            let rest = rest.trim_start().strip_prefix('"')?;
            let (field, _) = rest.split_once('"')?;
            Some(field.to_owned())
        })
        .filter(|field| field != "code" && field != "error")
        .collect()
}

#[test]
fn every_field_a_crew_refusal_carries_is_one_crew_error_declares() {
    assert_eq!(
        refusal_fields(r#"x.with("host", h).with( "code", c).with(key, v)"#),
        BTreeSet::from(["host".to_owned()])
    );
    let (_, schema) = <CrewError as utoipa::ToSchema>::schema();
    let schema = serde_json::to_value(schema).expect("CrewError's schema");
    let declared: BTreeSet<String> = schema["properties"]
        .as_object()
        .expect("CrewError has properties")
        .keys()
        .cloned()
        .collect();
    let mut fields = BTreeSet::new();
    for source in crew_sources() {
        fields.extend(refusal_fields(
            &std::fs::read_to_string(&source).expect("a Crew source"),
        ));
    }
    assert!(
        fields.len() > 20,
        "only {} refusal fields were read",
        fields.len()
    );
    let undeclared: Vec<&String> = fields.difference(&declared).collect();
    assert!(
        undeclared.is_empty(),
        "a Crew refusal carries fields CrewError does not declare: {undeclared:?}"
    );
    assert_eq!(
        schema["required"],
        json!(["code", "error"]),
        "every refusal carries exactly these two"
    );
}

#[test]
fn the_institution_refusal_schema_names_what_the_core_sends() {
    let details = biorouter::crew::institution_refusal_details("model", None, None, None);
    let sent: BTreeSet<String> = details
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    let (_, schema) = <super::wire::CrewInstitutionRefusal as utoipa::ToSchema>::schema();
    let schema = serde_json::to_value(schema).expect("its schema");
    let declared: BTreeSet<String> = schema["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .cloned()
        .collect();
    assert_eq!(sent, declared);
}

/// A body a Crew route cannot read is a coded refusal with the reader's status, never axum's
/// plain-text answer (a refusal without a code reads to the desktop as an outdated daemon).
#[tokio::test]
async fn an_unreadable_body_is_a_coded_crew_refusal() {
    use axum::body::Body;
    use axum::extract::FromRequest;
    use axum::http::{Request, StatusCode};
    use axum::response::IntoResponse;

    async fn refusal(content_type: Option<&str>, body: &'static str) -> (StatusCode, Value) {
        let mut request = Request::builder().method("POST").uri("/crew/resolve");
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        let request = request.body(Body::from(body)).expect("a request");
        let Err(rejection) =
            super::wire::CrewJson::<super::ResolveRequest>::from_request(request, &()).await
        else {
            panic!("{body} was read");
        };
        let response = rejection.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a body");
        (
            status,
            serde_json::from_slice(&bytes).expect("a JSON refusal"),
        )
    }

    for (content_type, body, status) in [
        (Some("application/json"), "{", StatusCode::BAD_REQUEST),
        (
            None,
            r#"{"selectors":[]}"#,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            Some("application/json"),
            r#"{"selectors":[],"surprise":1}"#,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let (answered, refusal) = refusal(content_type, body).await;
        assert_eq!(answered, status, "{body}");
        assert_eq!(refusal["code"], "crew_request_invalid", "{body}");
        assert_eq!(refusal["error"], super::wire::UNREADABLE_REQUEST, "{body}");
        assert!(refusal["detail"]
            .as_str()
            .is_some_and(|detail| !detail.is_empty()));
    }
}
