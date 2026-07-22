//! Pricing database loader.
//!
//! Load priority (highest wins):
//!   1. `~/.lumen/pricing.json`          — user-managed override
//!   2. `~/.lumen/pricing.json.cache`    — last successful remote fetch
//!   3. Compiled-in defaults             — always available
//!
//! On startup, after returning the best available database, a background task
//! fetches the canonical pricing.json from the repo and writes it to the cache
//! path so the next restart gets fresh rates — without blocking boot or
//! requiring a hot-swap of the in-process database.

use std::path::PathBuf;

use super::{PricingDatabase, PricingFile};

const REMOTE_URL: &str =
    "https://raw.githubusercontent.com/DataGrout/lumen/main/lumen-core/pricing.json";

const FETCH_TIMEOUT_SECS: u64 = 10;

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
    Some(PricingDatabase::from_file(&file))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load the best available pricing database synchronously, then spawn a
/// background task to refresh the cache from the remote URL.
///
/// Call this once at startup before creating the `Aggregator`.
pub fn load_pricing() -> PricingDatabase {
    // 1. User override (highest priority — never overwritten by background fetch)
    let override_path = user_override_path();
    if override_path.exists() {
        if let Some(db) = try_load(&override_path) {
            tracing::info!(
                "pricing: loaded from user override {}",
                override_path.display()
            );
            spawn_background_refresh(); // still refresh cache for next boot
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
            spawn_background_refresh();
            return db;
        }
        tracing::warn!(
            "pricing: remote cache {} unreadable, continuing",
            cache.display()
        );
    }

    // 3. Compiled-in defaults
    tracing::info!("pricing: using compiled-in defaults (no local file found)");
    spawn_background_refresh();
    PricingDatabase::with_defaults()
}

/// Spawn a best-effort background task that fetches the remote pricing file
/// and writes it to `~/.lumen/pricing.json.cache`.  Failures are logged at
/// warn level and never panic — the next restart will retry.
pub fn spawn_background_refresh() {
    tokio::spawn(async move {
        match fetch_remote().await {
            Ok(content) => {
                let path = cache_path();
                if let Err(e) = std::fs::write(&path, &content) {
                    tracing::warn!("pricing: failed to write cache {}: {}", path.display(), e);
                } else {
                    tracing::info!("pricing: remote cache updated ({})", path.display());
                }
            }
            Err(e) => {
                tracing::warn!("pricing: remote refresh failed — {}", e);
            }
        }
    });
}

async fn fetch_remote() -> anyhow::Result<String> {
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

    Ok(text)
}
