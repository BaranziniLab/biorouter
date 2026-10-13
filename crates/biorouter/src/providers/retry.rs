use super::errors::ProviderError;
use crate::providers::base::Provider;
use async_trait::async_trait;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

/// Unix-millis until which a provider was last observed rate-limited (HTTP 429).
/// Set by [`retry_operation`] on any rate-limit error; read by the scheduler
/// ([`crate::scheduler`]) to defer scheduled (non-interactive) jobs so background
/// work doesn't pile onto a throttled provider while the per-request retry budget
/// is already absorbing the 429.
pub static RATE_LIMITED_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Record that the provider is rate-limited for at least `dur` from now.
pub fn note_rate_limited(dur: Duration) {
    let until = now_ms().saturating_add(dur.as_millis() as u64);
    RATE_LIMITED_UNTIL_MS.fetch_max(until, Ordering::Relaxed);
}

/// Whether a provider is currently within an observed rate-limit backoff window.
pub fn is_rate_limited() -> bool {
    now_ms() < RATE_LIMITED_UNTIL_MS.load(Ordering::Relaxed)
}

pub const DEFAULT_MAX_RETRIES: usize = 3;
pub const DEFAULT_INITIAL_RETRY_INTERVAL_MS: u64 = 1000;
pub const DEFAULT_BACKOFF_MULTIPLIER: f64 = 2.0;
pub const DEFAULT_MAX_RETRY_INTERVAL_MS: u64 = 30_000;

/// Rate-limit (HTTP 429) responses are always transient, but sustained
/// throttling (e.g. several concurrent sessions against one key) routinely lasts
/// longer than the generic `max_retries` window (3 retries ≈ 7s). Give rate-limit
/// errors a deeper dedicated budget so a transient 429 doesn't abort the turn.
/// With the default 1s→2s backoff capped at 30s, 8 attempts span ~2 minutes.
pub const RATE_LIMIT_MAX_RETRIES: usize = 8;

/// The effective retry ceiling for a given error: rate-limit errors get the
/// larger of the configured `max_retries` and [`RATE_LIMIT_MAX_RETRIES`].
fn effective_max_retries(error: &ProviderError, config: &RetryConfig) -> usize {
    if matches!(error, ProviderError::RateLimitExceeded { .. }) {
        config.max_retries.max(RATE_LIMIT_MAX_RETRIES)
    } else {
        config.max_retries
    }
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub(crate) max_retries: usize,
    /// Initial interval between retries in milliseconds
    pub(crate) initial_interval_ms: u64,
    /// Multiplier for backoff (exponential)
    pub(crate) backoff_multiplier: f64,
    /// Maximum interval between retries in milliseconds
    pub(crate) max_interval_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_interval_ms: DEFAULT_INITIAL_RETRY_INTERVAL_MS,
            backoff_multiplier: DEFAULT_BACKOFF_MULTIPLIER,
            max_interval_ms: DEFAULT_MAX_RETRY_INTERVAL_MS,
        }
    }
}

impl RetryConfig {
    pub fn new(
        max_retries: usize,
        initial_interval_ms: u64,
        backoff_multiplier: f64,
        max_interval_ms: u64,
    ) -> Self {
        Self {
            max_retries,
            initial_interval_ms,
            backoff_multiplier,
            max_interval_ms,
        }
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn delay_for_attempt(&self, attempt: usize) -> Duration {
        if attempt == 0 {
            return Duration::from_millis(0);
        }

        let exponent = (attempt - 1) as u32;
        let base_delay_ms = (self.initial_interval_ms as f64
            * self.backoff_multiplier.powi(exponent as i32)) as u64;

        let capped_delay_ms = std::cmp::min(base_delay_ms, self.max_interval_ms);

        let jitter_factor_to_avoid_thundering_herd = 0.8 + (rand::random::<f64>() * 0.4);
        let jitter_delay_ms =
            (capped_delay_ms as f64 * jitter_factor_to_avoid_thundering_herd) as u64;

        Duration::from_millis(jitter_delay_ms)
    }
}

pub fn should_retry(error: &ProviderError) -> bool {
    error.kind().is_transient()
}

/// What a retry loop does with a request that TIMED OUT, as opposed to one that
/// failed in any other transient way.
///
/// Chosen per call site, by the provider making the call, rather than carried
/// on [`RetryConfig`]: that struct is built field-by-field by several providers
/// and persisted by `provider_binding`, so a new field there is a breaking
/// change for each of them, while this is a decision only a provider that
/// talks to a single-slot local server has to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeoutRetry {
    /// Retry a timeout like any other transient failure. Every provider's
    /// behaviour, and what [`ProviderRetry::with_retry`] / [`retry_operation`]
    /// do.
    #[default]
    Retry,
    /// Hand a timeout straight back. For a loopback inference server
    /// (Ollama, Llama Server) a timeout means the server is still busy with
    /// THIS request — a long prefill on a slow machine sends nothing until it
    /// is done — and re-sending queues a second copy of a ~40k-token prompt
    /// behind the abandoned first, each retry later than the last. Connection
    /// failures (a server still starting) are not timeouts and are retried as
    /// before.
    GiveUp,
}

/// Whether `error` is a request that timed out. `From<anyhow::Error>` tags a
/// reqwest timeout `(timeout)`; a body read that stalls says "timed out".
pub fn is_timeout(error: &ProviderError) -> bool {
    match error {
        ProviderError::IdleTimeout(_) => true,
        ProviderError::RequestFailed(details) | ProviderError::ExecutionError(details) => {
            let details = details.to_ascii_lowercase();
            error.kind() == super::errors::ProviderErrorKind::Network
                && (details.contains("timeout") || details.contains("timed out"))
        }
        _ => false,
    }
}

/// [`should_retry`], with the call site's [`TimeoutRetry`] decision applied.
pub fn should_retry_with(error: &ProviderError, timeouts: TimeoutRetry) -> bool {
    should_retry(error) && !(timeouts == TimeoutRetry::GiveUp && is_timeout(error))
}

pub async fn retry_operation<F, Fut, T>(
    config: &RetryConfig,
    operation: F,
) -> Result<T, ProviderError>
where
    F: Fn() -> Fut + Send,
    Fut: Future<Output = Result<T, ProviderError>> + Send,
    T: Send,
{
    retry_operation_with(config, TimeoutRetry::Retry, operation).await
}

/// [`retry_operation`] with an explicit [`TimeoutRetry`] decision.
pub async fn retry_operation_with<F, Fut, T>(
    config: &RetryConfig,
    timeouts: TimeoutRetry,
    operation: F,
) -> Result<T, ProviderError>
where
    F: Fn() -> Fut + Send,
    Fut: Future<Output = Result<T, ProviderError>> + Send,
    T: Send,
{
    let mut attempts = 0;

    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                // Surface rate-limiting to the scheduler so background jobs back
                // off, whether or not we retry this request.
                if let ProviderError::RateLimitExceeded { retry_delay, .. } = &error {
                    let window = retry_delay
                        .unwrap_or(Duration::from_secs(60))
                        .max(Duration::from_secs(30));
                    note_rate_limited(window);
                }
                if should_retry_with(&error, timeouts)
                    && attempts < effective_max_retries(&error, config)
                {
                    attempts += 1;
                    tracing::warn!(
                        attempt = attempts,
                        max_retries = effective_max_retries(&error, config),
                        error_type = error.telemetry_type(),
                        "Request failed, retrying"
                    );

                    let delay = match &error {
                        ProviderError::RateLimitExceeded {
                            retry_delay: Some(d),
                            ..
                        } => *d,
                        _ => config.delay_for_attempt(attempts),
                    };

                    sleep(delay).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
}

/// Trait for retry functionality to keep Provider dyn-compatible
#[async_trait]
pub trait ProviderRetry {
    fn retry_config(&self) -> RetryConfig {
        RetryConfig::default()
    }

    async fn with_retry<F, Fut, T>(&self, operation: F) -> Result<T, ProviderError>
    where
        F: Fn() -> Fut + Send,
        Fut: Future<Output = Result<T, ProviderError>> + Send,
        T: Send,
    {
        self.with_retry_policy(TimeoutRetry::Retry, operation).await
    }

    /// [`Self::with_retry`] with the call site's decision about timeouts. A
    /// provider talking to a loopback single-slot server passes
    /// [`TimeoutRetry::GiveUp`]; everything else is unchanged, including the
    /// provider's own [`RetryConfig`] and the rate-limit budget.
    async fn with_retry_policy<F, Fut, T>(
        &self,
        timeouts: TimeoutRetry,
        operation: F,
    ) -> Result<T, ProviderError>
    where
        F: Fn() -> Fut + Send,
        Fut: Future<Output = Result<T, ProviderError>> + Send,
        T: Send,
    {
        let mut attempts = 0;
        let config = self.retry_config();

        loop {
            return match operation().await {
                Ok(result) => Ok(result),
                Err(error) => {
                    if should_retry_with(&error, timeouts)
                        && attempts < effective_max_retries(&error, &config)
                    {
                        attempts += 1;
                        tracing::warn!(
                            attempt = attempts,
                            max_retries = effective_max_retries(&error, &config),
                            error_type = error.telemetry_type(),
                            "Request failed, retrying"
                        );

                        let delay = match &error {
                            ProviderError::RateLimitExceeded {
                                retry_delay: Some(provider_delay),
                                ..
                            } => *provider_delay,
                            _ => config.delay_for_attempt(attempts),
                        };

                        let skip_backoff = std::env::var("BIOROUTER_PROVIDER_SKIP_BACKOFF")
                            .unwrap_or_default()
                            .parse::<bool>()
                            .unwrap_or(false);

                        if skip_backoff {
                            tracing::info!(
                                "Skipping backoff due to BIOROUTER_PROVIDER_SKIP_BACKOFF"
                            );
                        } else {
                            tracing::info!("Backing off for {:?} before retry", delay);
                            sleep(delay).await;
                        }
                        continue;
                    }

                    Err(error)
                }
            };
        }
    }
}

impl<P: Provider> ProviderRetry for P {
    fn retry_config(&self) -> RetryConfig {
        Provider::retry_config(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_gets_deeper_retry_budget_than_generic() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, DEFAULT_MAX_RETRIES);

        // A transient 429 should be retried far more than the generic ceiling,
        // because sustained throttling outlasts ~7s of generic retries.
        let rate_limit = ProviderError::RateLimitExceeded {
            details: "Too many requests".to_string(),
            retry_delay: None,
        };
        assert_eq!(
            effective_max_retries(&rate_limit, &config),
            RATE_LIMIT_MAX_RETRIES
        );
        const { assert!(RATE_LIMIT_MAX_RETRIES > DEFAULT_MAX_RETRIES) };

        // Non-rate-limit retryable errors keep the generic ceiling.
        let server = ProviderError::ServerError("boom".to_string());
        assert_eq!(effective_max_retries(&server, &config), DEFAULT_MAX_RETRIES);

        // should_retry still classifies rate limits as retryable.
        assert!(should_retry(&rate_limit));
    }

    #[test]
    fn rate_limit_budget_respects_a_higher_configured_max() {
        // If a provider configures an even larger max_retries, keep theirs.
        let config = RetryConfig {
            max_retries: 20,
            ..RetryConfig::default()
        };
        let rate_limit = ProviderError::RateLimitExceeded {
            details: "x".to_string(),
            retry_delay: None,
        };
        assert_eq!(effective_max_retries(&rate_limit, &config), 20);
    }

    #[test]
    fn permanent_request_rejections_are_not_retried() {
        for message in [
            "insufficient_quota",
            "Bad request (400): unsupported parameter",
            "model_not_found",
            "content blocked by safety filter",
            "provider rejected the request",
        ] {
            assert!(!should_retry(&ProviderError::RequestFailed(message.into())));
        }

        assert!(should_retry(&ProviderError::RequestFailed(
            "connection timed out".into()
        )));
    }

    /// The two shapes a reqwest timeout reaches a provider in, and the
    /// transient failures that are NOT timeouts.
    #[test]
    fn timeouts_are_told_apart_from_other_transient_failures() {
        for timeout in [
            // `From<anyhow::Error>` on a reqwest timeout.
            "error sending request for url (http://127.0.0.1:11434/api/chat) (timeout)",
            "error decoding response body: operation timed out",
        ] {
            assert!(
                is_timeout(&ProviderError::RequestFailed(timeout.into())),
                "{timeout}"
            );
        }
        for other in [
            ProviderError::RequestFailed(
                "error sending request for url (http://127.0.0.1:11434/api/chat) (failed to connect to 127.0.0.1:11434, check that the port is correct)".into(),
            ),
            ProviderError::ServerError("Server error (500): Context size has been exceeded.".into()),
            ProviderError::RateLimitExceeded {
                details: "timeout".into(),
                retry_delay: None,
            },
            ProviderError::ContextLengthExceeded("timeout".into()),
        ] {
            assert!(!is_timeout(&other), "{other:?}");
        }
    }

    #[test]
    fn give_up_on_timeouts_changes_only_timeouts() {
        let timeout = ProviderError::RequestFailed("request (timeout)".into());
        let refused = ProviderError::RequestFailed("connection refused".into());
        let server = ProviderError::ServerError("boom".into());
        let auth = ProviderError::Authentication("no".into());

        assert!(should_retry_with(&timeout, TimeoutRetry::Retry));
        assert!(!should_retry_with(&timeout, TimeoutRetry::GiveUp));
        for (error, expected) in [(&refused, true), (&server, true), (&auth, false)] {
            for policy in [TimeoutRetry::Retry, TimeoutRetry::GiveUp] {
                assert_eq!(
                    should_retry_with(error, policy),
                    expected,
                    "{error:?} under {policy:?}"
                );
            }
        }
        assert_eq!(TimeoutRetry::default(), TimeoutRetry::Retry);
    }

    struct CountingProvider;
    impl ProviderRetry for CountingProvider {
        fn retry_config(&self) -> RetryConfig {
            RetryConfig::new(3, 0, 1.0, 0)
        }
    }

    async fn attempts_for(
        timeouts: TimeoutRetry,
        error: ProviderError,
        through_trait: bool,
    ) -> usize {
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        let operation = || {
            attempts.fetch_add(1, Ordering::SeqCst);
            let error = match &error {
                ProviderError::RequestFailed(text) => ProviderError::RequestFailed(text.clone()),
                ProviderError::ServerError(text) => ProviderError::ServerError(text.clone()),
                other => panic!("unexpected fixture {other:?}"),
            };
            async move { Err::<(), _>(error) }
        };
        let result = if through_trait {
            CountingProvider
                .with_retry_policy(timeouts, operation)
                .await
        } else {
            retry_operation_with(&CountingProvider.retry_config(), timeouts, operation).await
        };
        assert!(result.is_err());
        attempts.load(Ordering::SeqCst)
    }

    /// A timeout against a loopback single-slot server is sent ONCE under
    /// `GiveUp`, and the default still retries it; any other transient error
    /// keeps its full budget either way.
    #[tokio::test]
    async fn a_timed_out_request_is_not_resent_when_the_caller_gives_up_on_timeouts() {
        for through_trait in [true, false] {
            let timeout = || ProviderError::RequestFailed("request (timeout)".into());
            assert_eq!(
                attempts_for(TimeoutRetry::GiveUp, timeout(), through_trait).await,
                1
            );
            assert_eq!(
                attempts_for(TimeoutRetry::Retry, timeout(), through_trait).await,
                4,
                "default: 1 + max_retries"
            );
            assert_eq!(
                attempts_for(
                    TimeoutRetry::GiveUp,
                    ProviderError::ServerError("busy".into()),
                    through_trait
                )
                .await,
                4
            );
        }
    }
}
