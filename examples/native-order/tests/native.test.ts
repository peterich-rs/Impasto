import { assert, test } from "impasto:test";
import { openStore } from "@app/order";

test("opens and queries the application's linked Rust resource", async () => {
  const store = await openStore();
  try {
    assert.deepEqual(await store.query(), [{ id: "order-1", totalCents: 2599 }]);
  } finally {
    store.close();
  }
});
