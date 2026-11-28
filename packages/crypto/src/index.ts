import type { DirectoryScope } from "@impasto/file-system";
import { cryptoDriver as native, readRandom } from "./driver";
import type {
  AeadDecryptOptions, AeadEncryptOptions, AeadEncryptResult, Argon2Options, BinaryInput,
  HashAlgorithm, HkdfOptions, KeyExchangeAlgorithm, KeyPair, KeyPairAlgorithm, Pbkdf2Options, SigningAlgorithm,
} from "./types";
export type * from "./types";

type Sha = Exclude<HashAlgorithm, "blake3">;
type NativeBytes = Uint8Array | ArrayBuffer;

function binary(input: BinaryInput): Uint8Array {
  if (typeof input === "string") return new TextEncoder().encode(input);
  return input instanceof ArrayBuffer ? new Uint8Array(input)
    : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
}
function optional(input: BinaryInput | undefined) { return input === undefined ? undefined : binary(input); }
function bytes(input: NativeBytes): Uint8Array { return input instanceof ArrayBuffer ? new Uint8Array(input) : input; }

export const digestSync = (algorithm: HashAlgorithm, data: BinaryInput): Uint8Array =>
  bytes(native.digestSync(algorithm, binary(data)));
export async function digest(algorithm: HashAlgorithm, data: BinaryInput): Promise<Uint8Array> {
  return bytes(await native.digest(algorithm, binary(data)));
}
export async function hashFile(path: string, options?: { readonly scope?: DirectoryScope; readonly algorithm?: HashAlgorithm }) {
  return bytes(await native.hashFile(path, options));
}
export const constantTimeEqual = (a: Uint8Array, b: Uint8Array) => native.constantTimeEqual(a, b);
export async function hmac(algorithm: Sha, key: Uint8Array, data: BinaryInput) {
  return bytes(await native.hmac(algorithm, key, binary(data)));
}
export const verifyHmac = (algorithm: Sha, key: Uint8Array, data: BinaryInput, mac: Uint8Array) =>
  native.verifyHmac(algorithm, key, binary(data), mac);

export async function encryptAead(options: AeadEncryptOptions): Promise<AeadEncryptResult> {
  const nonce = options.nonce ?? readRandom(options.algorithm === "xchacha20-poly1305" ? 24 : 12);
  const result = await native.encryptAead({ ...options, nonce, plaintext: binary(options.plaintext), aad: optional(options.aad) });
  return { ciphertext: bytes(result.ciphertext), nonce: bytes(result.nonce), tag: bytes(result.tag), combined: bytes(result.combined) };
}
export async function decryptAead(options: AeadDecryptOptions) {
  return bytes(await native.decryptAead({ ...options, aad: optional(options.aad) }));
}
export async function hkdf(options: HkdfOptions) {
  return bytes(await native.hkdf({ ...options, info: optional(options.info) }));
}
export async function pbkdf2(options: Pbkdf2Options) {
  return bytes(await native.pbkdf2({ ...options, password: binary(options.password) }));
}
export async function argon2id(options: Argon2Options) {
  return bytes(await native.argon2id({ ...options, password: binary(options.password) }));
}
export async function generateKeyPair(algorithm: KeyPairAlgorithm): Promise<KeyPair> {
  const pair = await native.generateKeyPair(algorithm);
  return { ...pair, publicKey: bytes(pair.publicKey), privateKey: bytes(pair.privateKey) };
}
export async function sign(algorithm: SigningAlgorithm, privateKey: Uint8Array, data: BinaryInput) {
  return bytes(await native.sign(algorithm, privateKey, binary(data)));
}
export const verify = (algorithm: SigningAlgorithm, publicKey: Uint8Array, data: BinaryInput, signature: Uint8Array) =>
  native.verify(algorithm, publicKey, binary(data), signature);
export async function deriveSharedSecret(algorithm: KeyExchangeAlgorithm, privateKey: Uint8Array, peerPublicKey: Uint8Array) {
  return bytes(await native.deriveSharedSecret(algorithm, privateKey, peerPublicKey));
}
export const randomBytes = readRandom;
export const encodeBase64 = (data: Uint8Array, options?: { readonly urlSafe?: boolean; readonly padding?: boolean }) =>
  native.encodeBase64(data, options);
export const decodeBase64 = (text: string, options?: { readonly urlSafe?: boolean }) => bytes(native.decodeBase64(text, options?.urlSafe));
export const encodeHex = (data: Uint8Array) => native.encodeHex(data);
export const decodeHex = (text: string) => bytes(native.decodeHex(text));
