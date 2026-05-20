use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_cpu_threshold")]
    pub cpu_threshold: u32,

    #[serde(default = "default_watch_interval")]
    pub watch_interval: u64,

    #[serde(default)]
    pub auto_clean: bool,

    #[serde(default = "default_thermal_limit")]
    pub thermal_limit: String,

    #[serde(default = "default_wrap_cpu_limit")]
    pub wrap_cpu_limit: u32,

    #[serde(default)]
    pub whitelist: Vec<String>,
}

fn default_cpu_threshold() -> u32 {
    50
}

fn default_watch_interval() -> u64 {
    30
}

fn default_thermal_limit() -> String {
    "critical".to_string()
}

fn default_wrap_cpu_limit() -> u32 {
    80
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cpu_threshold: default_cpu_threshold(),
            watch_interval: default_watch_interval(),
            auto_clean: false,
            thermal_limit: default_thermal_limit(),
            wrap_cpu_limit: default_wrap_cpu_limit(),
            whitelist: Vec::new(),
        }
    }
}

impl Config {
    pub fn config_dir() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Could not find config directory")?
            .join("mac-cooldown");

        if !config_dir.exists() {
            fs::create_dir_all(&config_dir)
                .context("Failed to create config directory")?;
        }

        Ok(config_dir)
    }

    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config: {}", path.display()))?;

        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;

        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;

        fs::write(&path, content)
            .with_context(|| format!("Failed to write config: {}", path.display()))?;

        Ok(())
    }

    pub fn exists() -> Result<bool> {
        let path = Self::config_path()?;
        Ok(path.exists())
    }

    pub fn init() -> Result<PathBuf> {
        let config = Self::default();
        config.save()?;
        Self::config_path()
    }
}

pub fn display_config(config: &Config) {
    use colored::*;

    println!();
    println!("{}", "CONFIGURATION".cyan().bold());
    println!("{}", "=============".dimmed());
    println!();
    println!("  {} {}", "CPU threshold:".bold(), config.cpu_threshold);
    println!("  {} {}s", "Watch interval:".bold(), config.watch_interval);
    println!("  {} {}", "Auto clean:".bold(), config.auto_clean);
    println!("  {} {}", "Thermal limit:".bold(), config.thermal_limit);
    println!("  {} {}%", "Wrap CPU limit:".bold(), config.wrap_cpu_limit);
    println!(
        "  {} {} pattern(s)",
        "Whitelist:".bold(),
        config.whitelist.len()
    );

    if !config.whitelist.is_empty() {
        for pattern in &config.whitelist {
            println!("    - {}", pattern.dimmed());
        }
    }

    println!();

    if let Ok(path) = Config::config_path() {
        println!("  {} {}", "Config file:".dimmed(), path.display());
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert_eq!(config.cpu_threshold, 50);
        assert_eq!(config.watch_interval, 30);
        assert!(!config.auto_clean);
    }

    #[test]
    fn test_config_serialize() {
        let config = Config::default();
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("cpu_threshold"));
    }

    #[test]
    fn test_config_deserialize() {
        let toml_str = r#"
cpu_threshold = 75
watch_interval = 60
auto_clean = true
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.cpu_threshold, 75);
        assert_eq!(config.watch_interval, 60);
        assert!(config.auto_clean);
    }
}
