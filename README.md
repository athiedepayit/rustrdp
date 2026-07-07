# rustrdp

100% vibecoded rust RDP client

Stores your creds in plaintext by default, you have been warned. See `cmd:` passwords below for a more secure option.

## Config file

Located at `~/.config/rustrdp/servers.json`.

```json
{
  "clipboard_passthrough": true,
  "credentials": [
    {
      "id": "corp-admin",
      "label": "Corp Admin",
      "username": "administrator",
      "password": "plaintext-password-here",
      "domain": "CORP"
    },
    {
      "id": "corp-admin-keychain",
      "label": "Corp Admin (keychain)",
      "username": "administrator",
      "password": "cmd:security find-generic-password -a administrator -s corp-rdp -w",
      "domain": "CORP"
    }
  ],
  "servers": [
    {
      "name": "Dev Server",
      "host": "10.0.0.10",
      "port": 3389,
      "credential_id": "corp-admin"
    },
    {
      "name": "Prod Server",
      "host": "10.0.0.20",
      "port": 3389,
      "credential_id": "corp-admin-keychain"
    },
    {
      "name": "Home Lab",
      "host": "192.168.1.50",
      "port": 3389,
      "username": "localuser",
      "password": "cmd:op read op://Personal/homelab/password",
      "domain": ""
    }
  ]
}
```

### Fields

**`credentials`** — reusable named credential sets. Reference them from servers via `credential_id`.

| Field | Description |
|---|---|
| `id` | Unique slug, referenced by servers |
| `label` | Display name shown in the UI |
| `username` | RDP username |
| `password` | Plaintext password, or a `cmd:` expression (see below) |
| `domain` | Windows domain, or empty string for local accounts |

**`servers`** — list of RDP targets.

| Field | Default | Description |
|---|---|---|
| `name` | | Display name shown in the UI |
| `host` | | Hostname or IP address |
| `port` | `3389` | RDP port |
| `credential_id` | | ID of a credential from the `credentials` list |
| `username` | | Inline username (used when no `credential_id` is set) |
| `password` | | Inline password (used when no `credential_id` is set) |
| `domain` | | Inline domain (used when no `credential_id` is set) |

When both `credential_id` and inline fields are present, the linked credential takes priority.

### `cmd:` passwords

If a password field starts with `cmd:`, the rest is executed as a shell command via `sh -c` and its stdout (trimmed) is used as the password. This lets you retrieve credentials from a keychain or password manager without storing them in plaintext.

```
# macOS Keychain
"password": "cmd:security find-generic-password -a myuser -s my-server -w"

# 1Password CLI
"password": "cmd:op read op://vault/item/password"

# pass (standard unix password manager)
"password": "cmd:pass show rdp/myserver"

# Any script
"password": "cmd:/usr/local/bin/get-rdp-password.sh myserver"
```

If the command exits with a non-zero status, the connection is aborted and the error is shown in the tab.

