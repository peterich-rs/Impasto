import { assert, test } from "impasto:test";
import { cancel, impact, isSupported, vibratePattern } from "../src/index";
import { mockHaptics } from "../src/testing";

test("haptic interactions can be observed with no platform hardware", async () => {
  const mock = mockHaptics();
  try {
    assert.equal(await isSupported(), true);
    await impact("soft");
    await vibratePattern([0, 15], [0, 100]);
    await cancel();
    assert.equal(mock.events.map(({ operation }) => operation).join(","), "impact,vibratePattern,cancel");
  } finally { mock.restore(); }
});
