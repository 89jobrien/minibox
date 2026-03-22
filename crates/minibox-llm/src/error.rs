use thiserror::Error;

/// An explicit HTTP error carrying the status code and response body text.
///
/// Provider implementations box this as the `source` of a
/// [`LlmError::ProviderError`] whenever the server returns a non-2xx status.
/// [`LlmError::is_transient`] downcasts to this type to classify the error:
///
/// | Status range | Classification |
/// |---|---|
/// | 429 (Too Many Requests) | Transient — safe to retry |
/// | 500, 502, 503, 504 (server errors) | Transient — safe to retry |
/// | Other 4xx (client errors) | Permanent — do not retry |
#[derive(Debug)]
pub struct HttpStatusError {
    /// The raw HTTP status code returned by the provider.
    pub status: u16,

    /// The response body text, or the `error.message` field extracted from a
    /// JSON error envelope if present.
    pub body: String,
}

impl std::fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.body)
    }
}

impl std::error::Error for HttpStatusError {}

/// Top-level error type for all minibox-llm operations.
#[derive(Debug, Error)]
pub enum LlmError {
    /// All providers in a [`FallbackChain`](crate::FallbackChain) were attempted
    /// and none succeeded. The inner string contains a semicolon-separated list
    /// of `"provider: error"` pairs for debugging.
    #[error("all providers failed: {0}")]
    AllProvidersFailed(String),

    /// A single provider returned an error. The `source` is the underlying
    /// error, which may be a `reqwest::Error` (network failure, timeout) or an
    /// [`HttpStatusError`] (non-2xx HTTP response).
    #[error("provider {provider} failed: {source}")]
    ProviderError {
        /// Display name of the provider that failed, matching [`LlmProvider::name`](crate::LlmProvider::name).
        provider: String,

        /// The underlying cause. Downcast to [`HttpStatusError`] to inspect the
        /// HTTP status, or to `reqwest::Error` for network-level failures.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The provider returned a structured-output response that could not be
    /// extracted. This is a permanent error — retrying the same request will
    /// produce the same result.
    #[error("structured output failed to parse: {0}")]
    SchemaParseError(String),
}

impl LlmError {
    /// Returns `true` for errors that are likely transient and safe to retry.
    ///
    /// Transient conditions:
    /// - `reqwest` timeout errors (`is_timeout()`)
    /// - `reqwest` connection errors (`is_connect()`)
    /// - `reqwest` request errors (`is_request()`)
    /// - HTTP 429 Too Many Requests
    /// - HTTP 500, 502, 503, 504 (server-side errors)
    ///
    /// Permanent conditions (returns `false`):
    /// - HTTP 4xx other than 429 (e.g. 400 Bad Request, 401 Unauthorized,
    ///   403 Forbidden, 404 Not Found)
    /// - [`LlmError::SchemaParseError`] — bad schema will produce the same
    ///   malformed response on every attempt
    /// - [`LlmError::AllProvidersFailed`] — aggregate error; individual
    ///   providers were already retried before contributing to this error
    pub fn is_transient(&self) -> bool {
        match self {
            LlmError::ProviderError { source, .. } => {
                // Check for reqwest errors (timeout, connect, request).
                // reqwest is feature-gated, so use cfg to avoid referencing the type
                // when no provider features are enabled.
                #[cfg(any(feature = "anthropic", feature = "openai", feature = "gemini"))]
                if let Some(reqwest_err) = source.downcast_ref::<reqwest::Error>() {
                    return reqwest_err.is_timeout()
                        || reqwest_err.is_connect()
                        || reqwest_err.is_request();
                }
                if let Some(status_err) = source.downcast_ref::<HttpStatusError>() {
                    return matches!(status_err.status, 429 | 500 | 502 | 503 | 504);
                }
                false
            }
            LlmError::SchemaParseError(_) => false,
            LlmError::AllProvidersFailed(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_error(status: u16) -> LlmError {
        LlmError::ProviderError {
            provider: "test".to_string(),
            source: Box::new(HttpStatusError {
                status,
                body: "test".to_string(),
            }),
        }
    }

    #[test]
    fn transient_http_statuses() {
        assert!(http_error(429).is_transient());
        assert!(http_error(500).is_transient());
        assert!(http_error(502).is_transient());
        assert!(http_error(503).is_transient());
        assert!(http_error(504).is_transient());
    }

    #[test]
    fn permanent_http_statuses() {
        assert!(!http_error(400).is_transient());
        assert!(!http_error(401).is_transient());
        assert!(!http_error(403).is_transient());
        assert!(!http_error(404).is_transient());
    }

    #[test]
    fn schema_parse_error_is_permanent() {
        let e = LlmError::SchemaParseError("bad".to_string());
        assert!(!e.is_transient());
    }

    #[test]
    fn all_providers_failed_is_permanent() {
        let e = LlmError::AllProvidersFailed("all failed".to_string());
        assert!(!e.is_transient());
    }
}
