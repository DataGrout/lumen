# Lumen

[![CI](https://github.com/datagrout/lumen/actions/workflows/ci.yml/badge.svg)](https://github.com/datagrout/lumen/actions/workflows/ci.yml)

**Real-time LLM usage monitor and cost tracker** -- a native macOS status bar app by [DataGrout](https://datagrout.ai).

<p align="center"><img src="docs/screenshot.png" alt="Lumen screenshot" /></p>

Lumen intercepts LLM API traffic, extracts token usage and cost metadata, and displays it in a live gauge interface from your menu bar. Think of it like Activity Monitor for your AI spending.

## Architecture

Two processes, zero npm dependencies:

```
┌─────────────┐     ┌────────────────────────────────┐     ┌─────────────┐
│  LLM Client │────▶│  lumen-core (Rust)             │────▶│  LLM API    │
│  (Cursor)   │◀────│  HTTP proxy :9090              │◀────│  (OpenAI)   │
└─────────────┘     │  JSON API   :9091              │     └─────────────┘
                    │  parser · pricing · aggregator │
                    └──────────────┬─────────────────┘
                                   │ GET /stats
                    ┌──────────────▼────────────────┐
                    │  Lumen.app (Swift)            │
                    │  NSStatusItem + SwiftUI       │
                    │  gauges · events · settings   │
                    └───────────────────────────────┘
```

- **lumen-core** -- Rust binary. Runs an HTTP forward proxy on `:9090` that intercepts LLM API calls, parses token usage from responses, calculates costs, and aggregates stats. Exposes a JSON API on `:9091` for the UI.
- **Lumen.app** -- Native macOS status bar app. SwiftUI popover with arc gauges, event feed, endpoint manager, and DataGrout integration toggles. Launches and manages `lumen-core` as a child process.

## Features

- **Live gauges** -- Lap cost (`$`), token rate, and total spend displayed as real-time arc meters
- **Multi-provider** -- OpenAI, Anthropic, Cursor, and Google AI supported out of the box
- **Token breakdown** -- Input vs output with cache hit visualization
- **Event feed** -- Scrollable log of every API call with model, tokens, and cost
- **Lap tracking** -- Lap button marks a session boundary for before/after cost comparisons
- **Right-click menu** -- Status bar icon supports right-click for quick spending summary, lap creation, tab navigation, app launchers, and quit
- **Endpoint monitoring** -- See exactly which URLs are intercepted and what data is captured
- **Custom endpoints** -- Whitelist additional hosts (local LLMs, hosted models, MCP servers)
- **DataGrout integration** -- Toggle DG tools visibility and Intelligent Interface
- **Configurable crypto backend** -- Default `ring` (cross-platform, no C toolchain); opt into `aws-lc-rs` for FIPS environments
- **Privacy-first** -- In normal operation, only token counts and pricing metadata are captured; message content is never stored or transmitted. An opt-in debug capture mode (`POST /api/debug/arm`) can temporarily buffer raw request/response payloads in memory for diagnostics -- it is off by default and payloads are cleared on disarm.

## Platform Support

| Platform | Interface | Status |
|---|---|---|
| macOS (Apple Silicon + Intel) | Native status bar app + browser dashboard | ✓ |
| Linux | Browser dashboard | ✓ |
| Windows | Browser dashboard | ✓ |

The `lumen-core` daemon is pure Rust and runs on all three platforms. The macOS Swift app is optional -- on Linux and Windows, open `http://127.0.0.1:9091/dashboard` in any browser to get a live dashboard with the same stats, event feed, and lap history.

The macOS DMG produced by `scripts/build_dmg.sh` is a **universal binary** -- a single download works on both Apple Silicon and Intel Macs (macOS 14+). Windows native execution is covered by CI (a `windows-latest` runner builds and runs the test suite on every PR). Cross-compiling a Windows `.exe` from a Mac is documented under Installation.

## Prerequisites

**macOS (full app)**
- macOS 14.0+
- [Rust](https://rustup.rs/) (1.70+)
- Xcode Command Line Tools (`xcode-select --install`)

**Linux / Windows (daemon + browser dashboard)**
- [Rust](https://rustup.rs/) (1.70+)
- Linux: `libpcap-dev` only if using `--features passive` (passive packet capture)

## Installation

**macOS (local install for development):**

```bash
sh install.sh
```

This builds both binaries in release mode for the host architecture, assembles a `Lumen.app` bundle in `~/Applications`, and runs `mdimport` so Spotlight picks it up immediately. After that, **Cmd+Space -> "Lumen"** launches the app.

**macOS (distributable universal DMG):**

```bash
./scripts/build_dmg.sh
# → dist/Lumen-0.2.0.dmg
```

By default this produces a **universal binary** (x86_64 + arm64) that runs on both Intel and Apple Silicon Macs. The script will install the `x86_64-apple-darwin` Rust target on first run (~150 MB, one-time) and `lipo` both binaries together. To speed up local iteration on Apple Silicon, override:

```bash
ARCHS=arm64 ./scripts/build_dmg.sh   # arm64-only, ~2x faster build
```

The script logs the architectures of both bundled binaries so a stray single-arch build can't ship to Intel users undetected.

**Linux:**

```bash
cargo build --release
./target/release/lumen-core &
# Then open http://127.0.0.1:9091/dashboard
```

Passive packet capture (optional, requires `libpcap-dev` and root/BPF access):
```bash
cargo build --release --features passive
sudo ./target/release/lumen-core --passive
```

**Windows:**

Native build on Windows (MSVC toolchain — comes with Visual Studio Build Tools):

```powershell
cargo build --release
Start-Process .\target\release\lumen-core.exe
# Then open http://127.0.0.1:9091/dashboard
```

Cross-compile from a Mac via MinGW:

```bash
brew install mingw-w64
rustup target add x86_64-pc-windows-gnu
CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
cargo build --release --target x86_64-pc-windows-gnu
# → target/x86_64-pc-windows-gnu/release/lumen-core.exe (~13 MB)
```

The daemon resolves `~/.lumen/` via `$USERPROFILE` on Windows (falls back to `$HOME` first for parity with macOS/Linux), so all state files land under `C:\Users\<you>\.lumen\` without extra configuration. Platform-specific privileged features (macOS `pf`-based transparent capture, the `passive` packet-capture feature) are compiled out on Windows; the HTTP forward proxy, JSON API, parser, pricing, and dashboard all work identically to the other platforms.

To run in development without installing (any platform):

```bash
cargo run   # debug build, live logs in the terminal
```

### Crypto backend

`lumen-core` builds with the `ring` TLS crypto backend by default — pure Rust + assembly, no C toolchain at build time, cross-compiles cleanly to all targets (including Windows from macOS).

For FIPS 140-3 environments, post-quantum hybrid KEMs, or strict alignment with AWS's TLS substrate, rebuild with the `aws-lc-rs` backend:

```bash
cargo build --release --no-default-features --features crypto-aws-lc
```

The two backends are runtime-equivalent for everything lumen-core does — pick `aws-lc-rs` only if you have a concrete compliance or interop reason. Switching between them does not change the on-wire TLS behavior; it only changes which library performs the underlying cryptographic operations.

## Setup

### Choose your capture mode

Lumen supports two ways of seeing LLM traffic. Which one you need depends on the client:

| Mode | How it works | Requires CA trust | Best for |
|---|---|---|---|
| **Relay** | Client sends plain HTTP to `http://127.0.0.1:9090/anthropic` (etc); the daemon re-originates HTTPS to the real API | No | Claude Code, OpenCode, CLI tools, scripts, anything that respects `ANTHROPIC_BASE_URL` |
| **Proxy** | Client uses Lumen as an `HTTPS_PROXY`; daemon does TLS MITM with a self-signed CA | Yes -- requires trusting `~/.lumen/ca.pem` once | Claude Desktop, Cursor, any GUI tool that respects system proxy |

**Enterprise / managed laptops:** relay mode side-steps Keychain CA installation entirely, which is the #1 blocker on Jamf/Intune-managed devices. Lead with relay mode if you can.

The **Launch** tab in the menu bar app starts your LLM client in the right mode automatically. Manual configuration is documented below.

### 1. Trust the Lumen CA certificate (proxy mode only)

If you're only using relay mode (Claude Code / OpenCode), **skip this step.**

For proxy-mode clients (Claude Desktop, Cursor), Lumen does TLS interception to read encrypted HTTPS traffic. This requires trusting a locally-generated CA certificate once.

The setup wizard (launched on first run) walks through this automatically. To do it manually:

```bash
# The CA cert is generated at first launch and lives at:
~/.lumen/ca.pem

# Trust it system-wide (prompts for your password):
sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain ~/.lumen/ca.pem
```

You can also do this from **Settings -> Certificate -> Trust CA** inside the Lumen UI.

### 2. Configure your LLM client

The **Launch** tab in the menu bar app launches all four supported clients with the right environment pre-configured. The manual incantations are below if you'd rather wire it up yourself.

**Claude Code / OpenCode (relay mode -- no CA trust needed):**

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:9090/anthropic
claude       # or: opencode
```

**Cursor (proxy mode):**

```
Lumen -> Launch -> Cursor -> Launch
```

This sets `HTTPS_PROXY=http://127.0.0.1:9090` and `NODE_EXTRA_CA_CERTS=~/.lumen/ca.pem` automatically.

To configure Cursor manually instead:
1. Trust the Lumen CA (step 1 above)
2. Cursor Settings -> Network -> **HTTP Compatibility -> HTTP/1.1** (required for gRPC capture)
3. Set system proxy to `127.0.0.1:9090` or `export HTTPS_PROXY=http://127.0.0.1:9090`

**Claude Desktop (proxy mode):**

```
Lumen -> Launch -> Claude Desktop -> Launch
```

Or manually: `HTTPS_PROXY=http://127.0.0.1:9090 open -a "Claude"` (requires CA trust).

**CLI / scripts (proxy mode):**

```bash
export HTTPS_PROXY=http://127.0.0.1:9090
export NODE_EXTRA_CA_CERTS=~/.lumen/ca.pem  # Node.js
export SSL_CERT_FILE=~/.lumen/ca.pem         # Python / curl
```

### 3. Watch the gauges

Click the Lumen menu bar icon. **Lap Cost** (`$`), **Token Rate**, and **Total Spend** (`$`) update in real time as you use your LLM tools. Right-click the icon for a quick spending summary, lap creation, and shortcut access to launchers and settings.

## Custom Endpoints

Lumen ships with built-in support for `api.openai.com`, `api.anthropic.com`, `*.cursor.sh`, `generativelanguage.googleapis.com`, and `claude.ai`. To monitor additional hosts (self-hosted models, OpenAI-compatible APIs, MCP servers):

1. Open **Lumen -> Endpoints**
2. Click **+** and enter the hostname (e.g. `my-llm.internal` or `api.together.xyz`)
3. Lumen will proxy and parse traffic to that host on the next request

Custom hosts are stored in the daemon config and persist across restarts. Any host that returns OpenAI-compatible or Anthropic-compatible JSON or SSE will have its token usage extracted automatically; others will use byte-based estimation.

## Relay Routes

For tools that don't support proxies, Lumen can act as a relay endpoint -- requests to `http://127.0.0.1:9090/anthropic` are forwarded to `https://api.anthropic.com`, adding monitoring transparently.

Built-in relay routes: `/openai`, `/anthropic`, `/google`

## DataGrout Integration

Connect Lumen to a DataGrout server to sync usage events and lap snapshots for team reporting:

1. **Settings -> DataGrout -> Connect** -- paste your DataGrout server URL (or bare UUID)
2. Complete the OAuth flow in the browser
3. Usage events sync every 30 seconds; lap snapshots sync immediately when you press the lap button

## Project Structure

```
lumen/
  lumen-core/             # Rust daemon
    src/
      main.rs             # entry point + crypto provider install
      api.rs              # JSON API on :9091 (incl. /dashboard + / -> /dashboard redirect)
      proxy/mod.rs        # HTTP forward proxy on :9090
      parser/mod.rs       # LLM response parser (OpenAI, Anthropic, Cursor, Google)
      pricing/mod.rs      # token pricing database with fuzzy matching
      pricing/loader.rs   # JSON pricing file loader (local override + remote fetch + cache)
      pricing.json        # canonical pricing data, schema_version 1
      aggregator/mod.rs   # real-time stats aggregation + lap tracking
      ca.rs               # self-signed CA generation and persistence
      tls.rs              # rustls cert cache for MITM
      state.rs            # shared app state
      sync.rs             # DataGrout usage sync
  Lumen/                  # Swift macOS app
    Sources/
      LumenApp.swift      # NSStatusItem + NSPopover + right-click menu + click-outside dismiss
      PopoverView.swift   # main SwiftUI view, tab navigation, common-mode timer
      GaugeView.swift     # arc gauge component (supports $ prefix)
      EventFeed.swift     # recent events list
      HostsView.swift     # monitored endpoints panel
      SettingsView.swift  # DG toggles, proxy config, live CA trust check
      LaunchersView.swift # one-click launch for Claude Code, Cursor, Claude Desktop, OpenCode
      APIClient.swift     # polls lumen-core JSON API (common-mode timer)
      DaemonManager.swift # manages lumen-core process lifecycle
      StatusIconManager.swift # menu bar icon animation
      WizardView.swift    # first-run setup wizard
  scripts/
    build_dmg.sh          # universal (Intel + arm64) DMG build pipeline
  CHANGELOG.md            # release notes
```

## License

[MIT](LICENSE)

Copyright 2026 [DataGrout AI](https://datagrout.ai)
