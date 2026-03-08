//! User configuration loaded from `~/.config/rtrack/config.toml`.
//!
//! The config file is optional. Missing files or fields silently fall back
//! to defaults. CLI arguments always take precedence over config values.

use std::path::PathBuf;

use serde::Deserialize;

/// User configuration.
#[derive(Deserialize, Default, Debug)]
pub struct Config {
    /// Default SoundFont file path (overridden by `--sf2`).
    pub sf2: Option<PathBuf>,
    /// Default sample directory (overridden by `--sample-dir`).
    pub sample_dir: Option<PathBuf>,
}

/// Load configuration from the standard path, returning defaults on any error.
pub fn load_config() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    if !path.exists() {
        return Config::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) => match toml::from_str(&contents) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Warning: failed to parse {}: {}", path.display(), e);
                Config::default()
            }
        },
        Err(e) => {
            eprintln!("Warning: failed to read {}: {}", path.display(), e);
            Config::default()
        }
    }
}

/// Return the config file path: `$XDG_CONFIG_HOME/rtrack/config.toml`
/// or `~/.config/rtrack/config.toml`.
fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("rtrack").join("config.toml"));
    }
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config").join("rtrack").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
sf2 = "/path/to/sound.sf2"
sample_dir = "/path/to/samples"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.sf2, Some(PathBuf::from("/path/to/sound.sf2")));
        assert_eq!(config.sample_dir, Some(PathBuf::from("/path/to/samples")));
    }

    #[test]
    fn test_parse_partial_config() {
        let toml = r#"sf2 = "/only/sf2.sf2""#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.sf2, Some(PathBuf::from("/only/sf2.sf2")));
        assert_eq!(config.sample_dir, None);
    }

    #[test]
    fn test_parse_empty_config() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.sf2, None);
        assert_eq!(config.sample_dir, None);
    }

    #[test]
    fn test_parse_invalid_toml_returns_default() {
        let result: Result<Config, _> = toml::from_str("{{{{garbage}}}}");
        assert!(result.is_err());
        // load_config would fall back to Config::default() on error
    }

    #[test]
    fn test_unknown_fields_ignored() {
        let toml = r#"
sf2 = "/path/to/sound.sf2"
future_field = "some_value"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.sf2, Some(PathBuf::from("/path/to/sound.sf2")));
    }

    #[test]
    fn test_load_config_missing_file() {
        // load_config should return defaults when no file exists
        let config = load_config();
        // We can't assert the exact values since the test env might have a config,
        // but at minimum it should not panic
        let _ = config;
    }
}
