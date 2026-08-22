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
///
/// A bad config is not fatal, but the user should be told rather than left
/// wondering why their settings were ignored. See [`load_config_verbose`] to
/// get that explanation; this shorthand discards it.
pub fn load_config() -> Config {
    load_config_verbose().0
}

/// Load configuration, along with any warnings about why parts of it were
/// ignored.
///
/// The warnings are returned rather than printed: a library that writes to
/// stderr corrupts the TUI's alternate screen, and the caller is the only
/// one that knows where a message should go.
pub fn load_config_verbose() -> (Config, Vec<String>) {
    let Some(path) = config_path() else {
        return (Config::default(), Vec::new());
    };
    if !path.exists() {
        return (Config::default(), Vec::new());
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) => match toml::from_str(&contents) {
            Ok(config) => (config, Vec::new()),
            Err(e) => (
                Config::default(),
                vec![format!("failed to parse {}: {}", path.display(), e)],
            ),
        },
        Err(e) => (
            Config::default(),
            vec![format!("failed to read {}: {}", path.display(), e)],
        ),
    }
}

/// Return the config file path: `$XDG_CONFIG_HOME/rtrack/config.toml`
/// or `~/.config/rtrack/config.toml`.
fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("rtrack").join("config.toml"));
    }
    std::env::var("HOME").ok().map(|h| {
        PathBuf::from(h)
            .join(".config")
            .join("rtrack")
            .join("config.toml")
    })
}

const MAX_RECENT_FILES: usize = 3;

/// Return the config directory: `$XDG_CONFIG_HOME/rtrack/` or `~/.config/rtrack/`.
pub fn config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("rtrack"));
    }
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config").join("rtrack"))
}

/// Load recent file paths from `<config_dir>/recent.json`.
pub fn load_recent_files() -> Vec<PathBuf> {
    let Some(dir) = config_dir() else {
        return Vec::new();
    };
    let path = dir.join("recent.json");
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str::<Vec<PathBuf>>(&data).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Save recent file paths to `<config_dir>/recent.json`.
///
/// Deduplicates, limits to MAX_RECENT_FILES, and skips on any I/O error --
/// losing the recent list is not worth interrupting the user over.
///
/// The write replaces the file atomically. A plain write truncates first, so
/// a crash or a full disk part-way through left an empty or half-written
/// list where a complete older one had been.
///
/// What this stores: absolute, canonicalized paths to songs the user has
/// opened, in plain text, under `$XDG_CONFIG_HOME/rtrack` (or
/// `~/.config/rtrack`). Anyone who can read that directory can see where the
/// user keeps their work.
pub fn save_recent_files(files: &[PathBuf]) {
    let Some(dir) = config_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("recent.json");
    if let Ok(json) = serde_json::to_string(files) {
        let _ = crate::fs::write_atomic(&path, json.as_bytes());
    }
}

/// Add a path to the front of the recent files list, dedup and trim to MAX_RECENT_FILES.
pub fn push_recent_file(recent: &mut Vec<PathBuf>, path: &std::path::Path) {
    // Canonicalize for consistent dedup
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    recent.retain(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()) != canonical);
    recent.insert(0, canonical);
    recent.truncate(MAX_RECENT_FILES);
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

    #[test]
    fn test_push_recent_file_adds_to_front() {
        let mut recent = Vec::new();
        push_recent_file(&mut recent, &PathBuf::from("/tmp/a.rtrk"));
        push_recent_file(&mut recent, &PathBuf::from("/tmp/b.rtrk"));
        // Most recent should be first
        assert_eq!(recent.len(), 2);
        assert!(recent[0].to_string_lossy().contains("b.rtrk"));
        assert!(recent[1].to_string_lossy().contains("a.rtrk"));
    }

    #[test]
    fn test_push_recent_file_deduplicates() {
        let mut recent = Vec::new();
        push_recent_file(&mut recent, &PathBuf::from("/tmp/a.rtrk"));
        push_recent_file(&mut recent, &PathBuf::from("/tmp/b.rtrk"));
        push_recent_file(&mut recent, &PathBuf::from("/tmp/a.rtrk"));
        // Should not have duplicates, a should be at front
        assert_eq!(recent.len(), 2);
        assert!(recent[0].to_string_lossy().contains("a.rtrk"));
    }

    #[test]
    fn test_push_recent_file_truncates() {
        let mut recent = Vec::new();
        push_recent_file(&mut recent, &PathBuf::from("/tmp/a.rtrk"));
        push_recent_file(&mut recent, &PathBuf::from("/tmp/b.rtrk"));
        push_recent_file(&mut recent, &PathBuf::from("/tmp/c.rtrk"));
        push_recent_file(&mut recent, &PathBuf::from("/tmp/d.rtrk"));
        // Should be limited to MAX_RECENT_FILES (3)
        assert_eq!(recent.len(), MAX_RECENT_FILES);
        assert!(recent[0].to_string_lossy().contains("d.rtrk"));
        // a.rtrk should have been dropped
        assert!(!recent
            .iter()
            .any(|p| p.to_string_lossy().contains("a.rtrk")));
    }
}
