import { assert, test } from "impasto:test";
import { Clipboard, availableFormats, clear, hasHtml, hasImages, hasStrings, read, watchChanges, write } from "../src/index";
import { mockClipboard } from "../src/testing";

test("preserves synchronous text alongside the rich clipboard API", async () => {
  const mock = mockClipboard();
  try {
    Clipboard.setString("copied");
    assert.equal(Clipboard.getString(), "copied");
    await write({ text: "hello", html: "<strong>hello</strong>", uri: "https://example.com", imagePng: new Uint8Array([1, 2]) });
    assert.deepEqual(await availableFormats(), ["text", "html", "uri", "image"]);
    assert.equal(await hasStrings(), true);
    assert.equal(await hasHtml(), true);
    assert.equal(await hasImages(), true);
    assert.deepEqual(Array.from((await read()).imagePng ?? []), [1, 2]);
    await clear();
    assert.equal(Clipboard.getString(), "");
    assert.deepEqual(await availableFormats(), []);
  } finally {
    mock.restore();
  }
});

test("reports consecutive clipboard changes with the same formats", async () => {
  const mock = mockClipboard();
  const reader = watchChanges().getReader();
  try {
    mock.simulateExternalCopy({ text: "first" });
    mock.simulateExternalCopy({ text: "second" });
    assert.deepEqual((await reader.read()).value, ["text"]);
    assert.deepEqual((await reader.read()).value, ["text"]);
    assert.equal(Clipboard.getString(), "second");
  } finally {
    await reader.cancel();
    mock.restore();
  }
});
