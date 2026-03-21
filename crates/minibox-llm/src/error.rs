use thiserror::Error;

/// Typed HTTP error for status code classification via downcasting.
#[derive(Debug)]
pub struct HttpStatusError {
    pub status: u16,
    pub body: String,
}

impl std::fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.body)
    }
}

impl std::error::Error for HttpStatusError {}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("all providers failed: {0}")]
    AllProvidersFailed(String),

    #[error("provider {provider} failed: {source}")]
    ProviderError {
        provider: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("structured output failed to parse: {0}")]
    SchemaParseError(String),
}

impl LlmError {
    /// Returns `true` for errors that may succeed on retry (timeouts, 429, 5xx).
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
