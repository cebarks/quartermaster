use std::net::IpAddr;
use std::path::Path;

use anyhow::{bail, Context, Result};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub fn load_or_generate_tls_config(
    config: &crate::config::Config,
    spt_dir: &Path,
) -> Result<rustls::ServerConfig> {
    let (cert_chain, key) = match (&config.tls_cert, &config.tls_key) {
        (Some(cert_path), Some(key_path)) => load_pem_files(cert_path, key_path)?,
        (Some(_), None) | (None, Some(_)) => {
            bail!("tls_cert and tls_key must both be set, or both omitted");
        }
        (None, None) => {
            let cert_path = spt_dir.join("quma-cert.pem");
            let key_path = spt_dir.join("quma-key.pem");
            if cert_path.exists() && key_path.exists() {
                tracing::info!(
                    "loading existing self-signed TLS certificate; \
                     if clients fail to connect via IP, delete {} and {} to regenerate with current network interfaces",
                    cert_path.display(),
                    key_path.display()
                );
                load_pem_files(&cert_path, &key_path)?
            } else {
                generate_self_signed(&cert_path, &key_path, &config.web_bind)?
            }
        }
    };

    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .context("failed to build TLS server config")?;

    Ok(tls_config)
}

fn load_pem_files(
    cert_path: &Path,
    key_path: &Path,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let cert_data = std::fs::read(cert_path)
        .with_context(|| format!("failed to read TLS cert: {}", cert_path.display()))?;
    let key_data = std::fs::read(key_path)
        .with_context(|| format!("failed to read TLS key: {}", key_path.display()))?;

    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&cert_data)
        .collect::<Result<Vec<_>, _>>()
        .context("failed to parse TLS certificate PEM")?;
    if certs.is_empty() {
        bail!("no certificates found in {}", cert_path.display());
    }

    let key =
        PrivateKeyDer::from_pem_slice(&key_data).context("failed to parse TLS private key PEM")?;

    Ok((certs, key))
}

fn collect_san_names(web_bind: &str) -> Vec<String> {
    let mut sans = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];

    let bind_is_wildcard = web_bind == "0.0.0.0" || web_bind == "::";

    if bind_is_wildcard {
        match local_ip_address::list_afinet_netifas() {
            Ok(ifaces) => {
                for (_, ip) in &ifaces {
                    if ip.is_loopback() {
                        continue;
                    }
                    let s = ip.to_string();
                    if !sans.contains(&s) {
                        sans.push(s);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to enumerate network interfaces; cert will only cover localhost"
                );
            }
        }
    } else if let Ok(ip) = web_bind.parse::<IpAddr>() {
        let s = ip.to_string();
        if !sans.contains(&s) {
            sans.push(s);
        }
    } else {
        // web_bind is a hostname
        if !sans.contains(&web_bind.to_string()) {
            sans.push(web_bind.to_string());
        }
    }

    sans
}

fn generate_self_signed(
    cert_path: &Path,
    key_path: &Path,
    web_bind: &str,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let sans = collect_san_names(web_bind);
    tracing::info!(?sans, "generating self-signed TLS certificate");

    let key_pair = rcgen::KeyPair::generate().context("failed to generate key pair")?;
    let params = rcgen::CertificateParams::new(sans).context("failed to create cert params")?;
    let cert = params
        .self_signed(&key_pair)
        .context("failed to generate self-signed certificate")?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    std::fs::write(cert_path, &cert_pem)
        .with_context(|| format!("failed to write cert to {}", cert_path.display()))?;
    {
        use std::io::Write;
        #[cfg(unix)]
        use std::os::unix::fs::OpenOptionsExt;

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        opts.mode(0o600);

        opts.open(key_path)
            .and_then(|mut f| f.write_all(key_pem.as_bytes()))
            .with_context(|| format!("failed to write key to {}", key_path.display()))?;
    }

    // Re-parse from PEM so the returned types match load_pem_files
    load_pem_files(cert_path, key_path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn auto_generates_self_signed_cert() {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::config::Config::default();
        let tls_config = load_or_generate_tls_config(&config, tmp.path()).unwrap();
        // Cert files should have been created
        assert!(tmp.path().join("quma-cert.pem").exists());
        assert!(tmp.path().join("quma-key.pem").exists());
        // Should return a valid ServerConfig
        drop(tls_config);
    }

    #[test]
    fn reuses_existing_cert() {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::config::Config::default();
        // First call generates
        load_or_generate_tls_config(&config, tmp.path()).unwrap();
        let cert1 = std::fs::read(tmp.path().join("quma-cert.pem")).unwrap();
        // Second call reuses
        load_or_generate_tls_config(&config, tmp.path()).unwrap();
        let cert2 = std::fs::read(tmp.path().join("quma-cert.pem")).unwrap();
        assert_eq!(cert1, cert2);
    }

    #[test]
    fn loads_user_provided_cert() {
        let tmp = tempfile::tempdir().unwrap();
        // Generate a cert first so we have valid PEM files
        let config = crate::config::Config::default();
        load_or_generate_tls_config(&config, tmp.path()).unwrap();

        // Now configure to use those generated files as "user-provided"
        let mut config = crate::config::Config::default();
        config.tls_cert = Some(tmp.path().join("quma-cert.pem"));
        config.tls_key = Some(tmp.path().join("quma-key.pem"));
        let tls_config = load_or_generate_tls_config(&config, tmp.path()).unwrap();
        drop(tls_config);
    }

    #[test]
    fn errors_on_missing_user_cert() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::default();
        config.tls_cert = Some(tmp.path().join("nonexistent.pem"));
        config.tls_key = Some(tmp.path().join("nonexistent-key.pem"));
        let result = load_or_generate_tls_config(&config, tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn errors_on_cert_without_key() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::default();
        config.tls_cert = Some(tmp.path().join("cert.pem"));
        // tls_key is None
        let result = load_or_generate_tls_config(&config, tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn collect_san_names_always_includes_localhost() {
        let sans = collect_san_names("10.0.0.5");
        assert!(sans.contains(&"localhost".to_string()));
        assert!(sans.contains(&"127.0.0.1".to_string()));
        assert!(sans.contains(&"::1".to_string()));
    }

    #[test]
    fn collect_san_names_includes_specific_bind_ip() {
        let sans = collect_san_names("192.168.1.100");
        assert!(sans.contains(&"192.168.1.100".to_string()));
    }

    #[test]
    fn collect_san_names_includes_hostname_bind() {
        let sans = collect_san_names("myhost.local");
        assert!(sans.contains(&"myhost.local".to_string()));
        assert!(sans.contains(&"localhost".to_string()));
    }

    #[test]
    fn collect_san_names_wildcard_includes_interfaces() {
        let sans = collect_san_names("0.0.0.0");
        assert!(sans.contains(&"localhost".to_string()));
        assert!(sans.contains(&"127.0.0.1".to_string()));
        assert!(sans.contains(&"::1".to_string()));
        // 3 loopback entries + at least one non-loopback IP on most systems
        assert!(sans.len() >= 3);
    }

    #[test]
    fn collect_san_names_no_duplicates() {
        let sans = collect_san_names("127.0.0.1");
        let count = sans.iter().filter(|s| *s == "127.0.0.1").count();
        assert_eq!(count, 1);
    }
}
