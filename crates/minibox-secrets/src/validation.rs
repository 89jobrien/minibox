use chrono::Utc;
use secrecy::ExposeSecret;

use crate::domain::{ApiKey, DatabaseUrl, SshKey, Token, Validate, ValidationError};

impl Validate for ApiKey {
    fn validate(&self) -> Result<(), ValidationError> {
        let raw = self.key.expose_secret();
        if raw.is_empty() {
            return Err(ValidationError::Empty { field: "key" });
        }
        if raw.len() < 8 {
            return Err(ValidationError::Invalid(
                "api key must be at least 8 characters".into(),
            ));
        }
        if raw.chars().any(|c| c.is_ascii_whitespace()) {
            return Err(ValidationError::Invalid(
                "api key must not contain whitespace".into(),
            ));
        }
        Ok(())
    }
}

impl Validate for Token {
    fn validate(&self) -> Result<(), ValidationError> {
        let raw = self.token.expose_secret();
        if raw.is_empty() {
            return Err(ValidationError::Empty { field: "token" });
        }
        if let Some(exp) = self.expires_at {
            if Utc::now() >= exp {
                return Err(ValidationError::Expired);
            }
        }
        Ok(())
    }
}

const ALLOWED_DB_SCHEMES: &[&str] = &["postgres", "postgresql", "mysql", "sqlite", "redis"];

impl Validate for DatabaseUrl {
    fn validate(&self) -> Result<(), ValidationError> {
        let raw = self.url.expose_secret();
        if raw.is_empty() {
            return Err(ValidationError::Empty { field: "url" });
        }
        // Extract scheme: everything before the first ':'
        let scheme = raw.split(':').next().unwrap_or("");
        if !ALLOWED_DB_SCHEMES.contains(&scheme) {
            return Err(ValidationError::Invalid(format!(
                "unsupported database scheme `{scheme}`; expected one of: {}",
                ALLOWED_DB_SCHEMES.join(", ")
            )));
        }
        Ok(())
    }
}

impl Validate for SshKey {
    fn validate(&self) -> Result<(), ValidationError> {
        let raw = self.private_key.expose_secret();
        if raw.is_empty() {
            return Err(ValidationError::Empty {
                field: "private_key",
            });
        }
        if !raw.starts_with("-----BEGIN") {
            return Err(ValidationError::Invalid(
                "ssh key must start with a PEM header (-----BEGIN ...)".into(),
            ));
        }
        if !raw.contains("PRIVATE KEY") {
            return Err(ValidationError::Invalid(
                "ssh key PEM block must contain PRIVATE KEY".into(),
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domain::{ApiKey, DatabaseUrl, SshKey, Token};

    // ApiKey
    #[test]
    fn api_key_valid() {
        assert!(ApiKey::new("sk-12345678").validate().is_ok());
    }

    #[test]
    fn api_key_empty() {
        let e = ApiKey::new("").validate().unwrap_err();
        assert!(matches!(e, ValidationError::Empty { field: "key" }));
    }

    #[test]
    fn api_key_too_short() {
        assert!(ApiKey::new("abc123").validate().is_err());
    }

    #[test]
    fn api_key_whitespace() {
        assert!(ApiKey::new("sk abc 1234").validate().is_err());
    }

    // Token
    #[test]
    fn token_valid() {
        assert!(Token::new("some-valid-token", None).validate().is_ok());
    }

    #[test]
    fn token_empty() {
        assert!(matches!(
            Token::new("", None).validate().unwrap_err(),
            ValidationError::Empty { field: "token" }
        ));
    }

    #[test]
    fn token_expired() {
        let past = Utc::now() - Duration::hours(1);
        assert!(matches!(
            Token::new("tok", Some(past)).validate().unwrap_err(),
            ValidationError::Expired
        ));
    }

    #[test]
    fn token_not_yet_expired() {
        let future = Utc::now() + Duration::hours(1);
        assert!(
            Token::new("valid-token-abc", Some(future))
                .validate()
                .is_ok()
        );
    }

    // DatabaseUrl
    #[test]
    fn db_url_postgres() {
        assert!(
            DatabaseUrl::new("postgres://user:pass@host/db")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn db_url_bad_scheme() {
        assert!(
            DatabaseUrl::new("http://user:pass@host/db")
                .validate()
                .is_err()
        );
    }

    #[test]
    fn db_url_empty() {
        assert!(DatabaseUrl::new("").validate().is_err());
    }

    // SshKey
    #[test]
    fn ssh_key_valid() {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
        assert!(SshKey::new(pem).validate().is_ok());
    }

    #[test]
    fn ssh_key_encrypted_accepted() {
        let pem = "-----BEGIN ENCRYPTED PRIVATE KEY-----\nabc\n-----END ENCRYPTED PRIVATE KEY-----";
        assert!(SshKey::new(pem).validate().is_ok());
    }

    #[test]
    fn ssh_key_missing_header() {
        assert!(SshKey::new("not a key").validate().is_err());
    }

    #[test]
    fn ssh_key_wrong_block_type() {
        assert!(
            SshKey::new("-----BEGIN CERTIFICATE-----\nabc")
                .validate()
                .is_err()
        );
    }
}
