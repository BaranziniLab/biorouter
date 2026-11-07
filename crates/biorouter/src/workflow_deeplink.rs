use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use thiserror::Error;

use crate::workflow::Workflow;

#[derive(Error, Debug)]
pub enum DecodeError {
    #[error("Failed to decode workflow deeplink")]
    AllMethodsFailed,
    /// The link decodes to JSON that is not a readable workflow, such as one
    /// with `monitors` at the top level. It says why rather than reading as a
    /// link that failed to decode.
    #[error("The workflow in this link cannot be read: {0}")]
    Unreadable(String),
}

/// Render a workflow as a shareable link.
///
/// ⚠ The extensions are REDACTED on the way out, and this is the choke point
/// that makes that true everywhere. A deeplink is the one artefact in this
/// system whose whole purpose is to leave the machine — it is mailed, pasted
/// into chat, put in a ticket — and `ExtensionConfig` carries resolved
/// connector auth: `StreamableHttp` holds `headers` (a `Bearer` typed into
/// Settings → Extensions is stored verbatim, never keyring-migrated), `Stdio`
/// holds `envs`, and both hold locators that can embed credentials in userinfo.
///
/// The same projection `workflow::service::session_enrichment` applies to a
/// *generated* workflow, applied here as well because generation is not the
/// only way a workflow acquires extensions: one hand-authored, imported, or
/// written by the workflow builder reaches this function with whatever its file
/// says. Redacting at the three call sites instead would be three chances to
/// forget — and the encoder is the one place all of them pass through.
///
/// It costs the recipient nothing: a consumer re-enables an extension by
/// matching its NAME against its own installed set, which is the only thing
/// that could work on another machine anyway.
pub fn encode(workflow: &Workflow) -> Result<String, serde_json::Error> {
    let mut workflow = workflow.clone();
    workflow.extensions = workflow.extensions.map(|extensions| {
        extensions
            .iter()
            .map(crate::agents::extension::ExtensionConfig::redacted_for_session_export)
            .collect()
    });
    let workflow_json = serde_json::to_string(&workflow)?;
    let encoded = URL_SAFE_NO_PAD.encode(workflow_json.as_bytes());
    Ok(encoded)
}

pub fn decode(link: &str) -> Result<Workflow, DecodeError> {
    // The first reason a decoded link's JSON was not a workflow (a data error,
    // not a syntax one: text that is not JSON at all is just a wrong guess at
    // the encoding).
    let mut unreadable: Option<String> = None;
    let mut parse = |workflow_json: &str| match serde_json::from_str::<Workflow>(workflow_json) {
        Ok(workflow) => Some(workflow),
        Err(error) => {
            if error.is_data() && unreadable.is_none() {
                unreadable = Some(error.to_string());
            }
            None
        }
    };

    // Handle the current format: URL-safe Base64 without padding.
    if let Ok(decoded_bytes) = URL_SAFE_NO_PAD.decode(link) {
        if let Ok(workflow_json) = String::from_utf8(decoded_bytes) {
            if let Some(workflow) = parse(&workflow_json) {
                return Ok(workflow);
            }
        }
    }

    // Handle legacy formats of 'standard base64 encoded' and standard base64 encoded that was then url encoded.
    if let Ok(url_decoded) = urlencoding::decode(link) {
        if let Ok(decoded_bytes) =
            base64::engine::general_purpose::STANDARD.decode(url_decoded.as_bytes())
        {
            if let Ok(workflow_json) = String::from_utf8(decoded_bytes) {
                if let Some(workflow) = parse(&workflow_json) {
                    return Ok(workflow);
                }
            }
        }
    }

    Err(unreadable.map_or(DecodeError::AllMethodsFailed, DecodeError::Unreadable))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::Workflow;

    fn create_test_workflow() -> Workflow {
        Workflow::builder()
            .title("Test Workflow")
            .description("A test workflow for deeplink encoding/decoding")
            .instructions("Act as a helpful assistant")
            .build()
            .expect("Failed to build test workflow")
    }

    #[test]
    fn test_encode_decode_round_trip() {
        let original_workflow = create_test_workflow();

        let encoded = encode(&original_workflow).expect("Failed to encode workflow");
        assert!(!encoded.is_empty());

        let decoded_workflow = decode(&encoded).expect("Failed to decode workflow");

        assert_eq!(original_workflow.title, decoded_workflow.title);
        assert_eq!(original_workflow.description, decoded_workflow.description);
        assert_eq!(
            original_workflow.instructions,
            decoded_workflow.instructions
        );
        assert_eq!(original_workflow.version, decoded_workflow.version);
    }

    #[test]
    fn test_decode_legacy_standard_base64() {
        let workflow = create_test_workflow();
        let workflow_json = serde_json::to_string(&workflow).unwrap();
        let legacy_encoded =
            base64::engine::general_purpose::STANDARD.encode(workflow_json.as_bytes());

        let decoded_workflow = decode(&legacy_encoded).expect("Failed to decode legacy format");
        assert_eq!(workflow.title, decoded_workflow.title);
        assert_eq!(workflow.description, decoded_workflow.description);
        assert_eq!(workflow.instructions, decoded_workflow.instructions);
    }

    #[test]
    fn test_decode_legacy_url_encoded_base64() {
        let workflow = create_test_workflow();
        let workflow_json = serde_json::to_string(&workflow).unwrap();
        let base64_encoded =
            base64::engine::general_purpose::STANDARD.encode(workflow_json.as_bytes());
        let url_encoded = urlencoding::encode(&base64_encoded);

        let decoded_workflow =
            decode(&url_encoded).expect("Failed to decode URL-encoded legacy format");
        assert_eq!(workflow.title, decoded_workflow.title);
        assert_eq!(workflow.description, decoded_workflow.description);
        assert_eq!(workflow.instructions, decoded_workflow.instructions);
    }

    #[test]
    fn test_decode_invalid_input() {
        let result = decode("invalid_base64!");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DecodeError::AllMethodsFailed));
    }

    /// A share link is the one artefact whose whole purpose is to leave the
    /// machine, and `ExtensionConfig` carries resolved connector auth.
    ///
    /// ⚠ The credential does not have to have come from a generated workflow.
    /// `service::session_enrichment` redacts what it captures, but a workflow
    /// authored by hand, imported from someone else, or built in the workflow
    /// builder arrives here with whatever its file says — so the projection has
    /// to happen at the encoder, which is the one place the tool, the CLI and
    /// the HTTP route all pass through.
    #[test]
    fn a_share_link_never_carries_connector_auth() {
        let mut workflow = create_test_workflow();
        workflow.extensions = Some(vec![
            crate::agents::extension::ExtensionConfig::StreamableHttp {
                name: "cdw".to_string(),
                description: "Clinical Data Warehouse".to_string(),
                uri: "https://cdw.example.org/mcp?token=sk-live-not-a-real-key".to_string(),
                envs: Default::default(),
                env_keys: vec![],
                headers: std::collections::HashMap::from([(
                    "Authorization".to_string(),
                    "Bearer ghs-not-a-real-token".to_string(),
                )]),
                timeout: None,
                bundled: None,
                available_tools: vec![],
            },
            crate::agents::extension::ExtensionConfig::Stdio {
                name: "local".to_string(),
                description: String::new(),
                cmd: "/usr/local/bin/secret-tool".to_string(),
                args: vec!["--api-key".to_string(), "hunter2hunter2".to_string()],
                envs: Default::default(),
                env_keys: vec![],
                timeout: None,
                bundled: None,
                available_tools: vec![],
            },
        ]);

        let encoded = encode(&workflow).expect("encodes");
        let decoded_json = String::from_utf8(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(&encoded)
                .expect("valid base64"),
        )
        .expect("valid utf-8");

        for secret in [
            "Bearer ghs-not-a-real-token",
            "sk-live-not-a-real-key",
            "hunter2hunter2",
            "/usr/local/bin/secret-tool",
        ] {
            assert!(
                !decoded_json.contains(secret),
                "`{secret}` rode the share link: {decoded_json}"
            );
        }

        // The NAMES survive, because that is all a recipient needs: a consumer
        // re-enables an extension by matching its name against its own
        // installed set, which is the only thing that could work on another
        // machine anyway.
        let round_tripped = decode(&encoded).expect("decodes");
        let names: Vec<String> = round_tripped
            .extensions
            .expect("extensions survive")
            .iter()
            .map(crate::agents::extension::ExtensionConfig::name)
            .collect();
        assert_eq!(names, vec!["cdw".to_string(), "local".to_string()]);
    }
}
