use std::path::PathBuf;

use serde::{Deserialize, Serialize};

fn default_port() -> u16 {
    3389
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub domain: String,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: default_port(),
            username: String::new(),
            password: String::new(),
            domain: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub servers: Vec<Server>,
}

pub fn config_path() -> anyhow::Result<PathBuf> {
    // The user explicitly requested ~/.config regardless of platform.
    // (On macOS, dirs::config_dir() would return ~/Library/Application Support.)
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    let dir = home.join(".config").join("rustrdp");
    Ok(dir.join("servers.json"))
}

impl Config {
    pub fn load() -> Self {
        let path = match config_path() {
            Ok(p) => p,
            Err(_) => return Config::default(),
        };
        match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, contents)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_serialization() {
        let cfg = Config {
            servers: vec![Server {
                name: "test".into(),
                host: "10.0.0.5".into(),
                port: 3389,
                username: "user".into(),
                password: "pw".into(),
                domain: "WORKGROUP".into(),
            }],
        };
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.servers.len(), 1);
        assert_eq!(back.servers[0].host, "10.0.0.5");
        assert_eq!(back.servers[0].port, 3389);
    }

    #[test]
    fn config_path_uses_dot_config() {
        let p = config_path().unwrap();
        let s = p.to_string_lossy();
        assert!(s.contains("/.config/rustrdp/servers.json"), "path was {s}");
    }

    #[test]
    fn missing_port_defaults() {
        let json = r#"{"servers":[{"name":"x","host":"h"}]}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.servers[0].port, 3389);
    }
}
