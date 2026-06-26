use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::ConfluenceError;

/// The source from which the API token was loaded.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum TokenSource {
    /// Loaded from the `CONFLUENCE_TOKEN` environment variable.
    Env,
    /// Loaded from the system keyring (macOS Keychain, Windows Credential Manager, etc.).
    Keyring,
}

/// Per-profile settings from the TOML config file.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct ProfileConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_spaces: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_space: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_page_chars: Option<usize>,
}

/// Resolved, ready-to-use configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: String,
    pub api_path: String,
    /// Token is stored but never logged.
    pub token: String,
    pub token_source: TokenSource,
    pub allowed_spaces: Option<Vec<String>>,
    pub default_space: Option<String>,
    pub default_limit: u32,
    pub max_limit: u32,
    pub max_page_chars: usize,
}

/// Default path for the TOML config file.
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("cnowledje").join("config.toml"))
}

/// Load the named profile from a TOML config file, if it exists.
fn load_file_config(profile: &str) -> Result<ProfileConfig, ConfluenceError> {
    let path = match default_config_path() {
        Some(p) => p,
        None => return Ok(ProfileConfig::default()),
    };

    if !path.exists() {
        return Ok(ProfileConfig::default());
    }

    let text = std::fs::read_to_string(&path).map_err(|e| {
        ConfluenceError::ConfigError(format!("cannot read {}: {}", path.display(), e))
    })?;

    let table: HashMap<String, ProfileConfig> = toml::from_str(&text).map_err(|e| {
        ConfluenceError::ConfigError(format!("cannot parse {}: {}", path.display(), e))
    })?;

    Ok(table.get(profile).cloned().unwrap_or_default())
}

/// Build a [`Config`] by layering environment variables over the file config.
///
/// Priority (highest first):
/// 1. Environment variables (all settings; token via `CONFLUENCE_TOKEN`)
/// 2. System keyring (token only; service `cnowledje`, account = profile name)
/// 3. Config file (selected profile)
/// 4. Hard-coded defaults
pub fn load_config(profile: Option<&str>) -> Result<Config, ConfluenceError> {
    let profile = profile.unwrap_or("default");
    let file = load_file_config(profile)?;

    let base_url = std::env::var("CONFLUENCE_BASE_URL")
        .ok()
        .or(file.base_url)
        .ok_or(ConfluenceError::MissingBaseUrl)?;

    let api_path = std::env::var("CONFLUENCE_API_PATH")
        .unwrap_or_else(|_| file.api_path.unwrap_or_else(|| "/rest/api".to_string()));

    // Token resolution: env var > system keyring > error. Never stored in config file.
    let (token, token_source) = resolve_token(profile)?;

    let allowed_spaces = std::env::var("CONFLUENCE_ALLOWED_SPACES")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect::<Vec<_>>()
        })
        .or(file.allowed_spaces);

    let default_space = std::env::var("CONFLUENCE_DEFAULT_SPACE")
        .ok()
        .or(file.default_space);

    Ok(Config {
        base_url,
        api_path,
        token,
        token_source,
        allowed_spaces,
        default_space,
        default_limit: file.default_limit.unwrap_or(10),
        max_limit: file.max_limit.unwrap_or(50),
        max_page_chars: file.max_page_chars.unwrap_or(50_000),
    })
}

pub fn resolve_token(profile: &str) -> Result<(String, TokenSource), ConfluenceError> {
    if let Ok(token) = std::env::var("CONFLUENCE_TOKEN") {
        if !token.trim().is_empty() {
            return Ok((token, TokenSource::Env));
        }
    }
    let entry = keyring::Entry::new("cnowledje", profile)
        .map_err(|e| ConfluenceError::KeyringError(e.to_string()))?;
    match entry.get_password() {
        Ok(token) => Ok((token, TokenSource::Keyring)),
        Err(keyring::Error::NoEntry) => Err(ConfluenceError::MissingToken),
        Err(e) => Err(ConfluenceError::KeyringError(e.to_string())),
    }
}

pub fn store_token_in_keyring(profile: &str, token: &str) -> Result<(), ConfluenceError> {
    let entry = keyring::Entry::new("cnowledje", profile)
        .map_err(|e| ConfluenceError::KeyringError(e.to_string()))?;
    entry
        .set_password(token)
        .map_err(|e| ConfluenceError::KeyringError(e.to_string()))
}

pub fn delete_token_from_keyring(profile: &str) -> Result<(), ConfluenceError> {
    let entry = keyring::Entry::new("cnowledje", profile)
        .map_err(|e| ConfluenceError::KeyringError(e.to_string()))?;
    entry.delete_credential().map_err(|e| match e {
        keyring::Error::NoEntry => {
            ConfluenceError::KeyringError("no token found in keyring".to_string())
        }
        e => ConfluenceError::KeyringError(e.to_string()),
    })
}

/// Validate that all requested spaces are in the allowlist.
pub fn validate_spaces(spaces: &[String], config: &Config) -> Result<(), ConfluenceError> {
    if let Some(allowed) = &config.allowed_spaces {
        for space in spaces {
            if !allowed.iter().any(|a| a.eq_ignore_ascii_case(space)) {
                return Err(ConfluenceError::SpaceNotAllowed(space.clone()));
            }
        }
    }
    Ok(())
}

/// Resolve the effective space list from CLI args and config defaults.
pub fn resolve_spaces(
    cli_spaces: Vec<String>,
    config: &Config,
) -> Result<Vec<String>, ConfluenceError> {
    if cli_spaces.is_empty() {
        match &config.default_space {
            Some(s) => Ok(vec![s.clone()]),
            None => Err(ConfluenceError::NoSpaceSpecified),
        }
    } else {
        Ok(cli_spaces)
    }
}

/// 指定プロファイルが設定ファイルに存在するか確認する。
/// ファイルが存在しない場合は false を返す。
pub fn profile_exists(profile_name: &str) -> Result<bool, ConfluenceError> {
    match default_config_path() {
        Some(path) => profile_exists_at_path(profile_name, &path),
        None => Ok(false),
    }
}

/// テスト用: 指定パスの設定ファイルでプロファイル存在確認。
pub fn profile_exists_at_path(
    profile_name: &str,
    path: &std::path::Path,
) -> Result<bool, ConfluenceError> {
    if !path.exists() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(path).map_err(|e| {
        ConfluenceError::ConfigError(format!("cannot read {}: {}", path.display(), e))
    })?;
    let table: HashMap<String, toml::Value> = toml::from_str(&text).map_err(|e| {
        ConfluenceError::ConfigError(format!("cannot parse {}: {}", path.display(), e))
    })?;
    Ok(table.contains_key(profile_name))
}

/// 指定プロファイルを設定ファイルに書き込む（他のプロファイルは保持）。
/// ファイルや親ディレクトリが存在しない場合は作成する。
/// 成功時に書き込んだファイルのパスを返す。
pub fn save_profile(
    profile_name: &str,
    config: &ProfileConfig,
) -> Result<PathBuf, ConfluenceError> {
    let path = default_config_path().ok_or_else(|| {
        ConfluenceError::ConfigError("cannot determine config directory".to_string())
    })?;
    save_profile_to_path(profile_name, config, &path)?;
    Ok(path)
}

/// テスト用: 指定パスに書き込む save_profile。
pub fn save_profile_to_path(
    profile_name: &str,
    config: &ProfileConfig,
    path: &std::path::Path,
) -> Result<(), ConfluenceError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ConfluenceError::ConfigError(format!(
                "cannot create directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    let mut table: HashMap<String, toml::Value> = if path.exists() {
        let text = std::fs::read_to_string(path).map_err(|e| {
            ConfluenceError::ConfigError(format!("cannot read {}: {}", path.display(), e))
        })?;
        toml::from_str(&text).map_err(|e| {
            ConfluenceError::ConfigError(format!("cannot parse {}: {}", path.display(), e))
        })?
    } else {
        HashMap::new()
    };

    let profile_value = toml::Value::try_from(config)
        .map_err(|e| ConfluenceError::ConfigError(format!("cannot serialize profile: {}", e)))?;
    table.insert(profile_name.to_string(), profile_value);

    let toml_str = toml::to_string_pretty(&table)
        .map_err(|e| ConfluenceError::ConfigError(format!("cannot serialize config: {}", e)))?;
    std::fs::write(path, toml_str).map_err(|e| {
        ConfluenceError::ConfigError(format!("cannot write {}: {}", path.display(), e))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn save_creates_new_file_with_profile() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let profile = ProfileConfig {
            base_url: Some("https://confluence.example.com".to_string()),
            api_path: Some("/rest/api".to_string()),
            ..Default::default()
        };

        save_profile_to_path("default", &profile, &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[default]"));
        assert!(content.contains("https://confluence.example.com"));
        assert!(content.contains("/rest/api"));
    }

    #[test]
    fn save_preserves_other_profiles() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        std::fs::write(
            &path,
            "[staging]\nbase_url = \"https://staging.example.com\"\n",
        )
        .unwrap();

        let profile = ProfileConfig {
            base_url: Some("https://prod.example.com".to_string()),
            ..Default::default()
        };

        save_profile_to_path("default", &profile, &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[staging]"));
        assert!(content.contains("https://staging.example.com"));
        assert!(content.contains("[default]"));
        assert!(content.contains("https://prod.example.com"));
    }

    #[test]
    fn save_omits_none_fields() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let profile = ProfileConfig {
            base_url: Some("https://example.com".to_string()),
            ..Default::default()
        };

        save_profile_to_path("default", &profile, &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("default_space"));
        assert!(!content.contains("allowed_spaces"));
        assert!(!content.contains("api_path"));
        assert!(!content.contains("default_limit"));
    }

    #[test]
    fn save_overwrites_existing_profile() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        std::fs::write(&path, "[default]\nbase_url = \"https://old.example.com\"\n").unwrap();

        let profile = ProfileConfig {
            base_url: Some("https://new.example.com".to_string()),
            ..Default::default()
        };

        save_profile_to_path("default", &profile, &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            !content.contains("old.example.com"),
            "old URL should be gone"
        );
        assert!(content.contains("new.example.com"));
    }

    #[test]
    fn profile_exists_returns_false_for_missing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert!(!profile_exists_at_path("default", &path).unwrap());
    }

    #[test]
    fn profile_exists_returns_true_for_existing_and_false_for_absent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        std::fs::write(&path, "[default]\nbase_url = \"https://example.com\"\n").unwrap();

        assert!(profile_exists_at_path("default", &path).unwrap());
        assert!(!profile_exists_at_path("staging", &path).unwrap());
    }

    #[test]
    fn resolve_token_prefers_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old = std::env::var("CONFLUENCE_TOKEN").ok();
        std::env::set_var("CONFLUENCE_TOKEN", "test_env_token_cnowledje_xyz");
        let result = resolve_token("__cnowledje_test_env_priority__");
        match &old {
            Some(v) => std::env::set_var("CONFLUENCE_TOKEN", v),
            None => std::env::remove_var("CONFLUENCE_TOKEN"),
        }
        let (token, source) = result.unwrap();
        assert_eq!(token, "test_env_token_cnowledje_xyz");
        assert_eq!(source, TokenSource::Env);
    }

    #[test]
    fn resolve_token_errors_without_env_and_keyring() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old = std::env::var("CONFLUENCE_TOKEN").ok();
        std::env::remove_var("CONFLUENCE_TOKEN");
        let result = resolve_token("__cnowledje_test_nonexistent_profile_12345__");
        match old {
            Some(v) => std::env::set_var("CONFLUENCE_TOKEN", v),
            None => {}
        }
        assert!(result.is_err());
    }
}
