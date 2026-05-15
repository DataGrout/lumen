# Changelog

All notable changes to this project will be documented in this file.

## [0.1.4] — 2026-05-15

### Fixed

- **Cursor cost overestimation by 15–30x** — Cursor traffic was being priced at the underlying Anthropic/OpenAI direct API rates (e.g. `claude-opus-4-7` at $15/$75 per million tokens), which dramatically overshoots Cursor's "On-Demand" billing. A 963K-token Opus call invoiced by Cursor at ~$2.74 was being reported as ~$72. Fix: added a calibrated `cursor:` namespace to the pricing database with effective blended rates derived from real Cursor invoices (`claude-opus-4-7`, `claude-opus-4`, `claude-sonnet-4-6`, `claude-sonnet-4`, `claude-haiku-4-5`, `claude-haiku-4`, `composer-2`, `composer-2-fast`, `gpt-5.5`, `gpt-5`, `gpt-5-mini`, `gpt-5-nano`). Lumen's reported costs now land within ~2x of the actual Cursor bill.
- **Cursor pricing lookup precedence** — `calculate_cost` now does an exact match in the `cursor:` namespace, then a fuzzy match within `cursor:`, then cross-provider fallback, then the `cursor-unknown` sentinel. The new fuzzy step is what makes variants like `claude-opus-4-7-thinking-high` resolve to `cursor:claude-opus-4-7` instead of leaking through to the (much pricier) Anthropic-direct entry.
- **`composer-2-fast` priced as unknown** — Cursor's first-party Composer models had no pricing entries and fell through to the $5/$15 unknown-model fallback. They now have explicit `cursor:composer-2` and `cursor:composer-2-fast` entries (~$0.75/MTok blended, matching observed invoices).
- **`cursor-unknown` fallback rate** — lowered from $3/$15 to $0.50/$2.50 per million tokens, an honest blended rate for unidentified Cursor traffic.

### Tests

- `pricing::test_cursor_uses_cursor_pricing_not_anthropic_direct` — locks in that Cursor traffic never gets Anthropic-direct rates.
- `pricing::test_cursor_opus_thinking_variant_resolves_to_cursor_opus` — locks in the cursor-namespace fuzzy match for `-thinking-high` (and similar) variants.
- `pricing::test_cursor_composer_priced` — covers `composer-2-fast` no longer falling through to the unknown-model fallback.
- `pricing::test_cursor_opus_realistic_call_against_invoice` — calibration check against a real Cursor invoice line (963.1K tokens for $2.74).

### Known limitations (not addressed in this release)

- Cursor token counts are still byte-estimated (`BYTES_PER_TOKEN = 4.0`), which over-counts heavily for SSE-wrapped streams and lands almost everything in `output_tokens`. The new pricing absorbs this bias but doesn't fix it.
- Cursor's gRPC responses don't expose `cache_read_input_tokens` the way Anthropic's JSON API does, so cache savings from Cursor traffic remain invisible. Real-usage extraction would require decoding Cursor's protobuf schema.

---

## [0.1.3] — 2026-05-10

### Added

- **Web dashboard** — `GET /dashboard` serves a self-contained HTML dashboard at `http://127.0.0.1:9091/dashboard`. The token is injected at build time via `include_str!` + template substitution so the page authenticates automatically. The endpoint is auth-exempt (no `X-Lumen-Token` required) since the token is embedded in the served HTML itself.
- **DataGrout sign-up link** — "Don't have an account? Create one free →" button in the DataGrout settings panel opens `https://app.datagrout.ai` in the default browser.

---

## [0.1.2] — 2026-05-07

### Fixed

- **Cursor BidiAppend over-estimation** — `BidiAppend` calls with no response body are now correctly identified as Cursor's background context-sync traffic and excluded from usage tracking. Previously, Cursor's ~195KB codebase-snapshot uploads (sent every ~1 second before any AI call) were each estimated as ~49,000-input-token AI completions, inflating cost estimates by orders of magnitude. The fix requires a non-trivial response (`resp_bytes > 50`) before a BidiAppend call is treated as AI inference — real AI completions always produce output.
- **`ReportAiCodeChangeMetrics` telemetry** — added to the Cursor noise-path list so these background reports don't generate usage events.

### Tests

- `proxy::test_cursor_is_significant_call` — verifies BidiAppend calls with 0B response are not counted as AI inference.
- `proxy::test_cursor_is_noise` — verifies `ReportAiCodeChangeMetrics` and other known-telemetry paths are excluded.

---

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
