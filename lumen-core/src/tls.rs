use anyhow::Result;
use parking_lot::RwLock;
use rustls::ServerConfig;
use std::collections::HashMap;
use std::io::BufReader;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::ca::LumenCA;

const MAX_CACHED_CERTS: usize = 256;

/// Reissue a cached leaf once it is this close to its `notAfter`.
///
/// Leaves are minted with 30 days of validity, and this cache had no notion of
/// expiry at all — a `HashMap` that only ever evicted to stay under a size cap.
/// The daemon starts at login and is rarely restarted, so on any machine left
/// running for a month the cache went on serving a leaf that had already
/// expired, and every proxied request failed with ERR_CERT_DATE_INVALID until
/// somebody restarted Lumen and silently cleared it.
///
/// A day of margin means a connection never rides a certificate that expires
/// mid-session.
const REISSUE_BEFORE_EXPIRY: Duration = Duration::from_secs(24 * 3600);

struct CachedCert {
    config: Arc<ServerConfig>,
    /// When the leaf inside `config` stops being valid.
    not_after: SystemTime,
}

pub struct CertCache {
    ca: Arc<LumenCA>,
    cache: RwLock<HashMap<String, CachedCert>>,
}

impl CertCache {
    pub fn new(ca: Arc<LumenCA>) -> Self {
        Self {
            ca,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn get_or_create(&self, hostname: &str) -> Result<Arc<ServerConfig>> {
        let cutoff = SystemTime::now() + REISSUE_BEFORE_EXPIRY;

        {
            let cache = self.cache.read();
            if let Some(entry) = cache.get(hostname) {
                if entry.not_after > cutoff {
                    return Ok(entry.config.clone());
                }
                // Expired or about to be: fall through and mint a fresh one,
                // replacing this entry rather than handing out a dead cert.
            }
        }

        let (config, not_after) = self.create_server_config(hostname)?;
        let config = Arc::new(config);

        {
            let mut cache = self.cache.write();
            if cache.len() >= MAX_CACHED_CERTS && !cache.contains_key(hostname) {
                // Prefer dropping something already expired over evicting a live
                // entry at random; fall back to an arbitrary key when all are live.
                let now = SystemTime::now();
                let victim = cache
                    .iter()
                    .find(|(_, e)| e.not_after <= now)
                    .map(|(k, _)| k.clone())
                    .or_else(|| cache.keys().next().cloned());

                if let Some(key) = victim {
                    cache.remove(&key);
                }
            }
            cache.insert(
                hostname.to_string(),
                CachedCert {
                    config: config.clone(),
                    not_after,
                },
            );
        }

        Ok(config)
    }

    /// Build the TLS config for a host, and report when the leaf inside it dies.
    ///
    /// The expiry is read back out of the certificate rather than recomputed as
    /// "now + 30 days": `issue_leaf` truncates `notAfter` to a whole day, so an
    /// assumed window would be up to 24 hours optimistic — long enough to hand
    /// out a dead certificate near the boundary.
    fn create_server_config(&self, hostname: &str) -> Result<(ServerConfig, SystemTime)> {
        let (cert_pem, key_pem) = self.ca.issue_leaf(hostname)?;

        let not_after = leaf_not_after(&cert_pem)
            .ok_or_else(|| anyhow::anyhow!("issued leaf for {hostname} has no readable notAfter"))?;

        let certs: Vec<rustls::pki_types::CertificateDer> = {
            let mut reader = BufReader::new(cert_pem.as_bytes());
            rustls_pemfile::certs(&mut reader).collect::<std::result::Result<Vec<_>, _>>()?
        };

        let key = {
            let mut reader = BufReader::new(key_pem.as_bytes());
            rustls_pemfile::private_key(&mut reader)?
                .ok_or_else(|| anyhow::anyhow!("No private key found in PEM"))?
        };

        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;

        // Advertise both h2 and http/1.1 via ALPN so clients (including gRPC)
        // can negotiate their preferred protocol.
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        Ok((config, not_after))
    }
}

/// `notAfter` of the first certificate in a PEM chain.
fn leaf_not_after(cert_pem: &str) -> Option<SystemTime> {
    use x509_parser::prelude::*;

    let (_, pem) = parse_x509_pem(cert_pem.as_bytes()).ok()?;
    let (_, cert) = parse_x509_certificate(&pem.contents).ok()?;

    let ts = cert.validity().not_after.timestamp();
    if ts < 0 {
        return None;
    }
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(ts as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_crypto() {
        use std::sync::Once;
        static CRYPTO_INIT: Once = Once::new();
        CRYPTO_INIT.call_once(crate::install_crypto_provider);
    }

    #[test]
    fn test_cert_cache_creates_config() {
        init_crypto();
        let ca = Arc::new(LumenCA::generate_ephemeral().unwrap());
        let cache = CertCache::new(ca);
        let config = cache.get_or_create("api.openai.com");
        assert!(config.is_ok());
    }

    #[test]
    fn test_cert_cache_returns_cached() {
        init_crypto();
        let ca = Arc::new(LumenCA::generate_ephemeral().unwrap());
        let cache = CertCache::new(ca);
        let c1 = cache.get_or_create("api.openai.com").unwrap();
        let c2 = cache.get_or_create("api.openai.com").unwrap();
        assert!(Arc::ptr_eq(&c1, &c2));
    }

    #[test]
    fn test_alpn_includes_h2() {
        init_crypto();
        let ca = Arc::new(LumenCA::generate_ephemeral().unwrap());
        let cache = CertCache::new(ca);
        let config = cache.get_or_create("api.openai.com").unwrap();
        assert!(config.alpn_protocols.contains(&b"h2".to_vec()));
        assert!(config.alpn_protocols.contains(&b"http/1.1".to_vec()));
        // h2 should be first (preferred)
        assert_eq!(config.alpn_protocols[0], b"h2");
    }

    #[test]
    fn test_different_hosts_different_configs() {
        init_crypto();
        let ca = Arc::new(LumenCA::generate_ephemeral().unwrap());
        let cache = CertCache::new(ca);
        let c1 = cache.get_or_create("api.openai.com").unwrap();
        let c2 = cache.get_or_create("api.anthropic.com").unwrap();
        assert!(!Arc::ptr_eq(&c1, &c2));
    }
}
