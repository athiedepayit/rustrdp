use std::path::PathBuf;

use serde::{Deserialize, Serialize};

fn default_port() -> u16 {
    3389
}

/// A named set of RDP credentials that can be shared across servers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    /// Unique identifier (a short human-readable slug, e.g. "corp-admin").
    pub id: String,
    /// Display label shown in the UI.
    pub label: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub domain: String,
}

impl Default for Credential {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            username: String::new(),
            password: String::new(),
            domain: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// If set, credentials are taken from the matching entry in
    /// `Config::credentials` instead of the inline fields below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
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
            credential_id: None,
            username: String::new(),
            password: String::new(),
            domain: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub credentials: Vec<Credential>,
    #[serde(default)]
    pub servers: Vec<Server>,
}

impl Config {
    /// Resolve the effective (username, password, domain) for a server,
    /// preferring a linked credential over the server's inline fields.
    pub fn resolve_credentials<'a>(&'a self, server: &'a Server) -> (&'a str, &'a str, &'a str) {
        if let Some(id) = &server.credential_id {
            if let Some(cred) = self.credentials.iter().find(|c| &c.id == id) {
                return (&cred.username, &cred.password, &cred.domain);
            }
        }
        (&server.username, &server.password, &server.domain)
    }
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
            credentials: vec![],
            servers: vec![Server {
                name: "test".into(),
                host: "10.0.0.5".into(),
                port: 3389,
                credential_id: None,
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
    fn credential_round_trip() {
        let cfg = Config {
            credentials: vec![Credential {
                id: "corp".into(),
                label: "Corp Admin".into(),
                username: "dummy_username_for_testing".into(),
                password: "dummy_password_for_testing".into(),
                domain: "CORP".into(),
            }],
            servers: vec![Server {
                name: "dc01".into(),
                host: "10.0.0.1".into(),
                port: 3389,
                credential_id: Some("corp".into()),
                username: String::new(),
                password: String::new(),
                domain: String::new(),
            }],
        };
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.credentials.len(), 1);
        assert_eq!(back.credentials[0].id, "corp");
        let (u, p, d) = back.resolve_credentials(&back.servers[0]);
        assert_eq!(u, "administrator");
        assert_eq!(p, "secret");
        assert_eq!(d, "CORP");
    }

    #[test]
    fn resolve_falls_back_to_inline_when_no_credential_id() {
        let cfg = Config {
            credentials: vec![],
            servers: vec![Server {
                name: "s".into(),
                host: "h".into(),
                port: 3389,
                credential_id: None,
                username: "dummy_username_for_testing".into(),
                password: "dummy_password_for_testing".into(),
                domain: String::new(),
            }],
        };
        let (u, p, _d) = cfg.resolve_credentials(&cfg.servers[0]);
        assert_eq!(u, "inlineuser");
        assert_eq!(p, "inlinepw");
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
