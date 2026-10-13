use reqwest::StatusCode;
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ProviderError {
    #[error("Authentication error: {0}")]
    Authentication(String),

    #[error("Context length exceeded: {0}")]
    ContextLengthExceeded(String),

    #[error("Rate limit exceeded: {details}")]
    RateLimitExceeded {
        details: String,
        retry_delay: Option<Duration>,
    },

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Request failed: {0}")]
    RequestFailed(String),

    #[error("Execution error: {0}")]
    ExecutionError(String),

    #[error("Usage data error: {0}")]
    UsageError(String),

    #[error("Unsupported operation: {0}")]
    NotImplemented(String),

    /// A local inference server sent nothing for the provider's idle bound
    /// (`OLLAMA_TIMEOUT`). The text is a whole sentence naming the setting to
    /// raise.
    ///
    /// Its own variant because it must not be re-sent by ANYONE. The provider
    /// already hands a timeout straight back (`retry::TimeoutRetry::GiveUp`):
    /// the server is still reading the abandoned request, and a second copy
    /// queues behind it. As a `RequestFailed` it was classed a network blip
    /// and the agent re-sent the whole prompt once anyway. Diagnostics
    /// 20261006_2: on an Intel Mac reading about 25 prompt tokens a second, a
    /// first prompt of about 37,000 tokens outlasts a 600-second bound, so the
    /// person waited twenty minutes for "error sending request ... (timeout)".
    #[error("{0}")]
    IdleTimeout(String),
}

/// The coarse class of a provider failure — enough for a caller to decide
/// "retry", "fix your credentials", or "give up", without string-matching the
/// error message.
///
/// Exists because a 403 and a transient 502 were both flattened into the same
/// assistant chat message, so nothing downstream could tell a misconfigured key
/// from a blip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    /// 401/403 — the credentials are wrong, absent, or not allowed here.
    Auth,
    /// 429 — back off and retry.
    RateLimit,
    /// The account has no remaining credit or has hit a billing limit.
    Quota,
    /// 5xx from the provider.
    Server,
    /// The request never got a usable answer (timeout, DNS, connection reset).
    Network,
    /// The prompt exceeded the model's context window.
    ContextLength,
    /// The request shape or one of its parameters is not supported.
    InvalidRequest,
    /// The requested model or deployment does not exist or is unavailable.
    ModelUnavailable,
    /// The provider rejected the request under a safety or content policy.
    Policy,
    /// Anything else.
    Other,
}

impl ProviderErrorKind {
    /// True when the operator must fix a credential — the one class that will
    /// never succeed on retry.
    pub fn is_auth(&self) -> bool {
        matches!(self, Self::Auth)
    }

    /// True when retrying the same request could plausibly succeed.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::RateLimit | Self::Server | Self::Network)
    }

    pub fn wire_code(&self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::RateLimit => "rate_limit",
            Self::Quota => "quota",
            Self::Server => "server",
            Self::Network => "network",
            Self::ContextLength => "context_length",
            Self::InvalidRequest => "invalid_request",
            Self::ModelUnavailable => "model_unavailable",
            Self::Policy => "policy",
            Self::Other => "unknown",
        }
    }
}

/// `details` with every URL cut out, so the port or path a request was sent to
/// cannot be read as the status it got back.
///
/// Measured (2026-10): `From<anyhow::Error>` renders a reqwest timeout as
/// `error sending request for url (http://127.0.0.1:50451/v1/chat/completions)
/// (timeout)`, and the bundled llama-server falls back to an ephemeral port
/// when 11543 is taken. Sniffed whole, that `:50451` read as a 504, so a timeout
/// was classified `Server` — retried, and never seen by `retry::is_timeout` — and
/// a port like `:54031` read as a 403 and failed the turn as bad credentials.
fn without_urls(details: &str) -> String {
    let mut out = String::with_capacity(details.len());
    let mut rest = details;
    while let Some(start) = ["http://", "https://"]
        .iter()
        .filter_map(|scheme| rest.find(scheme))
        .min()
    {
        // `find` returns char boundaries, so these splits cannot fail.
        let (before, url) = rest.split_at(start);
        out.push_str(before);
        let end = url
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '>' | ','))
            .unwrap_or(url.len());
        out.push(' ');
        rest = url.split_at(end).1;
    }
    out.push_str(rest);
    out
}

/// Whether `code` appears in `details` as a number of its own: not a slice of a
/// longer number (`50451`), not a host's port (`localhost:504`,
/// `127.0.0.1:50451`, `[::1]:504`), not part of a dotted number. A URL is
/// already gone (see [`without_urls`]); this catches the `failed to connect to
/// 127.0.0.1:50451` form, which has no scheme.
///
/// ⚠ A colon alone does not make a port. An error body quoted into the message
/// says `"code":401` or `status:429`, and those ARE the status: only a colon
/// after something shaped like a host (dotted, `localhost`, or a bracketed
/// IPv6 address) is read as one.
fn mentions_status(details: &str, code: &str) -> bool {
    details.match_indices(code).any(|(at, _)| {
        // `match_indices` yields char boundaries, so the split cannot fail.
        let (head, tail) = details.split_at(at);
        let before = head.chars().next_back();
        let mut after = tail.chars().skip(code.chars().count());
        let next = after.next();
        let standalone_start = match before {
            Some(c) if c.is_ascii_digit() || c == '.' => false,
            Some(':') => !ends_with_host(head.strip_suffix(':').unwrap_or(head)),
            _ => true,
        };
        let standalone_end = match next {
            Some(c) if c.is_ascii_digit() => false,
            // `504.` ends a sentence; `504.5` is not a status.
            Some('.') => !after.next().is_some_and(|c| c.is_ascii_digit()),
            _ => true,
        };
        standalone_start && standalone_end
    })
}

/// Whether `text` ends in a host name or address: the token a `:port` follows.
fn ends_with_host(text: &str) -> bool {
    if text.ends_with(']') {
        return true;
    }
    let token = text
        .rsplit(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
        .next()
        .unwrap_or_default();
    token == "localhost" || (token.contains('.') && !token.starts_with('.'))
}

fn classify_provider_details(details: &str) -> ProviderErrorKind {
    let details = without_urls(&details.to_ascii_lowercase());
    let status = |code: &str| mentions_status(&details, code);
    if status("401")
        || status("403")
        || details.contains("unauthorized")
        || details.contains("authentication failed")
        || details.contains("authentication error")
        || details.contains("invalid api key")
        || details.contains("incorrect api key")
    {
        ProviderErrorKind::Auth
    } else if details.contains("insufficient_quota")
        || details.contains("quota_exceeded")
        || details.contains("billing_hard_limit")
        || details.contains("payment required")
        || details.contains("credit balance")
    {
        ProviderErrorKind::Quota
    } else if status("429") || details.contains("rate_limit") {
        ProviderErrorKind::RateLimit
    } else if details.contains("context_length")
        || details.contains("context window")
        || details.contains("maximum context")
        || details.contains("too many tokens")
    {
        ProviderErrorKind::ContextLength
    } else if details.contains("content_policy")
        || details.contains("safety policy")
        || details.contains("safety filter")
        || details.contains("content blocked")
    {
        ProviderErrorKind::Policy
    } else if status("404")
        || details.contains("model_not_found")
        || details.contains("model not found")
        || details.contains("resource not found")
        || details.contains("deployment not found")
    {
        ProviderErrorKind::ModelUnavailable
    } else if status("400")
        || status("422")
        || details.contains("bad request")
        || details.contains("invalid_request")
        || details.contains("invalid argument")
        || details.contains("invalid_argument")
        || details.contains("unsupported parameter")
        || details.contains("not supported")
    {
        ProviderErrorKind::InvalidRequest
    } else if status("500") || status("502") || status("503") || status("504") {
        ProviderErrorKind::Server
    } else if details.contains("timeout")
        || details.contains("timed out")
        || details.contains("connection")
        // `From<anyhow::Error>`'s own wording for a refused connect.
        || details.contains("failed to connect")
        || details.contains("dns")
        || details.contains("network")
        || details.contains("failed to fetch")
        || details.contains("error sending request")
    {
        ProviderErrorKind::Network
    } else {
        ProviderErrorKind::Other
    }
}

impl ProviderError {
    /// Classify this error. Used to decide the CLI exit code and the wire frame
    /// for [`crate::agents::turn_abort::TurnAbortCode::ProviderFailure`].
    pub fn kind(&self) -> ProviderErrorKind {
        match self {
            ProviderError::Authentication(_) => ProviderErrorKind::Auth,
            ProviderError::RateLimitExceeded { .. } => ProviderErrorKind::RateLimit,
            ProviderError::ServerError(_) => ProviderErrorKind::Server,
            ProviderError::ContextLengthExceeded(_) => ProviderErrorKind::ContextLength,
            // `RequestFailed` is what `From<anyhow::Error>` produces for a
            // reqwest failure, so it covers timeouts/connection errors — but it
            // also carries HTTP status text for some providers. Sniff the status
            // so a 401/403 surfaced this way is still classified as auth rather
            // than as a retryable network blip.
            ProviderError::RequestFailed(details)
            | ProviderError::ExecutionError(details)
            | ProviderError::UsageError(details) => classify_provider_details(details),
            ProviderError::NotImplemented(_) => ProviderErrorKind::InvalidRequest,
            ProviderError::IdleTimeout(_) => ProviderErrorKind::Network,
        }
    }

    pub fn telemetry_type(&self) -> &'static str {
        match self {
            ProviderError::Authentication(_) => "auth",
            ProviderError::ContextLengthExceeded(_) => "context_length",
            ProviderError::RateLimitExceeded { .. } => "rate_limit",
            ProviderError::ServerError(_) => "server",
            ProviderError::RequestFailed(_) => "request",
            ProviderError::ExecutionError(_) => "execution",
            ProviderError::UsageError(_) => "usage",
            ProviderError::NotImplemented(_) => "not_implemented",
            ProviderError::IdleTimeout(_) => "timeout",
        }
    }
}

impl From<anyhow::Error> for ProviderError {
    fn from(error: anyhow::Error) -> Self {
        if let Some(reqwest_err) = error.downcast_ref::<reqwest::Error>() {
            let mut details = vec![];

            if let Some(status) = reqwest_err.status() {
                details.push(format!("status: {}", status));
            }
            if reqwest_err.is_timeout() {
                details.push("timeout".to_string());
            }
            if reqwest_err.is_connect() {
                if let Some(url) = reqwest_err.url() {
                    if let Some(host) = url.host_str() {
                        let port_info = url.port().map(|p| format!(":{}", p)).unwrap_or_default();

                        details.push(format!("failed to connect to {}{}", host, port_info));

                        if url.port().is_some() {
                            details.push("check that the port is correct".to_string());
                        }
                    }
                } else {
                    details.push("connection failed".to_string());
                }
            }
            let msg = if details.is_empty() {
                reqwest_err.to_string()
            } else {
                format!("{} ({})", reqwest_err, details.join(", "))
            };
            return ProviderError::RequestFailed(msg);
        }
        ProviderError::ExecutionError(error.to_string())
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(error: reqwest::Error) -> Self {
        ProviderError::RequestFailed(error.to_string())
    }
}

#[derive(Debug)]
pub enum GoogleErrorCode {
    BadRequest = 400,
    Unauthorized = 401,
    Forbidden = 403,
    NotFound = 404,
    TooManyRequests = 429,
    InternalServerError = 500,
    ServiceUnavailable = 503,
}

impl GoogleErrorCode {
    pub fn to_status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
            Self::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    pub fn from_code(code: u64) -> Option<Self> {
        match code {
            400 => Some(Self::BadRequest),
            401 => Some(Self::Unauthorized),
            403 => Some(Self::Forbidden),
            404 => Some(Self::NotFound),
            429 => Some(Self::TooManyRequests),
            500 => Some(Self::InternalServerError),
            503 => Some(Self::ServiceUnavailable),
            _ => Some(Self::InternalServerError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_provider_rejections_without_treating_them_as_network_errors() {
        let cases = [
            ("insufficient_quota", ProviderErrorKind::Quota),
            (
                "Bad request (400): unsupported parameter reasoning_effort",
                ProviderErrorKind::InvalidRequest,
            ),
            ("model_not_found", ProviderErrorKind::ModelUnavailable),
            (
                "request rejected by safety filter",
                ProviderErrorKind::Policy,
            ),
            ("429 rate_limit_exceeded", ProviderErrorKind::RateLimit),
            ("connection timed out", ProviderErrorKind::Network),
        ];

        for (message, expected) in cases {
            assert_eq!(
                ProviderError::RequestFailed(message.into()).kind(),
                expected
            );
        }
    }

    /// A port or a path is not a status. The bundled llama-server falls back to
    /// an ephemeral port, and the first two shapes are what `From<anyhow::Error>`
    /// produces for a timeout and a refused connection there; before the fix
    /// the first read as a 504 (retried, and not a timeout to
    /// `retry::is_timeout`) and the others as a 403 (the turn failed as bad
    /// credentials).
    #[test]
    fn a_port_or_path_that_contains_a_status_code_is_not_that_status() {
        let cases = [
            (
                "error sending request for url (http://127.0.0.1:50451/v1/chat/completions) (timeout)",
                ProviderErrorKind::Network,
            ),
            (
                "error sending request for url (http://127.0.0.1:54031/v1/chat/completions) \
                 (failed to connect to 127.0.0.1:54031, check that the port is correct)",
                ProviderErrorKind::Network,
            ),
            (
                "error sending request for url (https://gateway.example/deployments/500/chat) (timeout)",
                ProviderErrorKind::Network,
            ),
            ("failed to connect to localhost:4290", ProviderErrorKind::Network),
            ("failed to connect to [::1]:50451", ProviderErrorKind::Network),
            ("failed to connect to llama.local:4031", ProviderErrorKind::Network),
        ];
        for (message, expected) in cases {
            assert_eq!(
                ProviderError::RequestFailed(message.into()).kind(),
                expected,
                "{message}"
            );
        }
        assert!(crate::providers::retry::is_timeout(
            &ProviderError::RequestFailed(cases[0].0.into())
        ));

        // A status the server really sent still counts, wherever it sits.
        for (message, expected) in [
            (
                "error sending request for url (http://127.0.0.1:50451/api/chat) (status: 403 Forbidden)",
                ProviderErrorKind::Auth,
            ),
            ("Server error 504.", ProviderErrorKind::Server),
            ("upstream returned 502", ProviderErrorKind::Server),
            ("HTTP 429 from http://127.0.0.1:11434", ProviderErrorKind::RateLimit),
            // An error body quoted into the message: a colon before the code
            // is not a port unless a host stands before it.
            (
                r#"Request failed: {"error":{"code":401,"message":"bad"}}"#,
                ProviderErrorKind::Auth,
            ),
            (r#"upstream said {"code":429}"#, ProviderErrorKind::RateLimit),
            ("status:503", ProviderErrorKind::Server),
        ] {
            assert_eq!(
                ProviderError::RequestFailed(message.into()).kind(),
                expected,
                "{message}"
            );
        }
    }

    #[test]
    fn unknown_rejections_are_explicitly_unknown_and_not_retryable() {
        let kind = ProviderError::RequestFailed("provider rejected the request".into()).kind();
        assert_eq!(kind, ProviderErrorKind::Other);
        assert_eq!(kind.wire_code(), "unknown");
        assert!(!kind.is_transient());
    }

    #[test]
    fn classifies_wrapped_provider_errors_from_their_preserved_details() {
        assert_eq!(
            ProviderError::ExecutionError(
                "Authentication error: Authentication failed. Status: 401 Unauthorized".into()
            )
            .kind(),
            ProviderErrorKind::Auth
        );
        assert_eq!(
            ProviderError::UsageError("provider returned insufficient_quota".into()).kind(),
            ProviderErrorKind::Quota
        );
    }
}
