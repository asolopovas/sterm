use anyhow::{Context, Result};
use quinn::{
    crypto::rustls::{QuicClientConfig, QuicServerConfig},
    ClientConfig, ServerConfig, TransportConfig,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::RootCertStore;
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, sync::Arc};

use crate::config;

pub struct GeneratedServerConfig {
    pub config: ServerConfig,
    pub certificate_sha256_pin: String,
}

pub fn trusted_client_config() -> Result<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls_config.alpn_protocols = vec![config::ALPN.to_vec()];

    let mut client_config = ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls_config)?));
    client_config.transport_config(Arc::new(transport_config()));
    Ok(client_config)
}

pub fn pinned_client_config(expected_sha256: [u8; config::SHA256_LEN]) -> Result<ClientConfig> {
    let mut tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(PinnedServerCertificate::new(expected_sha256))
        .with_no_client_auth();
    tls_config.alpn_protocols = vec![config::ALPN.to_vec()];

    let mut client_config = ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls_config)?));
    client_config.transport_config(Arc::new(transport_config()));
    Ok(client_config)
}

pub fn self_signed_server_config(names: impl Into<Vec<String>>) -> Result<GeneratedServerConfig> {
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(names)?;
    let cert_der = CertificateDer::from(cert.der().to_vec());
    let certificate_sha256_pin = format_sha256_fingerprint(cert_der.as_ref());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(signing_key.serialize_der()));
    let config = server_config(vec![cert_der], key_der)?;

    Ok(GeneratedServerConfig {
        config,
        certificate_sha256_pin,
    })
}

pub fn server_config(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<ServerConfig> {
    let mut tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    tls_config.alpn_protocols = vec![config::ALPN.to_vec()];

    let mut server_config =
        ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls_config)?));
    server_config.transport_config(Arc::new(transport_config()));
    Ok(server_config)
}

pub fn transport_config() -> TransportConfig {
    let mut transport = TransportConfig::default();
    transport.keep_alive_interval(Some(config::TLS_KEEP_ALIVE_INTERVAL));
    transport.max_concurrent_bidi_streams(config::MAX_CONCURRENT_BIDI_STREAMS.into());
    transport.max_concurrent_uni_streams(config::MAX_CONCURRENT_UNI_STREAMS.into());
    transport
}

pub fn format_sha256_fingerprint(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut fingerprint = String::with_capacity(config::SHA256_HEX_LEN + config::SHA256_LEN - 1);
    for (idx, byte) in digest.iter().enumerate() {
        if idx > 0 {
            fingerprint.push(':');
        }
        let _ = write!(&mut fingerprint, "{byte:02x}");
    }
    fingerprint
}

pub fn parse_sha256_fingerprint(value: &str) -> Result<[u8; config::SHA256_LEN]> {
    let compact: String = value
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | ' '))
        .collect();
    anyhow::ensure!(
        compact.len() == config::SHA256_HEX_LEN,
        "SHA-256 fingerprint must be 64 hex characters"
    );

    let mut out = [0u8; config::SHA256_LEN];
    for (idx, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&compact[idx * 2..idx * 2 + 2], 16)
            .with_context(|| format!("invalid hex at byte {idx}"))?;
    }
    Ok(out)
}

pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[derive(Debug)]
struct PinnedServerCertificate {
    expected_sha256: [u8; config::SHA256_LEN],
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl PinnedServerCertificate {
    fn new(expected_sha256: [u8; config::SHA256_LEN]) -> Arc<Self> {
        Arc::new(Self {
            expected_sha256,
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        })
    }
}

impl rustls::client::danger::ServerCertVerifier for PinnedServerCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let got: [u8; config::SHA256_LEN] = Sha256::digest(end_entity.as_ref()).into();
        if got == self.expected_sha256 {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "server certificate SHA-256 fingerprint mismatch".to_owned(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_configs_are_constructible() {
        install_crypto_provider();

        assert!(trusted_client_config().is_ok());
        assert!(pinned_client_config([0u8; config::SHA256_LEN]).is_ok());
    }

    #[test]
    fn self_signed_server_config_returns_fingerprint() {
        install_crypto_provider();

        let generated =
            self_signed_server_config(vec![config::DEFAULT_DIRECT_SERVER_NAME.to_string()])
                .unwrap();

        assert!(!generated.certificate_sha256_pin.is_empty());
    }

    #[test]
    fn fingerprint_format_and_parse_round_trip() {
        let bytes = [1u8; config::SHA256_LEN];
        let formatted = format_sha256_fingerprint(&bytes);
        let parsed = parse_sha256_fingerprint(&formatted).unwrap();

        let expected: [u8; config::SHA256_LEN] = Sha256::digest(bytes).into();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn fingerprint_parse_accepts_compact_hex() {
        let bytes = [1u8; config::SHA256_LEN];

        assert_eq!(
            parse_sha256_fingerprint(&"01".repeat(config::SHA256_LEN)).unwrap(),
            bytes
        );
    }

    #[test]
    fn fingerprint_parse_accepts_dash_separators() {
        let bytes = [1u8; config::SHA256_LEN];

        assert_eq!(
            parse_sha256_fingerprint(&"01-".repeat(config::SHA256_LEN)).unwrap(),
            bytes
        );
    }

    #[test]
    fn fingerprint_parse_rejects_bad_length() {
        assert!(parse_sha256_fingerprint("abc").is_err());
    }

    #[test]
    fn fingerprint_parse_rejects_bad_hex() {
        let value = format!("{}zz", "01".repeat(config::SHA256_LEN - 1));

        assert!(parse_sha256_fingerprint(&value).is_err());
    }
}
