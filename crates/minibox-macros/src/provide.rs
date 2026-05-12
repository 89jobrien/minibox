//! `provide!` -- generate provider constructors for LLM adapters.
//!
//! Intended for use inside LLM provider modules. References
//! `crate::ProviderConfig` at the call site, so it expands against
//! `minibox_llm`, not `minibox_macros`.
//!
//! # Examples
//!
//! ```rust,ignore
//! minibox_macros::provide!(OpenAiProvider, "OPENAI_API_KEY", "gpt-4.1");
//! ```

#[allow(clippy::crate_in_macro_def)]
#[macro_export]
macro_rules! provide {
    ($provider:ty, $env_var:expr, $default_model:expr) => {
        impl $provider {
            /// Construct this provider from the environment using default HTTP timeouts.
            ///
            /// Returns `None` if the required API key environment variable is not set.
            pub fn from_env() -> Option<Self> {
                Self::from_env_with_config(&crate::ProviderConfig::default())
            }

            /// Construct this provider from the environment with explicit HTTP configuration.
            ///
            /// Returns `None` if the required API key environment variable is not set.
            pub fn from_env_with_config(config: &crate::ProviderConfig) -> Option<Self> {
                ::std::env::var($env_var)
                    .ok()
                    .map(|k| Self::with_config(k, $default_model.to_string(), config))
            }

            /// Test helper -- inject a key without reading the environment.
            #[cfg(test)]
            pub(crate) fn from_key(key: String) -> Self {
                Self::new(key, $default_model.to_string())
            }
        }
    };
}
