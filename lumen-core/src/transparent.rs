//! Transparent proxy listener for pf-redirected connections.
//!
//! Accepts raw TCP connections redirected by macOS pf, recovers the original
//! destination via DIOCNATLOOK, performs TLS MITM for monitored hosts, and
//! relays traffic while extracting LLM usage data.
//!
//! This listener runs on a separate port (default 9443) from the explicit
//! HTTP proxy (9090). It does NOT expect CONNECT requests — connections arrive
//! as raw TCP to port 443 destinations that pf redirected here.

use anyhow::Result;
use chrono::Utc;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info, warn};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

fn box_body<B>(body: B) -> BoxBody
where
    B: hyper::body::Body<Data = Bytes, Error = std::convert::Infallible> + Send + Sync + 'static,
{
    BoxBody::new(body.map_err(|e| match e {}))
}

use crate::aggregator::Aggregator;
use crate::nat_lookup::{self, NatHandle};
use crate::parser;
use crate::proxy::BodyLimits;
use crate::state::SampleCapture;
use crate::tls::CertCache;
use crate::traffic::{TrafficEntry, TrafficLog};

pub struct TransparentProxy {
    aggregator: Arc<Aggregator>,
    traffic_log: Arc<TrafficLog>,
    cert_cache: Arc<CertCache>,
    http_client: reqwest::Client,
    sample_capture: Arc<SampleCapture>,
    body_limits: Arc<parking_lot::RwLock<BodyLimits>>,
    port: u16,
}

impl TransparentProxy {
    pub fn new(
        aggregator: Arc<Aggregator>,
        traffic_log: Arc<TrafficLog>,
        cert_cache: Arc<CertCache>,
        sample_capture: Arc<SampleCapture>,
        body_limits: Arc<parking_lot::RwLock<BodyLimits>>,
        port: u16,
    ) -> Self {
        let http_client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("failed to build HTTP client");

        Self {
            aggregator,
            traffic_log,
            cert_cache,
            http_client,
            sample_capture,
            body_limits,
            port,
        }
    }

    /// Start the transparent proxy listener.
    ///
    /// Opens /dev/pf for NAT lookups (requires root), then accepts connections
    /// on the configured port.
    pub async fn start(self: Arc<Self>) -> Result<()> {
        let nat_handle = NatHandle::open()?;
        info!("Opened /dev/pf for transparent NAT lookups");

        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(addr).await?;
        info!("Transparent proxy listening on {}", addr);

        loop {
            let (stream, client_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    error!("Transparent accept error: {}", e);
                    continue;
                }
            };

            let this = self.clone();
            let nat = nat_handle.clone();
            let listen_addr = addr;

            tokio::spawn(async move {
                if let Err(e) = this
                    .handle_connection(stream, client_addr, &nat, listen_addr)
                    .await
                {
                    debug!(
                        "Transparent connection from {} failed: {:#}",
                        client_addr, e
                    );
                }
            });
        }
    }

    async fn handle_connection(
        self: Arc<Self>,
        stream: TcpStream,
        client_addr: SocketAddr,
        nat_handle: &NatHandle,
        listen_addr: SocketAddr,
    ) -> Result<()> {
        let local_addr = stream.local_addr()?;

        let orig_dest = nat_lookup::get_original_dest(
            nat_handle,
            &stream,
            client_addr,
            local_addr,
            listen_addr,
        )?;

        let hostname = if orig_dest.port() == 443 {
            extract_sni_hostname(&stream).await.ok().flatten()
        } else {
            None
        };

        let host = hostname
            .clone()
            .unwrap_or_else(|| format!("{}", orig_dest.ip()));

        info!(
            "Transparent: {} -> {}:{} (host: {})",
            client_addr,
            orig_dest.ip(),
            orig_dest.port(),
            host
        );

        if orig_dest.port() == 443 {
            match hostname {
                Some(ref h) => self.handle_tls_mitm(stream, orig_dest, h).await,
                None => {
                    // No SNI in ClientHello — can't issue a valid cert for an IP.
                    // Relay the raw TLS bytes through without inspection.
                    warn!(
                        "Transparent: no SNI from {}, relaying {}:{} without MITM",
                        client_addr,
                        orig_dest.ip(),
                        orig_dest.port()
                    );
                    self.handle_plaintext_relay(stream, orig_dest, &host).await
                }
            }
        } else {
            self.handle_plaintext_relay(stream, orig_dest, &host).await
        }
    }

    /// TLS MITM: terminate TLS with client using our CA-signed leaf cert,
    /// then connect upstream and relay decrypted HTTP.
    async fn handle_tls_mitm(
        self: Arc<Self>,
        stream: TcpStream,
        orig_dest: SocketAddrV4,
        host: &str,
    ) -> Result<()> {
        let tls_config = self.cert_cache.get_or_create(host)?;
        let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);

        let tls_stream = match acceptor.accept(stream).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    "Transparent TLS handshake failed for {} (client may not trust CA): {}",
                    host, e
                );
                self.traffic_log.record(TrafficEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    host: host.to_string(),
                    method: "TRANSPARENT".to_string(),
                    path: format!("TLS handshake failed: {}", e),
                    status: 525,
                    request_bytes: 0,
                    response_bytes: 0,
                    is_monitored: true,
                    data_captured: vec!["tls_error".to_string()],
                    latency_ms: 0,
                });
                return Ok(());
            }
        };

        info!("Transparent MITM TLS established for {}", host);

        let proxy = self.clone();
        let host_owned = host.to_string();
        let upstream_addr = format!("{}:{}", orig_dest.ip(), orig_dest.port());

        let service = service_fn(move |req: Request<Incoming>| {
            let proxy = proxy.clone();
            let host = host_owned.clone();
            let addr = upstream_addr.clone();
            async move { proxy.handle_mitm_request(req, &host, &addr).await }
        });

        let io = TokioIo::new(tls_stream);

        if let Err(e) =
            hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                .serve_connection(io, service)
                .await
        {
            debug!("Transparent MITM connection to {} closed: {}", host, e);
        }

        Ok(())
    }

    /// Plain relay for non-TLS redirected connections.
    async fn handle_plaintext_relay(
        self: Arc<Self>,
        mut inbound: TcpStream,
        orig_dest: SocketAddrV4,
        host: &str,
    ) -> Result<()> {
        let upstream_addr = format!("{}:{}", orig_dest.ip(), orig_dest.port());
        let mut outbound = TcpStream::connect(&upstream_addr).await?;

        let start = std::time::Instant::now();
        let (client_bytes, server_bytes) =
            tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await?;
        let latency_ms = start.elapsed().as_millis() as u64;

        self.traffic_log.record(TrafficEntry {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            host: host.to_string(),
            method: "TRANSPARENT".to_string(),
            path: format!("relay {}:{}", orig_dest.ip(), orig_dest.port()),
            status: 200,
            request_bytes: client_bytes,
            response_bytes: server_bytes,
            is_monitored: true,
            data_captured: vec!["relay".to_string()],
            latency_ms,
        });

        info!(
            "Transparent relay to {} closed: tx={} rx={}",
            upstream_addr, client_bytes, server_bytes
        );
        Ok(())
    }

    /// Handle a decrypted HTTP request from a transparent MITM connection.
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
                    "Rejecting oversized transparent request: {} bytes > {} limit for {}",
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

        let max_resp_bytes = self.body_limits.read().max_response_bytes;

        let provider = parser::detect_provider(&upstream_url);
        let request_body_str = String::from_utf8_lossy(&body_bytes);
        let model = provider.and_then(|p| parser::extract_model(p, &request_body_str));

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
            if name == "host" {
                continue;
            }
            if let Ok(v) = value.to_str() {
                builder = builder.header(name.as_str(), v);
            }
        }
        builder = builder.header("host", upstream_host);

        if !body_bytes.is_empty() {
            builder = builder.body(body_bytes.to_vec());
        }

        match builder.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let resp_headers = resp.headers().clone();
                let resp_headers_for_task = resp_headers.clone();

                let (tx, rx) = tokio::sync::mpsc::channel::<
                    Result<Frame<Bytes>, std::convert::Infallible>,
                >(32);

                let this = self.clone();
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

                    while let Some(chunk_result) = stream.next().await {
                        match chunk_result {
                            Ok(chunk) => {
                                if max_resp_bytes.map_or(true, |max| (collected.len() as u64) < max)
                                {
                                    collected.extend_from_slice(&chunk);
                                }
                                if tx.send(Ok(Frame::data(chunk))).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                warn!("Transparent stream read error from {}: {}", addr_owned, e);
                                break;
                            }
                        }
                    }
                    drop(tx);

                    let resp_body = Bytes::from(collected);
                    let response_bytes = resp_body.len() as u64;
                    let latency_ms = start.elapsed().as_millis() as u64;
                    let mut data_captured = vec!["transparent".to_string()];

                    if let Some(provider) = provider {
                        if (200..300).contains(&(status as usize)) {
                            let body_str = String::from_utf8_lossy(&resp_body);

                            let is_ai_call = provider == parser::LLMProvider::Cursor
                                && !path_owned.contains("DashboardService")
                                && !path_owned.contains("AnalyticsService")
                                && !path_owned.contains("ServerConfigService")
                                && !path_owned.contains("ReportClient")
                                && !path_owned.contains("ReportProcess")
                                && (request_bytes > 50 || response_bytes > 200);

                            if this.sample_capture.is_armed() {
                                let content_type = resp_headers_for_task
                                    .get("content-type")
                                    .and_then(|v| v.to_str().ok())
                                    .map(String::from);
                                this.sample_capture.push(crate::state::PayloadSample {
                                    timestamp: Utc::now().to_rfc3339(),
                                    host: host_owned.clone(),
                                    path: path_owned.clone(),
                                    method: method_owned.clone(),
                                    content_type,
                                    request_preview: req_body_str_owned
                                        .chars()
                                        .take(2048)
                                        .collect(),
                                    response_preview: body_str.chars().take(4096).collect(),
                                    request_hex: hex::encode(
                                        &req_bytes_owned[..req_bytes_owned.len().min(2048)],
                                    ),
                                    response_hex: hex::encode(
                                        &resp_body[..resp_body.len().min(4096)],
                                    ),
                                    request_bytes: req_bytes_owned.len(),
                                    response_bytes: resp_body.len(),
                                });
                            }

                            let usage =
                                parser::extract_usage(provider, &body_str).ok().or_else(|| {
                                    if is_ai_call {
                                        Some(parser::estimate_usage_from_bytes(
                                            request_bytes,
                                            response_bytes,
                                        ))
                                    } else {
                                        None
                                    }
                                });

                            if let Some(usage) = usage {
                                if usage.input_tokens > 0 {
                                    data_captured.push("tokens_in".to_string());
                                }
                                if usage.output_tokens > 0 {
                                    data_captured.push("tokens_out".to_string());
                                }
                                if usage.cache_read_tokens.unwrap_or(0) > 0
                                    || usage.cache_creation_tokens.unwrap_or(0) > 0
                                {
                                    data_captured.push("cache".to_string());
                                }
                                data_captured.push("cost".to_string());

                                let final_model = match model_owned.as_deref() {
                                    Some(m) => m.to_string(),
                                    None if provider == parser::LLMProvider::Cursor => {
                                        parser::scan_bytes_for_model(&resp_body)
                                            .unwrap_or_else(|| "unknown".to_string())
                                    }
                                    None => "unknown".to_string(),
                                };
                                this.aggregator.record_usage(
                                    provider,
                                    &final_model,
                                    &url_owned,
                                    usage,
                                );
                            }
                        }
                    }

                    this.traffic_log.record(TrafficEntry {
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
                    host: upstream_host.to_string(),
                    method: method_string,
                    path,
                    status: 502,
                    request_bytes,
                    response_bytes: 0,
                    is_monitored: true,
                    data_captured: vec!["transparent".to_string()],
                    latency_ms,
                });

                warn!(
                    "Transparent upstream request to {} failed: {}",
                    upstream_addr, e
                );
                Ok(Response::builder()
                    .status(502)
                    .body(box_body(Full::new(Bytes::from(format!(
                        "Lumen transparent proxy error: {}",
                        e
                    )))))
                    .unwrap())
            }
        }
    }
}

/// Peek at the TLS ClientHello to extract the SNI hostname without consuming
/// the stream data.
async fn extract_sni_hostname(stream: &TcpStream) -> Result<Option<String>> {
    let mut buf = [0u8; 1024];
    let n = stream.peek(&mut buf).await?;
    if n < 5 {
        return Ok(None);
    }

    // TLS record: content_type(1) + version(2) + length(2) + body
    if buf[0] != 0x16 {
        return Ok(None); // not a TLS handshake
    }

    let record_len = ((buf[3] as usize) << 8) | buf[4] as usize;
    let handshake_end = std::cmp::min(5 + record_len, n);
    let handshake = &buf[5..handshake_end];

    if handshake.is_empty() || handshake[0] != 0x01 {
        return Ok(None); // not ClientHello
    }

    // ClientHello: type(1) + length(3) + version(2) + random(32) + session_id_len(1) + ...
    if handshake.len() < 38 {
        return Ok(None);
    }

    let mut pos = 38; // skip to session_id

    // Skip session ID
    if pos >= handshake.len() {
        return Ok(None);
    }
    let session_id_len = handshake[pos] as usize;
    pos += 1 + session_id_len;

    // Skip cipher suites
    if pos + 2 > handshake.len() {
        return Ok(None);
    }
    let cipher_len = ((handshake[pos] as usize) << 8) | handshake[pos + 1] as usize;
    pos += 2 + cipher_len;

    // Skip compression methods
    if pos >= handshake.len() {
        return Ok(None);
    }
    let comp_len = handshake[pos] as usize;
    pos += 1 + comp_len;

    // Extensions
    if pos + 2 > handshake.len() {
        return Ok(None);
    }
    let ext_total = ((handshake[pos] as usize) << 8) | handshake[pos + 1] as usize;
    pos += 2;
    let ext_end = std::cmp::min(pos + ext_total, handshake.len());

    while pos + 4 <= ext_end {
        let ext_type = ((handshake[pos] as u16) << 8) | handshake[pos + 1] as u16;
        let ext_len = ((handshake[pos + 2] as usize) << 8) | handshake[pos + 3] as usize;
        pos += 4;

        if ext_type == 0x0000 {
            // SNI extension
            if pos + 2 > ext_end {
                break;
            }
            let _sni_list_len = ((handshake[pos] as usize) << 8) | handshake[pos + 1] as usize;
            pos += 2;

            if pos + 3 > ext_end {
                break;
            }
            let name_type = handshake[pos];
            let name_len = ((handshake[pos + 1] as usize) << 8) | handshake[pos + 2] as usize;
            pos += 3;

            if name_type == 0 && pos + name_len <= ext_end {
                let hostname = std::str::from_utf8(&handshake[pos..pos + name_len])
                    .ok()
                    .map(String::from);
                return Ok(hostname);
            }
            break;
        }

        pos += ext_len;
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sni_parser_non_tls() {
        // Non-TLS data should return None
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();

            let client = tokio::spawn(async move {
                let mut stream = TcpStream::connect(addr).await.unwrap();
                tokio::io::AsyncWriteExt::write_all(&mut stream, b"GET / HTTP/1.1\r\n")
                    .await
                    .unwrap();
                // Keep alive briefly
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            });

            let (conn, _) = listener.accept().await.unwrap();
            let result = extract_sni_hostname(&conn).await.unwrap();
            assert!(result.is_none());

            client.await.unwrap();
        });
    }
}
