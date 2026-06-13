mod aggregator;
mod api;
mod ca;
mod conduit;
mod parser;
mod pricing;
mod proxy;
#[cfg(feature = "passive")]
mod sniffer;
mod state;
mod sync;
mod tls;
mod traffic;
#[cfg(target_os = "macos")]
mod nat_lookup;
#[cfg(target_os = "macos")]
mod transparent;

use state::AppState;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

fn parse_port_arg(args: &[String], flag: &str, default: u16) -> u16 {
    args.windows(2)
        .find(|w| w[0] == flag)
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(default)
}

/// Used by macOS-only `--transparent` and passive-feature `--passive` flag
/// detection. On Windows (no macOS, no passive feature), nothing calls this
/// — the `allow(dead_code)` keeps the build quiet rather than threading
/// duplicate `#[cfg]` predicates through the call sites.
#[allow(dead_code)]
fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

// rustls crypto backend — selected at compile time via Cargo features:
//   crypto-ring (default)  : pure Rust+asm, no C toolchain, cross-compiles freely
//   crypto-aws-lc          : AWS-LC backend, required for FIPS 140-3 environments
// Both backends are runtime-equivalent for everything lumen-core does (TLS MITM,
// reqwest outbound). See README "Crypto Backend" for trade-offs.
// pub so tests in submodules can call it (binary crates expose `crate::*` to
// their own #[cfg(test)] modules).
#[cfg(all(feature = "crypto-ring", not(feature = "crypto-aws-lc")))]
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(feature = "crypto-aws-lc")]
pub fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

#[cfg(not(any(feature = "crypto-ring", feature = "crypto-aws-lc")))]
compile_error!(
    "lumen-core requires exactly one crypto backend feature: \
     `crypto-ring` (default) or `crypto-aws-lc`"
);

#[tokio::main]
async fn main() {
    install_crypto_provider();

    // RUST_LOG takes full control when set; otherwise default to info for lumen_core.
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("lumen_core=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args: Vec<String> = std::env::args().collect();
    let api_port = parse_port_arg(&args, "--api-port", 9091);
    let proxy_port = parse_port_arg(&args, "--proxy-port", 9090);
    #[cfg(target_os = "macos")]
    let transparent_port = parse_port_arg(&args, "--transparent-port", 9443);
    #[cfg(target_os = "macos")]
    let transparent_enabled = has_flag(&args, "--transparent");
    #[cfg(feature = "passive")]
    let passive_enabled = has_flag(&args, "--passive");
    #[cfg(feature = "passive")]
    let passive_interface = args
        .windows(2)
        .find(|w| w[0] == "--passive-iface")
        .map(|w| w[1].clone());

    let pricing = pricing::loader::load_pricing();
    let aggregator = Arc::new(aggregator::Aggregator::new(pricing));

    let ca = ca::LumenCA::load_or_generate().expect("Failed to initialize CA");
    tracing::info!(
        "CA loaded: {}",
        ca::LumenCA::cert_path().unwrap_or_default().display()
    );

    let app_state = Arc::new(AppState::new(aggregator, ca));

    app_state.proxy_config.write().port = proxy_port;
    *app_state.api_port.write() = api_port;

    let shutdown_rx = app_state.shutdown_rx.clone();

    let api_state = app_state.clone();
    let api_handle = tokio::spawn(async move {
        if let Err(e) = api::start_api_server(api_state, api_port, shutdown_rx).await {
            tracing::error!("API server error: {}", e);
        }
    });

    let proxy_state = app_state.clone();
    let proxy_handle = tokio::spawn(async move {
        let port = proxy_state.proxy_config.read().port;
        let proxy = Arc::new(proxy::LumenProxy::new(
            proxy_state.aggregator.clone(),
            proxy_state.traffic_log.clone(),
            proxy_state.cert_cache.clone(),
            proxy_state.sample_capture.clone(),
            proxy_state.body_limits.clone(),
            port,
        ));
        let proxy_clone = proxy.clone();

        *proxy_state.proxy.write() = Some(proxy);
        proxy_state.proxy_config.write().running = true;

        if let Err(e) = proxy_clone.start().await {
            tracing::error!("Proxy error: {}", e);
        }
    });

    // Transparent proxy (requires root, opt-in via --transparent, macOS only)
    #[cfg(target_os = "macos")]
    if transparent_enabled {
        {
            let mut tc = app_state.transparent_config.write();
            tc.enabled = true;
            tc.port = transparent_port;
            tc.mode = "local".to_string();
        }

        let tp_state = app_state.clone();
        tokio::spawn(async move {
            let tp = Arc::new(transparent::TransparentProxy::new(
                tp_state.aggregator.clone(),
                tp_state.traffic_log.clone(),
                tp_state.cert_cache.clone(),
                tp_state.sample_capture.clone(),
                tp_state.body_limits.clone(),
                transparent_port,
            ));
            tp_state.transparent_config.write().running = true;

            if let Err(e) = tp.start().await {
                tracing::error!("Transparent proxy error: {}", e);
                tp_state.transparent_config.write().running = false;
            }
        });
        tracing::info!("Transparent capture enabled on port {}", transparent_port);
    }

    let sync_state = app_state.clone();
    let sync_shutdown_rx = app_state.shutdown_rx.clone();
    tokio::spawn(async move {
        let syncer = sync::DGSyncer::new(
            sync_state.aggregator.clone(),
            sync_state.dg_server_url.clone(),
            sync_state.dg_http_client.clone(),
            sync_state.dg_bearer_token.clone(),
            sync_state.dg_sync_token.clone(),
            sync_state.dg_lumen_sub_id.clone(),
            sync_state.dg_identity.clone(),
            sync_state.dg_cert_status.clone(),
        );
        syncer.start(sync_shutdown_rx).await;
    });

    // Passive packet capture (requires root/BPF, opt-in via --passive)
    #[cfg(feature = "passive")]
    if passive_enabled {
        let sniff = Arc::new(sniffer::PassiveSniffer::new(
            app_state.aggregator.clone(),
            app_state.traffic_log.clone(),
            passive_interface,
        ));
        tokio::spawn(async move {
            if let Err(e) = sniff.start().await {
                tracing::error!("Passive sniffer error: {}", e);
            }
        });
        tracing::info!("Passive packet capture enabled");
    }

    // `mut` is required on macOS / with the `passive` feature; without those,
    // nothing pushes to the vec — silence the warning rather than splitting
    // into platform-specific bindings.
    #[allow(unused_mut)]
    let mut mode_parts = vec![
        format!("proxy :{}", proxy_port),
        format!("API :{}", api_port),
    ];
    #[cfg(target_os = "macos")]
    if transparent_enabled {
        mode_parts.push(format!("transparent :{}", transparent_port));
    }
    #[cfg(feature = "passive")]
    if passive_enabled {
        mode_parts.push("passive".to_string());
    }
    tracing::info!("Lumen core started — {}", mode_parts.join(", "));

    let sigterm_state = app_state.clone();
    let sigterm_handle = tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl-c");
        tracing::info!("Received SIGINT/SIGTERM, initiating shutdown");
        let _ = sigterm_state.shutdown_tx.send(true);
    });

    #[cfg(unix)]
    {
        let unix_state = app_state.clone();
        tokio::spawn(async move {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to listen for SIGTERM");
            sigterm.recv().await;
            tracing::info!("Received SIGTERM, initiating shutdown");
            let _ = unix_state.shutdown_tx.send(true);
        });
    }

    let mut shutdown_rx = app_state.shutdown_rx.clone();
    tokio::select! {
        _ = api_handle => tracing::info!("API server exited"),
        _ = proxy_handle => tracing::info!("Proxy exited"),
        _ = sigterm_handle => tracing::info!("Signal handler triggered"),
        _ = async {
            loop {
                if shutdown_rx.changed().await.is_err() {
                    break;
                }
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        } => {
            tracing::info!("Shutdown signal received, exiting");
        }
    }

    // Cleanup: flush pf anchor rules if transparent mode was enabled (macOS only)
    #[cfg(target_os = "macos")]
    if transparent_enabled {
        tracing::info!("Cleaning up pf anchor rules...");
        let cleanup = std::process::Command::new("pfctl")
            .args(["-a", "com.datagrout.lumen", "-F", "all"])
            .output();
        match cleanup {
            Ok(out) if out.status.success() => {
                tracing::info!("pf anchor com.datagrout.lumen flushed");
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                tracing::warn!(
                    "pf cleanup returned non-zero (may need root): {}",
                    stderr.trim()
                );
            }
            Err(e) => {
                tracing::warn!("pf cleanup failed (may need root): {}", e);
            }
        }
    }

    tracing::info!("Lumen core shut down cleanly");
}
