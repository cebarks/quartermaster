use std::path::Path;

use anyhow::{bail, Context, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

// TODO(task-5): Remove this allow once the proxy handler uses this function
#[allow(dead_code)]
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
                tracing::info!("loading existing self-signed TLS certificate");
                load_pem_files(&cert_path, &key_path)?
            } else {
                tracing::info!("generating self-signed TLS certificate");
                generate_self_signed(&cert_path, &key_path)?
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

    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut &cert_data[..])
        .collect::<Result<Vec<_>, _>>()
        .context("failed to parse TLS certificate PEM")?;
    if certs.is_empty() {
        bail!("no certificates found in {}", cert_path.display());
    }

    let key = rustls_pemfile::private_key(&mut &key_data[..])
        .context("failed to parse TLS private key PEM")?
        .context("no private key found in PEM file")?;

    Ok((certs, key))
}

fn generate_self_signed(
    cert_path: &Path,
    key_path: &Path,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let key_pair = rcgen::KeyPair::generate().context("failed to generate key pair")?;
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()])
        .context("failed to create cert params")?;
    let cert = params
        .self_signed(&key_pair)
        .context("failed to generate self-signed certificate")?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    std::fs::write(cert_path, &cert_pem)
        .with_context(|| format!("failed to write cert to {}", cert_path.display()))?;
    std::fs::write(key_path, &key_pem)
        .with_context(|| format!("failed to write key to {}", key_path.display()))?;

    // Re-parse from PEM so the returned types match load_pem_files
    load_pem_files(cert_path, key_path)
}

#[cfg(test)]
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
}
