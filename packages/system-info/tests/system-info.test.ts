import { assert, test } from "impasto:test";
import { getDeviceInfo, getRuntimeInfo } from "../src/index";
import { mockSystemInfo } from "../src/testing";

test("a test can supply platform and power information independently", async () => {
  const mock = mockSystemInfo({ device: { platform: "android", sdkInt: 35 }, runtime: { freeDiskBytes: 123 } });
  try {
    assert.equal((await getDeviceInfo()).platform, "android");
    assert.equal((await getRuntimeInfo()).freeDiskBytes, 123);
    mock.setRuntime({ isLowPowerMode: true });
    assert.equal((await getRuntimeInfo()).isLowPowerMode, true);
  } finally { mock.restore(); }
});
