import { advanceFrames, assert, test } from "impasto:test";
import { createForm } from "./index.ts";

test("src index", async () => {
  let asyncRuns = 0;
  const form = createForm({
    initial: { name: "", city: "" },
    validate: (values) => ({
      name: values.name.length < 2 ? "short" : null,
      city: values.city.length === 0 ? "required" : null,
    }),
    validateAsync: async (values) => {
      asyncRuns += 1;
      return { name: values.name === "abcd" ? null : "stale" };
    },
    debounceMs: 30,
  });

  form.setField("name", "a");
  form.setField("name", "ab");
  form.setField("name", "abcd");
  await advanceFrames();
  assert.equal(form.errors().name, undefined);
  await advanceFrames(6);
  await advanceFrames();
  assert.equal(asyncRuns, 1);
  assert.equal(form.errors().name, undefined);

  form.setField("name", "");
  await advanceFrames();
  let focused = "";
  const accepted = form.handleSubmit({
    name: {
      focus() {
        focused = "name";
      },
    },
    city: {
      focus() {
        focused = "city";
      },
    },
  });
  assert.equal(accepted, false);
  assert.equal(focused, "name");
});
