# Changelog

All notable changes to this project will be documented in this file.

## [0.1.5] — 2026-05-26

### Fixed

- **`claude-opus-4-7` priced at $15/$75 instead of $5/$25** — The model was using the deprecated Opus 4.0/4.1 rate tier, making every Opus 4.7 call report 3× the expected cost. The correct Opus 4.7 rate is `$5.00/$25.00` per million tokens (same tier as Opus 4.5 and 4.6).
- **`claude-haiku-4-5` priced at $0.80/$4.00 instead of $1.00/$5.00** — Was using the retired Haiku 3.5 rates. Now correctly uses the Haiku 4.5 rate tier.
- **Fuzzy model-name match too broad** — `gpt-4.1` was resolving to `gpt-4` (30× more expensive) because the prefix check used a plain `starts_with` without requiring a `-` separator after the prefix. Fixed: fuzzy matching now only considers a prefix match valid when the character immediately following the prefix in the longer name is `-`, so `gpt-4.1` no longer matches `gpt-4` but `gpt-4o-2024-08-06` still matches `gpt-4o`.
- **Stale lumen-core process surviving `run.sh` restart** — A lumen-core process launched from a different path (e.g. a recorded demo session) could ignore `SIGTERM`, leaving ports 9090/9091 occupied and preventing the new binary from starting. `run.sh` now sends `SIGKILL` as a fallback if the process is still alive after the initial `pkill`.

### Added

- **Externally-updatable JSON pricing file** (`lumen-core/pricing.json`) — All model pricing is now defined in a single JSON file (schema version 1) committed to the repo. This lets rates be updated without recompiling.
- **Pricing loader** (`pricing::loader`) — On startup, `load_pricing()` selects the best available source: user override at `~/.lumen/pricing.json`, last remote fetch at `~/.lumen/pricing.json.cache`, or compiled-in defaults. After loading, a background task fetches the canonical file from the repository and writes it to the cache path so the next restart automatically gets fresh rates.
- **Current OpenAI models** added to compiled-in defaults and `pricing.json`:
  - **GPT-5.x family** (released 2025–2026): `gpt-5.5` ($5.00/$30.00), `gpt-5.5-pro` ($30.00/$180.00), `gpt-5.4` ($2.50/$15.00), `gpt-5.4-mini` ($0.75/$4.50), `gpt-5.4-nano` ($0.20/$1.25), `gpt-5.4-pro` ($30.00/$180.00) — all cached at 10% of input rate
  - **Codex family**: `gpt-5.3-codex` ($1.75/$14.00, cache $0.175) and `gpt-5.1-codex-mini` ($0.25/$2.00, cache $0.025)
  - **GPT-4.1 family**: `gpt-4.1` ($2.00/$8.00), `gpt-4.1-mini` ($0.40/$1.60), `gpt-4.1-nano` ($0.10/$0.40) with dated aliases (`-2025-04-14`)
  - **GPT-4o dated aliases**: `gpt-4o-2024-11-20`, `gpt-4o-2024-08-06`, `gpt-4o-mini-2024-07-18` (current-rate); `gpt-4o-2024-05-13` ($5.00/$15.00) and `chatgpt-4o-latest` ($5.00/$15.00) as explicit entries since the older snapshot is priced higher than the current `gpt-4o`
  - `codex-mini-latest` ($1.50/$6.00, cache $0.375)
- **Retired o-series entries** added with `"note": "retired"` for historical traffic attribution — the entire OpenAI o-series was retired from ChatGPT on 2026-02-13 (o1-preview hard-shutdown 2025-07-28; o1-mini hard-shutdown 2025-10-27). Entries are kept so cost reporting remains accurate for any residual API calls from apps that haven't migrated: `o1`, `o1-2024-12-17`, `o1-pro`, `o1-mini`, `o1-mini-2024-09-12`, `o1-preview`, `o3`, `o3-2025-04-16`, `o3-pro`, `o3-mini`, `o3-mini-2025-01-31`, `o4-mini`, `o4-mini-2025-04-16`.
- **`notes.retired` convention** added to `pricing.json` header to document what the `"note": "retired"` tag means and why those entries are preserved.
- **Expanded Anthropic model coverage** — Added explicit entries for `claude-opus-4-0`, `claude-opus-4-1`, `claude-opus-4-1-20250805`, `claude-opus-4-5`, `claude-opus-4-5-20251101`, `claude-opus-4-6`, `claude-sonnet-4-0`, `claude-sonnet-4-5`, `claude-sonnet-4-5-20250929` so dated and minor variants resolve correctly without relying solely on fuzzy matching.

### Tests

- `pricing::test_opus_4_7_is_5_25_not_15_75` — regression guard for the Opus 4.7 rate fix.
- `pricing::test_haiku_4_5_is_1_5_not_0_80_4` — regression guard for the Haiku 4.5 rate fix.
- `pricing::test_gpt_4_1_family_priced` — verifies `gpt-4.1`, `gpt-4.1-mini`, and `gpt-4.1-nano` resolve to correct rates and are not fuzzy-matched to `gpt-4`.
- `pricing::test_from_file_roundtrip` — verifies `PricingDatabase::from_file()` correctly parses an inline JSON snippet and produces accurate cost calculations.
- `pricing::test_from_file_rejects_unknown_schema_version` — verifies the loader falls back to compiled-in defaults (not a crash or silent success) when `schema_version` is not 1.
- `pricing::test_repo_pricing_json_is_valid` — compile-time inclusion of `pricing.json` via `include_str!`, parsed and validated on every `cargo test` run to catch malformed JSON or unsupported schema versions before they ship.
- `pricing::test_o1_pro_and_o3_pro_not_misprice_as_o1_o3` — verifies `o1-pro` ($150/$600), `o3-pro` ($20/$80), and `codex-mini-latest` are not fuzzy-matched down to the cheaper base model rate (relevant even for retired models that still appear in traffic from unmigrated apps).
- `pricing::test_gpt4o_old_snapshot_uses_higher_rate` — verifies `gpt-4o-2024-05-13` uses the $5.00 input rate and is not overridden by fuzzy-matching to the cheaper current `gpt-4o` entry.
- `pricing::test_gpt5_family_no_fuzzy_bleed` — verifies all eight GPT-5.x/Codex models resolve to their correct prices and `-pro`, `-mini`, `-nano` suffixed variants don't bleed into the cheaper base model price via fuzzy matching.

---

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
