//! Pricing database loader.
//!
//! Load priority (highest wins):
//!   1. `~/.lumen/pricing.json`          — user-managed override
//!   2. `~/.lumen/pricing.json.cache`    — last successful remote fetch
//!   3. Compiled-in defaults             — always available
//!
//! Every source is laid over the compiled-in defaults rather than replacing
//! them, so a stale list can fail to add a model but never remove one the
//! binary already knows — see `PricingDatabase::from_file_over_defaults`.
//!
//! Once the aggregator exists, `spawn_refresh_loop` fetches the published list
//! immediately and then every few hours, writes it to the cache, and swaps it
//! into the running daemon. It used to be fetched once at startup and applied
//! only on the next restart, so a daemon left running for weeks priced newly
//! released models by fuzzy match for as long as nobody restarted it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::aggregator::Aggregator;

use super::{PricingDatabase, PricingFile};

const REMOTE_URL: &str =
    "https://raw.githubusercontent.com/DataGrout/lumen/main/lumen-core/pricing.json";

const FETCH_TIMEOUT_SECS: u64 = 10;

/// Between successful refreshes. Published prices move on the scale of days, so
/// this is far more often than the list changes and still one small request.
const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// After a failed refresh. Short enough that a network that was down when the
/// daemon started does not leave it stale for a whole interval.
const RETRY_INTERVAL: Duration = Duration::from_secs(30 * 60);

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

fn lumen_dir() -> PathBuf {
    // Use the shared cross-platform resolver (HOME, then USERPROFILE on Windows)
    // — the same one ca.rs / conduit.rs use, so pricing lands in the same
    // `~/.lumen` dir as everything else. Falling back to the raw `HOME` var here
    // produced a Unix `/tmp/.lumen/...` path on Windows (no HOME) that the OS
    // rejected. `std::env::temp_dir()` is the correct last-resort per platform.
    crate::state::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".lumen")
}

pub fn user_override_path() -> PathBuf {
    lumen_dir().join("pricing.json")
}

pub fn cache_path() -> PathBuf {
    lumen_dir().join("pricing.json.cache")
}

// ---------------------------------------------------------------------------
// Synchronous load helpers
// ---------------------------------------------------------------------------

fn try_load(path: &PathBuf) -> Option<PricingDatabase> {
    let content = std::fs::read_to_string(path).ok()?;
    let file: PricingFile = match serde_json::from_str(&content) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("pricing: failed to parse {}: {}", path.display(), e);
            return None;
        }
    };
    Some(PricingDatabase::from_file_over_defaults(&file))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load the best available pricing database synchronously.
///
/// Call this once at startup before creating the `Aggregator`, then hand the
/// aggregator to `spawn_refresh_loop`.
pub fn load_pricing() -> PricingDatabase {
    // 1. User override (highest priority — never overwritten by background fetch)
    let override_path = user_override_path();
    if override_path.exists() {
        if let Some(db) = try_load(&override_path) {
            tracing::info!(
                "pricing: loaded from user override {}",
                override_path.display()
            );
            return db;
        }
        tracing::warn!(
            "pricing: user override {} unreadable, continuing",
            override_path.display()
        );
    }

    // 2. Last remote fetch cache
    let cache = cache_path();
    if cache.exists() {
        if let Some(db) = try_load(&cache) {
            tracing::info!("pricing: loaded from remote cache {}", cache.display());
            return db;
        }
        tracing::warn!(
            "pricing: remote cache {} unreadable, continuing",
            cache.display()
        );
    }

    // 3. Compiled-in defaults
    tracing::info!("pricing: using compiled-in defaults (no local file found)");
    PricingDatabase::with_defaults()
}

/// Keep the running daemon on the published price list.
///
/// Fetches immediately, then every `REFRESH_INTERVAL` (or `RETRY_INTERVAL`
/// after a failure). Each good list is written to the cache for the next start
/// and swapped into `aggregator` straight away, unless a user override exists —
/// the override is deliberately the user's to manage, so it keeps winning and the
/// cache is only kept fresh for when it is removed. Failures are logged and
/// retried; they never touch the table already in use.
pub fn spawn_refresh_loop(aggregator: Arc<Aggregator>) {
    tokio::spawn(async move {
        // The text last swapped in, so an unchanged list is neither re-applied
        // nor re-announced every few hours.
        let mut applied: Option<String> = None;

        loop {
            let wait = match fetch_remote().await {
                Ok((text, file)) => {
                    let path = cache_path();
                    if let Err(e) = std::fs::write(&path, &text) {
                        tracing::warn!("pricing: failed to write cache {}: {}", path.display(), e);
                    }

                    if user_override_path().exists() {
                        tracing::debug!("pricing: user override present; refreshed cache only");
                    } else if applied.as_deref() != Some(text.as_str()) {
                        aggregator.replace_pricing(PricingDatabase::from_file_over_defaults(&file));
                        tracing::info!(
                            "pricing: applied published price list (updated {})",
                            file.updated
                        );
                        applied = Some(text);
                    }
                    REFRESH_INTERVAL
                }
                Err(e) => {
                    tracing::warn!("pricing: refresh failed — {}; retrying later", e);
                    RETRY_INTERVAL
                }
            };
            tokio::time::sleep(wait).await;
        }
    });
}

async fn fetch_remote() -> anyhow::Result<(String, PricingFile)> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .no_proxy()
        .build()?;

    let resp = client.get(REMOTE_URL).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }

    let text = resp.text().await?;

    // Validate before caching — reject corrupt or future-schema files.
    let file: PricingFile =
        serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("invalid JSON: {}", e))?;

    if file.schema_version != 1 {
        anyhow::bail!("unsupported schema_version {}", file.schema_version);
    }

    Ok((text, file))
}
