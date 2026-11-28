use std::sync::Arc;

use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use rustls_platform_verifier::Verifier;

use crate::{
    HttpError,
    options::{Pins, error},
};

pub(super) const MISMATCH: &str = "HTTP certificate pin mismatch";

#[derive(Debug)]
struct PinVerifier {
    system: Verifier,
    pins: Arc<Pins>,
}

pub(super) fn connector(pins: Arc<Pins>) -> Result<openwire_rustls::RustlsTlsConnector, HttpError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let system = Verifier::new(Arc::clone(&provider))
        .map_err(|_| error("network", "System TLS trust is unavailable"))?;
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| error("network", "TLS protocol configuration is unavailable"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinVerifier { system, pins }))
        .with_no_client_auth();
    Ok(openwire_rustls::RustlsTlsConnector::from_config(config))
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        leaf: &CertificateDer<'_>,
        chain: &[CertificateDer<'_>],
        server: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let verified = self
            .system
            .verify_server_cert(leaf, chain, server, ocsp, now)?;
        if !self.pins.verify(
            &server.to_str(),
            std::iter::once(leaf.as_ref()).chain(chain.iter().map(AsRef::as_ref)),
        ) {
            return Err(rustls::Error::General(MISMATCH.to_owned()));
        }
        Ok(verified)
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.system.verify_tls12_signature(message, cert, signature)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.system.verify_tls13_signature(message, cert, signature)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.system.supported_verify_schemes()
    }
}
