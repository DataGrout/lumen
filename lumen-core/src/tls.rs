use anyhow::Result;
use parking_lot::RwLock;
use rustls::ServerConfig;
use std::collections::HashMap;
use std::io::BufReader;
use std::sync::Arc;

use crate::ca::LumenCA;

const MAX_CACHED_CERTS: usize = 256;

pub struct CertCache {
    ca: Arc<LumenCA>,
    cache: RwLock<HashMap<String, Arc<ServerConfig>>>,
}

impl CertCache {
    pub fn new(ca: Arc<LumenCA>) -> Self {
        Self {
            ca,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn get_or_create(&self, hostname: &str) -> Result<Arc<ServerConfig>> {
        {
            let cache = self.cache.read();
            if let Some(config) = cache.get(hostname) {
                return Ok(config.clone());
            }
        }

        let config = self.create_server_config(hostname)?;
        let config = Arc::new(config);

        {
            let mut cache = self.cache.write();
            if cache.len() >= MAX_CACHED_CERTS {
                // Evict a random entry
                if let Some(key) = cache.keys().next().cloned() {
                    cache.remove(&key);
                }
            }
            cache.insert(hostname.to_string(), config.clone());
        }

        Ok(config)
    }

    fn create_server_config(&self, hostname: &str) -> Result<ServerConfig> {
        let (cert_pem, key_pem) = self.ca.issue_leaf(hostname)?;

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

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_crypto() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
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
