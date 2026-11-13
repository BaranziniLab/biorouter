pub mod anthropic;
pub mod audience;
#[cfg(feature = "aws-providers")]
pub mod bedrock;
pub mod databricks;
pub mod gcpvertexai;
pub mod google;
pub mod ollama;
pub mod openai;
pub mod openai_responses;
pub mod openrouter;
pub mod snowflake;
pub mod text_tool_calls;

use rmcp::model::ErrorData;

/// What a model reads for a failed tool call, or for a tool request that could
/// not be used: the error's own sentence.
///
/// `ErrorData`'s `Display` writes the JSON-RPC code in front of the message and
/// the `data` payload after it. So a refusal written to be read as it stands
/// ("This chat is running on a public model, so ...") reached the model as
/// "Error: -32602: This chat ...", and an unparseable call carried its internal
/// `biorouterToolCallFailure` tag along. The code and the data are wire
/// details. Every format renders a tool error through this, never through
/// `Display`, so the model reads the same words whichever provider it runs on.
///
/// A message with nothing in it gets a plain sentence, so the model is never
/// handed an empty error.
pub fn tool_error_text(error: &ErrorData) -> String {
    let said = error.message.trim();
    if said.is_empty() {
        "The tool call failed without saying why.".to_string()
    } else {
        said.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::tool_error_text;
    use rmcp::model::{ErrorCode, ErrorData};
    use serde_json::json;

    #[test]
    fn a_tool_error_reads_as_its_own_sentence() {
        let sentence = "This chat is running on a public model, so it cannot delegate to a \
                        private one.";
        let refused = ErrorData::new(ErrorCode::INVALID_PARAMS, sentence.to_string(), None);
        assert_eq!(tool_error_text(&refused), sentence);
        assert!(
            refused.to_string().starts_with("-32602: "),
            "Display still carries the code"
        );

        let tagged = ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            "Could not interpret tool use parameters".to_string(),
            Some(json!({"biorouterToolCallFailure": "invalid_arguments"})),
        );
        assert_eq!(
            tool_error_text(&tagged),
            "Could not interpret tool use parameters"
        );
    }

    #[test]
    fn an_empty_tool_error_still_says_something() {
        for empty in ["", "   ", "\n"] {
            let error = ErrorData::new(ErrorCode::INTERNAL_ERROR, empty.to_string(), None);
            assert_eq!(
                tool_error_text(&error),
                "The tool call failed without saying why."
            );
        }
    }
}
