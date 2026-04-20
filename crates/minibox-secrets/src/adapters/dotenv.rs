use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::{
    CacheHint, CredentialError, CredentialProvider, CredentialRef, CredentialScheme,
    FetchedCredential, FetchedCredentialInner, build_credential,
};

const DEFAULT_FILENAMES: [&str; 2] = [".env", ".env.local"];

/// Reads credentials from dotenv-style files discovered from the current repo.
///
/// Ref formats:
/// - `dotenv:MY_API_KEY:api_key` (search upward for `.env` / `.env.local`)
/// - `dotenv:config/dev.env#MY_API_KEY:api_key` (explicit file path + key)
pub struct DotenvProvider {
    start_dir: PathBuf,
}

impl DotenvProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_dir(start_dir: impl Into<PathBuf>) -> Self {
        Self {
            start_dir: start_dir.into(),
        }
    }

    fn resolve_ref(&self, raw_path: &str) -> Result<(Vec<PathBuf>, String), CredentialError> {
        if raw_path.is_empty() {
            return Err(CredentialError::InvalidFormat(
                "dotenv ref must include a variable name".into(),
            ));
        }

        if let Some((file_part, key)) = raw_path.split_once('#') {
            if file_part.is_empty() || key.is_empty() {
                return Err(CredentialError::InvalidFormat(format!(
                    "dotenv ref path must be `<file>#<key>`, got `{raw_path}`"
                )));
            }

            return Ok((vec![self.resolve_path(file_part)], key.to_string()));
        }

        if let Ok(path) = std::env::var("MINIBOX_SECRETS_FILE") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return Ok((vec![self.resolve_path(trimmed)], raw_path.to_string()));
            }
        }

        let dir = self.find_env_dir().ok_or_else(|| {
            CredentialError::NotFound(format!(
                "no .env or .env.local found while resolving `{raw_path}` from {}",
                self.start_dir.display()
            ))
        })?;

        let files = DEFAULT_FILENAMES
            .iter()
            .map(|name| dir.join(name))
            .filter(|path| path.exists())
            .collect::<Vec<_>>();

        Ok((files, raw_path.to_string()))
    }

    fn resolve_path(&self, raw: &str) -> PathBuf {
        let path = PathBuf::from(raw);
        if path.is_absolute() {
            path
        } else {
            self.start_dir.join(path)
        }
    }

    fn find_env_dir(&self) -> Option<PathBuf> {
        self.start_dir
            .ancestors()
            .find(|dir| DEFAULT_FILENAMES.iter().any(|name| dir.join(name).exists()))
            .map(Path::to_path_buf)
    }
}

impl Default for DotenvProvider {
    fn default() -> Self {
        Self {
            start_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

#[async_trait::async_trait]
impl CredentialProvider for DotenvProvider {
    async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError> {
        if r.scheme != CredentialScheme::Dotenv {
            return Err(CredentialError::NotFound(format!(
                "DotenvProvider cannot handle scheme {:?}",
                r.scheme
            )));
        }

        let (files, key) = self.resolve_ref(&r.path)?;
        let mut resolved = None;

        for file in files {
            let vars = parse_dotenv_file(&file)?;
            if let Some(value) = vars
                .into_iter()
                .find_map(|(candidate, value)| (candidate == key).then_some(value))
            {
                resolved = Some(value);
            }
        }

        let raw = resolved.ok_or_else(|| {
            CredentialError::NotFound(format!("dotenv key `{key}` not found for `{}`", r.as_str()))
        })?;

        let credential = build_credential(r.kind, raw);

        Ok(Arc::new(FetchedCredentialInner {
            credential,
            cache: CacheHint::Never,
        }))
    }
}

fn parse_dotenv_file(path: &Path) -> Result<Vec<(String, String)>, CredentialError> {
    let text = std::fs::read_to_string(path).map_err(|err| {
        CredentialError::ProviderUnavailable(format!(
            "could not read dotenv file {}: {err}",
            path.display()
        ))
    })?;

    let mut vars = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(entry) = parse_dotenv_line(line).map_err(|err| {
            CredentialError::InvalidFormat(format!("{}:{}: {err}", path.display(), index + 1))
        })? {
            vars.push(entry);
        }
    }

    Ok(vars)
}

fn parse_dotenv_line(line: &str) -> Result<Option<(String, String)>, String> {
    let mut trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(None);
    }

    if let Some(rest) = trimmed.strip_prefix("export ") {
        trimmed = rest.trim_start();
    }

    let (key, raw_value) = trimmed
        .split_once('=')
        .ok_or_else(|| "expected KEY=VALUE".to_string())?;

    let key = key.trim();
    if key.is_empty() {
        return Err("dotenv key is empty".into());
    }

    let value = parse_dotenv_value(raw_value.trim())?;
    Ok(Some((key.to_string(), value)))
}

fn parse_dotenv_value(raw: &str) -> Result<String, String> {
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        return Ok(decode_double_quoted(&raw[1..raw.len() - 1]));
    }

    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return Ok(raw[1..raw.len() - 1].to_string());
    }

    let value = raw
        .split_once(" #")
        .map_or(raw, |(value, _)| value)
        .trim_end();
    Ok(value.to_string())
}

fn decode_double_quoted(raw: &str) -> String {
    let mut decoded = String::with_capacity(raw.len());
    let mut chars = raw.chars();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => decoded.push('\n'),
                Some('r') => decoded.push('\r'),
                Some('t') => decoded.push('\t'),
                Some('\\') => decoded.push('\\'),
                Some('"') => decoded.push('"'),
                Some(other) => {
                    decoded.push('\\');
                    decoded.push(other);
                }
                None => decoded.push('\\'),
            }
        } else {
            decoded.push(ch);
        }
    }

    decoded
}

#[cfg(test)]
mod tests {
    use crate::domain::Credential;

    use super::*;

    fn ref_for(raw: &str) -> CredentialRef {
        CredentialRef::parse(raw).expect("valid credential ref")
    }

    #[tokio::test]
    async fn reads_value_from_nearest_env_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "OPENAI_API_KEY=sk-from-env\n").unwrap();
        let nested = dir.path().join("nested").join("deeper");
        std::fs::create_dir_all(&nested).unwrap();

        let provider = DotenvProvider::from_dir(&nested);
        let fetched = provider
            .get(&ref_for("dotenv:OPENAI_API_KEY:api_key"))
            .await
            .unwrap();

        match &fetched.credential {
            Credential::ApiKey(key) => {
                assert_eq!(key.hash, crate::domain::sha256_hex(b"sk-from-env"))
            }
            _ => panic!("expected api key"),
        }
    }

    #[tokio::test]
    async fn env_local_overrides_env() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "OPENAI_API_KEY=sk-base\n").unwrap();
        std::fs::write(
            dir.path().join(".env.local"),
            "OPENAI_API_KEY=\"sk-local\"\n",
        )
        .unwrap();

        let provider = DotenvProvider::from_dir(dir.path());
        let fetched = provider
            .get(&ref_for("dotenv:OPENAI_API_KEY:api_key"))
            .await
            .unwrap();

        match &fetched.credential {
            Credential::ApiKey(key) => assert_eq!(key.hash, crate::domain::sha256_hex(b"sk-local")),
            _ => panic!("expected api key"),
        }
    }

    #[tokio::test]
    async fn explicit_file_path_uses_hash_separator() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("custom.env"),
            "export GEMINI_API_KEY='gk-custom'\n",
        )
        .unwrap();

        let provider = DotenvProvider::from_dir(dir.path());
        let fetched = provider
            .get(&ref_for("dotenv:custom.env#GEMINI_API_KEY:api_key"))
            .await
            .unwrap();

        match &fetched.credential {
            Credential::ApiKey(key) => {
                assert_eq!(key.hash, crate::domain::sha256_hex(b"gk-custom"))
            }
            _ => panic!("expected api key"),
        }
    }

    #[tokio::test]
    async fn missing_key_returns_not_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "OPENAI_API_KEY=sk-base\n").unwrap();

        let provider = DotenvProvider::from_dir(dir.path());
        let result = provider
            .get(&ref_for("dotenv:ANTHROPIC_API_KEY:api_key"))
            .await;

        assert!(matches!(result, Err(CredentialError::NotFound(_))));
    }
}
