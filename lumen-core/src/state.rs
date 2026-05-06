use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::watch;

/// Loads the API token from `~/.lumen/api.token`, or generates and persists a new one.
/// The file is written with 0600 permissions so only the owning user can read it.
pub fn load_or_create_api_token() -> String {
    let Some(home) = std::env::var("HOME").ok() else {
        return ephemeral_token();
    };
    let path = std::path::PathBuf::from(home)
        .join(".lumen")
        .join("api.token");

    if let Ok(existing) = std::fs::read_to_string(&path) {
        let t = existing.trim().to_string();
        if t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            return t;
        }
    }

    let token = ephemeral_token();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .and_then(|mut f| f.write_all(token.as_bytes()));
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::write(&path, &token);
    }
    token
}

fn ephemeral_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn dg_config_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        std::path::PathBuf::from(home)
            .join(".lumen")
            .join("dg_config.json"),
    )
}

pub fn load_dg_config_from_disk() -> Option<DGConfig> {
    let path = dg_config_path()?;
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice::<DGConfig>(&bytes).ok()
}

pub fn save_dg_config_to_disk(config: &DGConfig) {
    let Some(path) = dg_config_path() else { return };
    // Don't persist bearer_token — it's a short-lived bootstrap credential.
    let saveable = DGConfig {
        bearer_token: None,
        ..config.clone()
    };
    if let Ok(json) = serde_json::to_string_pretty(&saveable) {
        let _ = std::fs::write(&path, json);
    }
}

use crate::aggregator::Aggregator;
use crate::ca::LumenCA;
use crate::conduit::ConduitIdentity;
use crate::proxy::{BodyLimits, LumenProxy};
use crate::tls::CertCache;
use crate::traffic::TrafficLog;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub port: u16,
    pub running: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransparentConfig {
    pub enabled: bool,
    pub port: u16,
    pub running: bool,
    pub mode: String, // "off", "local", "forwarded"
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            port: 9090,
            running: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DGConfig {
    pub enabled: bool,
    pub server_url: Option<String>,
    pub tools_hidden: bool,
    pub intelligent_interface: bool,
    /// Bearer token for DG sync before a Conduit mTLS identity is registered.
    /// Cleared once the identity is bootstrapped.
    #[serde(default)]
    pub bearer_token: Option<String>,
    /// Stable per-device HMAC sync token returned by DG during bootstrap.
    /// Used as `Bearer lm_<sub_id>.<sync_token>` since CloudFront strips mTLS certs.
    #[serde(default)]
    pub sync_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PayloadSample {
    pub timestamp: String,
    pub host: String,
    pub path: String,
    pub method: String,
    pub content_type: Option<String>,
    pub request_preview: String,
    pub response_preview: String,
    pub request_hex: String,
    pub response_hex: String,
    pub request_bytes: usize,
    pub response_bytes: usize,
}

/// Ring buffer of captured payload samples for debugging
pub struct SampleCapture {
    pub armed: RwLock<bool>,
    pub max_samples: usize,
    pub samples: RwLock<VecDeque<PayloadSample>>,
}

impl SampleCapture {
    pub fn new(max: usize) -> Self {
        Self {
            armed: RwLock::new(false),
            max_samples: max,
            samples: RwLock::new(VecDeque::new()),
        }
    }

    pub fn is_armed(&self) -> bool {
        *self.armed.read()
    }

    pub fn arm(&self) {
        *self.armed.write() = true;
        self.samples.write().clear();
    }

    pub fn disarm(&self) {
        *self.armed.write() = false;
    }

    pub fn push(&self, sample: PayloadSample) {
        let mut samples = self.samples.write();
        if samples.len() >= self.max_samples {
            samples.pop_front();
        }
        samples.push_back(sample);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DcrFlowStatus {
    WaitingForCode,
    ExchangingToken,
    Complete,
    Failed { error: String },
}

#[derive(Debug, Clone)]
pub struct DcrFlow {
    /// OAuth base URL (everything before /servers/…) — used for token exchange calls.
    pub server_url: String,
    /// Canonical per-server Lumen URL — written to dg_server_url on OAuth completion.
    pub lumen_server_url: String,
    pub client_id: String,
    pub code_verifier: String,
    pub redirect_uri: String,
    pub state_param: String,
    pub auth_url: String,
    pub status: DcrFlowStatus,
    pub device_name: String,
}

pub struct AppState {
    pub api_token: String,
    pub aggregator: Arc<Aggregator>,
    pub proxy: RwLock<Option<Arc<LumenProxy>>>,
    pub proxy_config: RwLock<ProxyConfig>,
    pub transparent_config: RwLock<TransparentConfig>,
    pub dg_config: RwLock<DGConfig>,
    pub dg_server_url: Arc<RwLock<Option<String>>>,
    /// Bearer token for DG sync before Conduit mTLS is bootstrapped.
    /// Shared so the bootstrap endpoint can install / clear it at runtime.
    pub dg_bearer_token: Arc<RwLock<Option<String>>>,
    /// Conduit mTLS identity for DG sync.  `None` until the user completes the
    /// OAuth bootstrap flow from the Swift UI.
    pub dg_identity: Arc<RwLock<Option<ConduitIdentity>>>,
    /// Shared HTTP client for DG sync.  Rebuilt when the identity changes.
    pub dg_http_client: Arc<RwLock<reqwest::Client>>,
    /// Stable per-device HMAC Bearer token returned by DG bootstrap_identity.
    /// Sent as `Bearer lm_<sub_id>.<sync_token>` since CloudFront strips mTLS certs.
    pub dg_sync_token: Arc<RwLock<Option<String>>>,
    /// Conduit sub_id of the registered device (from dg_identity or saved config).
    pub dg_lumen_sub_id: Arc<RwLock<Option<String>>>,
    pub traffic_log: Arc<TrafficLog>,
    pub ca: Arc<LumenCA>,
    pub cert_cache: Arc<CertCache>,
    pub shutdown_tx: watch::Sender<bool>,
    pub shutdown_rx: watch::Receiver<bool>,
    pub sample_capture: Arc<SampleCapture>,
    pub dcr_flow: Arc<RwLock<Option<DcrFlow>>>,
    pub api_port: RwLock<u16>,
    /// Shared body size limits — written by the API, read by both proxy types.
    pub body_limits: Arc<RwLock<BodyLimits>>,
}

impl AppState {
    pub fn new(aggregator: Arc<Aggregator>, ca: LumenCA) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let ca = Arc::new(ca);
        let cert_cache = Arc::new(CertCache::new(ca.clone()));

        // Try to load a pre-existing Conduit identity from ~/.conduit/.
        let identity = ConduitIdentity::try_load();
        let http_client = identity
            .as_ref()
            .and_then(|id| id.build_client().ok())
            .unwrap_or_else(|| reqwest::Client::new());

        let dg_config = load_dg_config_from_disk().unwrap_or_default();
        // Normalize on load — old configs may have stored /mcp-suffixed or non-canonical URLs.
        let dg_server_url = dg_config
            .server_url
            .as_deref()
            .and_then(crate::api::canonical_dg_server_url)
            .or_else(|| dg_config.server_url.clone());
        let dg_sync_token = dg_config.sync_token.clone();
        let dg_lumen_sub_id = identity.as_ref().and_then(|id| id.sub_id.clone());

        Self {
            api_token: load_or_create_api_token(),
            aggregator,
            proxy: RwLock::new(None),
            proxy_config: RwLock::new(ProxyConfig::default()),
            transparent_config: RwLock::new(TransparentConfig::default()),
            dg_config: RwLock::new(dg_config),
            dg_server_url: Arc::new(RwLock::new(dg_server_url)),
            dg_bearer_token: Arc::new(RwLock::new(None)),
            dg_identity: Arc::new(RwLock::new(identity)),
            dg_http_client: Arc::new(RwLock::new(http_client)),
            dg_sync_token: Arc::new(RwLock::new(dg_sync_token)),
            dg_lumen_sub_id: Arc::new(RwLock::new(dg_lumen_sub_id)),
            traffic_log: Arc::new(TrafficLog::new()),
            ca,
            cert_cache,
            shutdown_tx,
            shutdown_rx,
            sample_capture: Arc::new(SampleCapture::new(20)),
            dcr_flow: Arc::new(RwLock::new(None)),
            api_port: RwLock::new(9091),
            body_limits: Arc::new(RwLock::new(BodyLimits::default())),
        }
    }
}
