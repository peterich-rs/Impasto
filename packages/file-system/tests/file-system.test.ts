import { assert, test } from "impasto:test";
import { FileSystem } from "../src/index";
import { mockFileSystem } from "../src/testing";

test("scoped files and the atomic writer use distinct sandbox directories", async () => {
  const dirs = FileSystem.directories();
  assert.equal(new Set([dirs.documents, dirs.applicationSupport, dirs.cache, dirs.temp]).size, 4);
  await FileSystem.writeText("test.txt", "original", { scope: "cache", atomic: true });
  assert.equal(await FileSystem.exists("test.txt"), false);
  const writer = await FileSystem.openWriter("test.txt", { scope: "cache", atomic: true });
  await writer.write("pending");
  await writer.abort();
  assert.equal(await FileSystem.readText("test.txt", "cache"), "original");
  await FileSystem.copy("test.txt", "copy.txt", { fromScope: "cache", toScope: "temp" });
  const reader = FileSystem.readStream("copy.txt", { scope: "temp", chunkSize: 3 }).getReader();
  const first = await reader.read();
  assert.equal(first.value?.byteLength, 3);
  await reader.cancel();
  assert.equal((await FileSystem.listDir(undefined, "temp"))[0]?.name, "copy.txt");
});

test("dialog mocks preserve the legacy relative path and support cancellation", async () => {
  const mock = mockFileSystem({ files: [{ name: "x", relativePath: "Imports/test/x", size: 1, mimeType: null }] });
  try {
    assert.equal((await FileSystem.pick())?.name, "Imports/test/x");
    mock.setFiles([]);
    assert.equal(await FileSystem.pick(), null);
  } finally { mock.restore(); }
});
