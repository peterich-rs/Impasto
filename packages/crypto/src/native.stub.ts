export function isCryptoHostAvailable() { return false; }
export function randomBytes(length: number) {
  const bytes = new Uint8Array(length);
  for (let index = 0; index < bytes.length; index += 65536) {
    crypto.getRandomValues(bytes.subarray(index, index + 65536));
  }
  return bytes;
}

function unavailable(..._values: unknown[]): never {
  throw new Error("[crypto:host-unavailable] Impasto native cryptography is unavailable");
}
export {
  unavailable as digestSync, unavailable as digest, unavailable as hashFile, unavailable as constantTimeEqual,
  unavailable as hmac, unavailable as verifyHmac, unavailable as encryptAead, unavailable as decryptAead,
  unavailable as hkdf, unavailable as pbkdf2, unavailable as argon2id, unavailable as generateKeyPair,
  unavailable as sign, unavailable as verify, unavailable as deriveSharedSecret, unavailable as encodeBase64,
  unavailable as decodeBase64, unavailable as encodeHex, unavailable as decodeHex,
};
