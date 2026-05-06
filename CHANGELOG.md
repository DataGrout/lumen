# Changelog

All notable changes to this project will be documented in this file.

## [0.1.0] — 2026-05-06

First public release.

### lumen-core (Rust daemon)

- HTTP forward proxy on `:9090` with TLS MITM via self-signed CA
- Transparent proxy on `:9443` (pf-redirect mode, requires root)
- Passive sniffer via libpcap/BPF for zero-config traffic identification
- Multi-provider token extraction: OpenAI, Anthropic, Google AI (REST and SSE)
- Cursor protobuf/gRPC parsing: BidiAppend (gzip-compressed requests) and RunSSE (gRPC-framed gzip responses)
- Model detection for Cursor: binary pattern scan of decoded gRPC frames, system-prompt text extraction (`powered by Composer` → `composer`, `You are Codex N.M` → `codex-N.M`), analytics event scanning
- Token pricing database with fuzzy model-name matching
- Real-time stats aggregation with rolling 60-second windows
- Lap-based session tracking for before/after cost comparisons
- REST API on `:9091` (stats, events, traffic log, host aggregates, lap control, config)
- Per-provider relay routes (e.g. `/anthropic` → `https://api.anthropic.com`)
- DataGrout sync: 30-second usage batches and immediate lap snapshots via HMAC Bearer auth
- DataGrout DCR OAuth flow for device registration
- Body size limits configurable at runtime
- Payload capture / debug mode (ring buffer of recent request+response hex)

### Lumen.app (macOS)

- Status bar icon with animated live-cost gauge
- Arc gauge display (cost, token rate, cache savings)
- Scrollable event feed with per-request model, tokens, and cost
- Traffic log with host/path/status/latency filtering
- Monitored endpoint manager with custom host support
- DataGrout integration settings (server connect, tool visibility, Intelligent Interface)
- Setup wizard with CA certificate installation and per-tool launcher shortcuts
- System proxy auto-enable/restore on launch and cleanup on quit
- Automatic daemon lifecycle management (launch, health check, restart on crash)
