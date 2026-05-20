use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::scanner::ProcessInfo;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Whitelist {
    pub patterns: Vec<String>,
}

impl Whitelist {
    pub fn config_path() -> anyhow::Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Could not find config directory")?
            .join("cooldown");
        std::fs::create_dir_all(&config_dir)
            .with_context(|| format!("Could not create config dir {:?}", config_dir))?;
        Ok(config_dir.join("whitelist.json"))
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Could not read {:?}", path))?;
        let wl: Whitelist = serde_json::from_str(&content)
            .with_context(|| format!("Could not parse whitelist JSON at {:?}", path))?;
        Ok(wl)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path()?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
            .with_context(|| format!("Could not write whitelist to {:?}", path))?;
        Ok(())
    }

    pub fn add(&mut self, pattern: String) {
        if !self.patterns.contains(&pattern) {
            self.patterns.push(pattern);
        }
    }

    pub fn remove(&mut self, pattern: &str) -> bool {
        let len_before = self.patterns.len();
        self.patterns.retain(|p| p != pattern);
        self.patterns.len() < len_before
    }

    pub fn clear(&mut self) {
        self.patterns.clear();
    }

    /// Returns true if the process matches any whitelist pattern.
    pub fn matches(&self, proc: &ProcessInfo) -> bool {
        for pattern in &self.patterns {
            let pat = pattern.to_lowercase();
            let name = proc.name.to_lowercase();
            let cmd = proc.cmd.to_lowercase();

            // Simple substring matching with optional wildcard support
            if pat.contains('*') {
                let parts: Vec<&str> = pat.split('*').collect();
                if matches_wildcard(&cmd, &parts) || matches_wildcard(&name, &parts) {
                    return true;
                }
            } else if name.contains(&pat) || cmd.contains(&pat) {
                return true;
            }
        }
        false
    }
}

fn matches_wildcard(haystack: &str, parts: &[&str]) -> bool {
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if let Some(idx) = haystack[pos..].find(part) {
            if i == 0 && idx != 0 {
                // First part must match from beginning if no leading *
                return false;
            }
            pos += idx + part.len();
        } else {
            return false;
        }
    }
    true
}
