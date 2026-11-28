use ed25519_dalek::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use ed25519_dalek::{Signature, Signer as _, Verifier as _};
use impasto::{Bytes, ImpastoType};
use rand_core::OsRng;
use rsa::signature::{RandomizedSigner as _, SignatureEncoding as _};
use sha2::Sha256;

use crate::CryptoError;

#[derive(Clone, Copy, ImpastoType)]
pub(crate) enum KeyAlgorithm {
    Ed25519,
    P256,
    X25519,
    #[impasto(rename = "rsa-2048")]
    Rsa2048,
    #[impasto(rename = "rsa-3072")]
    Rsa3072,
}
#[derive(Clone, Copy, ImpastoType)]
pub(crate) enum SignatureAlgorithm {
    Ed25519,
    #[impasto(rename = "ecdsa-p256-sha256")]
    P256,
    #[impasto(rename = "rsa-pss-sha256")]
    Rsa,
}
#[derive(Clone, Copy, ImpastoType)]
pub(crate) enum Exchange {
    X25519,
    #[impasto(rename = "ecdh-p256")]
    P256,
}
#[derive(ImpastoType)]
pub(crate) struct KeyPair {
    algorithm: KeyAlgorithm,
    public_key: Bytes,
    private_key: Bytes,
    format: KeyFormat,
}
#[derive(ImpastoType)]
enum KeyFormat {
    Der,
    Raw,
}

pub(crate) fn generate(algorithm: KeyAlgorithm) -> Result<KeyPair, CryptoError> {
    fn encode(
        secret: impl EncodePrivateKey,
        public: impl EncodePublicKey,
    ) -> Result<(Vec<u8>, Vec<u8>, KeyFormat), CryptoError> {
        let secret = secret
            .to_pkcs8_der()
            .map_err(|_| CryptoError::key())?
            .as_bytes()
            .to_vec();
        let public = public
            .to_public_key_der()
            .map_err(|_| CryptoError::key())?
            .as_bytes()
            .to_vec();
        Ok((secret, public, KeyFormat::Der))
    }
    let (secret, public, format) = match algorithm {
        KeyAlgorithm::Ed25519 => {
            let secret = ed25519_dalek::SigningKey::generate(&mut OsRng);
            let public = secret.verifying_key();
            encode(secret, public)?
        }
        KeyAlgorithm::P256 => {
            let secret = p256::SecretKey::random(&mut OsRng);
            let public = secret.public_key();
            encode(secret, public)?
        }
        KeyAlgorithm::X25519 => {
            let secret = x25519_dalek::StaticSecret::random_from_rng(OsRng);
            let public = x25519_dalek::PublicKey::from(&secret);
            (
                secret.as_bytes().to_vec(),
                public.as_bytes().to_vec(),
                KeyFormat::Raw,
            )
        }
        KeyAlgorithm::Rsa2048 | KeyAlgorithm::Rsa3072 => {
            let bits = if matches!(algorithm, KeyAlgorithm::Rsa2048) {
                2048
            } else {
                3072
            };
            let secret =
                rsa::RsaPrivateKey::new(&mut OsRng, bits).map_err(|_| CryptoError::key())?;
            let public = rsa::RsaPublicKey::from(&secret);
            encode(secret, public)?
        }
    };
    Ok(KeyPair {
        algorithm,
        private_key: Bytes(secret),
        public_key: Bytes(public),
        format,
    })
}

pub(crate) fn sign(
    algorithm: SignatureAlgorithm,
    key: &[u8],
    data: &[u8],
) -> Result<Bytes, CryptoError> {
    Ok(Bytes(match algorithm {
        SignatureAlgorithm::Ed25519 => ed25519_dalek::SigningKey::from_pkcs8_der(key)
            .map_err(|_| CryptoError::key())?
            .sign(data)
            .to_bytes()
            .to_vec(),
        SignatureAlgorithm::P256 => {
            let signer =
                p256::ecdsa::SigningKey::from_pkcs8_der(key).map_err(|_| CryptoError::key())?;
            let signature: p256::ecdsa::Signature = signer.sign(data);
            signature.to_bytes().to_vec()
        }
        SignatureAlgorithm::Rsa => {
            let key = rsa::RsaPrivateKey::from_pkcs8_der(key).map_err(|_| CryptoError::key())?;
            rsa::pss::BlindedSigningKey::<Sha256>::new(key)
                .try_sign_with_rng(&mut OsRng, data)
                .map_err(|_| CryptoError::key())?
                .to_vec()
        }
    }))
}

pub(crate) fn verify(
    algorithm: SignatureAlgorithm,
    key: &[u8],
    data: &[u8],
    signature: &[u8],
) -> Result<bool, CryptoError> {
    Ok(match algorithm {
        SignatureAlgorithm::Ed25519 => {
            let key = ed25519_dalek::VerifyingKey::from_public_key_der(key)
                .map_err(|_| CryptoError::key())?;
            Signature::from_slice(signature)
                .is_ok_and(|signature| key.verify_strict(data, &signature).is_ok())
        }
        SignatureAlgorithm::P256 => {
            let key = p256::ecdsa::VerifyingKey::from_public_key_der(key)
                .map_err(|_| CryptoError::key())?;
            p256::ecdsa::Signature::from_slice(signature)
                .is_ok_and(|signature| key.verify(data, &signature).is_ok())
        }
        SignatureAlgorithm::Rsa => {
            let key =
                rsa::RsaPublicKey::from_public_key_der(key).map_err(|_| CryptoError::key())?;
            let verifier = rsa::pss::VerifyingKey::<Sha256>::new(key);
            rsa::pss::Signature::try_from(signature)
                .is_ok_and(|signature| verifier.verify(data, &signature).is_ok())
        }
    })
}

pub(crate) fn derive(
    algorithm: Exchange,
    secret: &[u8],
    peer: &[u8],
) -> Result<Bytes, CryptoError> {
    Ok(Bytes(match algorithm {
        Exchange::X25519 => {
            let secret = x25519_dalek::StaticSecret::from(
                <[u8; 32]>::try_from(secret).map_err(|_| CryptoError::key())?,
            );
            let peer = x25519_dalek::PublicKey::from(
                <[u8; 32]>::try_from(peer).map_err(|_| CryptoError::key())?,
            );
            let shared = secret.diffie_hellman(&peer);
            if !shared.was_contributory() {
                return Err(CryptoError::key());
            }
            shared.as_bytes().to_vec()
        }
        Exchange::P256 => {
            let secret = p256::SecretKey::from_pkcs8_der(secret).map_err(|_| CryptoError::key())?;
            let peer =
                p256::PublicKey::from_public_key_der(peer).map_err(|_| CryptoError::key())?;
            p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), peer.as_affine())
                .raw_secret_bytes()
                .to_vec()
        }
    }))
}
