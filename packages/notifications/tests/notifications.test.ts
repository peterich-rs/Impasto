import { assert, test } from "impasto:test";
import { cancel, getInitialResponse, getPending, schedule, watchResponses } from "../src/index";
import { mockNotifications } from "../src/testing";
import type { NotificationResponse } from "../src/types";

test("notifications consumes launch actions once and streams subsequent actions", async () => {
  const initial: NotificationResponse = { notificationId: "invite", actionId: "default", data: {}, receivedAtMs: 20 };
  const mock = mockNotifications({ initialResponse: initial });
  const responses = watchResponses().getReader();
  try {
    assert.deepEqual(await getInitialResponse(), initial);
    assert.equal(await getInitialResponse(), null);
    const trigger = { kind: "interval" as const, everySeconds: 120 };
    await schedule({ id: "invite", title: "Invite", body: "A new invite" }, trigger);
    assert.equal((await getPending())[0]?.id, "invite");
    await cancel("invite");
    assert.deepEqual(await getPending(), []);
    const action = { ...initial, actionId: "reply", userText: "Yes" };
    mock.simulateNotificationResponse(action);
    assert.deepEqual((await responses.read()).value, action);
  } finally {
    await responses.cancel();
    mock.restore();
  }
});
