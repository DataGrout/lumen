use std::process::{Child, Command};
use std::sync::Once;
use std::time::Duration;
use tempfile::TempDir;

// Integration tests build as separate test binaries, so they can't reach into
// the bin crate's `install_crypto_provider`. We duplicate the install logic
// here — small enough to be acceptable, and the cfg gates inherit the parent
// crate's `crypto-ring` / `crypto-aws-lc` feature selection automatically.
static CRYPTO_INIT: Once = Once::new();
fn ensure_crypto_installed() {
    CRYPTO_INIT.call_once(|| {
        #[cfg(all(feature = "crypto-ring", not(feature = "crypto-aws-lc")))]
        let _ = rustls::crypto::ring::default_provider().install_default();
        #[cfg(feature = "crypto-aws-lc")]
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

fn find_binary() -> String {
    // EXE_SUFFIX is "" on Unix and ".exe" on Windows. Concatenating it to
    // every candidate path makes the lookup work uniformly — without this,
    // the Windows CI runner couldn't find lumen-core.exe and every
    // integration test panicked at startup.
    let bin = format!("lumen-core{}", std::env::consts::EXE_SUFFIX);

    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        let path = format!("{}/debug/{}", target_dir, bin);
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }

    let targets = [
        format!("target/debug/{}", bin),
        format!("../target/debug/{}", bin),
    ];
    for t in &targets {
        if std::path::Path::new(t).exists() {
            return t.clone();
        }
    }

    let current_exe = std::env::current_exe().ok();
    if let Some(exe) = current_exe {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(&bin);
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }

    panic!(
        "lumen-core binary not found. Run `cargo build` first. Checked: {:?}",
        targets
    );
}

struct DaemonGuard {
    child: Option<Child>,
    home: TempDir,
}

impl DaemonGuard {
    fn new(api_port: u16, proxy_port: u16) -> Self {
        // Install the crypto provider before any reqwest::Client in this test
        // process. Every test creates a DaemonGuard, so routing it through here
        // gives us a single chokepoint without per-test boilerplate.
        ensure_crypto_installed();
        let temp_home = TempDir::new().expect("failed to create temp home");
        let bin = find_binary();
        let child = Command::new(&bin)
            .args([
                "--api-port",
                &api_port.to_string(),
                "--proxy-port",
                &proxy_port.to_string(),
            ])
            .env("HOME", temp_home.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to spawn {}: {}", bin, e));

        DaemonGuard {
            child: Some(child),
            home: temp_home,
        }
    }

    /// Read the API token written by the daemon to <home>/.lumen/api.token.
    /// Retries for up to 3 seconds to handle startup latency.
    fn token(&self) -> String {
        let path = self.home.path().join(".lumen").join("api.token");
        for _ in 0..60 {
            if let Ok(t) = std::fs::read_to_string(&path) {
                let t = t.trim().to_string();
                if t.len() == 64 {
                    return t;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("api.token not found at {:?} after 3 s", path);
    }

    // Only used by `test_sigterm_handling`, which is itself `#[cfg(unix)]`.
    // Gating the method with the same cfg keeps Windows builds warning-free.
    #[cfg(unix)]
    fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    fn kill_sigkill(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
    }

    fn try_wait(&mut self) -> Option<std::process::ExitStatus> {
        self.child
            .as_mut()
            .and_then(|c| c.try_wait().ok().flatten())
    }

    fn wait_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let start = std::time::Instant::now();
        loop {
            if let Some(status) = self.try_wait() {
                self.child = None;
                return Some(status);
            }
            if start.elapsed() > timeout {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Build a reqwest client that sends the API token on every request.
fn authed_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-lumen-token",
        reqwest::header::HeaderValue::from_str(token).expect("token is valid header value"),
    );
    reqwest::ClientBuilder::new()
        .default_headers(headers)
        .build()
        .expect("client build failed")
}

fn ephemeral_ports() -> (u16, u16) {
    let api = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let api_port = api.local_addr().unwrap().port();
    let proxy_port = proxy.local_addr().unwrap().port();
    drop(api);
    drop(proxy);
    (api_port, proxy_port)
}

async fn wait_healthy(api_port: u16, timeout: Duration) -> bool {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/health", api_port);
    let start = std::time::Instant::now();
    loop {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => return true,
            _ => {}
        }
        if start.elapsed() > timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

#[tokio::test]
async fn test_health_check() {
    let (api_port, proxy_port) = ephemeral_ports();
    let _daemon = DaemonGuard::new(api_port, proxy_port);

    assert!(
        wait_healthy(api_port, Duration::from_secs(5)).await,
        "Daemon failed to become healthy within 5 seconds"
    );

    let resp = reqwest::get(format!("http://127.0.0.1:{}/health", api_port))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert!(body["version"].is_string());
}

#[tokio::test]
async fn test_graceful_shutdown() {
    let (api_port, proxy_port) = ephemeral_ports();
    let mut daemon = DaemonGuard::new(api_port, proxy_port);

    assert!(
        wait_healthy(api_port, Duration::from_secs(5)).await,
        "Daemon failed to become healthy"
    );

    let client = authed_client(&daemon.token());
    let resp = client
        .post(format!("http://127.0.0.1:{}/shutdown", api_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let status = daemon
        .wait_exit(Duration::from_secs(5))
        .expect("Daemon didn't exit after /shutdown within 5 seconds");

    assert!(
        status.success(),
        "Daemon exited with non-zero status: {:?}",
        status
    );
}

#[tokio::test]
async fn test_port_cleanup_after_shutdown() {
    let (api_port, proxy_port) = ephemeral_ports();
    let mut daemon = DaemonGuard::new(api_port, proxy_port);

    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    let client = authed_client(&daemon.token());
    client
        .post(format!("http://127.0.0.1:{}/shutdown", api_port))
        .send()
        .await
        .unwrap();

    daemon
        .wait_exit(Duration::from_secs(5))
        .expect("Daemon didn't exit");

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        port_is_free(api_port),
        "API port {} still bound after shutdown",
        api_port
    );
    assert!(
        port_is_free(proxy_port),
        "Proxy port {} still bound after shutdown",
        proxy_port
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_sigterm_handling() {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;

    let (api_port, proxy_port) = ephemeral_ports();
    let mut daemon = DaemonGuard::new(api_port, proxy_port);

    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    let pid = daemon.pid();
    signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM).expect("Failed to send SIGTERM");

    let status = daemon
        .wait_exit(Duration::from_secs(5))
        .expect("Daemon didn't exit after SIGTERM within 5 seconds");

    assert!(
        status.success() || status.code().is_none(),
        "Daemon exited abnormally: {:?}",
        status
    );
}

#[tokio::test]
async fn test_stats_endpoint() {
    let (api_port, proxy_port) = ephemeral_ports();
    let daemon = DaemonGuard::new(api_port, proxy_port);
    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    let client = authed_client(&daemon.token());
    let resp = client
        .get(format!("http://127.0.0.1:{}/stats", api_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["event_count"], 0);
    assert_eq!(body["current_lap"], 1);
    assert!(body["total_cost"].is_number());
}

#[tokio::test]
async fn test_lap_endpoints() {
    let (api_port, proxy_port) = ephemeral_ports();
    let daemon = DaemonGuard::new(api_port, proxy_port);
    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    let client = authed_client(&daemon.token());

    let resp = client
        .post(format!("http://127.0.0.1:{}/lap", api_port))
        .json(&serde_json::json!({"label": "test lap"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let snap: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(snap["lap_number"], 1);
    assert_eq!(snap["label"], "test lap");
    assert_eq!(snap["event_count"], 0);

    let resp = client
        .post(format!("http://127.0.0.1:{}/lap", api_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let snap: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(snap["lap_number"], 2);
    assert_eq!(snap["label"], "Lap 2");

    let resp = client
        .get(format!("http://127.0.0.1:{}/laps", api_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let laps: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(laps.len(), 2);

    let stats: serde_json::Value = client
        .get(format!("http://127.0.0.1:{}/stats", api_port))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stats["current_lap"], 3);
}

#[tokio::test]
async fn test_traffic_endpoints() {
    let (api_port, proxy_port) = ephemeral_ports();
    let daemon = DaemonGuard::new(api_port, proxy_port);
    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    let client = authed_client(&daemon.token());

    let resp = client
        .get(format!("http://127.0.0.1:{}/traffic", api_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let entries: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert!(entries.is_empty());

    let resp = client
        .get(format!("http://127.0.0.1:{}/traffic/hosts", api_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let hosts: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert!(hosts.is_empty());
}

#[tokio::test]
async fn test_config_endpoints() {
    let (api_port, proxy_port) = ephemeral_ports();
    let daemon = DaemonGuard::new(api_port, proxy_port);
    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    let client = authed_client(&daemon.token());

    let resp = client
        .get(format!("http://127.0.0.1:{}/config", api_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let cfg: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(cfg["enabled"], false);
    assert_eq!(cfg["tools_hidden"], false);

    let new_cfg = serde_json::json!({
        "enabled": true,
        "server_url": "https://dg.example.com",
        "tools_hidden": true,
        "intelligent_interface": false
    });
    let resp = client
        .put(format!("http://127.0.0.1:{}/config", api_port))
        .json(&new_cfg)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let updated: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(updated["enabled"], true);
    assert_eq!(updated["tools_hidden"], true);
    assert_eq!(updated["server_url"], "https://dg.example.com");
}

#[tokio::test]
async fn test_unknown_route_404() {
    let (api_port, proxy_port) = ephemeral_ports();
    let daemon = DaemonGuard::new(api_port, proxy_port);
    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    let client = authed_client(&daemon.token());
    let resp = client
        .get(format!("http://127.0.0.1:{}/nonexistent", api_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_routes_api() {
    let (api_port, proxy_port) = ephemeral_ports();
    let daemon = DaemonGuard::new(api_port, proxy_port);
    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    let client = authed_client(&daemon.token());

    // Default routes should include openai, anthropic, google
    let resp = client
        .get(format!("http://127.0.0.1:{}/routes", api_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let routes: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(routes.len(), 3);
    assert!(routes
        .iter()
        .any(|r| r["prefix"] == "/openai" && r["upstream"] == "https://api.openai.com"));

    // Add a custom route
    let resp = client
        .post(format!("http://127.0.0.1:{}/routes", api_port))
        .json(&serde_json::json!({"prefix": "/local", "upstream": "http://localhost:11434"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let routes: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(routes.len(), 4);

    // Delete the custom route
    let resp = client
        .delete(format!("http://127.0.0.1:{}/routes/local", api_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let routes: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(routes.len(), 3);
}

#[tokio::test]
async fn test_relay_no_route_returns_404() {
    let (api_port, proxy_port) = ephemeral_ports();
    let _daemon = DaemonGuard::new(api_port, proxy_port);
    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    // Send a request directly to the proxy port with no matching route
    let resp = reqwest::get(format!("http://127.0.0.1:{}/unknown/v1/foo", proxy_port))
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_relay_openai_forwards() {
    let (api_port, proxy_port) = ephemeral_ports();
    let daemon = DaemonGuard::new(api_port, proxy_port);
    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    // Send a request to the /openai relay route (proxy port — no auth required)
    // This will fail upstream (no valid API key) but should get a proper HTTP error,
    // not a 404 — proving the relay resolved and forwarded
    let plain = reqwest::Client::new();
    let resp = plain
        .post(format!(
            "http://127.0.0.1:{}/openai/v1/chat/completions",
            proxy_port
        ))
        .header("content-type", "application/json")
        .header("authorization", "Bearer sk-test-invalid")
        .body(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .unwrap();

    // OpenAI returns 401 for invalid keys — anything other than 404/502 proves relay worked
    assert_ne!(resp.status().as_u16(), 404, "Route should have resolved");
    let status = resp.status().as_u16();
    assert!(
        status == 401 || status == 403 || status == 429,
        "Expected auth error from OpenAI, got {}",
        status
    );

    // Verify traffic was logged (API port — requires token)
    tokio::time::sleep(Duration::from_millis(200)).await;
    let client = authed_client(&daemon.token());
    let traffic: Vec<serde_json::Value> = client
        .get(format!("http://127.0.0.1:{}/traffic", api_port))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !traffic.is_empty(),
        "Traffic log should have the relayed request"
    );
    let entry = &traffic[0];
    assert_eq!(entry["host"], "api.openai.com");
    assert_eq!(entry["is_monitored"], true);
}

#[tokio::test]
async fn test_crash_recovery_ports_freed() {
    let (api_port, proxy_port) = ephemeral_ports();
    let mut daemon = DaemonGuard::new(api_port, proxy_port);

    assert!(wait_healthy(api_port, Duration::from_secs(5)).await);

    daemon.kill_sigkill();

    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(
        port_is_free(api_port),
        "API port {} still bound after SIGKILL",
        api_port
    );
    assert!(
        port_is_free(proxy_port),
        "Proxy port {} still bound after SIGKILL",
        proxy_port
    );
}
