import { assert, test } from "impasto:test";
import { decryptAead, digest, encodeHex, encryptAead } from "../src/index";
import { mockCryptoRandom } from "../src/testing";

test("hashes binary views and uses the injected nonce only in a test host", async () => {
  const data = new Uint8Array([0, 97, 98, 99, 0]);
  assert.equal(encodeHex(await digest("sha256", new DataView(data.buffer, 1, 3))),
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  const random = mockCryptoRandom(new Uint8Array(12));
  try {
    const key = new Uint8Array(32);
    const encrypted = await encryptAead({ algorithm: "aes-256-gcm", key, plaintext: new Uint8Array(16) });
    assert.equal(encodeHex(encrypted.combined), "cea7403d4d606b6e074ec5d3baf39d18d0d1c8a799996bf0265b98b5d48ab919");
    assert.equal((await decryptAead({ ...encrypted, algorithm: "aes-256-gcm", key })).byteLength, 16);
  } finally { random.restore(); }
});
