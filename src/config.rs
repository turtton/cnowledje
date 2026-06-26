use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::error::ConfluenceError;

/// Per-profile settings from the TOML config file.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct ProfileConfig {
    pub base_url: Option<String>,
    pub api_path: Option<String>,
    pub allowed_spaces: Option<Vec<String>>,
    pub default_space: Option<String>,
    pub default_limit: Option<u32>,
    pub max_limit: Option<u32>,
    pub max_page_chars: Option<usize>,
}

/// Resolved, ready-to-use configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: String,
    pub api_path: String,
    /// Token is stored but never logged.
    pub token: String,
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

    let text = std::fs::read_to_string(&path)
        .map_err(|e| ConfluenceError::ConfigError(format!("cannot read {}: {}", path.display(), e)))?;

    let table: HashMap<String, ProfileConfig> = toml::from_str(&text)
        .map_err(|e| ConfluenceError::ConfigError(format!("cannot parse {}: {}", path.display(), e)))?;

    Ok(table.get(profile).cloned().unwrap_or_default())
}

/// Build a [`Config`] by layering environment variables over the file config.
///
/// Priority (highest first):
/// 1. Environment variables
/// 2. Config file (selected profile)
/// 3. Hard-coded defaults
pub fn load_config(profile: Option<&str>) -> Result<Config, ConfluenceError> {
    let profile = profile.unwrap_or("default");
    let file = load_file_config(profile)?;

    let base_url = std::env::var("CONFLUENCE_BASE_URL")
        .ok()
        .or(file.base_url)
        .ok_or(ConfluenceError::MissingBaseUrl)?;

    let api_path = std::env::var("CONFLUENCE_API_PATH")
        .unwrap_or_else(|_| file.api_path.unwrap_or_else(|| "/rest/api".to_string()));

    // Token must come from the environment; never store in config file.
    let token = std::env::var("CONFLUENCE_TOKEN")
        .map_err(|_| ConfluenceError::MissingToken)?;

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
        allowed_spaces,
        default_space,
        default_limit: file.default_limit.unwrap_or(10),
        max_limit: file.max_limit.unwrap_or(50),
        max_page_chars: file.max_page_chars.unwrap_or(50_000),
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
