import { assert, test } from "impasto:test";
import { shareFiles, shareText } from "../src/index";
import { mockShare } from "../src/testing";

test("text and scoped attachments reach the same share driver", async () => {
  const mock = mockShare();
  try {
    assert.equal((await shareText("hello", { subject: "subject" })).status, "success");
    assert.equal(mock.calls[0]?.subject, "subject");
    mock.respondWith({ status: "dismissed" });
    assert.equal((await shareFiles([{ path: "notes.txt", scope: "cache" }])).status, "dismissed");
    assert.equal(mock.calls[1]?.files?.[0]?.mimeType, undefined);
  } finally { mock.restore(); }
});
