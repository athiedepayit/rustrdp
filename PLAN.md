# MacRDP — Minimal Rust RDP Client

A single self-contained Rust binary (no external tool invocation) presenting one
window: a server list on the left, a tabbed connection area on the right (like
the classic Microsoft Remote Desktop Connection Manager). Servers and their
connection details are persisted to `~/.config/macrdp/servers.json`. No features
beyond what was explicitly requested.

## Confirmed decisions

- **GUI**: `egui` + `eframe` (pure Rust, single binary, renders the RDP
  framebuffer as a texture).
- **RDP**: IronRDP crate suite, using the **blocking** connection flow (mirrors
  the official `screenshot` example).
- **Passwords**: stored plaintext in the config file.
- **TLS**: accept any certificate (mirrors IronRDP's `NoCertificateVerification`).

## Crate versions (verified on crates.io)

`ironrdp = 0.16`, `ironrdp-blocking = 0.9`, `ironrdp-connector = 0.9`,
`ironrdp-session = 0.10`, `ironrdp-graphics = 0.8`, `ironrdp-input = 0.6`,
`ironrdp-pdu = 0.8`, `sspi = 0.21`, `tokio-rustls = 0.26`, `x509-cert = 0.2`,
`eframe/egui = 0.35`, plus `serde`, `serde_json`, `dirs`, `anyhow`.

## Architecture

egui runs on the main thread; each RDP connection runs on its own worker thread
(IronRDP blocking API). Communication via channels:

- Worker -> UI: `mpsc` sending framebuffer updates (RGBA + dirty rect) and
  status/terminate events.
- UI -> Worker: `mpsc` sending input `Operation`s (mouse/keyboard) and shutdown.

The UI thread never blocks on the network.

## File layout

```
Cargo.toml
src/
  main.rs        # eframe entry, App struct, egui layout (left list + right tabs)
  config.rs      # Server struct, load/save ~/.config/macrdp/servers.json
  session.rs     # RDP worker thread: connect + active-stage loop, channels
  connection.rs  # IronRDP connect() + TLS upgrade (from screenshot example)
  input.rs       # egui event -> ironrdp-input Operation mapping
```

## Scope guard (no extra features)

Only: server list, add/edit/remove entries, tabbed connections, config
persistence in `~/.config`, and the RDP display + input needed to make a
connection usable. No clipboard sync, audio, drive redirection, reconnection
logic, themes, search, or shortcuts beyond raw input forwarding.
