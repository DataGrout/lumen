use anyhow::Result;
use parking_lot::RwLock;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::aggregator::{Aggregator, UsageEvent};

const SYNC_INTERVAL: Duration = Duration::from_secs(30);
const MAX_BATCH_SIZE: usize = 100;

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
    last_synced_count: RwLock<usize>,
}

impl DGSyncer {
    pub fn new(
        aggregator: Arc<Aggregator>,
        server_url: Arc<RwLock<Option<String>>>,
        dg_http_client: Arc<RwLock<reqwest::Client>>,
        bearer_token: Arc<RwLock<Option<String>>>,
        sync_token: Arc<RwLock<Option<String>>>,
        lumen_sub_id: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            aggregator,
            dg_http_client,
            server_url,
            bearer_token,
            sync_token,
            lumen_sub_id,
            last_synced_count: RwLock::new(0),
        }
    }

    pub async fn start(&self, mut shutdown_rx: watch::Receiver<bool>) {
        info!("DG sync service started");

        loop {
            tokio::select! {
                _ = tokio::time::sleep(SYNC_INTERVAL) => {
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

        let events = self.aggregator.recent_events(MAX_BATCH_SIZE);
        let current_count = events.len();
        let last = *self.last_synced_count.read();

        if current_count <= last {
            return Ok(());
        }

        let new_events: Vec<SyncEvent> = events[..current_count.saturating_sub(last)]
            .iter()
            .map(SyncEvent::from)
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
        // dg_server_url is https://host/servers/{uuid} — strip the server segment.
        let gateway_root = url
            .find("/servers/")
            .map(|pos| url[..pos].to_string())
            .unwrap_or_else(|| url.clone());
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
                *self.last_synced_count.write() = current_count;
                info!("Synced {} usage events to {}", count, url);
                Ok(())
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                warn!("DG sync got status {}: {} — {}", status, endpoint, body);
                Ok(())
            }
            Err(e) => {
                warn!("DG sync request failed: {}", e);
                Ok(())
            }
        }
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
        let _syncer = DGSyncer::new(aggregator, url, client, token, sync_token, sub_id);
    }
}
