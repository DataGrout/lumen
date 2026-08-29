use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SerialNumber,
};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const CA_DIR: &str = ".lumen";
const CA_CERT_FILE: &str = "ca.pem";
const CA_KEY_FILE: &str = "ca-key.pem";

/// Rotate the CA this long before its `notAfter`. A month is comfortably longer
/// than any plausible gap between daemon restarts, so the rotation happens while
/// the old certificate still works rather than after it has already broken.
const REGEN_BEFORE_EXPIRY: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);

/// How usable the local CA is for signing leaves clients will accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaHealth {
    Healthy,
    /// Still valid, but inside the rotation window — renew at next opportunity.
    ExpiringSoon,
    /// Past `notAfter`. Every leaf it signs is rejected as ERR_CERT_DATE_INVALID.
    Expired,
    /// Cannot be parsed. Treated as usable: a parser quirk must not be grounds
    /// for disabling capture on a CA that clients may be accepting perfectly well.
    Unreadable,
}

impl CaHealth {
    /// Can this CA still sign leaves worth presenting to a client?
    pub fn usable(self) -> bool {
        !matches!(self, CaHealth::Expired)
    }
}

pub struct LumenCA {
    pub cert_pem: String,
    pub key_pem: String,
    key_pair: KeyPair,
    // The CA certificate used as the issuer when signing leaves. rcgen consumes
    // `CertificateParams` on sign, so this used to be rebuilt and re-self-signed
    // on every `issue_leaf` call — wasted crypto on the proxy hot path during
    // the cold-start cert storm. Built once, lazily, and reused.
    signing_cert: std::sync::OnceLock<rcgen::Certificate>,
}

impl LumenCA {
    pub fn load_or_generate() -> Result<Self> {
        let dir = ca_dir()?;
        let cert_path = dir.join(CA_CERT_FILE);
        let key_path = dir.join(CA_KEY_FILE);

        if cert_path.exists() && key_path.exists() {
            let existing = Self::load(&cert_path, &key_path);

            // Ask the CERTIFICATE when it expires, not the filesystem.
            //
            // This used to read the file's mtime and assume "modified < 364 days
            // ago" meant "still valid", which is a different fact and routinely a
            // false one. Restoring ~/.lumen from a backup, copying it to a new Mac,
            // or any tool that rewrites the file gives a year-old certificate a
            // fresh mtime — so the check passed, the expired CA was loaded, and
            // every proxied request failed with ERR_CERT_DATE_INVALID while Lumen
            // reported itself healthy. It also failed OPEN: an unreadable mtime, or
            // one in the future from clock skew, resolved to "not stale".
            match existing {
                Ok(ca) => {
                    match ca.expires_within(REGEN_BEFORE_EXPIRY) {
                        // Parse failure is deliberately NOT a reason to regenerate:
                        // discarding a working CA (and its trust) over a parser
                        // quirk is worse than the staleness it would avoid.
                        None | Some(false) => {
                            info!("Loading existing CA from {}", dir.display());
                            return Ok(ca);
                        }
                        Some(true) => {
                            warn!(
                                "CA certificate at {} is expired or within {} days of it — \
                                 regenerating. You will need to re-trust the new certificate.",
                                cert_path.display(),
                                REGEN_BEFORE_EXPIRY.as_secs() / 86_400
                            );
                        }
                    }
                }
                Err(e) => {
                    warn!("Existing CA at {} is unreadable ({e}) — regenerating.", dir.display());
                }
            }
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

    /// Mint a new CA and write it over the existing one.
    ///
    /// Exposed so the app can offer "renew" when the certificate has expired,
    /// rather than the user's only route being to delete `~/.lumen` by hand from
    /// a terminal — which is what the situation previously demanded and what
    /// nobody who hits it is in a position to know.
    ///
    /// Writing the files is only half the repair. The new CA is untrusted until
    /// the system trust store is updated, which needs the user's password, so the
    /// caller must follow this with the trust step and a daemon restart to load it.
    pub fn regenerate() -> Result<()> {
        let dir = ca_dir()?;
        let ca = Self::generate()?;

        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(CA_CERT_FILE), &ca.cert_pem)?;
        let key_path = dir.join(CA_KEY_FILE);
        std::fs::write(&key_path, &ca.key_pem)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }

        info!("CA regenerated at {} — it must now be re-trusted", dir.display());
        Ok(())
    }

    /// What state is this CA in right now?
    ///
    /// The distinction that matters is `Expired` vs everything else: an expired
    /// CA cannot sign a leaf any client will accept, so continuing to MITM with
    /// it does not degrade capture — it breaks the user's connection entirely.
    /// See `CertCache::ca_usable`, which turns this into a decision to stop
    /// intercepting rather than to keep failing.
    pub fn health(&self) -> CaHealth {
        match self.not_after() {
            None => CaHealth::Unreadable,
            Some(not_after) => {
                let now = std::time::SystemTime::now();
                if not_after <= now {
                    CaHealth::Expired
                } else if not_after <= now + REGEN_BEFORE_EXPIRY {
                    CaHealth::ExpiringSoon
                } else {
                    CaHealth::Healthy
                }
            }
        }
    }

    /// The certificate's own `notAfter`, or `None` when it cannot be parsed.
    pub fn not_after(&self) -> Option<std::time::SystemTime> {
        use x509_parser::prelude::*;

        let (_, pem) = parse_x509_pem(self.cert_pem.as_bytes()).ok()?;
        let (_, cert) = parse_x509_certificate(&pem.contents).ok()?;

        let ts = cert.validity().not_after.timestamp();
        if ts < 0 {
            return None;
        }
        Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(ts as u64))
    }

    /// Is this CA already expired, or close enough that it will expire before a
    /// user plausibly restarts the daemon again?
    ///
    /// `None` when the certificate cannot be parsed — the caller treats that as
    /// "keep using it", since a working CA is worth more than a speculative
    /// rotation that costs the user their keychain trust.
    pub fn expires_within(&self, window: std::time::Duration) -> Option<bool> {
        let not_after = self.not_after()?;
        Some(not_after <= std::time::SystemTime::now() + window)
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
        // Ten-year validity, because renewal is not free for the user.
        //
        // A renewed CA is a NEW key pair and therefore a different certificate
        // with a different fingerprint, and macOS trust settings are per
        // certificate — so trust never carries across a renewal. Every rotation
        // costs the user an admin-password prompt. At 365 days that was one
        // forced re-approval per user per year, arriving without warning as a
        // TLS error mid-task, in exchange for very little: this root never
        // leaves the machine, and anyone able to read ~/.lumen/ca.key (0600)
        // already has the account and could install their own root anyway. The
        // answer to a compromised key is deleting it, not waiting out a year.
        //
        // This also matches what earlier builds issued — CAs minted in 2024 run
        // to 2034 — so long-lived local roots are the behaviour this codebase
        // already had in the field and users are already relying on.
        let now = chrono::Utc::now();
        let not_before = now - chrono::Duration::hours(1); // small backdate for clock skew
        let not_after = now + chrono::Duration::days(3650);
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
            signing_cert: std::sync::OnceLock::from(cert),
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
            signing_cert: std::sync::OnceLock::new(),
        })
    }

    /// The CA certificate used as the signing issuer for leaves, built once and
    /// cached. Its own serial/validity are irrelevant to leaf validation —
    /// clients trust the on-disk CA by its key + DN — so a params shape without
    /// them matches the previous per-leaf behavior, just computed a single time.
    fn signing_cert(&self) -> Result<&rcgen::Certificate> {
        if let Some(cert) = self.signing_cert.get() {
            return Ok(cert);
        }

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
        let cert = ca_params.self_signed(&self.key_pair)?;
        Ok(self.signing_cert.get_or_init(|| cert))
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

        let ca_cert = self.signing_cert()?;
        let leaf_cert = leaf_params.signed_by(&leaf_key, ca_cert, &self.key_pair)?;
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
    crate::state::home_dir().context("neither HOME nor USERPROFILE environment variable is set")
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
        let _guard = HOME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
        let _guard = HOME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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

    /// The reported failure, as a test: an EXPIRED certificate whose file looks
    /// freshly modified must still be replaced.
    ///
    /// This is the shape a restored backup or a copy onto a new Mac produces —
    /// `cp` and Time Machine both stamp a new mtime on a year-old certificate.
    /// The old check read that mtime, concluded the CA was current, loaded it,
    /// and every proxied request then failed with ERR_CERT_DATE_INVALID while
    /// Lumen reported itself healthy.
    #[test]
    fn test_regenerates_expired_cert_even_with_fresh_mtime() {
        let _guard = HOME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", dir.path());

        let ca1 = LumenCA::load_or_generate().expect("first generate");

        // An expired CA on disk, written now — so its mtime is as fresh as it gets.
        let ca_dir = dir.path().join(".lumen");
        let expired = expired_ca_pem();
        std::fs::write(ca_dir.join("ca.pem"), &expired).expect("write expired cert");

        let ca2 = LumenCA::load_or_generate().expect("regen");

        match original_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }

        assert_ne!(ca2.cert_pem, expired, "an expired CA must not be loaded");
        assert_ne!(ca1.cert_pem, ca2.cert_pem, "a new CA should have been issued");
    }

    #[test]
    fn test_expires_within_reads_the_certificate() {
        let ca = LumenCA::generate().expect("generate");

        // Freshly minted: a decade out, so well outside the rotation window.
        assert_eq!(ca.expires_within(std::time::Duration::from_secs(30 * 86_400)), Some(false));

        // ...but inside a window that brackets its notAfter.
        assert_eq!(
            ca.expires_within(std::time::Duration::from_secs(3700 * 86_400)),
            Some(true)
        );
    }

    #[test]
    fn test_expires_within_is_none_for_unparseable_pem() {
        // Deliberately not `Some(true)`: a parser failure must never be the
        // reason a working CA is thrown away along with the user's trust.
        let mut ca = LumenCA::generate().expect("generate");
        ca.cert_pem =
            "-----BEGIN CERTIFICATE-----\nnot a certificate\n-----END CERTIFICATE-----\n"
                .to_string();

        assert_eq!(ca.expires_within(std::time::Duration::from_secs(86_400)), None);
    }

    /// A syntactically valid CA whose validity window closed in 2021.
    fn expired_ca_pem() -> String {
        use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};

        let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("key");
        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, "Lumen Local CA");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        params.not_after = rcgen::date_time_ymd(2021, 1, 1);

        params.self_signed(&key_pair).expect("self sign").pem()
    }
}
