import { assert, test } from "impasto:test";
import { capturePhoto, pickImage, pickMedia } from "../src/index";
import { mockImagePicker } from "../src/testing";
import type { PickedMedia } from "../src/types";

test("image picker handles multiple selections and a cancelled picker", async () => {
  const image: PickedMedia = {
    kind: "image", relativePath: "image-picker/one.png", scope: "cache", fileName: "one.png",
    mimeType: "image/png", size: 68, width: 1, height: 1, durationMs: null,
  };
  const mock = mockImagePicker([image, { ...image, relativePath: "image-picker/two.png" }]);
  try {
    assert.deepEqual(await pickMedia({ multiple: true, maxSelection: 1 }), [image]);
    assert.deepEqual(await capturePhoto(), image);
    mock.setFiles([]);
    assert.equal(await pickImage(), null);
  } finally { mock.restore(); }
});
