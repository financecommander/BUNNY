use std::sync::Arc;
use std::time::Duration;

use crate::error::{NetworkError, Result};

/// Generate an ephemeral self-signed certificate for QUIC transport.
///
/// Actual transport-layer security is provided by bunny-crypto's hybrid
/// post-quantum key exchange — the TLS layer is only needed to satisfy
/// QUIC's mandatory encryption requirement.
fn generate_self_signed() -> Result<(rustls::pki_types::CertificateDer<'static>, rustls::pki_types::PrivateKeyDer<'static>)> {
    let cert = rcgen::generate_simple_self_signed(vec!["bunny".into()])
        .map_err(|e| NetworkError::Connection(format!("cert generation: {e}")))?;
    let cert_der = rustls::pki_types::CertificateDer::from(cert.cert);
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(cert.key_pair.serialize_der())
        .map_err(|e| NetworkError::Connection(format!("key serialization: {e}")))?;
    Ok((cert_der, key_der))
}

/// Create QUIC server config with ephemeral self-signed cert.
pub fn server_config(
    idle_timeout: Duration,
    keep_alive: Duration,
) -> Result<quinn::ServerConfig> {
    let (cert, key) = generate_self_signed()?;

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| NetworkError::Connection(format!("tls server config: {e}")))?;

    server_crypto.alpn_protocols = vec![b"bunny/1".to_vec()];

    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(
        idle_timeout
            .try_into()
            .map_err(|e| NetworkError::Connection(format!("idle timeout: {e}")))?,
    ));
    transport.keep_alive_interval(Some(keep_alive));

    let mut server_config =
        quinn::ServerConfig::with_crypto(Arc::new(quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
            .map_err(|e| NetworkError::Connection(format!("quic server config: {e}")))?));
    server_config.transport_config(Arc::new(transport));

    Ok(server_config)
}

/// Create QUIC client config that accepts any certificate.
///
/// Certificate verification is not needed — authentication is handled
/// by bunny-crypto's signed identity handshake with hybrid PQ verification.
pub fn client_config() -> Result<quinn::ClientConfig> {
    let mut client_crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipVerification))
        .with_no_client_auth();

    client_crypto.alpn_protocols = vec![b"bunny/1".to_vec()];

    Ok(quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
            .map_err(|e| NetworkError::Connection(format!("quic client config: {e}")))?,
    )))
}

/// Certificate verifier that accepts all certificates.
///
/// This is safe because:
/// 1. QUIC still provides encryption (preventing passive eavesdropping)
/// 2. Authentication happens at the bunny-crypto layer via signed announcements
///    with hybrid PQ (Ed25519 + ML-DSA-65) verification
/// 3. Session keys are derived from hybrid PQ key exchange (X25519 + ML-KEM-768)
#[derive(Debug)]
struct SkipVerification;

impl rustls::client::danger::ServerCertVerifier for SkipVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}
