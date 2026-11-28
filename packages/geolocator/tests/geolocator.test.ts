import { assert, test } from "impasto:test";
import { distanceBetween, watchPosition, watchServiceStatus } from "../src/index";
import { mockGeolocator } from "../src/testing";
import type { Position } from "../src/types";

test("geolocator streams independent location and service updates", async () => {
  const mock = mockGeolocator();
  const positions = watchPosition().getReader();
  const services = watchServiceStatus().getReader();
  try {
    assert.equal((await services.read()).value, "enabled");
    const position: Position = {
      latitude: 0, longitude: 1, accuracy: 10, altitude: null, altitudeAccuracy: null,
      heading: null, headingAccuracy: null, speed: null, speedAccuracy: null, timestampMs: 5, isMocked: true,
    };
    mock.emitPosition(position);
    assert.deepEqual((await positions.read()).value, position);
    mock.emitServiceStatus("disabled");
    assert.equal((await services.read()).value, "disabled");
    assert.equal(Math.round(distanceBetween({ latitude: 0, longitude: 0 }, position)), 111195);
  } finally {
    await positions.cancel();
    await services.cancel();
    mock.restore();
  }
});
