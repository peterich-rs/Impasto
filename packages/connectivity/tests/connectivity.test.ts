import { assert, test } from "impasto:test";
import { getStatus, watchStatus } from "../src/index";
import { mockConnectivity } from "../src/testing";

test("network watchers receive the current state and the next change", async () => {
  const mock = mockConnectivity();
  try {
    const reader = watchStatus().getReader();
    assert.equal((await reader.read()).value?.types[0], "none");
    mock.emit({ types: ["wifi"], isConnected: true, isExpensive: false, isConstrained: true });
    assert.equal((await reader.read()).value?.isConstrained, true);
    assert.equal((await getStatus()).types[0], "wifi");
    await reader.cancel();
  } finally { mock.restore(); }
});
