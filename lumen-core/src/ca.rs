use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SerialNumber,
};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const CA_DIR: &str = ".lumen";
const CA_CERT_FILE: &str = "ca.pem";
const CA_KEY_FILE: &str = "ca-key.pem";

pub struct LumenCA {
    pub cert_pem: String,
    pub key_pem: String,
    key_pair: KeyPair,
}

impl LumenCA {
    pub fn load_or_generate() -> Result<Self> {
        let dir = ca_dir()?;
        let cert_path = dir.join(CA_CERT_FILE);
        let key_path = dir.join(CA_KEY_FILE);

        if cert_path.exists() && key_path.exists() {
            let needs_regen = cert_path
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|mtime| mtime.elapsed().ok())
                .map(|age| age.as_secs() > 364 * 24 * 3600)
                .unwrap_or(false);

            if !needs_regen {
                info!("Loading existing CA from {}", dir.display());
                return Self::load(&cert_path, &key_path);
            }
            warn!("CA certificate is nearing expiry — regenerating. You will need to re-trust the new cert.");
        }

        info!("Generating new CA in {}", dir.display());
        let ca = Self::generate()?;
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&cert_path, &ca.cert_pem)?;
        std::fs::write(&key_path, &ca.key_pem)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }

        info!("CA certificate written to {}", cert_path.display());
        Ok(ca)
    }

    fn generate() -> Result<Self> {
        let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;

        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, "Lumen Local CA");
        params
            .distinguished_name
            .push(DnType::OrganizationName, "Lumen by DataGrout");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
        ];
        params.serial_number = Some(rand_serial());
        // 1-year validity — short-lived for a local dev tool.
        // Users re-generate by deleting ~/.lumen/ and restarting.
        let now = chrono::Utc::now();
        let not_before = now - chrono::Duration::hours(1); // small backdate for clock skew
        let not_after = now + chrono::Duration::days(365);
        params.not_before = rcgen::date_time_ymd(
            not_before.format("%Y").to_string().parse().unwrap_or(2026),
            not_before.format("%m").to_string().parse().unwrap_or(1),
            not_before.format("%d").to_string().parse().unwrap_or(1),
        );
        params.not_after = rcgen::date_time_ymd(
            not_after.format("%Y").to_string().parse().unwrap_or(2027),
            not_after.format("%m").to_string().parse().unwrap_or(1),
            not_after.format("%d").to_string().parse().unwrap_or(1),
        );

        let cert = params.self_signed(&key_pair)?;
        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();

        Ok(Self {
            cert_pem,
            key_pem,
            key_pair,
        })
    }

    fn load(cert_path: &Path, key_path: &Path) -> Result<Self> {
        let cert_pem =
            std::fs::read_to_string(cert_path).context("Failed to read CA certificate")?;
        let key_pem = std::fs::read_to_string(key_path).context("Failed to read CA key")?;
        let key_pair = KeyPair::from_pem(&key_pem).context("Failed to parse CA key")?;

        Ok(Self {
            cert_pem,
            key_pem,
            key_pair,
        })
    }

    /// Issue a leaf certificate for the given hostname, signed by this CA.
    pub fn issue_leaf(&self, hostname: &str) -> Result<(String, String)> {
        let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;

        let mut leaf_params = CertificateParams::new(vec![hostname.to_string()])?;
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, hostname);
        leaf_params.serial_number = Some(rand_serial());
        leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        // Leaf certs valid for 30 days, backdated 1 hour for clock skew
        let now = chrono::Utc::now();
        let leaf_before = now - chrono::Duration::hours(1);
        let leaf_after = now + chrono::Duration::days(30);
        leaf_params.not_before = rcgen::date_time_ymd(
            leaf_before.format("%Y").to_string().parse().unwrap_or(2026),
            leaf_before.format("%m").to_string().parse().unwrap_or(1),
            leaf_before.format("%d").to_string().parse().unwrap_or(1),
        );
        leaf_params.not_after = rcgen::date_time_ymd(
            leaf_after.format("%Y").to_string().parse().unwrap_or(2027),
            leaf_after.format("%m").to_string().parse().unwrap_or(1),
            leaf_after.format("%d").to_string().parse().unwrap_or(1),
        );

        // Re-create CA params for signing (rcgen consumes params on sign)
        let mut ca_params = CertificateParams::default();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Lumen Local CA");
        ca_params
            .distinguished_name
            .push(DnType::OrganizationName, "Lumen by DataGrout");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
        ];
        let ca_cert = ca_params.self_signed(&self.key_pair)?;

        let leaf_cert = leaf_params.signed_by(&leaf_key, &ca_cert, &self.key_pair)?;
        Ok((leaf_cert.pem(), leaf_key.serialize_pem()))
    }

    pub fn cert_path() -> Result<PathBuf> {
        Ok(ca_dir()?.join(CA_CERT_FILE))
    }

    /// Generate an in-memory CA without touching the filesystem.
    #[cfg(test)]
    pub fn generate_ephemeral() -> Result<Self> {
        Self::generate()
    }
}

fn ca_dir() -> Result<PathBuf> {
    let home = dirs_path()?;
    Ok(home.join(CA_DIR))
}

fn dirs_path() -> Result<PathBuf> {
    crate::state::home_dir()
        .context("neither HOME nor USERPROFILE environment variable is set")
}

fn rand_serial() -> SerialNumber {
    let bytes: [u8; 16] = rand_bytes();
    SerialNumber::from_slice(&bytes)
}

fn rand_bytes() -> [u8; 16] {
    use rand::RngCore;
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests that mutate HOME must not run concurrently — env vars are process-global.
    static HOME_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_generate_ca() {
        let ca = LumenCA::generate().unwrap();
        assert!(ca.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca.key_pem.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn test_issue_leaf() {
        let ca = LumenCA::generate().unwrap();
        let (cert_pem, key_pem) = ca.issue_leaf("api.openai.com").unwrap();
        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(key_pem.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn test_different_hosts_different_certs() {
        let ca = LumenCA::generate().unwrap();
        let (cert1, _) = ca.issue_leaf("api.openai.com").unwrap();
        let (cert2, _) = ca.issue_leaf("api.anthropic.com").unwrap();
        assert_ne!(cert1, cert2);
    }

    #[test]
    fn test_rand_bytes_produces_nonzero_output() {
        let bytes = rand_bytes();
        // All-zeros from a CSPRNG is astronomically unlikely; if it happens the RNG is broken.
        assert_ne!(bytes, [0u8; 16]);
    }

    #[test]
    fn test_rand_bytes_produces_unique_values() {
        // Two independent calls should not collide.
        let a = rand_bytes();
        let b = rand_bytes();
        assert_ne!(a, b);
    }

    #[test]
    fn test_rand_serial_produces_valid_serial() {
        let serial = rand_serial();
        // SerialNumber serialises to a hex string via Debug; just confirm it's non-trivial.
        let dbg = format!("{:?}", serial);
        assert!(!dbg.is_empty());
    }

    #[test]
    fn test_load_or_generate_uses_tempdir() {
        let _guard = HOME_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", dir.path());

        let result = LumenCA::load_or_generate();

        // Restore HOME before asserting so failures don't poison other tests.
        match original_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }

        let ca = result.expect("load_or_generate should succeed");
        assert!(ca.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca.key_pem.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn test_load_or_generate_skips_regen_for_fresh_cert() {
        let _guard = HOME_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", dir.path());

        // First call generates the cert.
        let ca1 = LumenCA::load_or_generate().expect("first generate");

        // Second call should load (not regenerate) because the file is fresh.
        let ca2 = LumenCA::load_or_generate().expect("second load");

        match original_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }

        assert_eq!(ca1.cert_pem, ca2.cert_pem);
    }

    #[test]
    fn test_load_or_generate_regenerates_old_cert() {
        use std::time::{Duration, SystemTime};
        let _guard = HOME_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", dir.path());

        // Generate a fresh CA first.
        let ca1 = LumenCA::load_or_generate().expect("first generate");

        // Back-date the cert file mtime to 366 days ago.
        let ca_path = dir.path().join(".lumen").join("ca.pem");
        let old_mtime = SystemTime::now() - Duration::from_secs(366 * 24 * 3600);
        filetime::set_file_mtime(&ca_path, filetime::FileTime::from_system_time(old_mtime))
            .expect("set mtime");

        // Next call should regenerate because mtime is >364 days old.
        let ca2 = LumenCA::load_or_generate().expect("regen");

        match original_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }

        assert_ne!(
            ca1.cert_pem, ca2.cert_pem,
            "CA should be regenerated when near expiry"
        );
    }
}
