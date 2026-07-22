use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::proxy::{BodyLimits, LumenProxy};
use crate::state::AppState;

pub async fn start_api_server(
    state: Arc<AppState>,
    port: u16,
    mut shutdown_rx: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    // Bind IPv4 loopback (required).
    let v4 = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(v4).await?;
    info!("Lumen API server listening on http://{}", v4);

    // Also listen on IPv6 loopback so browsers that resolve `localhost` to
    // `::1` first — the default on Windows — can reach the daemon. Without it,
    // the dashboard HTML loads over 127.0.0.1 but its same-origin `fetch()`
    // calls to `localhost` hit `[::1]:port`, which nothing is listening on, and
    // fail with a bare "Failed to fetch". Best-effort: keep serving on IPv4 if
    // the ::1 bind is unavailable. Both are loopback — no wider exposure.
    let listener_v6 =
        match TcpListener::bind(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port))).await {
            Ok(l) => {
                info!("Lumen API server also listening on http://[::1]:{}", port);
                Some(l)
            }
            Err(e) => {
                warn!(
                    "Lumen API: could not bind IPv6 loopback [::1]:{} ({}). `localhost` may fail \
                     in browsers that prefer IPv6 — use http://127.0.0.1:{} instead.",
                    port, e, port
                );
                None
            }
        };

    let serve = |stream, state: Arc<AppState>| {
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| {
                let state = state.clone();
                async move { handle_api_request(req, state).await }
            });

            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await
            {
                error!("API connection error: {}", e);
            }
        });
    };

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => serve(stream, state.clone()),
                    Err(e) => error!("API accept error (v4): {}", e),
                }
            }
            result = accept_optional(&listener_v6) => {
                match result {
                    Ok((stream, _)) => serve(stream, state.clone()),
                    Err(e) => error!("API accept error (v6): {}", e),
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("API server shutting down");
                    break;
                }
            }
        }
    }
    Ok(())
}

// Await a connection from an optional listener. When there is none (the IPv6
// loopback bind failed), pend forever so this branch never resolves in
// `select!`, leaving the IPv4 and shutdown branches active.
async fn accept_optional(
    listener: &Option<TcpListener>,
) -> std::io::Result<(tokio::net::TcpStream, SocketAddr)> {
    match listener {
        Some(l) => l.accept().await,
        None => std::future::pending().await,
    }
}

async fn handle_api_request(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

    // Redirect bare `/` to `/dashboard` so users who hit the API port in a
    // browser land somewhere useful instead of getting a 401/404.
    if method == Method::GET && (path == "/" || path.is_empty()) {
        return Ok(Response::builder()
            .status(StatusCode::FOUND)
            .header("Location", "/dashboard")
            .body(Full::new(Bytes::new()))
            .unwrap());
    }

    // Authenticate every request except:
    //   - /health           : DaemonManager probes without a token
    //   - /dg/oauth/callback: browser redirect, can't send custom headers
    //   - /dashboard        : served HTML, token is injected into the page itself
    //   - /ca/pem           : public certificate, needed by `<a download>` from
    //                         the dashboard (which is a browser navigation,
    //                         can't add headers). The PEM contains no secret
    //                         material — the private key lives elsewhere.
    let auth_exempt = matches!(
        (method.as_str(), path.as_str()),
        ("GET", "/health")
            | ("GET", "/dg/oauth/callback")
            | ("GET", "/dashboard")
            | ("GET", "/ca/pem")
    );
    if !auth_exempt {
        let provided = req
            .headers()
            .get("x-lumen-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if provided != state.api_token {
            return Ok(json_response(
                StatusCode::UNAUTHORIZED,
                &serde_json::json!({"error": "unauthorized"}),
            ));
        }
    }

    let response = match (method, path.as_str()) {
        (Method::GET, "/stats") => {
            let stats = state.aggregator.compute_stats();
            json_response(StatusCode::OK, &stats)
        }

        (Method::GET, "/events") => {
            let limit = parse_query_param(&query, "limit").unwrap_or(50);
            let events = state.aggregator.recent_events(limit);
            json_response(StatusCode::OK, &events)
        }

        (Method::POST, "/clear") => {
            state.aggregator.clear();
            json_response(StatusCode::OK, &serde_json::json!({"ok": true}))
        }

        (Method::POST, "/proxy/start") => {
            let port = state.proxy_config.read().port;

            if state.proxy_config.read().running {
                json_response(StatusCode::OK, &state.proxy_config.read().clone())
            } else {
                let proxy = Arc::new(LumenProxy::new(
                    state.aggregator.clone(),
                    state.traffic_log.clone(),
                    state.cert_cache.clone(),
                    state.sample_capture.clone(),
                    state.body_limits.clone(),
                    port,
                ));
                let proxy_clone = proxy.clone();

                tokio::spawn(async move {
                    if let Err(e) = proxy_clone.start().await {
                        tracing::error!("Proxy error: {}", e);
                    }
                });

                *state.proxy.write() = Some(proxy);
                state.proxy_config.write().running = true;

                json_response(StatusCode::OK, &state.proxy_config.read().clone())
            }
        }

        (Method::GET, "/proxy/config") => {
            json_response(StatusCode::OK, &state.proxy_config.read().clone())
        }

        (Method::GET, "/hosts") => {
            let hosts = if let Some(proxy) = state.proxy.read().as_ref() {
                proxy.list_hosts()
            } else {
                vec![
                    "api.openai.com".to_string(),
                    "api.anthropic.com".to_string(),
                    "generativelanguage.googleapis.com".to_string(),
                    "*.cursor.sh (suffix)".to_string(),
                    "*.cursorapi.com (suffix)".to_string(),
                ]
            };
            json_response(StatusCode::OK, &hosts)
        }

        (Method::POST, "/hosts") => {
            let body_bytes = req.into_body().collect().await?.to_bytes();
            let result: Result<Response<Full<Bytes>>, hyper::Error> = (|| {
                let payload: serde_json::Value =
                    serde_json::from_slice(&body_bytes).unwrap_or_default();
                let host = payload["host"].as_str().unwrap_or("");

                if host.is_empty() {
                    return Ok(json_response(
                        StatusCode::BAD_REQUEST,
                        &serde_json::json!({"error": "missing host field"}),
                    ));
                }

                if let Some(proxy) = state.proxy.read().as_ref() {
                    proxy.add_host(host);
                    Ok(json_response(StatusCode::OK, &proxy.list_hosts()))
                } else {
                    Ok(json_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        &serde_json::json!({"error": "proxy not running"}),
                    ))
                }
            })();
            return result;
        }

        (Method::DELETE, path) if path.starts_with("/hosts/") => {
            let host = &path[7..]; // strip "/hosts/"
            if let Some(proxy) = state.proxy.read().as_ref() {
                proxy.remove_host(host);
                json_response(StatusCode::OK, &proxy.list_hosts())
            } else {
                json_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    &serde_json::json!({"error": "proxy not running"}),
                )
            }
        }

        (Method::GET, "/routes") => {
            let routes = if let Some(proxy) = state.proxy.read().as_ref() {
                proxy.list_routes()
            } else {
                vec![
                    crate::proxy::RelayRoute {
                        prefix: "/openai".into(),
                        upstream: "https://api.openai.com".into(),
                    },
                    crate::proxy::RelayRoute {
                        prefix: "/anthropic".into(),
                        upstream: "https://api.anthropic.com".into(),
                    },
                    crate::proxy::RelayRoute {
                        prefix: "/google".into(),
                        upstream: "https://generativelanguage.googleapis.com".into(),
                    },
                ]
            };
            json_response(StatusCode::OK, &routes)
        }

        (Method::POST, "/routes") => {
            let body_bytes = req.into_body().collect().await?.to_bytes();
            let result: Result<Response<Full<Bytes>>, hyper::Error> = (|| {
                let payload: serde_json::Value =
                    serde_json::from_slice(&body_bytes).unwrap_or_default();
                let prefix = payload["prefix"].as_str().unwrap_or("");
                let upstream = payload["upstream"].as_str().unwrap_or("");

                if prefix.is_empty() || upstream.is_empty() {
                    return Ok(json_response(
                        StatusCode::BAD_REQUEST,
                        &serde_json::json!({"error": "missing prefix or upstream field"}),
                    ));
                }

                if let Some(proxy) = state.proxy.read().as_ref() {
                    proxy.add_route(prefix, upstream);
                    Ok(json_response(StatusCode::OK, &proxy.list_routes()))
                } else {
                    Ok(json_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        &serde_json::json!({"error": "proxy not running"}),
                    ))
                }
            })();
            return result;
        }

        (Method::DELETE, path) if path.starts_with("/routes/") => {
            let prefix = format!("/{}", &path[8..]); // strip "/routes/" and re-add leading /
            if let Some(proxy) = state.proxy.read().as_ref() {
                proxy.remove_route(&prefix);
                json_response(StatusCode::OK, &proxy.list_routes())
            } else {
                json_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    &serde_json::json!({"error": "proxy not running"}),
                )
            }
        }

        (Method::GET, "/config/limits") => {
            json_response(StatusCode::OK, &*state.body_limits.read())
        }

        (Method::PUT, "/config/limits") => {
            let body_bytes = req.into_body().collect().await?.to_bytes();
            return match serde_json::from_slice::<BodyLimits>(&body_bytes) {
                Ok(limits) => {
                    *state.body_limits.write() = limits.clone();
                    Ok(json_response(StatusCode::OK, &limits))
                }
                Err(e) => Ok(json_response(
                    StatusCode::BAD_REQUEST,
                    &serde_json::json!({"error": e.to_string()}),
                )),
            };
        }

        (Method::GET, "/config") => json_response(StatusCode::OK, &state.dg_config.read().clone()),

        (Method::PUT, "/config") => {
            let body_bytes = req.into_body().collect().await?.to_bytes();
            return match serde_json::from_slice::<crate::state::DGConfig>(&body_bytes) {
                Ok(config) => {
                    info!(
                        enabled = config.enabled,
                        tools_hidden = config.tools_hidden,
                        intelligent_interface = config.intelligent_interface,
                        "DG config updated"
                    );
                    *state.dg_server_url.write() = config
                        .server_url
                        .as_deref()
                        .and_then(canonical_dg_server_url)
                        .or_else(|| config.server_url.clone());
                    *state.dg_config.write() = config;
                    crate::state::save_dg_config_to_disk(&state.dg_config.read());
                    Ok(json_response(
                        StatusCode::OK,
                        &state.dg_config.read().clone(),
                    ))
                }
                Err(e) => {
                    warn!("DG config PUT failed to parse: {}", e);
                    Ok(json_response(
                        StatusCode::BAD_REQUEST,
                        &serde_json::json!({"error": e.to_string()}),
                    ))
                }
            };
        }

        (Method::DELETE, "/dg/identity") => {
            // Clear in-memory identity and server URL.
            *state.dg_identity.write() = None;
            *state.dg_server_url.write() = None;
            *state.dg_bearer_token.write() = None;
            {
                let mut cfg = state.dg_config.write();
                cfg.server_url = None;
                cfg.bearer_token = None;
            }
            crate::state::save_dg_config_to_disk(&state.dg_config.read());
            *state.dg_http_client.write() = reqwest::Client::new();

            // Delete identity files from ~/.conduit/.
            if let Some(home) = crate::state::home_dir() {
                let dir = home.join(".conduit");
                for file in &["identity.pem", "identity_key.pem", "ca.pem", "sub_id.txt"] {
                    let _ = std::fs::remove_file(dir.join(file));
                }
            }

            info!("DG identity disconnected and identity files removed");
            json_response(StatusCode::OK, &serde_json::json!({"ok": true}))
        }

        (Method::GET, "/traffic") => {
            let limit = parse_query_param(&query, "limit").unwrap_or(100);
            let host_filter = parse_query_string(&query, "host");
            let monitored_only = parse_query_string(&query, "monitored")
                .map(|v| v == "true")
                .unwrap_or(false);

            let entries =
                state
                    .traffic_log
                    .recent_filtered(limit, host_filter.as_deref(), monitored_only);
            json_response(StatusCode::OK, &entries)
        }

        (Method::GET, "/traffic/hosts") => {
            let aggs = state.traffic_log.host_aggregates();
            json_response(StatusCode::OK, &aggs)
        }

        (Method::GET, "/traffic/revision") => json_response(
            StatusCode::OK,
            &serde_json::json!({ "revision": state.traffic_log.revision() }),
        ),

        (Method::POST, "/lap") => {
            let body_bytes = req.into_body().collect().await?.to_bytes();
            let label = if body_bytes.is_empty() {
                None
            } else {
                serde_json::from_slice::<serde_json::Value>(&body_bytes)
                    .ok()
                    .and_then(|v| v["label"].as_str().map(String::from))
            };
            let snapshot = state.aggregator.create_lap(label);

            // Fire-and-forget: sync the lap snapshot to DG if configured.
            {
                let url_guard = state.dg_server_url.read();
                if let Some(ref server_url) = *url_guard {
                    let gateway_root = server_url
                        .find("/servers/")
                        .map(|pos| server_url[..pos].to_string())
                        .unwrap_or_else(|| server_url.to_string());
                    let endpoint =
                        format!("{}/api/v1/lumen/laps", gateway_root.trim_end_matches('/'));
                    let client = state.dg_http_client.read().clone();
                    let snap = snapshot.clone();
                    let sub_id = state.dg_lumen_sub_id.read().clone();
                    let sync_token = state.dg_sync_token.read().clone();
                    tokio::spawn(async move {
                        let mut req = client.post(&endpoint).json(&snap);
                        if let (Some(sid), Some(token)) = (sub_id, sync_token) {
                            req = req.bearer_auth(format!("lm_{}.{}", sid, token));
                        }
                        let _ = req.send().await;
                    });
                }
            }

            return Ok(json_response(StatusCode::OK, &snapshot));
        }

        (Method::GET, "/laps") => {
            let laps = state.aggregator.get_laps();
            json_response(StatusCode::OK, &laps)
        }

        (Method::POST, "/shutdown") => {
            info!("Shutdown requested via API");
            let _ = state.shutdown_tx.send(true);
            json_response(
                StatusCode::OK,
                &serde_json::json!({"status": "shutting_down"}),
            )
        }

        (Method::GET, "/ca/pem") => {
            let pem = state.ca.cert_pem.clone();
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/x-pem-file")
                .header(
                    "Content-Disposition",
                    "attachment; filename=\"lumen-ca.pem\"",
                )
                .body(Full::new(Bytes::from(pem)))
                .unwrap());
        }

        (Method::GET, "/ca/info") => json_response(
            StatusCode::OK,
            &serde_json::json!({
                "path": crate::ca::LumenCA::cert_path().ok().map(|p| p.to_string_lossy().into_owned()),
                "subject": "Lumen Local CA",
                "issuer": "Lumen by DataGrout",
            }),
        ),

        (Method::POST, "/debug/arm") => {
            state.sample_capture.arm();
            json_response(
                StatusCode::OK,
                &serde_json::json!({"armed": true, "message": "Capturing next Cursor payloads"}),
            )
        }

        (Method::POST, "/debug/disarm") => {
            state.sample_capture.disarm();
            json_response(StatusCode::OK, &serde_json::json!({"armed": false}))
        }

        (Method::GET, "/debug/samples") => {
            let samples: Vec<_> = state
                .sample_capture
                .samples
                .read()
                .iter()
                .cloned()
                .collect();
            json_response(StatusCode::OK, &samples)
        }

        #[cfg(target_os = "macos")]
        (Method::GET, "/transparent/config") => {
            json_response(StatusCode::OK, &state.transparent_config.read().clone())
        }

        #[cfg(target_os = "macos")]
        (Method::POST, "/transparent/pf/enable") => {
            let body_bytes = req.into_body().collect().await?.to_bytes();
            let result: Result<Response<Full<Bytes>>, hyper::Error> = (|| {
                let payload: serde_json::Value =
                    serde_json::from_slice(&body_bytes).unwrap_or_default();
                let extra_hosts: Vec<String> = payload["extra_hosts"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                let script_path = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                    .map(|d| d.join("../../scripts/pf_setup.sh"))
                    .unwrap_or_default();

                let result_msg = format!(
                    "pf rules require admin privileges. Run:\n  sudo {} --local\n\nOr use the Swift UI 'Enable Transparent Capture' button.",
                    script_path.display()
                );

                Ok(json_response(
                    StatusCode::OK,
                    &serde_json::json!({
                        "status": "manual_setup_required",
                        "message": result_msg,
                        "extra_hosts": extra_hosts,
                    }),
                ))
            })();
            return result;
        }

        #[cfg(target_os = "macos")]
        (Method::POST, "/transparent/pf/disable") => json_response(
            StatusCode::OK,
            &serde_json::json!({
                "status": "manual_teardown_required",
                "message": "Run: sudo scripts/pf_setup.sh --teardown"
            }),
        ),

        (Method::GET, "/health") => json_response(
            StatusCode::OK,
            &serde_json::json!({
                "status": "ok",
                "version": env!("CARGO_PKG_VERSION"),
                "proxy_running": state.proxy_config.read().running,
                "transparent_enabled": state.transparent_config.read().enabled,
            }),
        ),

        (Method::GET, "/dashboard") => {
            let html = include_str!("dashboard.html")
                .replace("__LUMEN_TOKEN__", &state.api_token)
                .replace("__LUMEN_VERSION__", env!("CARGO_PKG_VERSION"));
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/html; charset=utf-8")
                .body(Full::new(Bytes::from(html)))
                .unwrap()
        }

        // ─── Conduit / DG identity ─────────────────────────────────────────────
        (Method::GET, "/dg/status") => {
            let has_identity = state.dg_identity.read().is_some();
            let sub_id = state
                .dg_identity
                .read()
                .as_ref()
                .and_then(|id| id.sub_id.clone());
            let cert = state.dg_cert_status.read().clone();
            json_response(
                StatusCode::OK,
                &serde_json::json!({
                    "connected": has_identity,
                    "sub_id": sub_id,
                    "server_url": state.dg_server_url.read().clone(),
                    // Cert health for the UI: expiry, current auth mode, and a
                    // flag the UI can show as "reconnect to restore mTLS".
                    "cert_expires_at": cert.expires_at,
                    "auth_mode": cert.mode.as_str(),
                    "needs_reconnect": cert.mode == crate::state::DgAuthMode::BearerFallback,
                }),
            )
        }

        (Method::POST, "/dg/bootstrap") => {
            let body_bytes = req.into_body().collect().await?.to_bytes();
            let state_clone = state.clone();
            let result: Result<Response<Full<Bytes>>, hyper::Error> = async {
                let payload: serde_json::Value =
                    serde_json::from_slice(&body_bytes).unwrap_or_default();

                let server_url = match payload["server_url"].as_str() {
                    Some(u) if !u.is_empty() => u.to_string(),
                    _ => {
                        return Ok(json_response(
                            StatusCode::BAD_REQUEST,
                            &serde_json::json!({"error": "server_url is required"}),
                        ));
                    }
                };

                let token = match payload["token"].as_str() {
                    Some(t) if !t.is_empty() => t.to_string(),
                    _ => {
                        return Ok(json_response(
                            StatusCode::BAD_REQUEST,
                            &serde_json::json!({"error": "token is required"}),
                        ));
                    }
                };

                let device_name = payload["device_name"]
                    .as_str()
                    .unwrap_or("lumen-device")
                    .to_string();

                match crate::conduit::register_with_dg(&server_url, &token, &device_name).await {
                    Ok(identity) => {
                        let sub_id = identity.sub_id.clone().unwrap_or_default();

                        // Build a new mTLS client from the fresh identity.
                        match identity.build_client() {
                            Ok(new_client) => {
                                *state_clone.dg_http_client.write() = new_client;
                                *state_clone.dg_identity.write() = Some(identity);
                                // Clear the bearer token — mTLS takes over.
                                *state_clone.dg_bearer_token.write() = None;
                                let canonical = canonical_dg_server_url(&server_url)
                                    .unwrap_or_else(|| server_url.clone());
                                *state_clone.dg_server_url.write() = Some(canonical.clone());

                                tracing::info!(
                                    "Conduit bootstrap complete: sub_id={} server={}",
                                    sub_id,
                                    canonical
                                );

                                Ok(json_response(
                                    StatusCode::OK,
                                    &serde_json::json!({
                                        "ok": true,
                                        "sub_id": sub_id,
                                        "server_url": server_url,
                                    }),
                                ))
                            }
                            Err(e) => Ok(json_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                &serde_json::json!({"error": format!("mTLS client build failed: {e}")}),
                            )),
                        }
                    }
                    Err(e) => Ok(json_response(
                        StatusCode::BAD_GATEWAY,
                        &serde_json::json!({"error": format!("DG registration failed: {e}")}),
                    )),
                }
            }
            .await;
            return result;
        }

        // ─── DCR OAuth flow ────────────────────────────────────────────────────
        (Method::POST, "/dg/dcr") => {
            let body_bytes = req.into_body().collect().await?.to_bytes();
            let state_clone = state.clone();
            let result: Result<Response<Full<Bytes>>, hyper::Error> = async {
                let payload: serde_json::Value =
                    serde_json::from_slice(&body_bytes).unwrap_or_default();

                let raw_url = match payload["server_url"].as_str() {
                    Some(u) if !u.is_empty() => u.trim_end_matches('/').to_string(),
                    _ => {
                        return Ok(json_response(
                            StatusCode::BAD_REQUEST,
                            &serde_json::json!({"error": "server_url required"}),
                        ));
                    }
                };

                // Parse out DCR base (server-level, strip /mcp suffix) and auth root
                // (everything before /servers/) so we can handle both root URLs
                // (https://datagrout.ai) and per-server MCP URLs
                // (https://datagrout.ai/servers/UUID/mcp).
                let (dcr_base, auth_root) = parse_dg_url(&raw_url);

                let device_name = payload["device_name"]
                    .as_str()
                    .unwrap_or("lumen-device")
                    .to_string();

                let api_port = *state_clone.api_port.read();
                let redirect_uri =
                    format!("http://127.0.0.1:{}/dg/oauth/callback", api_port);

                match crate::conduit::dcr_register_client(
                    &dcr_base,
                    &redirect_uri,
                    &device_name,
                )
                .await
                {
                    Ok(client_id) => {
                        let (verifier, challenge) = crate::conduit::generate_pkce();
                        let state_param = uuid::Uuid::new_v4().to_string();

                        let auth_url = format!(
                            "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}&scope=mcp",
                            auth_root,
                            percent_encode(&client_id),
                            percent_encode(&redirect_uri),
                            percent_encode(&challenge),
                            percent_encode(&state_param),
                        );

                        use crate::state::{DcrFlow, DcrFlowStatus};
                        *state_clone.dcr_flow.write() = Some(DcrFlow {
                            server_url: auth_root,
                            lumen_server_url: dcr_base,
                            client_id,
                            code_verifier: verifier,
                            redirect_uri,
                            state_param,
                            auth_url: auth_url.clone(),
                            status: DcrFlowStatus::WaitingForCode,
                            device_name,
                        });

                        Ok(json_response(
                            StatusCode::OK,
                            &serde_json::json!({"ok": true, "auth_url": auth_url}),
                        ))
                    }
                    Err(e) => Ok(json_response(
                        StatusCode::BAD_GATEWAY,
                        &serde_json::json!({"error": format!("{e}")}),
                    )),
                }
            }
            .await;
            return result;
        }

        (Method::GET, "/dg/dcr/status") => {
            use crate::state::DcrFlowStatus;
            let flow = state.dcr_flow.read();
            match flow.as_ref() {
                None => json_response(StatusCode::OK, &serde_json::json!({"status": "idle"})),
                Some(f) => match &f.status {
                    DcrFlowStatus::WaitingForCode => json_response(
                        StatusCode::OK,
                        &serde_json::json!({"status": "waiting", "auth_url": f.auth_url}),
                    ),
                    DcrFlowStatus::ExchangingToken => {
                        json_response(StatusCode::OK, &serde_json::json!({"status": "exchanging"}))
                    }
                    DcrFlowStatus::Complete => {
                        json_response(StatusCode::OK, &serde_json::json!({"status": "complete"}))
                    }
                    DcrFlowStatus::Failed { error } => json_response(
                        StatusCode::OK,
                        &serde_json::json!({"status": "failed", "error": error}),
                    ),
                },
            }
        }

        (Method::GET, "/dg/oauth/callback") => {
            use crate::state::DcrFlowStatus;

            let code = parse_query_string(&query, "code");
            let incoming_state = parse_query_string(&query, "state");

            let flow_details = {
                let flow = state.dcr_flow.read();
                flow.as_ref().and_then(|f| {
                    if incoming_state.as_deref() == Some(f.state_param.as_str()) {
                        Some((
                            f.server_url.clone(),
                            f.lumen_server_url.clone(),
                            f.client_id.clone(),
                            f.code_verifier.clone(),
                            f.redirect_uri.clone(),
                            f.device_name.clone(),
                        ))
                    } else {
                        None
                    }
                })
            };

            match (code, flow_details) {
                (
                    Some(code),
                    Some((
                        server_url,
                        lumen_server_url,
                        client_id,
                        verifier,
                        redirect_uri,
                        device_name,
                    )),
                ) => {
                    // Mark as exchanging token
                    if let Some(f) = state.dcr_flow.write().as_mut() {
                        f.status = DcrFlowStatus::ExchangingToken;
                    }

                    let state_clone = state.clone();
                    tokio::spawn(async move {
                        let result = async {
                            let token = crate::conduit::exchange_code_for_token(
                                &server_url,
                                &client_id,
                                &code,
                                &verifier,
                                &redirect_uri,
                            )
                            .await?;
                            crate::conduit::register_with_oauth_token(
                                &server_url,
                                &token,
                                &device_name,
                            )
                            .await
                        }
                        .await;

                        match result {
                            Ok(identity) => match identity.build_client() {
                                Ok(new_client) => {
                                    *state_clone.dg_http_client.write() = new_client;
                                    *state_clone.dg_server_url.write() =
                                        Some(lumen_server_url.clone());
                                    *state_clone.dg_bearer_token.write() = None;
                                    let sub_id = identity.sub_id.clone().unwrap_or_default();
                                    let sync_token = identity.sync_token.clone();
                                    *state_clone.dg_identity.write() = Some(identity);
                                    {
                                        let mut cfg = state_clone.dg_config.write();
                                        cfg.server_url = Some(lumen_server_url.clone());
                                        if let Some(ref token) = sync_token {
                                            cfg.sync_token = Some(token.clone());
                                        }
                                    }
                                    crate::state::save_dg_config_to_disk(
                                        &state_clone.dg_config.read(),
                                    );
                                    *state_clone.dg_sync_token.write() = sync_token;
                                    *state_clone.dg_lumen_sub_id.write() = Some(sub_id.clone());
                                    if let Some(f) = state_clone.dcr_flow.write().as_mut() {
                                        f.status = DcrFlowStatus::Complete;
                                    }
                                    info!(
                                        "DCR OAuth complete: sub_id={} server={}",
                                        sub_id, server_url
                                    );
                                }
                                Err(e) => {
                                    if let Some(f) = state_clone.dcr_flow.write().as_mut() {
                                        f.status = DcrFlowStatus::Failed {
                                            error: e.to_string(),
                                        };
                                    }
                                }
                            },
                            Err(e) => {
                                if let Some(f) = state_clone.dcr_flow.write().as_mut() {
                                    f.status = DcrFlowStatus::Failed {
                                        error: e.to_string(),
                                    };
                                }
                            }
                        }
                    });

                    return Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "text/html")
                        .body(Full::new(Bytes::from(oauth_success_html())))
                        .unwrap());
                }
                _ => {
                    if let Some(f) = state.dcr_flow.write().as_mut() {
                        f.status = DcrFlowStatus::Failed {
                            error: "OAuth callback: state mismatch or missing code".to_string(),
                        };
                    }
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .header("Content-Type", "text/html")
                        .body(Full::new(Bytes::from(oauth_error_html(
                            "Authorization failed — invalid state or missing code.",
                        ))))
                        .unwrap());
                }
            }
        }

        _ => json_response(
            StatusCode::NOT_FOUND,
            &serde_json::json!({"error": "not found"}),
        ),
    };

    Ok(response)
}

/// Given any DataGrout URL the user might paste, return:
///   (dcr_base, auth_root)
/// - dcr_base:  where to POST /register — if the URL is a per-server MCP URL
///              like `https://dg.ai/servers/UUID/mcp`, this is `https://dg.ai/servers/UUID`
/// True if `s` looks like a UUID (8-4-4-4-12 hex + dashes, case-insensitive).
fn looks_like_uuid(s: &str) -> bool {
    if s.len() != 36 {
        return false;
    }
    let b = s.as_bytes();
    b[8] == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Normalize any DG server URL (or bare UUID) to canonical form:
/// `https://{host}/servers/{uuid}`.
///
/// Accepts:
///   - Full URL:  `https://gateway.datagrout.ai/servers/{uuid}/mcp`
///   - Bare UUID: `1e427a82-5d21-4f76-8ed7-5b4035114962`
///   - Already canonical: `https://gateway.datagrout.ai/servers/{uuid}`
///
/// Returns `None` only when no UUID can be found.
pub fn canonical_dg_server_url(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_end_matches('/');

    // Full URL containing /servers/{uuid}[/…]
    if raw.starts_with("http") {
        if let Some(pos) = raw.find("/servers/") {
            let host = &raw[..pos];
            let after = &raw[pos + "/servers/".len()..];
            let uuid = after.split('/').next().unwrap_or(after);
            if looks_like_uuid(uuid) {
                return Some(format!("{}/servers/{}", host, uuid.to_lowercase()));
            }
        }
        // URL but no /servers/ path — return trimmed as-is
        return Some(raw.to_string());
    }

    // Bare UUID
    if looks_like_uuid(raw) {
        return Some(format!(
            "https://gateway.datagrout.ai/servers/{}",
            raw.to_lowercase()
        ));
    }

    None
}

/// Returns `(dcr_base, auth_root)`:
/// - dcr_base:  canonical per-server URL (`https://host/servers/{uuid}`)
/// - auth_root: root DG host for OAuth endpoints (`https://host`)
fn parse_dg_url(raw: &str) -> (String, String) {
    let canonical =
        canonical_dg_server_url(raw).unwrap_or_else(|| raw.trim_end_matches('/').to_string());

    // Extract root URL (everything before /servers/...)
    let auth_root = if let Some(idx) = canonical.find("/servers/") {
        canonical[..idx].to_string()
    } else {
        canonical.clone()
    };

    (canonical, auth_root)
}

fn percent_encode(s: &str) -> String {
    s.chars().fold(String::new(), |mut acc, c| {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => acc.push(c),
            _ => {
                for byte in c.to_string().as_bytes() {
                    acc.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        acc
    })
}

fn oauth_success_html() -> String {
    r#"<!DOCTYPE html><html><head><meta charset="UTF-8"><title>Authorized</title>
<style>body{font-family:-apple-system,sans-serif;display:flex;justify-content:center;align-items:center;min-height:100vh;margin:0;background:#0a0a0f;color:#fff}
.card{max-width:400px;padding:40px;text-align:center}h1{color:#ff8c00}p{color:#ffffff99;margin-top:12px}</style></head>
<body><div class="card"><h1>&#10003; Authorized</h1><p>Lumen is connected to DataGrout.<br>You can close this window.</p></div></body></html>"#
    .to_string()
}

fn oauth_error_html(msg: &str) -> String {
    format!(
        r#"<!DOCTYPE html><html><head><title>Authorization Error</title>
<style>body{{font-family:-apple-system,sans-serif;display:flex;justify-content:center;align-items:center;min-height:100vh;margin:0;background:#0a0a0f;color:#fff}}
.card{{max-width:400px;padding:40px;text-align:center}}h1{{color:#ff4444}}p{{color:#ffffff99;margin-top:12px}}</style></head>
<body><div class="card"><h1>Authorization Error</h1><p>{}</p></div></body></html>"#,
        msg
    )
}

fn json_response<T: serde::Serialize>(status: StatusCode, body: &T) -> Response<Full<Bytes>> {
    let json = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(json)))
        .unwrap()
}

fn parse_query_param(query: &str, key: &str) -> Option<usize> {
    parse_query_string(query, key).and_then(|v| v.parse().ok())
}

fn parse_query_string(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            Some(v.to_string())
        } else {
            None
        }
    })
}
