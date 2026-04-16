use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{YrrError, Result};

/// Top-level configuration, loaded from `yrr.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub safety: SafetyConfig,
    pub defaults: DefaultsConfig,
    pub claude: ClaudeConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            safety: SafetyConfig::default(),
            defaults: DefaultsConfig::default(),
            claude: ClaudeConfig::default(),
        }
    }
}

/// Safety limits that apply globally.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct SafetyConfig {
    /// Hard cap on activations per agent, regardless of lifecycle config.
    pub max_activations: u32,
    /// Max query loop iterations per activation (prevents infinite query loops).
    pub max_query_iterations: u32,
    /// Default timeout for queries in seconds.
    pub default_query_timeout_secs: u64,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            max_activations: 50,
            max_query_iterations: 5,
            default_query_timeout_secs: 120,
        }
    }
}

/// Default values applied when agents don't specify their own.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DefaultsConfig {
    /// Default model when not specified in agent config.
    pub model: Option<String>,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self { model: None }
    }
}

/// Claude Code CLI-specific configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ClaudeConfig {
    /// How Claude Code handles tool permissions in non-interactive mode.
    ///
    /// - `"auto"` — translate agent `permissions.tools.allow` into `--allowedTools`
    ///   (recommended, agents can only use explicitly allowed tools)
    /// - `"bypass"` — pass `--dangerously-skip-permissions` (agents can do anything)
    /// - `"none"` — no permission flags at all (most tools will be denied in `--print` mode)
    pub permission_mode: String,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            permission_mode: "auto".to_string(),
        }
    }
}

/// Load config from a TOML file. Returns default config if the file doesn't exist.
pub fn load_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }

    let content = std::fs::read_to_string(path)?;
    let config: Config =
        toml::from_str(&content).map_err(|e| YrrError::Other(format!("bad config: {e}")))?;
    Ok(config)
}

/// Search for `yrr.toml` in the given directory and parent directories.
pub fn find_config(start_dir: &Path) -> Result<Config> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let candidate = dir.join("yrr.toml");
        if candidate.exists() {
            return load_config(&candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    Ok(Config::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = Config::default();
        assert_eq!(config.safety.max_activations, 50);
        assert_eq!(config.claude.permission_mode, "auto");
        assert!(config.defaults.model.is_none());
    }

    #[test]
    fn parse_partial_config() {
        let toml = r#"
[safety]
max_activations = 10

[defaults]
model = "haiku"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.safety.max_activations, 10);
        assert_eq!(config.defaults.model, Some("haiku".to_string()));
        // claude section uses defaults since not specified
        assert_eq!(config.claude.permission_mode, "auto");
    }
}
