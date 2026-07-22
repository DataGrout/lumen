use anyhow::Result;
use chrono::Utc;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Method, Request, Response};
use hyper_util::rt::TokioIo;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info, warn};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

// Await a connection from an optional listener; if there is none (the IPv6
// loopback bind failed), pend forever so this branch never resolves in
// `select!` and only the IPv4 listener stays active.
async fn accept_optional(
    listener: &Option<TcpListener>,
) -> std::io::Result<(tokio::net::TcpStream, SocketAddr)> {
    match listener {
        Some(l) => l.accept().await,
        None => std::future::pending().await,
    }
}

fn box_body<B>(body: B) -> BoxBody
where
    B: hyper::body::Body<Data = Bytes, Error = std::convert::Infallible> + Send + Sync + 'static,
{
    BoxBody::new(body.map_err(|e| match e {}))
}

use crate::aggregator::Aggregator;
use crate::parser;
use crate::state::SampleCapture;
use crate::tls::CertCache;
use crate::traffic::{TrafficEntry, TrafficLog};

const DEFAULT_TARGETS: &[&str] = &[
    "api.openai.com",
    "api.anthropic.com",
    "a-api.anthropic.com", // Claude Desktop (some versions)
    "claude.ai",           // Claude Desktop web app
    "generativelanguage.googleapis.com",
];

const DEFAULT_MAX_BODY_BYTES: u64 = 50 * 1024 * 1024; // 50 MB

/// Per-direction body size limits.  `None` means unlimited.
///
/// `max_request_bytes` — requests exceeding this are rejected with 413 before
/// being forwarded upstream (avoids OOM from huge uploads).
///
/// `max_response_bytes` — response collection for analysis is capped at this
/// value; chunks are still forwarded to the client after the cap is reached,
/// so the upstream stream is never interrupted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BodyLimits {
    pub max_request_bytes: Option<u64>,
    pub max_response_bytes: Option<u64>,
}

impl Default for BodyLimits {
    fn default() -> Self {
        Self {
            max_request_bytes: Some(DEFAULT_MAX_BODY_BYTES),
            max_response_bytes: Some(DEFAULT_MAX_BODY_BYTES),
        }
    }
}

/// Suffix patterns — any host ending with these is monitored.
const DEFAULT_SUFFIX_TARGETS: &[&str] = &[".cursor.sh", ".cursorapi.com"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayRoute {
    pub prefix: String,
    pub upstream: String,
}

fn default_routes() -> HashMap<String, String> {
    let mut routes = HashMap::new();
    routes.insert("/openai".to_string(), "https://api.openai.com".to_string());
    routes.insert(
        "/anthropic".to_string(),
        "https://api.anthropic.com".to_string(),
    );
    routes.insert(
        "/google".to_string(),
        "https://generativelanguage.googleapis.com".to_string(),
    );
    routes
}

pub struct LumenProxy {
    aggregator: Arc<Aggregator>,
    traffic_log: Arc<TrafficLog>,
    cert_cache: Arc<CertCache>,
    http_client: reqwest::Client,
    port: u16,
    monitored_hosts: RwLock<HashSet<String>>,
    monitored_suffixes: RwLock<Vec<String>>,
    relay_routes: RwLock<HashMap<String, String>>,
    sample_capture: Arc<SampleCapture>,
    body_limits: Arc<RwLock<BodyLimits>>,
    /// Last successfully extracted model name from a Cursor request.
    /// Used as fallback when subsequent requests don't contain the model field.
    cursor_last_model: RwLock<Option<String>>,
}

/// Returns true when a Cursor call is large enough to represent real AI inference.
fn cursor_is_significant_call(path: &str, req_bytes: u64, resp_bytes: u64) -> bool {
    if path.contains("BidiAppend") {
        // BidiAppend with no response is Cursor's background context sync (uploads the
        // current codebase snapshot every ~1 second). These can be 100-200KB but produce
        // zero response bytes. Real AI completions always return a non-trivial response.
        resp_bytes > 50
    } else {
        req_bytes > 50 || resp_bytes > 200
    }
}

/// Returns true for Cursor paths that are telemetry, metadata, or other non-inference traffic.
fn is_cursor_noise(host: &str, path: &str) -> bool {
    // Pure metrics/telemetry hosts
    if host == "metrics.cursor.sh" {
        return true;
    }
    // Known non-inference gRPC methods and telemetry endpoints
    const NOISE: &[&str] = &[
        "/tev1/",
        "rgstr",
        "AvailableModels",
        "GetDefaultModelNudgeData",
        "AvailableDocs",
        "UpdateVscodeProfile",
        "ReportAgentSnapshot",
        "OnlineMetricsService",
        "DashboardService",
        "AnalyticsService",
        "ServerConfigService",
        "BackgroundComposerService",
        "RepositoryService",
        "ReportClient",
        "ReportProcess",
        "ReportAiCodeChangeMetrics",
        "/envelope/",
        "GetAccessToken",
        "GetAuthToken",
        "StreamingSearch",
        "ServerTime",
        "GetTeamRules",
        "GetPlanInfo",
        "GetCurrentPeriodUsage",
        "GetUsageLimitStatus",
        "GetGlassEarlyPreview",
        "/auth/",
        "/extensions-control",
    ];
    NOISE.iter().any(|n| path.contains(n))
}

impl LumenProxy {
    pub fn new(
        aggregator: Arc<Aggregator>,
        traffic_log: Arc<TrafficLog>,
        cert_cache: Arc<CertCache>,
        sample_capture: Arc<SampleCapture>,
        body_limits: Arc<RwLock<BodyLimits>>,
        port: u16,
    ) -> Self {
        let mut hosts = HashSet::new();
        for h in DEFAULT_TARGETS {
            hosts.insert(h.to_string());
        }

        let suffixes: Vec<String> = DEFAULT_SUFFIX_TARGETS
            .iter()
            .map(|s| s.to_string())
            .collect();

        let http_client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("failed to build HTTP client");

        Self {
            aggregator,
            traffic_log,
            cert_cache,
            http_client,
            port,
            monitored_hosts: RwLock::new(hosts),
            monitored_suffixes: RwLock::new(suffixes),
            relay_routes: RwLock::new(default_routes()),
            sample_capture,
            body_limits,
            cursor_last_model: RwLock::new(None),
        }
    }

    pub fn add_host(&self, host: &str) {
        self.monitored_hosts.write().insert(host.to_string());
    }

    pub fn remove_host(&self, host: &str) {
        self.monitored_hosts.write().remove(host);
    }

    pub fn list_hosts(&self) -> Vec<String> {
        self.monitored_hosts.read().iter().cloned().collect()
    }

    pub fn add_route(&self, prefix: &str, upstream: &str) {
        let prefix = if prefix.starts_with('/') {
            prefix.to_string()
        } else {
            format!("/{}", prefix)
        };
        self.relay_routes
            .write()
            .insert(prefix, upstream.to_string());
    }

    pub fn remove_route(&self, prefix: &str) {
        self.relay_routes.write().remove(prefix);
    }

    pub fn list_routes(&self) -> Vec<RelayRoute> {
        self.relay_routes
            .read()
            .iter()
            .map(|(prefix, upstream)| RelayRoute {
                prefix: prefix.clone(),
                upstream: upstream.clone(),
            })
            .collect()
    }

    fn is_monitored(&self, host: &str) -> bool {
        if self.monitored_hosts.read().contains(host) {
            return true;
        }
        let suffixes = self.monitored_suffixes.read();
        suffixes
            .iter()
            .any(|suffix| host.ends_with(suffix.as_str()))
    }

    /// Try to resolve a relay route from a relative path.
    /// Returns (upstream_url, upstream_host, remaining_path) if matched.
    fn resolve_relay(&self, path: &str) -> Option<(String, String, String)> {
        let routes = self.relay_routes.read();
        // Match longest prefix first
        let mut best: Option<(&str, &str)> = None;
        for (prefix, upstream) in routes.iter() {
            if path.starts_with(prefix.as_str())
                && (path.len() == prefix.len() || path.as_bytes().get(prefix.len()) == Some(&b'/'))
                && best.map_or(true, |(p, _)| prefix.len() > p.len())
            {
                best = Some((prefix.as_str(), upstream.as_str()));
            }
        }
        let (prefix, upstream) = best?;
        let remainder = &path[prefix.len()..];
        let upstream_url = format!("{}{}", upstream.trim_end_matches('/'), remainder);
        let host = upstream
            .strip_prefix("https://")
            .or_else(|| upstream.strip_prefix("http://"))
            .unwrap_or(upstream)
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        Some((upstream_url, host, remainder.to_string()))
    }

    pub async fn start(self: Arc<Self>) -> Result<()> {
        // Bind IPv4 loopback (required).
        let v4 = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = match TcpListener::bind(v4).await {
            Ok(l) => l,
            Err(e) => {
                // A failed bind here means the proxy is NOT running: clients
                // pointed at this port get "connection refused", which looks
                // exactly like Lumen being off. Make it unmissable — on Windows
                // the usual cause is a reserved/excluded port range (Hyper-V /
                // WSL2 reserve blocks of ports; bind then fails with WSAEACCES
                // even though nothing is "using" the port), or another process.
                error!(
                    "Lumen proxy FAILED to bind {} — the proxy is NOT listening. \
                     Clients will get 'connection refused' (this looks identical to \
                     Lumen being off). Cause: {e}. On Windows this is usually a \
                     reserved port range or a port already in use: check \
                     `netsh interface ipv4 show excludedportrange protocol=tcp` and \
                     `netstat -ano | findstr :{port}`, then either free the port or \
                     start Lumen with a different `--proxy-port`.",
                    v4,
                    port = self.port
                );
                return Err(e.into());
            }
        };
        info!("Lumen proxy listening on http://{}", v4);

        // Also accept on IPv6 loopback so clients that resolve `localhost` to
        // `::1` — the default on Windows — can reach the proxy. Relay base URLs
        // and HTTP-proxy settings are commonly written as `localhost`; without
        // the ::1 listener those requests hit `[::1]:port`, never arrive, and no
        // traffic is captured (the daemon looks up but sees nothing). Best-effort
        // and loopback-only, so it never widens exposure.
        let listener_v6 = match TcpListener::bind(SocketAddr::from((
            [0, 0, 0, 0, 0, 0, 0, 1],
            self.port,
        )))
        .await
        {
            Ok(l) => {
                info!("Lumen proxy also listening on http://[::1]:{}", self.port);
                Some(l)
            }
            Err(e) => {
                warn!(
                    "Lumen proxy: could not bind IPv6 loopback [::1]:{} ({}). If traffic \
                         isn't captured, point clients at 127.0.0.1:{} rather than localhost.",
                    self.port, e, self.port
                );
                None
            }
        };

        loop {
            let stream = tokio::select! {
                r = listener.accept() => match r {
                    Ok((s, _)) => s,
                    Err(e) => {
                        error!("Failed to accept connection: {}", e);
                        continue;
                    }
                },
                r = accept_optional(&listener_v6) => match r {
                    Ok((s, _)) => s,
                    Err(e) => {
                        error!("Failed to accept connection (v6): {}", e);
                        continue;
                    }
                },
            };

            let proxy = self.clone();

            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |req| {
                    let proxy = proxy.clone();
                    async move { proxy.handle_request(req).await }
                });

                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .preserve_header_case(true)
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await
                {
                    debug!("Connection closed: {}", e);
                }
            });
        }
    }

    async fn handle_request(
        self: Arc<Self>,
        req: Request<Incoming>,
    ) -> Result<Response<BoxBody>, hyper::Error> {
        if req.method() == Method::CONNECT {
            return self.handle_connect(req).await;
        }

        let start = std::time::Instant::now();
        let uri = req.uri().clone();
        let is_relay = uri.scheme().is_none() && uri.host().is_none();

        let (url_str, host, path, is_monitored) = if is_relay {
            let raw_path = uri.path().to_string();
            if let Some((upstream_url, upstream_host, remainder)) = self.resolve_relay(&raw_path) {
                let monitored = self.is_monitored(&upstream_host);
                info!(
                    "Relay: {} -> {} (monitored: {})",
                    raw_path, upstream_url, monitored
                );
                (upstream_url, upstream_host, remainder, monitored)
            } else {
                return Ok(Response::builder()
                    .status(404)
                    .body(box_body(Full::new(Bytes::from(
                        "{\"error\": \"no relay route matched\", \"hint\": \"Use path prefixes like /openai/v1/chat/completions. See GET /routes for available prefixes.\"}",
                    ))))
                    .unwrap());
            }
        } else {
            let host = uri.host().unwrap_or("").to_string();
            let path = uri.path().to_string();
            let url_str = uri.to_string();
            let is_monitored = self.is_monitored(&host);
            (url_str, host, path, is_monitored)
        };

        let (parts, body) = req.into_parts();
        let method_string = parts.method.to_string();
        let body_bytes = body
            .collect()
            .await
            .map(|c| c.to_bytes())
            .unwrap_or_default();
        let request_bytes = body_bytes.len() as u64;

        if let Some(max) = self.body_limits.read().max_request_bytes {
            if request_bytes > max {
                warn!(
                    "Rejecting oversized request: {} bytes > {} limit for {}",
                    request_bytes, max, host
                );
                return Ok(Response::builder()
                    .status(413)
                    .header("content-type", "application/json")
                    .body(box_body(Full::new(Bytes::from(format!(
                        "{{\"error\":\"request body too large\",\"limit_bytes\":{max},\"received_bytes\":{request_bytes}}}"
                    )))))
                    .unwrap());
            }
        }

        let model = if is_monitored {
            parser::detect_provider(&url_str)
                .and_then(|p| parser::extract_model(p, &String::from_utf8_lossy(&body_bytes)))
        } else {
            None
        };

        let method_str = parts.method.as_str();
        let mut builder = match method_str {
            "POST" => self.http_client.post(&url_str),
            "GET" => self.http_client.get(&url_str),
            "PUT" => self.http_client.put(&url_str),
            "DELETE" => self.http_client.delete(&url_str),
            "PATCH" => self.http_client.patch(&url_str),
            _ => self.http_client.get(&url_str),
        };

        for (name, value) in parts.headers.iter() {
            if is_relay && name == "host" {
                continue;
            }
            // Prevent compressed responses: reqwest has no gzip/br feature, so
            // compressed bytes would be forwarded raw and break SSE parsing.
            // Apply to all monitored hosts (not just relay) — Chromium sends
            // accept-encoding: gzip, br by default in MITM mode.
            if (is_relay || is_monitored) && name == "accept-encoding" {
                continue;
            }
            if let Ok(v) = value.to_str() {
                builder = builder.header(name.as_str(), v);
            }
        }

        if is_relay {
            builder = builder.header("host", &host);
        }
        if is_relay || is_monitored {
            builder = builder.header("accept-encoding", "identity");
        }

        if !body_bytes.is_empty() {
            builder = builder.body(body_bytes.to_vec());
        }

        let max_resp_bytes = self.body_limits.read().max_response_bytes;

        match builder.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let resp_headers = resp.headers().clone();

                let (tx, rx) = tokio::sync::mpsc::channel::<
                    Result<Frame<Bytes>, std::convert::Infallible>,
                >(32);

                let proxy = self.clone();
                let host_owned = host.clone();
                let path_owned = path.clone();
                let url_owned = url_str.clone();
                let method_owned = method_string.clone();
                let model_owned = model.clone();

                tokio::spawn(async move {
                    let mut collected = Vec::new();
                    let mut stream = resp.bytes_stream();
                    // Populated as soon as message_delta arrives so events appear mid-stream.
                    let mut early_caps: Option<Vec<String>> = None;

                    while let Some(chunk_result) = stream.next().await {
                        match chunk_result {
                            Ok(chunk) => {
                                if max_resp_bytes.map_or(true, |max| (collected.len() as u64) < max)
                                {
                                    collected.extend_from_slice(&chunk);
                                }
                                // Record usage as soon as message_delta arrives rather than
                                // waiting for the full stream to close.
                                if early_caps.is_none()
                                    && is_monitored
                                    && (200..300).contains(&(status as usize))
                                    && chunk
                                        .windows(b"message_delta".len())
                                        .any(|w| w == b"message_delta")
                                {
                                    if let Some(provider) = parser::detect_provider(&url_owned) {
                                        let body_str = String::from_utf8_lossy(&collected);
                                        if let Ok(usage) =
                                            parser::extract_usage(provider, &body_str)
                                        {
                                            let mut caps = Vec::new();
                                            if usage.input_tokens > 0 {
                                                caps.push("tokens_in".to_string());
                                            }
                                            if usage.output_tokens > 0 {
                                                caps.push("tokens_out".to_string());
                                            }
                                            if usage.cache_read_tokens.unwrap_or(0) > 0
                                                || usage.cache_creation_tokens.unwrap_or(0) > 0
                                            {
                                                caps.push("cache".to_string());
                                            }
                                            caps.push("cost".to_string());
                                            let m = model_owned.as_deref().unwrap_or("unknown");
                                            proxy
                                                .aggregator
                                                .record_usage(provider, m, &url_owned, usage);
                                            early_caps = Some(caps);
                                        }
                                    }
                                }
                                if tx.send(Ok(Frame::data(chunk))).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                warn!("Stream read error from {}: {}", host_owned, e);
                                break;
                            }
                        }
                    }
                    drop(tx);

                    let resp_body = Bytes::from(collected);
                    let response_bytes = resp_body.len() as u64;
                    let latency_ms = start.elapsed().as_millis() as u64;
                    let data_captured;

                    if let Some(caps) = early_caps {
                        data_captured = caps;
                    } else {
                        let mut caps = Vec::new();
                        if is_monitored && (200..300).contains(&(status as usize)) {
                            if let Some(provider) = parser::detect_provider(&url_owned) {
                                let body_str = String::from_utf8_lossy(&resp_body);
                                if let Ok(usage) = parser::extract_usage(provider, &body_str) {
                                    if usage.input_tokens > 0 {
                                        caps.push("tokens_in".to_string());
                                    }
                                    if usage.output_tokens > 0 {
                                        caps.push("tokens_out".to_string());
                                    }
                                    if usage.cache_read_tokens.unwrap_or(0) > 0
                                        || usage.cache_creation_tokens.unwrap_or(0) > 0
                                    {
                                        caps.push("cache".to_string());
                                    }
                                    caps.push("cost".to_string());
                                    let m = model_owned.as_deref().unwrap_or("unknown");
                                    proxy
                                        .aggregator
                                        .record_usage(provider, m, &url_owned, usage);
                                }
                            }
                        }
                        data_captured = caps;
                    }

                    proxy.traffic_log.record(TrafficEntry {
                        id: uuid::Uuid::new_v4().to_string(),
                        timestamp: Utc::now(),
                        host: host_owned,
                        method: method_owned,
                        path: path_owned,
                        status,
                        request_bytes,
                        response_bytes,
                        is_monitored,
                        data_captured,
                        latency_ms,
                    });
                });

                let stream_body = StreamBody::new(ReceiverStream::new(rx));
                let mut response = Response::builder().status(status);
                for (name, value) in resp_headers.iter() {
                    if name == "content-length" {
                        continue;
                    }
                    response = response.header(name, value);
                }
                Ok(response
                    .body(box_body(stream_body))
                    .unwrap_or_else(|_| Response::new(box_body(Full::new(Bytes::new())))))
            }
            Err(e) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                self.traffic_log.record(TrafficEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    host: host.clone(),
                    method: method_string,
                    path,
                    status: 502,
                    request_bytes,
                    response_bytes: 0,
                    is_monitored,
                    data_captured: vec![],
                    latency_ms,
                });

                warn!("Upstream request failed: {}", e);
                Ok(Response::builder()
                    .status(502)
                    .body(box_body(Full::new(Bytes::from(format!(
                        "Lumen proxy error: {}",
                        e
                    )))))
                    .unwrap())
            }
        }
    }

    async fn handle_connect(
        self: Arc<Self>,
        req: Request<Incoming>,
    ) -> Result<Response<BoxBody>, hyper::Error> {
        let addr = req
            .uri()
            .authority()
            .map(|a| a.to_string())
            .unwrap_or_default();
        let host = addr.split(':').next().unwrap_or("").to_string();
        let is_monitored = self.is_monitored(&host);

        debug!("CONNECT {} (monitored: {})", addr, is_monitored);

        self.traffic_log.record(TrafficEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            host: host.clone(),
            method: "CONNECT".to_string(),
            path: addr.clone(),
            status: 200,
            request_bytes: 0,
            response_bytes: 0,
            is_monitored,
            data_captured: if is_monitored {
                vec!["mitm".to_string()]
            } else {
                vec!["tunnel".to_string()]
            },
            latency_ms: 0,
        });

        if is_monitored {
            return self.handle_connect_mitm(req, addr, host).await;
        }

        // Non-monitored: opaque TCP pass-through
        tokio::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    let mut upgraded = TokioIo::new(upgraded);
                    match tokio::net::TcpStream::connect(&addr).await {
                        Ok(mut server) => {
                            let _ = tokio::io::copy_bidirectional(&mut upgraded, &mut server).await;
                        }
                        Err(e) => {
                            warn!("CONNECT tunnel to {} failed: {}", addr, e);
                        }
                    }
                }
                Err(e) => {
                    warn!("CONNECT upgrade failed: {}", e);
                }
            }
        });

        Ok(Response::new(box_body(Full::new(Bytes::new()))))
    }

    async fn handle_connect_mitm(
        self: Arc<Self>,
        req: Request<Incoming>,
        addr: String,
        host: String,
    ) -> Result<Response<BoxBody>, hyper::Error> {
        let cert_cache = self.cert_cache.clone();

        tokio::spawn(async move {
            // Cert issuance is CPU-bound crypto (keygen + signing). When the OS
            // system proxy is enabled every host reroutes through the cold MITM
            // at once, so running this inline on the async workers starves the
            // runtime and stalls all connections. Offload to the blocking pool.
            let cc = cert_cache.clone();
            let host_for_cert = host.clone();
            let tls_config =
                match tokio::task::spawn_blocking(move || cc.get_or_create(&host_for_cert)).await {
                    Ok(Ok(config)) => config,
                    Ok(Err(e)) => {
                        error!("Failed to create TLS config for {}: {}", host, e);
                        return;
                    }
                    Err(e) => {
                        error!("Cert task join error for {}: {}", host, e);
                        return;
                    }
                };

            let upgraded = match hyper::upgrade::on(req).await {
                Ok(u) => u,
                Err(e) => {
                    warn!("CONNECT MITM upgrade failed for {}: {}", host, e);
                    return;
                }
            };

            let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
            let tls_stream = match acceptor.accept(TokioIo::new(upgraded)).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(
                        "TLS handshake failed for {} (client may not trust CA): {}",
                        host, e
                    );
                    self.traffic_log.record(TrafficEntry {
                        id: uuid::Uuid::new_v4().to_string(),
                        timestamp: Utc::now(),
                        host: host.clone(),
                        method: "CONNECT".to_string(),
                        path: format!("TLS handshake failed: {}", e),
                        status: 525,
                        request_bytes: 0,
                        response_bytes: 0,
                        is_monitored: true,
                        data_captured: vec!["tls_error".to_string()],
                        latency_ms: 0,
                    });
                    return;
                }
            };

            debug!("MITM TLS established for {}", host);

            let proxy = self.clone();
            let host_for_service = host.clone();
            let addr_for_service = addr.clone();

            let service = service_fn(move |req: Request<Incoming>| {
                let proxy = proxy.clone();
                let host = host_for_service.clone();
                let addr = addr_for_service.clone();
                async move { proxy.handle_mitm_request(req, &host, &addr).await }
            });

            let io = TokioIo::new(tls_stream);
            if let Err(e) =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(io, service)
                    .await
            {
                warn!("MITM connection to {} closed with error: {}", host, e);
            }
        });

        Ok(Response::new(box_body(Full::new(Bytes::new()))))
    }

    /// Handle a decrypted HTTP request from a MITM'd CONNECT tunnel.
    ///
    /// Streams the response through to the client in real time while
    /// capturing bytes for post-hoc usage analysis. This avoids buffering
    /// the full response (which would break streaming AI inference).
    async fn handle_mitm_request(
        self: Arc<Self>,
        req: Request<Incoming>,
        upstream_host: &str,
        upstream_addr: &str,
    ) -> Result<Response<BoxBody>, hyper::Error> {
        let start = std::time::Instant::now();
        let path = req.uri().path().to_string();
        let path_and_query = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| path.clone());
        let upstream_url = format!("https://{}{}", upstream_host, path_and_query);

        let (parts, body) = req.into_parts();
        let method_string = parts.method.to_string();
        let body_bytes = body
            .collect()
            .await
            .map(|c| c.to_bytes())
            .unwrap_or_default();
        let request_bytes = body_bytes.len() as u64;

        if let Some(max) = self.body_limits.read().max_request_bytes {
            if request_bytes > max {
                warn!(
                    "Rejecting oversized MITM request: {} bytes > {} limit for {}",
                    request_bytes, max, upstream_host
                );
                return Ok(Response::builder()
                    .status(413)
                    .header("content-type", "application/json")
                    .body(box_body(Full::new(Bytes::from(format!(
                        "{{\"error\":\"request body too large\",\"limit_bytes\":{max},\"received_bytes\":{request_bytes}}}"
                    )))))
                    .unwrap());
            }
        }

        let provider = parser::detect_provider(&upstream_url);
        let request_body_str = String::from_utf8_lossy(&body_bytes);
        let model = provider.and_then(|p| {
            parser::extract_model(p, &request_body_str).or_else(|| {
                // Cursor sends binary protobuf in BidiAppend requests; scan the raw
                // request bytes directly since JSON/SSE parsing yields nothing.
                // Skip noise paths — telemetry bodies contain feature-flag names that
                // look like model identifiers and would poison cursor_last_model.
                if p == parser::LLMProvider::Cursor && !is_cursor_noise(&upstream_url, &path) {
                    let found = parser::scan_bytes_for_model(&body_bytes);
                    if let Some(ref m) = found {
                        *self.cursor_last_model.write() = Some(m.clone());
                    }
                    found
                } else {
                    None
                }
            })
        });

        // Log every MITM request at INFO so we can see what paths flow through the handler
        info!(
            "MITM {} {} {} ({}B req)",
            upstream_host, method_string, path, request_bytes
        );

        if provider == Some(parser::LLMProvider::Cursor)
            && request_bytes > 40
            && (path.contains("BidiAppend") || path.contains("AgentService"))
        {
            let req_hex = hex::encode(&body_bytes[..body_bytes.len().min(160)]);
            debug!(
                "CURSOR req_hex {} [{}B] first 160B: {}",
                path, request_bytes, req_hex
            );
        }

        if provider == Some(parser::LLMProvider::Cursor) && !body_bytes.is_empty() {
            let preview_len =
                floor_char_boundary(&request_body_str, request_body_str.len().min(500));
            debug!(
                "CURSOR REQ {} {} body[..{}]: {}",
                method_string,
                path,
                preview_len,
                &request_body_str[..preview_len]
            );

            // AnalyticsService/Batch contains plain-text proto fields with the active
            // Cursor model (e.g. "model: composer-2", "currentModelSelected: composer-2").
            // Extract it here so cursor_last_model stays current even though analytics
            // requests are skipped as noise during response processing.
            if path.contains("AnalyticsService") {
                if let Some(m) = parser::scan_bytes_for_model(&body_bytes) {
                    info!("CURSOR model from analytics: {}", m);
                    *self.cursor_last_model.write() = Some(m);
                }
            }
        }

        let method_str = parts.method.as_str();
        let mut builder = match method_str {
            "POST" => self.http_client.post(&upstream_url),
            "GET" => self.http_client.get(&upstream_url),
            "PUT" => self.http_client.put(&upstream_url),
            "DELETE" => self.http_client.delete(&upstream_url),
            "PATCH" => self.http_client.patch(&upstream_url),
            _ => self.http_client.get(&upstream_url),
        };

        for (name, value) in parts.headers.iter() {
            if name == "host" || name == "accept-encoding" {
                continue;
            }
            if let Ok(v) = value.to_str() {
                builder = builder.header(name.as_str(), v);
            }
        }
        builder = builder.header("host", upstream_host);
        builder = builder.header("accept-encoding", "identity");

        if !body_bytes.is_empty() {
            builder = builder.body(body_bytes.to_vec());
        }

        let max_resp_bytes_mitm = self.body_limits.read().max_response_bytes;

        match builder.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let resp_headers = resp.headers().clone();
                let resp_headers_for_task = resp_headers.clone();

                // Stream the response body through a channel so the client
                // receives chunks in real time instead of waiting for the
                // full body to be buffered.
                let (tx, rx) = tokio::sync::mpsc::channel::<
                    Result<Frame<Bytes>, std::convert::Infallible>,
                >(32);

                let proxy = self.clone();
                let host_owned = upstream_host.to_string();
                let addr_owned = upstream_addr.to_string();
                let path_owned = path.clone();
                let url_owned = upstream_url.clone();
                let method_owned = method_string.clone();
                let model_owned = model.clone();
                let req_body_str_owned = request_body_str.to_string();
                let req_bytes_owned = body_bytes.clone();

                tokio::spawn(async move {
                    let mut collected = Vec::new();
                    let mut stream = resp.bytes_stream();
                    let mut early_caps: Option<Vec<String>> = None;

                    while let Some(chunk_result) = stream.next().await {
                        match chunk_result {
                            Ok(chunk) => {
                                if max_resp_bytes_mitm
                                    .map_or(true, |max| (collected.len() as u64) < max)
                                {
                                    collected.extend_from_slice(&chunk);
                                }
                                // Record non-Cursor streaming usage as soon as message_delta
                                // arrives so events appear mid-stream without waiting for close.
                                if early_caps.is_none()
                                    && provider.map_or(false, |p| p != parser::LLMProvider::Cursor)
                                    && (200..300).contains(&(status as usize))
                                    && chunk
                                        .windows(b"message_delta".len())
                                        .any(|w| w == b"message_delta")
                                {
                                    if let Some(p) = provider {
                                        let body_str = String::from_utf8_lossy(&collected);
                                        if let Ok(usage) = parser::extract_usage(p, &body_str) {
                                            let mut caps = Vec::new();
                                            if usage.input_tokens > 0 {
                                                caps.push("tokens_in".to_string());
                                            }
                                            if usage.output_tokens > 0 {
                                                caps.push("tokens_out".to_string());
                                            }
                                            if usage.cache_read_tokens.unwrap_or(0) > 0
                                                || usage.cache_creation_tokens.unwrap_or(0) > 0
                                            {
                                                caps.push("cache".to_string());
                                            }
                                            caps.push("cost".to_string());
                                            let m = model_owned.as_deref().unwrap_or("unknown");
                                            proxy.aggregator.record_usage(p, m, &url_owned, usage);
                                            early_caps = Some(caps);
                                        }
                                    }
                                }
                                if tx.send(Ok(Frame::data(chunk))).await.is_err() {
                                    break; // client disconnected
                                }
                            }
                            Err(e) => {
                                warn!("Stream read error from {}: {}", addr_owned, e);
                                break;
                            }
                        }
                    }
                    drop(tx);

                    let resp_body = Bytes::from(collected);
                    let response_bytes = resp_body.len() as u64;
                    let latency_ms = start.elapsed().as_millis() as u64;
                    let data_captured = if let Some(caps) = early_caps {
                        // Usage was already recorded when message_delta arrived mid-stream.
                        caps
                    } else {
                        let mut caps = Vec::new();
                        if let Some(provider) = provider {
                            if (200..300).contains(&(status as usize)) {
                                let body_str = String::from_utf8_lossy(&resp_body);

                                if provider == parser::LLMProvider::Cursor {
                                    // Debug-only hex dump for Cursor endpoints.
                                    if tracing::enabled!(tracing::Level::DEBUG) {
                                        let req_hex = hex::encode(
                                            &req_bytes_owned[..req_bytes_owned.len().min(160)],
                                        );
                                        let resp_hex =
                                            hex::encode(&resp_body[..resp_body.len().min(160)]);
                                        let safe_len = floor_char_boundary(&body_str, 400);
                                        debug!(
                                            "CURSOR payload {} [req {}B resp {}B]\n  req_hex[..160]: {}\n  resp_hex[..160]: {}\n  resp_text: {}",
                                            path_owned,
                                            req_bytes_owned.len(),
                                            resp_body.len(),
                                            req_hex,
                                            resp_hex,
                                            &body_str[..safe_len]
                                        );
                                    }

                                    // Intercept GetDefaultModel (exact match, not NudgeData) to
                                    // cache the active model. NudgeData lists all available models
                                    // and must not overwrite the user's actual selection.
                                    if path_owned.ends_with("GetDefaultModel") {
                                        let found = parser::extract_model_generic(&body_str)
                                            .or_else(|| parser::scan_bytes_for_model(&resp_body));
                                        if let Some(m) = found {
                                            info!("Cursor default model cached: {}", m);
                                            *proxy.cursor_last_model.write() = Some(m);
                                        } else {
                                            debug!("Cursor GetDefaultModel: no model found in {}B response", resp_body.len());
                                        }
                                    }
                                }

                                if proxy.sample_capture.is_armed()
                                    && (provider == parser::LLMProvider::Cursor
                                        || provider == parser::LLMProvider::Anthropic)
                                {
                                    let req_preview_len = req_body_str_owned.len().min(4096);
                                    let resp_preview_len = body_str.len().min(8192);
                                    let req_hex_len = req_bytes_owned.len().min(4096);
                                    let resp_hex_len = resp_body.len().min(10240);

                                    let content_type = resp_headers_for_task
                                        .get("content-type")
                                        .and_then(|v| v.to_str().ok())
                                        .map(String::from);

                                    let safe_req =
                                        floor_char_boundary(&req_body_str_owned, req_preview_len);
                                    let safe_resp =
                                        floor_char_boundary(&body_str, resp_preview_len);

                                    proxy.sample_capture.push(crate::state::PayloadSample {
                                        timestamp: Utc::now().to_rfc3339(),
                                        host: host_owned.clone(),
                                        path: path_owned.clone(),
                                        method: method_owned.clone(),
                                        content_type,
                                        request_preview: req_body_str_owned[..safe_req].to_string(),
                                        response_preview: body_str[..safe_resp].to_string(),
                                        request_hex: hex::encode(&req_bytes_owned[..req_hex_len]),
                                        response_hex: hex::encode(&resp_body[..resp_hex_len]),
                                        request_bytes: req_bytes_owned.len(),
                                        response_bytes: resp_body.len(),
                                    });
                                }

                                // claude.ai web API uses different SSE format than api.anthropic.com;
                                // only treat as inference if the path ends in a completion endpoint.
                                // Explicitly exclude uploads, telemetry, and other non-inference POSTs.
                                let is_claude_ai_inference = provider
                                    == parser::LLMProvider::Anthropic
                                    && host_owned.contains("claude.ai")
                                    && method_owned == "POST"
                                    && (path_owned.ends_with("/completion")
                                        || path_owned.ends_with("/completions"));

                                let is_cursor_ai_call = provider == parser::LLMProvider::Cursor
                                    && !is_cursor_noise(&host_owned, &path_owned)
                                    && cursor_is_significant_call(
                                        &path_owned,
                                        request_bytes,
                                        response_bytes,
                                    );

                                let usage = parser::extract_usage(provider, &body_str).ok().or_else(|| {
                                    if is_cursor_ai_call {
                                        let estimated = parser::estimate_usage_from_bytes(request_bytes, response_bytes);
                                        info!(
                                            "Cursor AI byte estimate: ~{}in/~{}out tokens ({} req/{} resp bytes) path={}",
                                            estimated.input_tokens, estimated.output_tokens,
                                            request_bytes, response_bytes, path_owned
                                        );
                                        Some(estimated)
                                    } else if provider == parser::LLMProvider::Cursor {
                                        debug!("Skipping byte estimate for non-AI path: {}", path_owned);
                                        None
                                    } else if is_claude_ai_inference {
                                        let estimated = parser::estimate_usage_from_bytes(request_bytes, response_bytes);
                                        info!(
                                            "claude.ai byte estimate: ~{}in/~{}out tokens ({} req/{} resp bytes) path={}",
                                            estimated.input_tokens, estimated.output_tokens,
                                            request_bytes, response_bytes, path_owned
                                        );
                                        Some(estimated)
                                    } else {
                                        None
                                    }
                                });

                                if let Some(usage) = usage {
                                    if usage.input_tokens > 0 {
                                        caps.push("tokens_in".to_string());
                                    }
                                    if usage.output_tokens > 0 {
                                        caps.push("tokens_out".to_string());
                                    }
                                    if usage.cache_read_tokens.unwrap_or(0) > 0
                                        || usage.cache_creation_tokens.unwrap_or(0) > 0
                                    {
                                        caps.push("cache".to_string());
                                    }
                                    caps.push("cost".to_string());
                                    if (is_cursor_ai_call || is_claude_ai_inference)
                                        && parser::extract_usage(provider, &body_str).is_err()
                                    {
                                        caps.push("estimated".to_string());
                                    }
                                    let final_model = if let Some(m) = model_owned.as_deref() {
                                        let m = m.to_string();
                                        if provider == parser::LLMProvider::Cursor {
                                            *proxy.cursor_last_model.write() = Some(m.clone());
                                        }
                                        m
                                    } else if provider == parser::LLMProvider::Cursor {
                                        // Try SSE/JSON text parsing first (AgentService/RunSSE
                                        // returns text SSE, not binary), then fall back to byte
                                        // scan for binary protobuf responses.
                                        let resp_str = String::from_utf8_lossy(&resp_body);
                                        let found = parser::extract_model_generic(&resp_str)
                                            .or_else(|| parser::scan_bytes_for_model(&resp_body));
                                        if let Some(m) = found {
                                            *proxy.cursor_last_model.write() = Some(m.clone());
                                            m
                                        } else {
                                            proxy
                                                .cursor_last_model
                                                .read()
                                                .clone()
                                                .unwrap_or_else(|| "cursor-unknown".to_string())
                                        }
                                    } else if is_claude_ai_inference {
                                        // Try SSE/JSON field first, then raw byte pattern scan
                                        parser::extract_model_generic(&body_str)
                                            .or_else(|| parser::scan_bytes_for_model(&resp_body))
                                            .unwrap_or_else(|| "claude-unknown".to_string())
                                    } else {
                                        "unknown".to_string()
                                    };
                                    proxy.aggregator.record_usage(
                                        provider,
                                        &final_model,
                                        &url_owned,
                                        usage,
                                    );
                                }
                            }
                        }
                        caps
                    };

                    proxy.traffic_log.record(TrafficEntry {
                        id: uuid::Uuid::new_v4().to_string(),
                        timestamp: Utc::now(),
                        host: host_owned,
                        method: method_owned,
                        path: path_owned,
                        status,
                        request_bytes,
                        response_bytes,
                        is_monitored: true,
                        data_captured,
                        latency_ms,
                    });
                });

                let stream_body = StreamBody::new(ReceiverStream::new(rx));
                let boxed: BoxBody = box_body(stream_body);

                let mut response = Response::builder().status(status);
                for (name, value) in resp_headers.iter() {
                    if name == "content-length" {
                        continue; // don't forward content-length for streamed responses
                    }
                    response = response.header(name, value);
                }
                Ok(response
                    .body(boxed)
                    .unwrap_or_else(|_| Response::new(box_body(Full::new(Bytes::new())))))
            }
            Err(e) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                self.traffic_log.record(TrafficEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    host: upstream_host.to_string(),
                    method: method_string,
                    path,
                    status: 502,
                    request_bytes,
                    response_bytes: 0,
                    is_monitored: true,
                    data_captured: vec![],
                    latency_ms,
                });

                warn!("MITM upstream request to {} failed: {}", upstream_addr, e);
                Ok(Response::builder()
                    .status(502)
                    .body(box_body(Full::new(Bytes::from(format!(
                        "Lumen proxy error: {}",
                        e
                    )))))
                    .unwrap())
            }
        }
    }
}

/// Find the largest valid char boundary at or before `index`.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ca::LumenCA;
    use crate::pricing::PricingDatabase;
    use std::sync::Once;

    // reqwest with `rustls-tls-no-provider` requires a crypto provider to be
    // installed before Client::builder() is called. Once is process-global, so
    // this runs at most once across all tests in this binary.
    static CRYPTO_INIT: Once = Once::new();
    fn ensure_crypto_installed() {
        CRYPTO_INIT.call_once(crate::install_crypto_provider);
    }

    fn test_proxy() -> LumenProxy {
        ensure_crypto_installed();
        let agg = Arc::new(Aggregator::new(PricingDatabase::with_defaults()));
        let tl = Arc::new(TrafficLog::new());
        let ca = Arc::new(LumenCA::generate_ephemeral().unwrap());
        let cc = Arc::new(CertCache::new(ca));
        let sc = Arc::new(SampleCapture::new(5));
        let bl = Arc::new(RwLock::new(BodyLimits::default()));
        LumenProxy::new(agg, tl, cc, sc, bl, 0)
    }

    #[test]
    fn test_resolve_openai_route() {
        let proxy = test_proxy();
        let result = proxy.resolve_relay("/openai/v1/chat/completions");
        assert!(result.is_some());
        let (url, host, remainder) = result.unwrap();
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(host, "api.openai.com");
        assert_eq!(remainder, "/v1/chat/completions");
    }

    #[test]
    fn test_resolve_anthropic_route() {
        let proxy = test_proxy();
        let result = proxy.resolve_relay("/anthropic/v1/messages");
        assert!(result.is_some());
        let (url, host, _) = result.unwrap();
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert_eq!(host, "api.anthropic.com");
    }

    #[test]
    fn test_resolve_google_route() {
        let proxy = test_proxy();
        let result = proxy.resolve_relay("/google/v1beta/models/gemini-pro:generateContent");
        assert!(result.is_some());
        let (url, host, _) = result.unwrap();
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:generateContent"
        );
        assert_eq!(host, "generativelanguage.googleapis.com");
    }

    #[test]
    fn test_resolve_no_match() {
        let proxy = test_proxy();
        assert!(proxy.resolve_relay("/unknown/v1/foo").is_none());
    }

    #[test]
    fn test_resolve_exact_prefix() {
        let proxy = test_proxy();
        let result = proxy.resolve_relay("/openai");
        assert!(result.is_some());
        let (url, _, _) = result.unwrap();
        assert_eq!(url, "https://api.openai.com");
    }

    #[test]
    fn test_custom_route() {
        let proxy = test_proxy();
        proxy.add_route("/local", "http://localhost:11434");
        let result = proxy.resolve_relay("/local/api/generate");
        assert!(result.is_some());
        let (url, host, _) = result.unwrap();
        assert_eq!(url, "http://localhost:11434/api/generate");
        assert_eq!(host, "localhost:11434");
    }

    #[test]
    fn test_remove_route() {
        let proxy = test_proxy();
        assert!(proxy.resolve_relay("/openai/v1/models").is_some());
        proxy.remove_route("/openai");
        assert!(proxy.resolve_relay("/openai/v1/models").is_none());
    }

    #[test]
    fn test_list_routes() {
        let proxy = test_proxy();
        let routes = proxy.list_routes();
        assert_eq!(routes.len(), 3);
        assert!(routes.iter().any(|r| r.prefix == "/openai"));
        assert!(routes.iter().any(|r| r.prefix == "/anthropic"));
        assert!(routes.iter().any(|r| r.prefix == "/google"));
    }

    #[test]
    fn test_prefix_boundary() {
        let proxy = test_proxy();
        // "/openaiextended" should NOT match "/openai"
        assert!(proxy.resolve_relay("/openaiextended/foo").is_none());
    }

    #[test]
    fn test_cursor_is_significant_call() {
        // BidiAppend background sync: large request, zero response -> not significant
        assert!(!cursor_is_significant_call(
            "/BidiService/BidiAppend",
            195050,
            0
        ));
        // BidiAppend small heartbeat, zero response -> not significant
        assert!(!cursor_is_significant_call(
            "/BidiService/BidiAppend",
            80,
            0
        ));
        // BidiAppend with a real response -> significant
        assert!(cursor_is_significant_call(
            "/BidiService/BidiAppend",
            100,
            200
        ));
        // Non-BidiAppend path with small request -> not significant
        assert!(!cursor_is_significant_call("/AgentService/Run", 30, 0));
        // Non-BidiAppend path with meaningful request -> significant
        assert!(cursor_is_significant_call("/AgentService/Run", 200, 0));
        // Non-BidiAppend path with meaningful response -> significant
        assert!(cursor_is_significant_call("/AgentService/Run", 10, 300));
    }

    #[test]
    fn test_cursor_is_noise() {
        assert!(is_cursor_noise("metrics.cursor.sh", "/anything"));
        assert!(is_cursor_noise("api2.cursor.sh", "/AnalyticsService/Batch"));
        assert!(is_cursor_noise(
            "api2.cursor.sh",
            "/ReportAiCodeChangeMetrics"
        ));
        assert!(is_cursor_noise("api2.cursor.sh", "/tev1/track"));
        assert!(!is_cursor_noise(
            "api2.cursor.sh",
            "/aiserver.v1.AiService/RunSSE"
        ));
        assert!(!is_cursor_noise(
            "api2.cursor.sh",
            "/BidiService/BidiAppend"
        ));
    }

    #[test]
    fn test_cursor_suffix_monitoring() {
        let proxy = test_proxy();
        assert!(proxy.is_monitored("api2.cursor.sh"));
        assert!(proxy.is_monitored("api3.cursor.sh"));
        assert!(proxy.is_monitored("us-east.api5.cursor.sh"));
        assert!(proxy.is_monitored("repo42.cursor.sh"));
        assert!(proxy.is_monitored("marketplace.cursorapi.com"));
        assert!(!proxy.is_monitored("cursor.sh")); // no leading dot
        assert!(!proxy.is_monitored("evil-cursor.sh"));
    }

    #[test]
    fn test_exact_and_suffix_monitoring() {
        let proxy = test_proxy();
        // Exact matches
        assert!(proxy.is_monitored("api.openai.com"));
        assert!(proxy.is_monitored("api.anthropic.com"));
        // Suffix matches
        assert!(proxy.is_monitored("api2.cursor.sh"));
        // Not monitored
        assert!(!proxy.is_monitored("example.com"));
    }
}
