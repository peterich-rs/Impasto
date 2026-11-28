use aes_gcm::aead::{AeadInPlace, KeyInit};
use hmac::{Hmac, Mac};
use impasto::{Bytes, ImpastoType};
use sha2::{Sha256, Sha384, Sha512};
use zeroize::Zeroizing;

use crate::{CryptoError, random};

#[derive(Clone, Copy, ImpastoType)]
pub(crate) enum Sha {
    Sha256,
    Sha384,
    Sha512,
}

#[derive(Clone, Copy, ImpastoType)]
pub(crate) enum Aead {
    #[impasto(rename = "aes-256-gcm")]
    Aes256Gcm,
    #[impasto(rename = "chacha20-poly1305")]
    ChaCha20Poly1305,
    #[impasto(rename = "xchacha20-poly1305")]
    XChaCha20Poly1305,
}

impl Aead {
    fn nonce_length(self) -> usize {
        if matches!(self, Self::XChaCha20Poly1305) {
            24
        } else {
            12
        }
    }
}

#[derive(ImpastoType)]
pub(crate) struct Encryption {
    algorithm: Aead,
    key: Bytes,
    plaintext: Bytes,
    nonce: Option<Bytes>,
    aad: Option<Bytes>,
}

#[derive(ImpastoType)]
pub(crate) struct Decryption {
    algorithm: Aead,
    key: Bytes,
    ciphertext: Bytes,
    nonce: Bytes,
    tag: Bytes,
    aad: Option<Bytes>,
}

#[derive(ImpastoType)]
pub(crate) struct AeadCiphertext {
    ciphertext: Bytes,
    nonce: Bytes,
    tag: Bytes,
    combined: Bytes,
}

pub(crate) fn hmac(algorithm: Sha, key: &[u8], data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    macro_rules! mac {
        ($sha:ty) => {{
            let mut mac =
                <Hmac<$sha> as Mac>::new_from_slice(key).map_err(|_| CryptoError::key())?;
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        }};
    }
    Ok(match algorithm {
        Sha::Sha256 => mac!(Sha256),
        Sha::Sha384 => mac!(Sha384),
        Sha::Sha512 => mac!(Sha512),
    })
}

fn nonce_error() -> CryptoError {
    CryptoError::new("invalid-nonce", "Invalid nonce length")
}

pub(crate) fn encrypt(options: Encryption) -> Result<AeadCiphertext, CryptoError> {
    let key = Zeroizing::new(options.key.0);
    let nonce = match options.nonce {
        Some(nonce) => nonce.0,
        None => random(options.algorithm.nonce_length())?,
    };
    if nonce.len() != options.algorithm.nonce_length() {
        return Err(nonce_error());
    }
    let mut bytes = Zeroizing::new(options.plaintext.0);
    let aad = options
        .aad
        .as_ref()
        .map_or(&[][..], |data| data.0.as_slice());
    macro_rules! encrypt {
        ($cipher:ty, $length:expr) => {{
            let nonce = <[u8; $length]>::try_from(nonce.as_slice()).map_err(|_| nonce_error())?;
            <$cipher>::new_from_slice(&key)
                .map_err(|_| CryptoError::key())?
                .encrypt_in_place_detached(&nonce.into(), aad, &mut bytes)
                .map_err(|_| CryptoError::invalid())?
        }};
    }
    let tag = match options.algorithm {
        Aead::Aes256Gcm => encrypt!(aes_gcm::Aes256Gcm, 12),
        Aead::ChaCha20Poly1305 => encrypt!(chacha20poly1305::ChaCha20Poly1305, 12),
        Aead::XChaCha20Poly1305 => encrypt!(chacha20poly1305::XChaCha20Poly1305, 24),
    };
    let mut combined = bytes.to_vec();
    combined.extend_from_slice(&tag);
    Ok(AeadCiphertext {
        ciphertext: Bytes(bytes.to_vec()),
        nonce: Bytes(nonce),
        tag: Bytes(tag.to_vec()),
        combined: Bytes(combined),
    })
}

pub(crate) fn decrypt(options: Decryption) -> Result<Bytes, CryptoError> {
    let key = Zeroizing::new(options.key.0);
    if options.nonce.0.len() != options.algorithm.nonce_length() {
        return Err(nonce_error());
    }
    let tag = <[u8; 16]>::try_from(options.tag.0.as_slice()).map_err(|_| CryptoError::auth())?;
    let mut bytes = Zeroizing::new(options.ciphertext.0);
    let aad = options
        .aad
        .as_ref()
        .map_or(&[][..], |data| data.0.as_slice());
    macro_rules! decrypt {
        ($cipher:ty, $length:expr) => {{
            let nonce =
                <[u8; $length]>::try_from(options.nonce.0.as_slice()).map_err(|_| nonce_error())?;
            <$cipher>::new_from_slice(&key)
                .map_err(|_| CryptoError::key())?
                .decrypt_in_place_detached(&nonce.into(), aad, &mut bytes, &tag.into())
                .map_err(|_| CryptoError::auth())?
        }};
    }
    match options.algorithm {
        Aead::Aes256Gcm => decrypt!(aes_gcm::Aes256Gcm, 12),
        Aead::ChaCha20Poly1305 => decrypt!(chacha20poly1305::ChaCha20Poly1305, 12),
        Aead::XChaCha20Poly1305 => decrypt!(chacha20poly1305::XChaCha20Poly1305, 24),
    }
    Ok(Bytes(bytes.to_vec()))
}

#[derive(ImpastoType)]
pub(crate) struct HkdfOptions {
    hash: Option<Sha>,
    ikm: Bytes,
    salt: Option<Bytes>,
    info: Option<Bytes>,
    length: u32,
}

pub(crate) fn hkdf(options: HkdfOptions) -> Result<Bytes, CryptoError> {
    let ikm = Zeroizing::new(options.ikm.0);
    let salt = options.salt.as_ref().map(|data| data.0.as_slice());
    let info = options
        .info
        .as_ref()
        .map_or(&[][..], |data| data.0.as_slice());
    let mut bytes = output(options.length)?;
    match options.hash.unwrap_or(Sha::Sha256) {
        Sha::Sha256 => hkdf::Hkdf::<Sha256>::new(salt, &ikm).expand(info, &mut bytes),
        Sha::Sha384 => hkdf::Hkdf::<Sha384>::new(salt, &ikm).expand(info, &mut bytes),
        Sha::Sha512 => hkdf::Hkdf::<Sha512>::new(salt, &ikm).expand(info, &mut bytes),
    }
    .map_err(|_| CryptoError::invalid())?;
    Ok(Bytes(bytes.to_vec()))
}

#[derive(ImpastoType)]
pub(crate) struct PbkdfOptions {
    hash: Option<Sha>,
    password: Bytes,
    salt: Bytes,
    iterations: u32,
    length: u32,
}

pub(crate) fn pbkdf2(options: PbkdfOptions) -> Result<Bytes, CryptoError> {
    let password = Zeroizing::new(options.password.0);
    if !(1..=10_000_000).contains(&options.iterations) || options.length > 1024 {
        return Err(CryptoError::invalid());
    }
    let mut bytes = output(options.length)?;
    let salt = &options.salt.0;
    match options.hash.unwrap_or(Sha::Sha256) {
        Sha::Sha256 => {
            pbkdf2::pbkdf2_hmac::<Sha256>(&password, salt, options.iterations, &mut bytes)
        }
        Sha::Sha512 => {
            pbkdf2::pbkdf2_hmac::<Sha512>(&password, salt, options.iterations, &mut bytes)
        }
        Sha::Sha384 => return Err(CryptoError::invalid()),
    }
    Ok(Bytes(bytes.to_vec()))
}

#[derive(ImpastoType)]
pub(crate) struct ArgonOptions {
    password: Bytes,
    salt: Bytes,
    memory_ki_b: Option<u32>,
    iterations: Option<u32>,
    parallelism: Option<u32>,
    length: Option<u32>,
}

pub(crate) fn argon2(options: ArgonOptions) -> Result<Bytes, CryptoError> {
    let password = Zeroizing::new(options.password.0);
    let memory = options.memory_ki_b.unwrap_or(19_456);
    let time = options.iterations.unwrap_or(2);
    let lanes = options.parallelism.unwrap_or(1);
    if memory > 256 * 1024 || time > 32 || lanes > 16 {
        return Err(CryptoError::invalid());
    }
    let mut bytes = output(options.length.unwrap_or(32))?;
    let params = argon2::Params::new(memory, time, lanes, Some(bytes.len()))
        .map_err(|_| CryptoError::invalid())?;
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(&password, &options.salt.0, &mut bytes)
        .map_err(|_| CryptoError::invalid())?;
    Ok(Bytes(bytes.to_vec()))
}

fn output(length: u32) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if !(1..=16_320).contains(&length) {
        return Err(CryptoError::invalid());
    }
    Ok(Zeroizing::new(vec![0; length as usize]))
}
