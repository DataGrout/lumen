# Changelog

All notable changes to this project will be documented in this file.

## [0.1.1] — 2026-05-06

### Fixed

- **Sync watermark stall** — `DGSyncer` now tracks against a monotonically-increasing lifetime event counter (`total_event_count`) instead of a window-relative position. Previously, after 100 events the watermark could never advance beyond `MAX_BATCH_SIZE`, silently dropping all subsequent usage events.
- **CORS header removed** from the local JSON API (`json_response`). The `Access-Control-Allow-Origin: *` header served no purpose — the API is consumed only by Swift `URLSession` on loopback — and was unnecessarily permissive.
- **Swift concurrency warning** — `Task.detached(priority: .utility)` in `WizardView` changed to `Task(priority: .utility)` to satisfy main-actor isolation requirements.
- **Privacy documentation** — README's "message content is never stored" claim now correctly scopes to normal operation and documents the opt-in debug capture mode.
- **Local API authentication** — the JSON API on `:9091` now requires an `X-Lumen-Token` header on all requests except `GET /health` and the OAuth callback redirect. A 32-byte random token is generated on first launch, written to `~/.lumen/api.token` (mode 0600), and reloaded on subsequent starts. The Swift app reads the same file and injects the header automatically. This closes the control-plane exposure where any local process could call `/shutdown`, `/debug/arm`, or `/dg/identity` without authentication.

### Tests

- `aggregator::test_total_event_count_is_lifetime` — verifies the lifetime counter is unaffected by lap creation, which is the invariant the watermark fix depends on.
- `sync::test_sync_skips_without_server_url` — verifies `sync_batch` returns `Ok` without advancing the watermark when no server URL is configured.
- `sync::test_sync_watermark_does_not_advance_when_no_new_events` — verifies the early-return path when `total == last`.

---

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
