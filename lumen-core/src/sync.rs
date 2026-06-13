use anyhow::Result;
use parking_lot::RwLock;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::aggregator::{Aggregator, UsageEvent};
use crate::conduit::ConduitIdentity;
use crate::state::{DgAuthMode, DgCertStatus};

const SYNC_INTERVAL: Duration = Duration::from_secs(30);
const MAX_BATCH_SIZE: usize = 100;
/// Rotate the mTLS cert when it's within this window of expiry (and still
/// valid). DG issues 30-day certs, so a 7-day lead gives ~23 days of slack
/// and many sync ticks to retry a transient rotation failure.
const ROTATE_THRESHOLD: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, Serialize)]
struct UsageBatch {
    source: &'static str,
    version: &'static str,
    events: Vec<SyncEvent>,
}

#[derive(Debug, Clone, Serialize)]
struct SyncEvent {
    timestamp: String,
    provider: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_creation_tokens: Option<u64>,
    cost_usd: f64,
    host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lap_id: Option<String>,
}

impl From<&UsageEvent> for SyncEvent {
    fn from(e: &UsageEvent) -> Self {
        // Extract the hostname from the full URL.
        let host = e
            .url
            .split("://")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .unwrap_or(&e.url)
            .to_string();

        Self {
            timestamp: e.timestamp.to_rfc3339(),
            provider: format!("{:?}", e.provider),
            model: e.model.clone(),
            input_tokens: e.usage.input_tokens,
            output_tokens: e.usage.output_tokens,
            total_tokens: e.usage.total_tokens,
            cache_read_tokens: e.usage.cache_read_tokens,
            cache_creation_tokens: e.usage.cache_creation_tokens,
            cost_usd: e.cost.total_cost,
            host,
            session_id: None,
            lap_id: Some(format!("lap_{}", e.lap_number)),
        }
    }
}

pub struct DGSyncer {
    aggregator: Arc<Aggregator>,
    /// Shared reqwest::Client — may carry an mTLS identity.  Replaced atomically
    /// when the user completes the Conduit bootstrap flow.
    dg_http_client: Arc<RwLock<reqwest::Client>>,
    server_url: Arc<RwLock<Option<String>>>,
    /// Bearer token used before mTLS identity is registered.  Cleared on bootstrap.
    bearer_token: Arc<RwLock<Option<String>>>,
    /// Stable per-device HMAC sync token (replaces mTLS when CloudFront strips certs).
    sync_token: Arc<RwLock<Option<String>>>,
    /// Conduit sub_id of the registered device.
    lumen_sub_id: Arc<RwLock<Option<String>>>,
    /// The mTLS identity — read to check expiry, swapped on rotation.
    dg_identity: Arc<RwLock<Option<ConduitIdentity>>>,
    /// Surfaced cert health (expiry + auth mode) for `/dg/status`.
    cert_status: Arc<RwLock<DgCertStatus>>,
    last_synced_count: RwLock<u64>,
    /// Count of consecutive failed sync batches — drives log backoff so a
    /// persistent failure doesn't spam a warning every 30 s.
    consecutive_failures: RwLock<u64>,
}

impl DGSyncer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        aggregator: Arc<Aggregator>,
        server_url: Arc<RwLock<Option<String>>>,
        dg_http_client: Arc<RwLock<reqwest::Client>>,
        bearer_token: Arc<RwLock<Option<String>>>,
        sync_token: Arc<RwLock<Option<String>>>,
        lumen_sub_id: Arc<RwLock<Option<String>>>,
        dg_identity: Arc<RwLock<Option<ConduitIdentity>>>,
        cert_status: Arc<RwLock<DgCertStatus>>,
    ) -> Self {
        Self {
            aggregator,
            dg_http_client,
            server_url,
            bearer_token,
            sync_token,
            lumen_sub_id,
            dg_identity,
            cert_status,
            last_synced_count: RwLock::new(0u64),
            consecutive_failures: RwLock::new(0u64),
        }
    }

    pub async fn start(&self, mut shutdown_rx: watch::Receiver<bool>) {
        info!("DG sync service started");

        loop {
            tokio::select! {
                _ = tokio::time::sleep(SYNC_INTERVAL) => {
                    // Keep the identity healthy (rotate before expiry / fall
                    // back to bearer if already expired) before each sync.
                    self.maintain_identity().await;
                    if let Err(e) = self.sync_batch().await {
                        debug!("DG sync skipped: {}", e);
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        if let Err(e) = self.sync_batch().await {
                            debug!("Final sync failed: {}", e);
                        }
                        info!("DG sync service shutting down");
                        break;
                    }
                }
            }
        }
    }

    async fn sync_batch(&self) -> Result<()> {
        let url = {
            let guard = self.server_url.read();
            match guard.as_ref() {
                Some(u) => u.clone(),
                None => return Ok(()),
            }
        };

        let total = self.aggregator.total_event_count();
        let last = *self.last_synced_count.read();

        if total <= last {
            return Ok(());
        }

        // Fetch the newest (total - last) events, capped at MAX_BATCH_SIZE.
        // recent_events returns newest-first; reverse to send oldest-first.
        let to_fetch = ((total - last) as usize).min(MAX_BATCH_SIZE);
        let new_events: Vec<SyncEvent> = self
            .aggregator
            .recent_events(to_fetch)
            .into_iter()
            .rev()
            .map(|e| SyncEvent::from(&e))
            .collect();

        if new_events.is_empty() {
            return Ok(());
        }

        let batch = UsageBatch {
            source: "lumen",
            version: env!("CARGO_PKG_VERSION"),
            events: new_events,
        };

        // Usage events go to the gateway root, not the per-server path.
        let gateway_root = Self::gateway_root(&url);
        let endpoint = format!("{}/api/v1/lumen/usage", gateway_root.trim_end_matches('/'));
        let client = self.dg_http_client.read().clone();

        let mut req = client
            .post(&endpoint)
            .json(&batch)
            .timeout(Duration::from_secs(10));

        // Prefer the stable per-device HMAC Bearer token (works through CloudFront).
        // Fall back to the short-lived bootstrap Bearer token if the sync token isn't set yet.
        let sync_token = self.sync_token.read().clone();
        let lumen_sub_id = self.lumen_sub_id.read().clone();
        if let (Some(sid), Some(token)) = (lumen_sub_id, sync_token) {
            req = req.bearer_auth(format!("lm_{}.{}", sid, token));
        } else {
            let bearer = self.bearer_token.read().clone();
            if bearer.is_none() {
                warn!("DG sync: no auth token available (sync_token and bearer_token both unset) — request will be rejected");
            }
            if let Some(token) = bearer {
                req = req.bearer_auth(token);
            }
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                let count = batch.events.len();
                *self.last_synced_count.write() = last + count as u64;
                *self.consecutive_failures.write() = 0;
                info!("Synced {} usage events to {}", count, url);
                Ok(())
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                self.log_sync_failure(format!("status {} from {} — {}", status, endpoint, body));
                Ok(())
            }
            Err(e) => {
                self.log_sync_failure(format!("request failed: {}", e));
                Ok(())
            }
        }
    }

    /// Derive the API origin from the stored server URL, which may be a
    /// per-server MCP URL (`https://host/servers/{uuid}`). Strips the
    /// `/servers/...` segment so we hit `https://host/api/v1/...`.
    fn gateway_root(url: &str) -> String {
        url.find("/servers/")
            .map(|pos| url[..pos].to_string())
            .unwrap_or_else(|| url.to_string())
    }

    /// Log a sync failure with backoff: full WARN on the first failure of a
    /// streak and then once per ~10 minutes (every 20th tick), DEBUG otherwise.
    /// Prevents a persistent failure (e.g. expired cert pre-fix) from emitting
    /// a multi-line gateway error every 30 seconds forever.
    fn log_sync_failure(&self, detail: String) {
        let mut n = self.consecutive_failures.write();
        *n += 1;
        let count = *n;
        drop(n);
        if count == 1 || count % 20 == 0 {
            warn!(
                "DG sync failing ({}x): {} — see DG status; reconnect may be required",
                count, detail
            );
        } else {
            debug!("DG sync failure #{}: {}", count, detail);
        }
    }

    /// Keep the mTLS identity healthy before each sync:
    ///   - cert valid & far from expiry  → ensure mode = Mtls
    ///   - cert valid & within threshold → rotate over mTLS, swap in fresh cert
    ///   - cert already expired          → drop it, fall back to bearer-only
    /// Updates `cert_status` for `/dg/status` surfacing.
    async fn maintain_identity(&self) {
        let identity = self.dg_identity.read().clone();
        let Some(id) = identity else {
            // No identity configured — nothing to maintain.
            *self.cert_status.write() = DgCertStatus {
                expires_at: None,
                mode: DgAuthMode::None,
            };
            return;
        };

        let expires_at = id
            .cert_not_after()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);

        if id.is_expired() {
            // Already expired: the server won't accept the dead cert over mTLS.
            // First TRY to self-heal — reissue a fresh cert authenticated by the
            // stored sync-token bearer (used when the live cert is gone). If the
            // server supports it we transparently restore full mTLS with no user
            // action; otherwise we degrade to bearer-only below.
            let sub_id = self.lumen_sub_id.read().clone();
            let token = self.sync_token.read().clone();
            let url = self.server_url.read().clone();
            if let (Some(sub_id), Some(token), Some(url)) = (sub_id, token, url) {
                let gateway_root = Self::gateway_root(&url);
                match crate::conduit::reissue_identity_via_sync_token(
                    &id, &gateway_root, &sub_id, &token,
                )
                .await
                {
                    Ok(new_id) => {
                        let new_expiry = new_id
                            .cert_not_after()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs() as i64);
                        if let Ok(c) = new_id.build_client() {
                            *self.dg_http_client.write() = c;
                            *self.dg_identity.write() = Some(new_id);
                            *self.cert_status.write() = DgCertStatus {
                                expires_at: new_expiry,
                                mode: DgAuthMode::Mtls,
                            };
                            info!("DG identity auto-reconnected via sync token — mTLS restored");
                            return;
                        }
                    }
                    Err(e) => {
                        // Gateway may not support sync-token reissue yet, or the
                        // token is gone — degrade to bearer-only below.
                        debug!("DG sync-token reissue unavailable: {}", e);
                    }
                }
            }

            // Fallback: don't poison the connection with the dead cert — switch
            // to bearer-only so sync keeps working in a degraded (no-mTLS) mode.
            let already_fallback =
                self.cert_status.read().mode == DgAuthMode::BearerFallback;
            if !already_fallback {
                match crate::conduit::bearer_only_client() {
                    Ok(c) => {
                        *self.dg_http_client.write() = c;
                        warn!(
                            "DG mTLS cert expired — switched to sync-token auth so sync \
                             continues. Reconnect DataGrout to restore mTLS."
                        );
                    }
                    Err(e) => warn!("DG: failed to build bearer-only client: {}", e),
                }
            }
            *self.cert_status.write() = DgCertStatus {
                expires_at,
                mode: DgAuthMode::BearerFallback,
            };
            return;
        }

        if id.needs_rotation(ROTATE_THRESHOLD) {
            // Still valid but nearing expiry — rotate proactively over mTLS.
            let url = self.server_url.read().clone();
            if let Some(url) = url {
                let gateway_root = Self::gateway_root(&url);
                match crate::conduit::rotate_identity(&id, &gateway_root).await {
                    Ok(new_id) => {
                        let new_expiry = new_id
                            .cert_not_after()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs() as i64);
                        match new_id.build_client() {
                            Ok(c) => {
                                *self.dg_http_client.write() = c;
                                *self.dg_identity.write() = Some(new_id);
                                *self.cert_status.write() = DgCertStatus {
                                    expires_at: new_expiry,
                                    mode: DgAuthMode::Mtls,
                                };
                                info!("DG mTLS cert rotated proactively before expiry");
                            }
                            Err(e) => {
                                warn!("DG: rotated cert but failed to build client: {}", e)
                            }
                        }
                    }
                    Err(e) => {
                        // Non-fatal: cert is still valid, we'll retry next tick.
                        warn!("DG cert rotation failed (will retry): {}", e);
                    }
                }
            }
            return;
        }

        // Valid and not near expiry — just keep the surfaced status current.
        *self.cert_status.write() = DgCertStatus {
            expires_at,
            mode: DgAuthMode::Mtls,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::PricingDatabase;

    #[test]
    fn test_sync_event_from_usage_event() {
        let pricing = PricingDatabase::with_defaults();
        let aggregator = Arc::new(Aggregator::new(pricing));

        let usage = crate::parser::TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
            cache_read_tokens: Some(10),
            cache_creation_tokens: None,
        };

        aggregator.record_usage(
            crate::parser::LLMProvider::OpenAI,
            "gpt-4o",
            "https://api.openai.com/v1/chat/completions",
            usage,
        );

        let events = aggregator.recent_events(10);
        assert_eq!(events.len(), 1);

        let sync_event = SyncEvent::from(&events[0]);
        assert_eq!(sync_event.input_tokens, 100);
        assert_eq!(sync_event.output_tokens, 50);
        assert_eq!(sync_event.provider, "OpenAI");
        assert_eq!(sync_event.model, "gpt-4o");
        assert_eq!(sync_event.cache_read_tokens, Some(10));
        assert_eq!(sync_event.lap_id, Some("lap_1".to_string()));
    }

    #[test]
    fn test_syncer_creation() {
        let pricing = PricingDatabase::with_defaults();
        let aggregator = Arc::new(Aggregator::new(pricing));
        let url = Arc::new(RwLock::new(None));
        let client = Arc::new(RwLock::new(reqwest::Client::new()));
        let token: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let sync_token: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let sub_id: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let identity = Arc::new(RwLock::new(None));
        let cert_status = Arc::new(RwLock::new(DgCertStatus::default()));
        let _syncer = DGSyncer::new(
            aggregator, url, client, token, sync_token, sub_id, identity, cert_status,
        );
    }

    fn make_syncer(aggregator: Arc<Aggregator>) -> DGSyncer {
        DGSyncer::new(
            aggregator,
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(reqwest::Client::new())),
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(DgCertStatus::default())),
        )
    }

    #[tokio::test]
    async fn test_sync_skips_without_server_url() {
        let pricing = PricingDatabase::with_defaults();
        let aggregator = Arc::new(Aggregator::new(pricing));

        let usage = crate::parser::TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        };
        aggregator.record_usage(
            crate::parser::LLMProvider::OpenAI,
            "gpt-4o",
            "https://api.openai.com/v1/chat/completions",
            usage,
        );

        let syncer = make_syncer(aggregator);
        // No server_url set — sync_batch must return Ok without touching HTTP
        assert!(syncer.sync_batch().await.is_ok());
        // Watermark must not advance (no sync happened)
        assert_eq!(*syncer.last_synced_count.read(), 0);
    }

    #[tokio::test]
    async fn test_sync_watermark_does_not_advance_when_no_new_events() {
        let pricing = PricingDatabase::with_defaults();
        let aggregator = Arc::new(Aggregator::new(pricing));
        let syncer = make_syncer(aggregator);

        // total == last (both 0) → early return, watermark stays at 0
        assert!(syncer.sync_batch().await.is_ok());
        assert_eq!(*syncer.last_synced_count.read(), 0);
    }
}
