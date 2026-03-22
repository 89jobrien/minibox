use std::{process::Command, sync::Arc, time::Duration};

use crate::domain::{
    CacheHint, CredentialError, CredentialProvider, CredentialRef, CredentialScheme,
    FetchedCredential, FetchedCredentialInner, build_credential,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Fetches credentials via the `bw` CLI (Bitwarden).
///
/// Ref format: `bw:<item-name>/<field>:<kind>`
///
/// `BW_SESSION` must be set in the environment by the caller. This adapter does not
/// manage Bitwarden session tokens.
///
/// Field resolution order:
/// 1. `fields` array — find the entry where `name == field`, use `.value`
/// 2. `login.password` — fallback when no matching custom field found
pub struct BitwardenProvider {
    timeout: Duration,
}

impl BitwardenProvider {
    pub fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Default for BitwardenProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl CredentialProvider for BitwardenProvider {
    async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError> {
        if r.scheme != CredentialScheme::Bitwarden {
            return Err(CredentialError::NotFound(format!(
                "BitwardenProvider cannot handle scheme {:?}",
                r.scheme
            )));
        }

        // path = "<item-name>/<field>"
        let (item_name, field_name) = r.path.split_once('/').ok_or_else(|| {
            CredentialError::InvalidFormat(format!(
                "bitwarden ref path must be `item-name/field`, got `{}`",
                r.path
            ))
        })?;

        let item_name = item_name.to_string();
        let field_name = field_name.to_string();
        let kind = r.kind;
        let timeout = self.timeout;

        let output = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                Command::new("bw")
                    .args(["get", "item", &item_name])
                    .output()
            }),
        )
        .await
        .map_err(|_| {
            CredentialError::ProviderUnavailable(format!(
                "bw timed out after {}s",
                timeout.as_secs()
            ))
        })?
        .map_err(|e| CredentialError::ProviderUnavailable(e.to_string()))?
        .map_err(|e| {
            CredentialError::ProviderUnavailable(format!("`bw` not found or not executable: {e}"))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
            return if stderr.contains("vault is locked") || stderr.contains("not logged in") {
                Err(CredentialError::ProviderUnavailable(
                    "Bitwarden: vault is locked or not logged in".into(),
                ))
            } else {
                Err(CredentialError::NotFound(
                    r.path.split('/').next().unwrap_or(&r.path).to_string(),
                ))
            };
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let raw = extract_field(&stdout, &field_name)?;

        Ok(Arc::new(FetchedCredentialInner {
            credential: build_credential(kind, raw),
            cache: CacheHint::Session,
        }))
    }
}

/// Extract a field value from `bw get item` JSON output.
///
/// Resolution order:
/// 1. `fields[].value` where `fields[].name == field_name`
/// 2. `login.password`
fn extract_field(json: &str, field_name: &str) -> Result<String, CredentialError> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| CredentialError::InvalidFormat(format!("bw JSON parse error: {e}")))?;

    // 1. Custom fields array
    if let Some(fields) = v.get("fields").and_then(|f| f.as_array()) {
        for field in fields {
            if field.get("name").and_then(|n| n.as_str()) == Some(field_name) {
                if let Some(value) = field.get("value").and_then(|v| v.as_str()) {
                    return Ok(value.to_string());
                }
            }
        }
    }

    // 2. login.password fallback
    if let Some(password) = v
        .get("login")
        .and_then(|l| l.get("password"))
        .and_then(|p| p.as_str())
    {
        return Ok(password.to_string());
    }

    Err(CredentialError::InvalidFormat(format!(
        "field `{field_name}` not found in Bitwarden item"
    )))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_custom_field() {
        let json = r#"{
            "fields": [
                {"name": "api_key", "value": "sk-abc123"},
                {"name": "other", "value": "nope"}
            ]
        }"#;
        assert_eq!(extract_field(json, "api_key").unwrap(), "sk-abc123");
    }

    #[test]
    fn extract_login_password_fallback() {
        let json = r#"{"login": {"password": "hunter2"}}"#;
        assert_eq!(extract_field(json, "password").unwrap(), "hunter2");
    }

    #[test]
    fn extract_missing_field_errors() {
        let json = r#"{"login": {"username": "joe"}}"#;
        assert!(extract_field(json, "api_key").is_err());
    }

    #[test]
    fn extract_prefers_fields_over_login() {
        let json = r#"{
            "fields": [{"name": "password", "value": "from-fields"}],
            "login": {"password": "from-login"}
        }"#;
        assert_eq!(extract_field(json, "password").unwrap(), "from-fields");
    }
}
