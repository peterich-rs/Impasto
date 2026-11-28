export type HashAlgorithm = "sha256" | "sha384" | "sha512" | "blake3";
export type AeadAlgorithm = "aes-256-gcm" | "chacha20-poly1305" | "xchacha20-poly1305";
export type KeyPairAlgorithm = "ed25519" | "p256" | "x25519" | "rsa-2048" | "rsa-3072";
export type SigningAlgorithm = "ed25519" | "ecdsa-p256-sha256" | "rsa-pss-sha256";
export type KeyExchangeAlgorithm = "x25519" | "ecdh-p256";

export type CryptoErrorCode =
  | "invalid-key"
  | "invalid-nonce"
  | "invalid-input"
  | "auth-failed"
  | "unsupported-algorithm"
  | "io"
  | "host-unavailable";

export type BinaryInput = Uint8Array | ArrayBuffer | ArrayBufferView | string;

export interface AeadEncryptOptions {
  readonly algorithm: AeadAlgorithm;
  /** AES-256-GCM / ChaCha20-Poly1305 须 32 字节 */
  readonly key: Uint8Array;
  readonly plaintext: BinaryInput;
  /** 缺省时自动生成 CSPRNG Nonce（GCM/ChaCha20 为 12 字节，XChaCha20 为 24 字节） */
  readonly nonce?: Uint8Array;
  readonly aad?: BinaryInput;
}

export interface AeadEncryptResult {
  readonly ciphertext: Uint8Array;
  readonly nonce: Uint8Array;
  /** 16 字节认证标签，同时提供拼好尾部的 combined 便于存储 */
  readonly tag: Uint8Array;
  readonly combined: Uint8Array;
}

export interface AeadDecryptOptions {
  readonly algorithm: AeadAlgorithm;
  readonly key: Uint8Array;
  readonly ciphertext: Uint8Array;
  readonly nonce: Uint8Array;
  readonly tag: Uint8Array;
  readonly aad?: BinaryInput;
}

export interface HkdfOptions {
  readonly hash?: "sha256" | "sha384" | "sha512";
  readonly ikm: Uint8Array;
  readonly salt?: Uint8Array;
  readonly info?: BinaryInput;
  readonly length: number;
}

export interface Pbkdf2Options {
  readonly hash?: "sha256" | "sha512";
  readonly password: BinaryInput;
  readonly salt: Uint8Array;
  readonly iterations: number;
  readonly length: number;
}

export interface Argon2Options {
  readonly password: BinaryInput;
  readonly salt: Uint8Array;
  /** 内存开销 KiB，默认 19456（19 MiB，OWASP 基线） */
  readonly memoryKiB?: number;
  /** 迭代次数，默认 2 */
  readonly iterations?: number;
  /** 并行度，默认 1 */
  readonly parallelism?: number;
  /** 输出密钥长度，默认 32 */
  readonly length?: number;
}

export interface KeyPair {
  readonly algorithm: KeyPairAlgorithm;
  readonly format: "der" | "raw";
  /** PKCS#8 DER / X25519/Ed25519 raw 编码，明确标注 format */
  readonly publicKey: Uint8Array;
  readonly privateKey: Uint8Array;
}
