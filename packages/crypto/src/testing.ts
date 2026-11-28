import { installRandom } from "./driver";

/** Returns a disposable, deterministic random source for headless tests. */
export function mockCryptoRandom(deterministicBytes: Uint8Array): { restore(): void } {
  const source = new Uint8Array(deterministicBytes);
  let position = 0;
  return { restore: installRandom(length => {
    if (length > source.length - position) throw new Error("The test random source is exhausted");
    const bytes = source.slice(position, position + length);
    position += length;
    return bytes;
  }) };
}
