# DevOps Client

[![Build](https://github.com/devops-client/devops-client/actions/workflows/build.yml/badge.svg)](https://github.com/devops-client/devops-client/actions/workflows/build.yml)

Desktop device authentication agent built with [Tauri v2](https://v2.tauri.app/). Provides secure device-bound session management via a local HTTPS server, system tray integration, and i18n support (English / 中文).

## How It Works

```
User logs in → Agent registers device fingerprint → Local HTTPS server starts
                                                    ↓
Dashboard JS → fetch('https://127.0.0.1:{port}/ping')
  → Success → Request is from the agent's machine → Allowed
  → Failure → Cookie copied to another machine → Denied
```

## Features

- **Device-bound security** — Sessions are tied to the physical machine via local HTTPS verification
- **Internationalization** — English and Chinese (detected from system locale)
- **System tray** — Runs quietly in the background
- **Cross-platform** — macOS (ARM64 / x64), Windows (x64), Linux (x64)
- **Self-signed TLS** — Automatic CA generation + system trust store installation

## Project Structure

```
src/                            # Frontend (WebView UI)
├── index.html                  # Shell page with data-i18n attributes
├── css/
│   └── styles.css              # Dark theme
├── js/
│   ├── app.js                  # Entry point, view switching, boot
│   ├── api.js                  # Tauri IPC wrapper
│   ├── i18n.js                 # i18n engine (loads locale JSON)
│   └── views/
│       ├── login.js            # Login form logic
│       └── panel.js            # Connected panel logic
└── locales/
    ├── en.json                 # English translations
    └── zh.json                 # Chinese translations

src-tauri/                      # Tauri Rust backend
├── Cargo.toml
├── tauri.conf.json             # Window 420×560, tray, CSP
└── src/
    ├── main.rs                 # Entry: window, tray, lifecycle
    ├── lib.rs                  # Module declarations
    ├── commands.rs             # All Tauri IPC commands
    ├── state.rs                # ProxyState, HeartbeatState
    ├── config.rs               # Config load/save, settings dir
    ├── fingerprint.rs          # ED25519 + SHA-256 device fingerprint
    ├── cert.rs                 # Self-signed CA + localhost TLS
    ├── proxy.rs                # Axum HTTPS server (/ping)
    ├── auth.rs                 # Reqwest HTTP client (login / status)
    ├── error.rs                # AppError with i18n messages
    └── i18n.rs                 # Lang detection + string tables
```

## Prerequisites

| Platform | Dependencies |
|----------|-------------|
| macOS | Xcode Command Line Tools |
| Linux | `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev` |
| Windows | Microsoft Visual Studio C++ Build Tools |
| All | Rust 1.80+, Node.js 20+, pnpm |

## Quick Start

```bash
just install          # pnpm install
just dev              # Development mode (hot reload)
just build            # Build macOS ARM64 → DMG
just check            # Rust type-check only (fast)
just lint             # cargo clippy
```

Build artifacts:
```
src-tauri/target/aarch64-apple-darwin/release/bundle/
├── macos/DevOps Client.app
└── dmg/DevOps Client_0.1.0_aarch64.dmg
```

## Configuration

All data stored in `~/.devops-client/`:

```
.devops-client/
├── device.key          # ED25519 keypair (JSON: seed + pub)
├── config.json         # Cached server URL + fingerprint
└── certs/
    ├── ca.pem          # Self-signed CA (auto-trusted on install)
    ├── localhost.pem   # 127.0.0.1 certificate
    └── localhost-key.pem
```

## Internationalization

System language is detected from environment variables (`LANG`, `AppleLocale`). Fallback: English.

| Language | Code |
|----------|------|
| English | `en` |
| 中文 | `zh` |

Adding a new language:
1. Create `src/locales/{code}.json`
2. Add the same keys to `src-tauri/src/i18n.rs`
3. Update language detection in `i18n.rs` > `detect_lang()`

## Platform Support

| Platform | Local Dev | CI |
|----------|-----------|----|
| macOS ARM64 | `just build` | ✓ |
| macOS x86_64 | — | ✓ |
| Windows x86_64 | — | ✓ |
| Linux x86_64 | — | ✓ |

## Architecture

### IPC Commands

| Command | Description |
|---------|------------|
| `get_lang` | Detect system language |
| `get_fingerprint` | Get device fingerprint |
| `load_config_cmd` | Load cached config |
| `save_config_cmd` | Save config to disk |
| `get_hostname` | Get OS hostname |
| `do_login` | POST /api/auth/login-device |
| `start_proxy` | Start HTTPS proxy on port |
| `stop_proxy` | Stop HTTPS proxy |
| `get_proxy_port` | Get current proxy port |
| `get_available_port_cmd` | Find available TCP port |
| `open_browser` | Open URL in system browser |
| `start_heartbeat` | Start 10s heartbeat loop |
| `stop_heartbeat` | Stop heartbeat loop |

### Fingerprint Algorithm

```
raw = systemUUID + "/" + hostname + "/" + base64url(ed25519_pubkey)
fingerprint = SHA256(raw) → 64-char lowercase hex
```

Identity is ED25519 keypair persistent at `~/.devops-client/device.key`.

## License

MIT
