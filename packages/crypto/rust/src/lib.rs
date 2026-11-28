//! Native cryptographic primitives with scoped file access.

mod asymmetric;
mod symmetric;

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use impasto::{Bytes, ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar};
use impasto_file_system::{Scope, resolve_path};
use sha2::{Digest as _, Sha256, Sha384, Sha512};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use asymmetric::{Exchange, KeyAlgorithm, KeyPair, SignatureAlgorithm};
use symmetric::{
    AeadCiphertext, ArgonOptions, Decryption, Encryption, HkdfOptions, PbkdfOptions, Sha,
};

#[derive(Clone, Copy, Debug, ImpastoType)]
pub enum HashAlgorithm {
    Sha256,
    Sha384,
    Sha512,
    Blake3,
}

#[derive(Debug)]
pub struct CryptoError {
    code: &'static str,
    message: &'static str,
}

impl CryptoError {
    const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
    fn invalid() -> Self {
        Self::new("invalid-input", "Invalid cryptographic input")
    }
    fn key() -> Self {
        Self::new("invalid-key", "Invalid cryptographic key")
    }
    fn auth() -> Self {
        Self::new("auth-failed", "Authentication failed")
    }
    fn io() -> Self {
        Self::new(
            "io",
            "Cryptographic operation could not access its resource",
        )
    }
}

impl ImpastoError for CryptoError {
    fn scope(&self) -> &'static str {
        "crypto"
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> String {
        self.message.to_owned()
    }
}

/// Computes a SHA-256 checksum.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Computes a SHA-256 message authentication code.
pub fn hmac_sha256(key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, CryptoError> {
    symmetric::hmac(Sha::Sha256, key, bytes)
}

enum Hash {
    Sha256(Sha256),
    Sha384(Sha384),
    Sha512(Sha512),
    Blake3(Box<blake3::Hasher>),
}

impl Hash {
    fn new(algorithm: HashAlgorithm) -> Self {
        match algorithm {
            HashAlgorithm::Sha256 => Self::Sha256(Sha256::new()),
            HashAlgorithm::Sha384 => Self::Sha384(Sha384::new()),
            HashAlgorithm::Sha512 => Self::Sha512(Sha512::new()),
            HashAlgorithm::Blake3 => Self::Blake3(Box::new(blake3::Hasher::new())),
        }
    }
    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha256(hash) => hash.update(bytes),
            Self::Sha384(hash) => hash.update(bytes),
            Self::Sha512(hash) => hash.update(bytes),
            Self::Blake3(hash) => {
                hash.update(bytes);
            }
        }
    }
    fn finish(self) -> Bytes {
        Bytes(match self {
            Self::Sha256(hash) => hash.finalize().to_vec(),
            Self::Sha384(hash) => hash.finalize().to_vec(),
            Self::Sha512(hash) => hash.finalize().to_vec(),
            Self::Blake3(hash) => hash.finalize().as_bytes().to_vec(),
        })
    }
}

/// Hashes a disk file incrementally. Resolve untrusted paths in the app scope first.
pub fn hash_file(path: &Path, algorithm: HashAlgorithm) -> Result<Bytes, CryptoError> {
    let file = File::open(path).map_err(|_| CryptoError::io())?;
    if !file.metadata().map_err(|_| CryptoError::io())?.is_file() {
        return Err(CryptoError::io());
    }
    let mut input = BufReader::new(file);
    let mut state = Hash::new(algorithm);
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|_| CryptoError::io())?;
        if count == 0 {
            return Ok(state.finish());
        }
        state.update(&buffer[..count]);
    }
}

fn random(length: usize) -> Result<Vec<u8>, CryptoError> {
    if length > 1 << 20 {
        return Err(CryptoError::invalid());
    }
    let mut bytes = vec![0; length];
    getrandom::fill(&mut bytes).map_err(|_| CryptoError::io())?;
    Ok(bytes)
}

#[derive(ImpastoType)]
struct HashFileOptions {
    scope: Option<Scope>,
    algorithm: Option<HashAlgorithm>,
}

#[derive(ImpastoType)]
struct Base64Options {
    url_safe: Option<bool>,
    padding: Option<bool>,
}

#[impasto::module(name = "Crypto", specifier = "@impasto/crypto/native", init = init)]
struct Crypto(PluginContext);

impl Crypto {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self(ctx.clone()))
    }
}

#[impasto::export]
impl Crypto {
    pub fn is_crypto_host_available(&self) -> bool {
        self.0.is_hosted()
    }

    pub fn digest_sync(&self, algorithm: HashAlgorithm, data: Bytes) -> Bytes {
        let mut hash = Hash::new(algorithm);
        hash.update(&data.0);
        hash.finish()
    }

    #[impasto::method(blocking)]
    pub fn digest(&self, algorithm: HashAlgorithm, data: Bytes) -> Bytes {
        self.digest_sync(algorithm, data)
    }

    #[impasto::method(blocking)]
    pub fn hash_file(
        &self,
        path: String,
        options: Option<HashFileOptions>,
    ) -> Result<Bytes, CryptoError> {
        let options = options.unwrap_or(HashFileOptions {
            scope: None,
            algorithm: None,
        });
        let file = resolve_path(&self.0, options.scope.unwrap_or_default(), &path)
            .map_err(|_| CryptoError::io())?;
        hash_file(&file, options.algorithm.unwrap_or(HashAlgorithm::Sha256))
    }

    pub fn constant_time_equal(&self, a: Bytes, b: Bytes) -> bool {
        bool::from(a.0.ct_eq(&b.0))
    }

    #[impasto::method(blocking)]
    pub fn hmac(&self, algorithm: Sha, key: Bytes, data: Bytes) -> Result<Bytes, CryptoError> {
        symmetric::hmac(algorithm, &Zeroizing::new(key.0), &data.0).map(Bytes)
    }

    #[impasto::method(blocking)]
    pub fn verify_hmac(
        &self,
        algorithm: Sha,
        key: Bytes,
        data: Bytes,
        mac: Bytes,
    ) -> Result<bool, CryptoError> {
        let computed = Zeroizing::new(symmetric::hmac(algorithm, &Zeroizing::new(key.0), &data.0)?);
        Ok(bool::from(computed.ct_eq(&mac.0)))
    }

    #[impasto::method(blocking)]
    pub fn encrypt_aead(&self, options: Encryption) -> Result<AeadCiphertext, CryptoError> {
        symmetric::encrypt(options)
    }

    #[impasto::method(blocking)]
    pub fn decrypt_aead(&self, options: Decryption) -> Result<Bytes, CryptoError> {
        symmetric::decrypt(options)
    }

    #[impasto::method(blocking)]
    pub fn hkdf(&self, options: HkdfOptions) -> Result<Bytes, CryptoError> {
        symmetric::hkdf(options)
    }

    #[impasto::method(blocking)]
    pub fn pbkdf2(&self, options: PbkdfOptions) -> Result<Bytes, CryptoError> {
        symmetric::pbkdf2(options)
    }

    #[impasto::method(blocking)]
    pub fn argon2id(&self, options: ArgonOptions) -> Result<Bytes, CryptoError> {
        symmetric::argon2(options)
    }

    #[impasto::method(blocking)]
    pub fn generate_key_pair(&self, algorithm: KeyAlgorithm) -> Result<KeyPair, CryptoError> {
        asymmetric::generate(algorithm)
    }

    #[impasto::method(blocking)]
    pub fn sign(
        &self,
        algorithm: SignatureAlgorithm,
        private_key: Bytes,
        data: Bytes,
    ) -> Result<Bytes, CryptoError> {
        asymmetric::sign(algorithm, &Zeroizing::new(private_key.0), &data.0)
    }

    #[impasto::method(blocking)]
    pub fn verify(
        &self,
        algorithm: SignatureAlgorithm,
        public_key: Bytes,
        data: Bytes,
        signature: Bytes,
    ) -> Result<bool, CryptoError> {
        asymmetric::verify(algorithm, &public_key.0, &data.0, &signature.0)
    }

    #[impasto::method(blocking)]
    pub fn derive_shared_secret(
        &self,
        algorithm: Exchange,
        private_key: Bytes,
        peer_public_key: Bytes,
    ) -> Result<Bytes, CryptoError> {
        asymmetric::derive(
            algorithm,
            &Zeroizing::new(private_key.0),
            &peer_public_key.0,
        )
    }

    pub fn random_bytes(&self, length: u32) -> Result<Bytes, CryptoError> {
        random(length as usize).map(Bytes)
    }

    pub fn encode_base64(&self, data: Bytes, options: Option<Base64Options>) -> String {
        let options = options.unwrap_or(Base64Options {
            url_safe: None,
            padding: None,
        });
        match (
            options.url_safe.unwrap_or(false),
            options.padding.unwrap_or(true),
        ) {
            (false, true) => STANDARD.encode(data.0),
            (false, false) => STANDARD_NO_PAD.encode(data.0),
            (true, true) => URL_SAFE.encode(data.0),
            (true, false) => URL_SAFE_NO_PAD.encode(data.0),
        }
    }

    pub fn decode_base64(
        &self,
        text: String,
        url_safe: Option<bool>,
    ) -> Result<Bytes, CryptoError> {
        let engine = match (url_safe.unwrap_or(false), text.ends_with('=')) {
            (false, true) => STANDARD,
            (false, false) => STANDARD_NO_PAD,
            (true, true) => URL_SAFE,
            (true, false) => URL_SAFE_NO_PAD,
        };
        engine
            .decode(text)
            .map(Bytes)
            .map_err(|_| CryptoError::invalid())
    }

    pub fn encode_hex(&self, data: Bytes) -> String {
        hex::encode(data.0)
    }

    pub fn decode_hex(&self, text: String) -> Result<Bytes, CryptoError> {
        hex::decode(text)
            .map(Bytes)
            .map_err(|_| CryptoError::invalid())
    }
}

/// Registers the native cryptography bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(Crypto::plugin());
}

#[cfg(test)]
mod tests {
    #[test]
    fn sha256_and_hmac_match_published_vectors() {
        assert_eq!(
            hex::encode(super::sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex::encode(super::hmac_sha256(&[0x0b; 20], b"Hi There").unwrap()),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }
}
