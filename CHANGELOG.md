# Changelog

All notable changes to this project will be documented in this file.

## [0.2.3] — 2026-07-25

### Added

- **Windows: Claude Desktop (MSIX) support + one-click launchers.** The Windows launcher now detects and drives the *packaged (MSIX)* Claude Desktop that `claude.ai/download` ships today — it trusts Lumen's CA in the Windows certificate store (Chromium validates there, not via `NODE_EXTRA_CA_CERTS`), persists the proxy env at user scope, and launches via `Invoke-CommandInDesktopPackage`. It also supports **Claude Code** (the CLI, via the relay), and **auto-detects** whichever client is installed (`run.bat` with no args). Per-client double-click shims are included: `run-code.bat`, `run-claude.bat`, `run-cursor.bat`.
- **`run.bat -Stop` / `run-stop.bat`** — stop the background daemon cleanly instead of ending it in Task Manager.
- **`run.bat -Cleanup`** (and `-NoTrustCA`) — revert the CA trust + persisted proxy env, or skip the CA import entirely.
- **Client OS/arch in usage sync** — each synced event now reports the daemon host's `platform` (windows/macos/linux) and `arch`, for origin attribution on the dashboard.
- **Pricing: Claude Opus 5** (`$5`/`$25`, same tier as Opus 4.8) and **Mythos 5**, so their costs are tracked instead of falling back to defaults.

### Fixed

- **Claude Desktop no longer triggers an infinite Cloudflare CAPTCHA.** `claude.ai` and `a-api.anthropic.com` are no longer MITM-intercepted — they sit behind Cloudflare bot management, and intercepting them broke Claude Desktop with an unsolvable CAPTCHA loop. They carry no per-token usage anyway; the Desktop "Code" tab and Claude Code use `api.anthropic.com`, which is still captured.

## [0.2.2] — 2026-07-21

### Added

- **Up-to-date model pricing.** Adds the newest families across all three providers so their costs are tracked instead of falling back to defaults:
  - **OpenAI** — the GPT-5.6 family (`gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`), plus `gpt-5.5`/`-pro`, `gpt-5.4` (`-mini`/`-nano`/`-pro`), `gpt-5.3-codex`, `gpt-5.1-codex-mini`.
  - **Anthropic** — `claude-fable-5` and `claude-sonnet-5` (standard rate), alongside the existing Opus 4.8 / Sonnet 4.6 / Haiku 4.5 lines.
  - **Google** — the Gemini 3 family (`gemini-3.1-pro-preview`, `gemini-3.6-flash`, `gemini-3.5-flash`, `gemini-3.5-flash-lite`, `gemini-3.1-flash-lite`) and `gemini-2.5-flash-lite`.
  - **Fixes** a stale `gemini-2.5-flash` rate that was tracked at `$0.15/$0.60` instead of the correct `$0.30/$2.50` (costs for it were under-reported), and adds cached-input rates to the Gemini 2.5/3 entries.
- **One-command Windows launcher** (`scripts/lumen.ps1` + `run.bat`, with a README). It starts the daemon, verifies the proxy is *actually listening*, and launches Claude Desktop or Cursor already wired through Lumen with the CA trusted — and if the proxy didn't come up, it says so plainly instead of the failure looking exactly like "Lumen is off." A `build_exe.sh` helper is included for producing the Windows binary.

### Fixed

- **`localhost` now works on Windows.** The API and proxy also listen on IPv6 loopback (`::1`) now. Windows resolves `localhost` to `::1` first, so clients, relay base URLs, and the dashboard's own `fetch()` calls that used `localhost` were hitting `[::1]:port` — where nothing was listening — and failing with a bare "Failed to fetch" (or silently capturing no traffic). Both listeners are loopback-only, so there's no wider exposure.
- **A proxy that can't start no longer looks like "Lumen is off."** If the proxy fails to bind its port (on Windows, usually a reserved/excluded port range that fails with `WSAEACCES` even when nothing is using the port), the daemon now logs an unmistakable diagnosis with the exact commands to check, and reports `running: false` on `/health` so the app and launcher can tell it went down.
- **Claude Desktop launches from either install location** — the standalone installer and the Microsoft Store / winget build — instead of assuming one hardcoded path.
- **No more connection stalls during the cold-start "cert storm."** When the system proxy is enabled, every host reroutes through the freshly-started MITM at once. TLS certificate generation (CPU-bound crypto) is now offloaded to a blocking thread pool, and the CA signing certificate is built once and reused instead of being rebuilt on every leaf — so the async runtime no longer starves and connections don't hang on startup.

## [0.2.1] — 2026-06-10

### Added

- **DataGrout sync sessions now stay alive automatically.** Previously the secure session credential expired after ~30 days and sync would quietly stop with no indication. Now the daemon:
  - **renews the session before it expires** while running normally, so a continuously-running daemon never lapses;
  - **recovers on its own if the session has already expired** (e.g. the app was closed for a while) — restoring the secure connection with no sign-in required;
  - **keeps syncing in a reduced mode** if automatic recovery isn't possible, and prompts you to reconnect rather than failing silently.
- **DataGrout session state is now visible in the app** (it previously showed "Connected" even after the session had lapsed):
  - **Settings → DataGrout** is three-state — *Connected*, *Session expired* (with a one-tap **Reconnect**), or *Not connected* — and shows when the session expires.
  - **Monitor tab banner** appears when the session has expired, with a Reconnect shortcut.
  - **Launch screen** no longer implies all-clear when the session has lapsed — it tells you to reconnect.
- **Version is now visible**, so it's easy to answer "what version are you on?":
  - Right-click the menu bar icon → a header line shows the app version (and the daemon version too, if they differ).
  - Settings → About lists both **App** and **Core** (daemon) versions, with a note if they don't match.
  - The web dashboard footer shows the running version.

### Fixed

- **No more repeated sync-error log spam.** A persistent sync failure now logs once and then only occasionally, instead of every 30 seconds.

### Internal

- Added certificate-expiry awareness and automatic renewal/recovery to the DataGrout sync client. New `x509-parser` dependency to read certificate validity windows (`rcgen` only generates certs, it can't parse them).

## [0.2.0] — 2026-05-29

This release is a cross-platform / enterprise readiness pass on top of the pricing accuracy work in 0.1.5. The headline shifts are universal (Intel + Apple Silicon) DMG builds, a configurable rustls crypto backend so Lumen can be deployed into FIPS environments or cross-compiled to Windows without C-toolchain pain, and a substantial UX pass driven by feedback from real non-technical users.

### Changed

- **Default crypto backend is now `ring`** (was `aws-lc-rs`). Pure Rust + assembly, no C/CMake at build time, cross-compiles cleanly to Windows and other constrained targets. Runtime-equivalent to `aws-lc-rs` for everything `lumen-core` does (TLS MITM, outbound HTTPS via reqwest, self-signed CA generation). The two backends are exposed as mutually-exclusive Cargo features:
  - `crypto-ring` (default) — laptop / Apple Silicon / Intel / Windows cross-build
  - `crypto-aws-lc` — FIPS 140-3 environments, post-quantum hybrid KEMs, AWS-aligned deployments
  - **Breaking change for source builds**: `cargo build` previously linked `aws-lc-rs`; it now links `ring`. Rebuild with `--no-default-features --features crypto-aws-lc` to keep the prior backend.
- **`rustls`, `tokio-rustls`, `rcgen`, and `reqwest` switched to `default-features = false`** so the crypto backend choice is fully controlled by `lumen-core`'s `[features]` block. Verified via `cargo tree`: each backend's dep tree contains only its own crypto crates with zero contamination from the other.
- **DMG build defaults to universal (Intel + Apple Silicon)**. `scripts/build_dmg.sh` now invokes `cargo build --target x86_64-apple-darwin` and `--target aarch64-apple-darwin` then `lipo`s them into a fat binary; SwiftPM uses `--arch x86_64 --arch arm64`. Single-arch builds remain available via `ARCHS=arm64 ./scripts/build_dmg.sh` for faster iteration. The script logs `lipo -archs` output for both bundled binaries after assembly so a stray single-arch build can't ship to Intel users undetected.

### Added

- **Web dashboard now has management UI**, not just live stats. Non-Mac users can now configure Lumen end-to-end from a browser:
  - **Session controls** — "New Lap" and "Clear Session" buttons at the top of the page
  - **Quick Setup** — copy-to-clipboard relay URLs derived from current routes + proxy port (`http://localhost:9090/openai`, etc.)
  - **Monitored Hosts** — list / add / delete the HTTPS hosts whose traffic Lumen intercepts
  - **Relay Routes** — list / add / delete the relay prefix → upstream mappings
  - **Certificate** — show CA subject + path, one-click download of `ca.pem`, and platform-specific trust-install commands (macOS / Linux / Windows tabs, defaulted by User-Agent detection)
  - **Launch Your Client** — per-platform copy-paste commands for Claude Code, OpenCode, Cursor, and Claude Desktop. macOS / Linux / Windows tabs auto-selected by User-Agent; commands use the live proxy port + CA path (no stale localhost:9090 references after port changes). Deliberately *not* a one-click launcher: spawning processes from a browser request would expand the post-compromise blast radius (an attacker with token access could launch GUI clients pointed at their own proxy or trust-store overrides) for the modest benefit of saving a paste. The command rows give the same effective outcome — copy, paste into a terminal, run — with zero new attack surface.
  - **DataGrout** — full connect / disconnect flow. Paste the DG server URL → daemon initiates DCR + PKCE registration → dashboard opens the auth tab → daemon's existing `/dg/oauth/callback` handler completes the exchange → dashboard polls `/dg/dcr/status` and flips to the connected view automatically. Same endpoints the macOS menu app already calls; nothing about the OAuth flow was Mac-specific. A `Disconnect` button calls `DELETE /dg/identity` to clear the local identity files in `~/.conduit/`.
  - UI lives in a collapsible "Settings & Configuration" panel below the existing stats grid + event feed so the live-stats view stays uncluttered for users who just want to monitor.
- **`/ca/pem` is now auth-exempt** (was: gated like the rest of the JSON API). The endpoint serves the public CA certificate — the same bytes anyone with the proxy intercepted would see — so the auth gate was theatre. Making it exempt lets a plain `<a download>` from the dashboard work without JS gymnastics. Private key material lives in a separate file (`ca.key.pem`) and remains protected; nothing about its handling changed.
- **Windows daemon support.** `lumen-core` now runs natively on Windows. The home-directory resolution falls back from `$HOME` to `$USERPROFILE`, so state files land under `C:\Users\<you>\.lumen\` without configuration. Unix-only file-permission calls (`chmod 0600` on the API token + DG identity files) are now `#[cfg(unix)]`-gated; on Windows the files inherit the user-profile directory's ACL, which is already access-controlled by the OS. Verified by cross-compiling to `x86_64-pc-windows-gnu` from a Mac (warning-free) and by a new Windows job in CI that runs the full test suite on every PR. Platform-specific privileged features (`--transparent` pf-based capture, `passive` feature) are compiled out on Windows.
- **`claude-opus-4-8`** ($5/$25, cache read $0.50, cache write $6.25) added to both compiled-in defaults and `pricing.json`, with the dated alias `claude-opus-4-8-20260528`.
- **Right-click context menu** on the status bar icon. Surfaces a spending summary (`Lap: $x.xxxx · Total: $x.xx`), New Lap, tab navigation (Monitor / Settings), a Launch submenu (Claude Code, Claude Desktop, Cursor, OpenCode), Open Dashboard in Browser, a read-only TLS trust indicator, and Quit. Addresses training feedback that Quit and lap creation were hard to find.
- **Persistent popover footer** with always-visible **Open Dashboard** and **Quit** buttons. The footer sits below the scrolling tab content so global actions are one click away from any tab — no need to flip back to Monitor to quit, or hunt through Settings to find the dashboard link. When the active tab is Monitor, the footer renders a second row above containing **Lap** and **Clear** (which were previously buried at the bottom of the Monitor scroll view, awkwardly adjacent to the footer's other buttons). The lap-naming text field also lives in this footer region so committing a lap doesn't push the footer off-screen. Other tabs see just the two-button global row.
- **`/` → `/dashboard` redirect** on the API port. Users who type `:9091` in a browser now land on the dashboard instead of getting an auth-gated response. Defangs the misdiagnosed-404 reports from initial DMG users.
- **Notification.Name extensions** (`lumenShowTab`, `lumenNavigateToMonitor`) so the right-click menu, the launchers, and any future surface can drive tab navigation through a single observable channel.
- **Click-outside dismisses the popover** via a global `NSEvent` monitor in addition to the existing `.transient` behavior. `NSApp.activate(ignoringOtherApps: true)` was removed from the open path — that call was preventing transient dismiss from firing reliably.
- **`$` prefix on cost gauges**. `ArcGauge` gained a `prefix` parameter; Lap Cost and Total gauges now show `$0.04` instead of `0.04 USD`. Addressed feedback that non-technical users scanning the gauges didn't read "USD" as currency at a glance.
- **Live CA trust status in Settings**. `checkCATrust()` now re-runs every 8 s, so the badge updates within seconds of any Keychain change rather than only on popover re-open.
- **Auto-navigate to the Monitor tab** after any Launch action. All four launcher functions now post `lumenNavigateToMonitor` once the spawn completes, so users see the live event feed start ticking without needing to manually switch tabs.

### Fixed

- **Integration tests couldn't find the daemon on Windows CI.** `tests/lifecycle.rs::find_binary()` hardcoded the binary name as `lumen-core` and panicked when Windows produced `lumen-core.exe`. Fixed by appending `std::env::consts::EXE_SUFFIX` (which is `""` on Unix, `".exe"` on Windows) to every candidate path — works uniformly on all platforms. Also gated the test-only `DaemonGuard::pid()` method with `#[cfg(unix)]` since its only caller (`test_sigterm_handling`) was already Unix-only, eliminating a `dead_code` warning on the Windows runner.
- **First click in the popover required a "wake up" tap.** With `LSUIElement = true` (accessory app) and `.transient` popover behavior (no `NSApp.activate`), AppKit didn't auto-promote the popover to key window on show — so the first click on a tab was consumed by focus-grab instead of hitting the button. Fixed by calling `popover.contentViewController?.view.window?.makeKey()` immediately after `popover.show(...)`. We deliberately don't use `makeKeyAndOrderFront` or `NSApp.activate` here because either would yank system focus from the user's foreground app; `makeKey()` alone gives the popover the key-window status it needs without stealing focus.
- **Tab buttons needed pixel-precise aim.** Under `.buttonStyle(.plain)`, SwiftUI only treats the rendered text glyph as hit-testable — clicks on the padded background area registered as misses. Added `.contentShape(Rectangle())` to all `.plain`-styled action buttons (tab buttons in the tab bar, Lap, Clear, footer buttons) so the full rendered frame is clickable.
- **UI not updating after Restart button** — the polling `Timer` was scheduled in `.default` runloop mode, which NSPopover can suspend while open. As a result, between clicking "Restart" and closing/reopening the popover, no polls fired and the "Daemon not running" banner stayed up. Fixed by scheduling the timer via `RunLoop.main.add(timer, forMode: .common)` so it ticks regardless of popover state. Combined with explicit `pollNow()` triggers at +0.5 s and +1.5 s after Restart, the banner now disappears within ~1 s of the daemon coming up.
- **Polling timer was hammering the OS scheduler** because manual `Timer(timeInterval:)` construction leaves `tolerance` at 0, defeating wakeup coalescing. Set `tolerance = 0.15` on the main poll timer (and `1.0` on the slower CA-trust refresh timer); the user-visible polling cadence is unchanged but battery wakeups are now coalesceable.
- **CA info never refreshed after first fetch**. `poll()` had `if caInfo == nil { await fetchCAInfo() }`, so cert details were captured once at startup and never re-fetched. Removed the guard — CA info now refreshes on every tick, which matters when a user regenerates the CA or the daemon swaps to a different cert.
- **CA trust check blocked the main thread when opening the right-click menu**. The status-bar context menu previously called `security dump-trust-settings` synchronously while building the menu, hitching the UI for 100–300 ms on every open. Moved trust checking into `APIClient.refreshCATrust()` which runs on a background queue every ~5 s and updates a published `caTrusted` property; menu and Settings view read the cached value instantly.
- **DMG build broken on macOS 15+** — `hdiutil attach`'s tab-separated output format changed in Sequoia, causing `awk '{print $NF}'` to return `1` instead of the mount path. Replaced with an explicit `-mountpoint` flag so we no longer parse hdiutil's output at all.
- **DMG build script silently swallowed Swift compile failures** — the `swift build … | grep -E 'error:|warning:|Build complete'` filter was fragile: multi-arch (XCBuild) builds emit "Build succeeded" instead of "Build complete", so under `pipefail` the grep returned 1 and aborted the script with a misleadingly clean exit, leaving no DMG and no error message. Removed the grep filter (full output streams through) and added an explicit check that fails loudly if the expected built binary isn't found.
- **Bundle version drift**. `Info.plist`'s `CFBundleShortVersionString` had fallen behind `Cargo.toml`; both are now driven from the same release tag (0.2.0).

### Internal

- **`LaunchService.swift` extracted** — the four LLM-client launch flows (Claude Code, OpenCode, Cursor, Claude Desktop) previously existed in two parallel implementations: one in `LaunchersView.swift` (the Launch tab), one as `@objc` handlers in `LumenApp.swift` (the right-click menu). They have now been unified into a single `LumenLauncher` enum + `LauncherSupport` helpers that both call sites use. Eliminates the drift risk where a fix in one path would silently not propagate to the other.
- **Tab navigation consolidated to a single notification.** Removed the redundant `lumenNavigateToMonitor` notification; all callers now post `lumenShowTab` with the target `AppTab.rawValue`. Simpler observation surface, no behavioral difference.
- **`install_crypto_provider()` extracted from `main`** and made `pub` so test setup can call it via `std::sync::Once`. Single chokepoint per test module (`proxy::tests`, `tls::tests`, `tests/lifecycle.rs`) — no per-test boilerplate, no risk of provider-not-installed panics in CI.
- **`compile_error!` guard** if neither `crypto-ring` nor `crypto-aws-lc` is enabled. Prevents the "compiles, then panics on first TLS handshake" failure mode that an over-zealous `--no-default-features` could otherwise produce.

### Tests / CI

- All **91 unit tests + 13 integration tests pass under both `crypto-ring` (default) and `crypto-aws-lc`** feature configurations on macOS, and under `crypto-ring` on Windows.
- **CI now runs a matrix** across `(macos-latest × crypto-ring, macos-latest × crypto-aws-lc, windows-latest × crypto-ring)` (`.github/workflows/ci.yml`). Cache keys are namespaced per `(os, backend)` to avoid cross-thrashing. Windows skips the `crypto-aws-lc` row to keep PR build time reasonable — it works in principle but the C-toolchain cost isn't justified until a real customer asks for FIPS-on-Windows.
- `cargo tree` verified to contain `ring` only (default) or `aws-lc-rs` only (`crypto-aws-lc`) — no cross-contamination between backends.
- **Universal DMG pipeline verified end-to-end**: built `dist/Lumen-0.2.0.dmg` (9.8 MB), confirmed both bundled binaries report `x86_64 arm64` via `lipo -archs`, launched the daemon directly from the mounted DMG, and confirmed `:9091/dashboard` returns HTTP 200 and `:9091/` 302-redirects to `/dashboard`.

### Known limitations (not addressed)

- **DMG is still ad-hoc signed**, not notarized. Managed-laptop users may have first-launch blocked by Gatekeeper policy. Pending org Apple Developer ID.
- **No Windows installer / `.msi`**. Windows users currently invoke the daemon binary directly from a terminal. A proper installer with a Start Menu entry and service registration is a future addition.

---

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
