import { assert, test } from "impasto:test";
import { authenticate, getCapabilities } from "../src/index";
import { mockBiometrics } from "../src/testing";

test("biometrics supports recoverable prompt outcomes in headless tests", async () => {
  const mock = mockBiometrics({ nextResult: { authenticated: false, reason: "lockout", message: "Try again later" } });
  try {
    assert.equal((await getCapabilities()).hasDeviceCredential, true);
    const options = { reason: "Unlock your account", allowDeviceCredential: true };
    assert.equal((await authenticate(options)).authenticated, false);
    mock.respondWith({ authenticated: true });
    assert.equal((await authenticate(options)).authenticated, true);
    assert.equal(mock.calls.at(-1)?.allowDeviceCredential, true);
  } finally { mock.restore(); }
});
