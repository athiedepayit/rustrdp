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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub credentials: Vec<Credential>,
    #[serde(default)]
    pub servers: Vec<Server>,
    /// When true, clipboard text is passed through between the local machine
    /// and every remote session (requires the RDP CLIPRDR channel).
    #[serde(default)]
    pub clipboard_passthrough: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            credentials: Vec::new(),
            servers: Vec::new(),
            clipboard_passthrough: false,
        }
    }
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

/// Resolve the actual password from a raw password field value.
///
/// If `raw` starts with `"cmd:"`, the remainder is executed as a shell command
/// via `sh -c` and its trimmed stdout is returned as the password.  This lets
/// users retrieve credentials from a keychain, password manager CLI, or any
/// other external source without storing the plaintext password in the config.
///
/// Returns `Ok(password)` on success, or `Err(message)` if the command fails
/// (non-zero exit status or process spawn error).  For plain passwords (no
/// `"cmd:"` prefix) this always returns `Ok(raw.to_owned())`.
pub fn resolve_password(raw: &str) -> Result<String, String> {
    if let Some(cmd) = raw.strip_prefix("cmd:") {
        match std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
        {
            Ok(output) if output.status.success() => {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
            }
            Ok(output) => Err(format!(
                "Credential Command Failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => Err(format!("Credential Command Failed: {e}")),
        }
    } else {
        Ok(raw.to_owned())
    }
}

pub fn config_path() -> anyhow::Result<PathBuf> {
    // The user explicitly requested ~/.config regardless of platform.
    // (On macOS, dirs::config_dir() would return ~/Library/Application Support.)
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
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
            clipboard_passthrough: false,
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
                username: "administrator".into(),
                password: "secret".into(),
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
            clipboard_passthrough: false,
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
                username: "dummytestuser".into(),
                password: "dummytestpassword".into(),
                domain: String::new(),
            }],
            clipboard_passthrough: false,
        };
        let (u, p, _d) = cfg.resolve_credentials(&cfg.servers[0]);
        assert_eq!(u, "dummytestuser");
        assert_eq!(p, "dummytestpassword");
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

    #[test]
    fn resolve_password_plain() {
        assert_eq!(resolve_password("hunter2").unwrap(), "hunter2");
    }

    #[test]
    fn resolve_password_cmd_success() {
        let result = resolve_password("cmd:echo mysecret").unwrap();
        assert_eq!(result, "mysecret");
    }

    #[test]
    fn resolve_password_cmd_trims_newline() {
        // printf to avoid a trailing newline — result should still be trimmed.
        let result = resolve_password("cmd:printf '  spaced  '").unwrap();
        assert_eq!(result, "spaced");
    }

    #[test]
    fn resolve_password_cmd_failure() {
        let err = resolve_password("cmd:sh -c 'exit 1'").unwrap_err();
        assert!(
            err.starts_with("Credential Command Failed"),
            "unexpected error: {err}"
        );
    }
}
