/// Conduit identity, mTLS support, and DCR OAuth flow for DataGrout sync.
///
/// On first use the Swift UI guides the user through OAuth, then calls
/// `POST /dg/bootstrap` with a short-lived Bearer token.  This module
/// generates an ECDSA P-256 keypair, registers it with the DG Substrate
/// endpoint, and persists the DG-signed cert to `~/.conduit/`.
///
/// On subsequent starts the identity is loaded automatically.  All sync
/// calls use the cert for mTLS; no Bearer token is transmitted again.
use anyhow::{bail, Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use serde::Deserialize;
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, info, warn};

// ─── Identity ────────────────────────────────────────────────────────────────

/// DG-CA-signed client identity for mTLS.  Loaded from `~/.conduit/` or
/// created via `register_with_dg()`.
#[derive(Clone)]
pub struct ConduitIdentity {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub ca_pem: Option<Vec<u8>>,
    pub sub_id: Option<String>,
    /// Stable per-device HMAC token returned by DG `bootstrap_identity`.
    /// Used as Bearer since CloudFront strips mTLS client certs.
    pub sync_token: Option<String>,
}

impl ConduitIdentity {
    /// Attempt to load an identity from the standard discovery chain:
    /// 1. `CONDUIT_IDENTITY_DIR` env var
    /// 2. `~/.conduit/`
    /// 3. `.conduit/` relative to cwd
    pub fn try_load() -> Option<Self> {
        if let Ok(dir_str) = std::env::var("CONDUIT_IDENTITY_DIR") {
            if let Some(id) = Self::try_load_from_dir(PathBuf::from(dir_str)) {
                return Some(id);
            }
        }

        if let Some(home) = home_dir() {
            if let Some(id) = Self::try_load_from_dir(home.join(".conduit")) {
                return Some(id);
            }
        }

        if let Ok(cwd) = std::env::current_dir() {
            if let Some(id) = Self::try_load_from_dir(cwd.join(".conduit")) {
                return Some(id);
            }
        }

        debug!("conduit: no mTLS identity found");
        None
    }

    fn try_load_from_dir(dir: PathBuf) -> Option<Self> {
        let cert_path = dir.join("identity.pem");
        let key_path = dir.join("identity_key.pem");

        if !cert_path.exists() || !key_path.exists() {
            return None;
        }

        let cert_pem = fs::read(&cert_path)
            .map_err(|e| warn!("conduit: cannot read {}: {}", cert_path.display(), e))
            .ok()?;
        let key_pem = fs::read(&key_path)
            .map_err(|e| warn!("conduit: cannot read {}: {}", key_path.display(), e))
            .ok()?;
        let ca_pem = fs::read(dir.join("ca.pem")).ok();

        let sub_id_path = dir.join("sub_id.txt");
        let sub_id = fs::read_to_string(&sub_id_path)
            .ok()
            .map(|s| s.trim().to_string());

        debug!("conduit: loaded mTLS identity from {}", dir.display());
        Some(Self {
            cert_pem,
            key_pem,
            ca_pem,
            sub_id,
            sync_token: None, // loaded from dg_config.json, not from conduit dir
        })
    }

    /// Build an mTLS-capable `reqwest::Client` using this identity.
    pub fn build_client(&self) -> Result<reqwest::Client> {
        // reqwest (rustls) needs key first, then cert in a single PEM blob.
        let mut combined = self.key_pem.clone();
        if !combined.ends_with(b"\n") {
            combined.push(b'\n');
        }
        combined.extend_from_slice(&self.cert_pem);

        let identity = reqwest::Identity::from_pem(&combined)
            .context("failed to build mTLS identity from PEM")?;

        // Explicitly request the rustls backend — this crate is compiled with both
        // native-tls (reqwest default) and rustls-tls.  Without use_rustls_tls(),
        // builder.build() uses native-tls which cannot accept a PEM identity.
        let mut builder = reqwest::Client::builder()
            .use_rustls_tls()
            .identity(identity)
            .timeout(Duration::from_secs(30));

        if let Some(ca_pem) = &self.ca_pem {
            let ca_cert = reqwest::Certificate::from_pem(ca_pem)
                .context("failed to parse DG CA certificate")?;
            builder = builder.add_root_certificate(ca_cert);
        }

        builder
            .build()
            .context("failed to build mTLS reqwest client")
    }

    /// Persist this identity to `~/.conduit/`.  Overwrites any existing files.
    fn save_to_dir(dir: &PathBuf, sub_id: &str, cert: &str, key: &str, ca: &str) -> Result<()> {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;

        write_file_private(dir.join("identity.pem"), cert.as_bytes())?;
        write_file_private(dir.join("identity_key.pem"), key.as_bytes())?;
        fs::write(dir.join("ca.pem"), ca.as_bytes()).context("cannot write ca.pem")?;
        fs::write(dir.join("sub_id.txt"), sub_id).context("cannot write sub_id.txt")?;

        info!(
            "conduit: saved identity sub_id={} to {}",
            sub_id,
            dir.display()
        );
        Ok(())
    }
}

// ─── Registration ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RegisterResponse {
    id: String,
    cert_pem: String,
    ca_cert_pem: String,
    #[serde(default)]
    sync_token: Option<String>,
}

/// Generate a fresh ECDSA P-256 keypair, register it with the DG Substrate
/// endpoint using `bearer_token`, persist to `~/.conduit/`, and return the
/// ready-to-use `ConduitIdentity`.
pub async fn register_with_dg(
    server_url: &str,
    bearer_token: &str,
    device_name: &str,
) -> Result<ConduitIdentity> {
    // 1. Generate an ECDSA P-256 keypair.
    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .context("keypair generation failed")?;

    let pub_key_der = key_pair.public_key_der();
    let pub_key_pem = format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
        base64_encode_chunks(&pub_key_der)
    );

    // Private key in PKCS#8 PEM format.
    let key_pem_str = key_pair.serialize_pem();

    // Temporary self-signed cert so we have a valid PEM cert for the SDK
    // compat path — DG only needs the public key, but we generate one anyway.
    let mut params = CertificateParams::default();
    params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, device_name);
        dn
    };
    let temp_cert = params
        .self_signed(&key_pair)
        .context("temp cert generation failed")?;
    let _ = temp_cert.pem(); // just to ensure it parses

    // 2. Register the public key with DG.
    let endpoint = format!(
        "{}/api/v1/substrate/identity/register",
        server_url.trim_end_matches('/')
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let resp = client
        .post(&endpoint)
        .bearer_auth(bearer_token)
        .json(&serde_json::json!({
            "public_key_pem": pub_key_pem,
            "name": device_name,
        }))
        .send()
        .await
        .context("DG registration request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("DG registration returned {}: {}", status, body);
    }

    let reg: RegisterResponse = resp
        .json()
        .await
        .context("failed to parse DG registration response")?;

    // 3. Persist to ~/.conduit/.
    let dir = home_dir()
        .map(|h| h.join(".conduit"))
        .unwrap_or_else(|| PathBuf::from(".conduit"));

    ConduitIdentity::save_to_dir(&dir, &reg.id, &reg.cert_pem, &key_pem_str, &reg.ca_cert_pem)?;

    info!("conduit: registered sub_id={} at {}", reg.id, server_url);

    Ok(ConduitIdentity {
        cert_pem: reg.cert_pem.into_bytes(),
        key_pem: key_pem_str.into_bytes(),
        ca_pem: Some(reg.ca_cert_pem.into_bytes()),
        sub_id: Some(reg.id),
        sync_token: reg.sync_token,
    })
}

/// Register a Substrate mTLS identity using an OAuth JWT from the DCR flow.
/// Calls `/api/v1/lumen/bootstrap_identity` which accepts the OAuth token
/// instead of the global ARBITER_API_KEY, making zero-config bootstrap possible.
pub async fn register_with_oauth_token(
    auth_root: &str,
    oauth_token: &str,
    device_name: &str,
) -> Result<ConduitIdentity> {
    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .context("keypair generation failed")?;

    let pub_key_der = key_pair.public_key_der();
    let pub_key_pem = format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
        base64_encode_chunks(&pub_key_der)
    );
    let key_pem_str = key_pair.serialize_pem();

    let endpoint = format!(
        "{}/api/v1/lumen/bootstrap_identity",
        auth_root.trim_end_matches('/')
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let resp = client
        .post(&endpoint)
        .bearer_auth(oauth_token)
        .json(&serde_json::json!({
            "public_key_pem": pub_key_pem,
            "name": device_name,
        }))
        .send()
        .await
        .context("DG bootstrap_identity request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("DG bootstrap_identity returned {}: {}", status, body);
    }

    let reg: RegisterResponse = resp
        .json()
        .await
        .context("failed to parse bootstrap_identity response")?;

    let dir = home_dir()
        .map(|h| h.join(".conduit"))
        .unwrap_or_else(|| PathBuf::from(".conduit"));

    ConduitIdentity::save_to_dir(&dir, &reg.id, &reg.cert_pem, &key_pem_str, &reg.ca_cert_pem)?;

    info!("conduit: registered sub_id={} at {}", reg.id, auth_root);

    Ok(ConduitIdentity {
        cert_pem: reg.cert_pem.into_bytes(),
        key_pem: key_pem_str.into_bytes(),
        ca_pem: Some(reg.ca_cert_pem.into_bytes()),
        sub_id: Some(reg.id),
        sync_token: reg.sync_token,
    })
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

// Shared home_dir helper lives in crate::state — local alias keeps this file
// short and avoids drifting from the rest of the codebase if the resolution
// logic ever changes.
fn home_dir() -> Option<PathBuf> {
    crate::state::home_dir()
}

/// Write `content` to `path` with owner-only read/write permissions where
/// the OS supports it. On Unix this means mode 0600; on Windows the file
/// inherits the parent directory's ACL (Windows has no direct equivalent of
/// chmod, and a robust ACL implementation would need the `winapi` crate —
/// not worth it for credential files that live under the user's profile
/// directory which is already access-controlled by the OS).
fn write_file_private(path: PathBuf, content: &[u8]) -> Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600);
    let mut f = opts
        .open(&path)
        .with_context(|| format!("cannot open {} for writing", path.display()))?;
    f.write_all(content)
        .with_context(|| format!("cannot write {}", path.display()))
}

// ─── PKCE + DCR helpers ───────────────────────────────────────────────────────

/// Generate a PKCE (S256) code verifier / challenge pair.
/// Returns `(verifier, challenge)` where both are base64url-encoded.
pub fn generate_pkce() -> (String, String) {
    use rand::RngCore;
    use sha2::{Digest, Sha256};

    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = base64_url_no_pad(&bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = base64_url_no_pad(&hash);

    (verifier, challenge)
}

fn base64_url_no_pad(data: &[u8]) -> String {
    base64_encode_chunks(data)
        .replace('+', "-")
        .replace('/', "_")
        .replace('=', "")
        .replace('\n', "")
}

/// RFC 7591 Dynamic Client Registration — returns the issued `client_id`.
pub async fn dcr_register_client(
    server_url: &str,
    redirect_uri: &str,
    client_name: &str,
) -> Result<String> {
    let endpoint = format!("{}/register", server_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let resp = client
        .post(&endpoint)
        .json(&serde_json::json!({
            "client_name": client_name,
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "scope": "mcp",
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .await
        .context("DCR POST failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("DCR returned {}: {}", status, body);
    }

    let dcr: serde_json::Value = resp.json().await.context("DCR response parse failed")?;
    dcr["client_id"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("DCR response missing client_id"))
}

/// Exchange an OAuth authorization code for a bearer access token.
pub async fn exchange_code_for_token(
    server_url: &str,
    client_id: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<String> {
    let token_endpoint = format!("{}/oauth/token", server_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let resp = client
        .post(&token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .context("token exchange POST failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("token exchange returned {}: {}", status, body);
    }

    let token_resp: serde_json::Value = resp.json().await.context("token response parse failed")?;
    token_resp["access_token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("token response missing access_token"))
}

/// Simple base64 encoder (RFC 4648, with line breaks every 64 chars).
fn base64_encode_chunks(data: &[u8]) -> String {
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut line_len = 0;

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let combined = (b0 << 16) | (b1 << 8) | b2;

        out.push(table[((combined >> 18) & 0x3f) as usize] as char);
        out.push(table[((combined >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            table[((combined >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            table[(combined & 0x3f) as usize] as char
        } else {
            '='
        });

        line_len += 4;
        if line_len >= 64 {
            out.push('\n');
            line_len = 0;
        }
    }
    out
}
