use anyhow::{Context, Result};
use quinn::ServerConfig;
use rterm_protocol::config;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::{io::BufReader, path::PathBuf};

pub fn server_config(
    cert_path: Option<PathBuf>,
    key_path: Option<PathBuf>,
) -> Result<ServerConfig> {
    if let (Some(cert_path), Some(key_path)) = (cert_path, key_path) {
        let (certs, key) = load_pem_identity(&cert_path, &key_path)?;
        if let Some(first_cert) = certs.first() {
            rterm_protocol::write_public_pin(
                config::RELAY_CERT_PIN_LABEL,
                &rterm_protocol::format_sha256_fingerprint(first_cert.as_ref()),
            )?;
        }
        rterm_protocol::server_config(certs, key)
    } else {
        let generated = rterm_protocol::self_signed_server_config(vec![
            config::DEFAULT_DIRECT_SERVER_NAME.to_string(),
        ])?;
        rterm_protocol::write_public_pin(
            config::RELAY_CERT_PIN_LABEL,
            &generated.certificate_sha256_pin,
        )?;
        Ok(generated.config)
    }
}

fn load_pem_identity(
    cert_path: &PathBuf,
    key_path: &PathBuf,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let cert_file = std::fs::File::open(cert_path)
        .with_context(|| format!("open certificate chain {}", cert_path.display()))?;
    let certs = rustls_pemfile::certs(&mut BufReader::new(cert_file))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if certs.is_empty() {
        anyhow::bail!("no certificates found in {}", cert_path.display());
    }

    let key_file = std::fs::File::open(key_path)
        .with_context(|| format!("open private key {}", key_path.display()))?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(key_file))?
        .with_context(|| format!("no private key found in {}", key_path.display()))?;

    Ok((certs, key))
}
