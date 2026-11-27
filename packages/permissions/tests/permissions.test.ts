import { assert, test } from "impasto:test";
import { Permission, checkStatuses, openAppSettings, openSystemSettings, presentLimitedLibraryPicker, request } from "../src/index";
import { mockPermissions } from "../src/testing";

test("permission rationale, settings and retry use the public API in a headless host", async () => {
  const mock = mockPermissions({ location: { serviceStatus: "enabled" } });
  try {
    mock.respondWith("camera", "deny", "doNotAskAgain");
    let denied = false;
    assert.equal(await Permission.camera.request({ onDenied() { denied = true; } }), "denied");
    assert.equal(denied, true);
    assert.equal(await Permission.camera.shouldShowRequestRationale, true);
    assert.equal(await Permission.camera.request(), "permanentlyDenied");
    assert.equal(await openAppSettings(), true);
    assert.equal(mock.settingsOpened.length, 1);
    mock.configure("camera", { status: "granted" });
    assert.equal(await Permission.camera.isGranted, true);
    assert.equal(await Permission.location.isServiceEnabled, true);
  } finally {
    mock.restore();
  }
});

test("partial media, expiring grants and alias results stay observable", async () => {
  const mock = mockPermissions();
  try {
    mock.respondWith("photos", "selectPhotos");
    mock.respondWith("calendarFullAccess", "grantOnce");
    const result = await request([Permission.photos, Permission.calendar, Permission.calendarFullAccess]);
    assert.equal(result.photos, "limited");
    assert.equal(result.calendar, "granted");
    assert.equal(result.calendarFullAccess, "granted");
    assert.equal(await presentLimitedLibraryPicker(), true);
    assert.equal(await openSystemSettings(Permission.photos), true);
    assert.equal(mock.settingsOpened[0], "photos");
    mock.expireOneTimeGrants();
    const states = await checkStatuses([Permission.calendar, Permission.photos]);
    assert.equal(states.calendar, "denied");
    assert.equal(states.photos, "limited");
  } finally {
    mock.restore();
  }
});
