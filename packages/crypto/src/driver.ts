import * as native from "@impasto/crypto/native";

let testRandom: ((length: number) => Uint8Array) | undefined;
export const cryptoDriver = native;

export function readRandom(length: number): Uint8Array {
  if (!Number.isInteger(length) || length < 0 || length > 1048576) {
    throw new RangeError("[crypto:invalid-input] Invalid random byte count");
  }
  return testRandom?.(length) ?? new Uint8Array(native.randomBytes(length));
}
export function installRandom(source: (length: number) => Uint8Array): () => void {
  if (native.isCryptoHostAvailable() || testRandom) {
    throw new Error("[crypto:host-unavailable] A random source override requires an unbound test host");
  }
  testRandom = source;
  return () => { if (testRandom === source) testRandom = undefined; };
}
